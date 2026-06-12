//! Port of `synth.core.Voice`.

use std::collections::HashMap;

use crate::components::envelope::{Envelope, Stage};
use crate::components::filters::resonant_low_pass_filter::ResonantLowPassFilter;
use crate::components::oscillators::oscillator::Oscillator;
use crate::components::oscillators::saw_oscillator::SawOscillator;
use crate::components::oscillators::sine_oscillator::SineOscillator;
use crate::components::oscillators::square_oscillator::SquareOscillator;
use crate::components::oscillators::triangle_oscillator::TriangleOscillator;
use crate::core::audio_component::AudioComponent;
use crate::core::synthesiser::Waveform;
use crate::utils::lookup_tables::{self, LookupTables};

/// Represents a single voice in the synthesiser, encapsulating all audio
/// components required to generate a sound, including an oscillator, filter,
/// and envelopes. This struct acts as a facade to simplify control over its
/// internal components.
///
/// Design note: the Java class keeps four pre-built oscillator objects and a
/// `oscillator` reference that is re-pointed when the waveform changes. In
/// Rust the four oscillators are owned fields and `current_waveform` selects
/// which one is dispatched to, preserving per-oscillator phase exactly as the
/// Java does.
pub struct Voice {
    // Audio Component Objects
    current_waveform: Waveform,
    sine: SineOscillator,
    saw: SawOscillator,
    triangle: TriangleOscillator,
    square: SquareOscillator,

    filter: ResonantLowPassFilter,
    amp_envelope: Envelope,
    filter_envelope: Envelope,

    // Oscillator Settings
    pitch_midi: u8,
    pitch_frequency: f64,

    // Filter settings
    filter_cutoff: f64,
    filter_resonance: f64,
    filter_mod_range: f64,

    // Gain Staging
    velocity_mult: f64,
    pre_filter_mult: f64,
    post_filter_mult: f64,

    // Panning
    /// Stored for parity with the Java field; not read in the render path yet.
    #[allow(dead_code)]
    pan_depth: f64,
    #[allow(dead_code)]
    pan_position: f64,
    left_gain: f64,
    right_gain: f64,

    // Cached lookup tables (resolved once; no LazyLock access in the audio path)
    tables: &'static LookupTables,
    /// Pre-computed scalar mapping a pan position to a lookup-table index
    /// (equal-power pan law over the first quarter of the sine/cosine
    /// cycle), derived from `tables.table_size` at construction time.
    pan_index_scalar: f64,

    // Output Buffers
    oscillator_output_buffer: Vec<f64>,
    filter_output_buffer: Vec<f64>,
    filter_envelope_output_buffer: Vec<f64>,
    amp_envelope_output_buffer: Vec<f64>,

    // Trackers
    note_on_time: u64,
}

impl Voice {
    /// Constructs a new Voice with the specified waveform, pitch, sample rate,
    /// and block size.
    ///
    /// * `waveform` - the oscillator waveform.
    /// * `pitch_frequency` - the initial pitch frequency of the oscillator.
    ///   Must not be negative.
    /// * `sample_rate` - the audio sample rate. Must be positive.
    ///
    /// # Panics
    /// Panics if `pitch_frequency` is negative or `sample_rate` is not positive.
    pub fn new(
        waveform: Waveform,
        pitch_frequency: f64,
        sample_rate: f64,
        block_size: usize,
    ) -> Self {
        assert!(
            pitch_frequency >= 0.0,
            "Initial pitch frequency cannot be negative."
        );
        let tables = lookup_tables::tables();
        let pan_index_scalar =
            tables.table_size as f64 / (2.0 * std::f64::consts::PI) * (std::f64::consts::PI / 4.0);

        let mut voice = Voice {
            current_waveform: Waveform::Sine,
            sine: SineOscillator::new(sample_rate),
            saw: SawOscillator::new(sample_rate),
            triangle: TriangleOscillator::new(sample_rate),
            square: SquareOscillator::new(sample_rate),
            filter: ResonantLowPassFilter::new(sample_rate),
            amp_envelope: Envelope::new(sample_rate),
            filter_envelope: Envelope::new(sample_rate),
            pitch_midi: 0,
            pitch_frequency,
            filter_cutoff: 20000.0,
            filter_resonance: 1.0,
            filter_mod_range: 2000.0,
            velocity_mult: 1.0,
            pre_filter_mult: 0.0,
            post_filter_mult: 0.0,
            pan_depth: 1.0,
            pan_position: 0.0,
            left_gain: 0.0,
            right_gain: 0.0,
            tables,
            pan_index_scalar,
            oscillator_output_buffer: vec![0.0; block_size],
            filter_output_buffer: vec![0.0; block_size],
            filter_envelope_output_buffer: vec![0.0; block_size],
            amp_envelope_output_buffer: vec![0.0; block_size],
            note_on_time: 0,
        };

        voice.set_oscillator_waveform(waveform);
        voice.set_pan_position(0.0);

        // Filter Defaults
        voice
            .filter
            .set_parameters(voice.filter_cutoff, voice.filter_resonance);

        // Set Oscillator starting pitch
        voice
            .current_oscillator_mut()
            .set_frequency(pitch_frequency);

        voice
    }

    /// Returns the currently selected oscillator (the Java `this.oscillator`
    /// reference).
    fn current_oscillator_mut(&mut self) -> &mut dyn Oscillator {
        match self.current_waveform {
            Waveform::Sine => &mut self.sine,
            Waveform::Saw => &mut self.saw,
            Waveform::Triangle => &mut self.triangle,
            Waveform::Square => &mut self.square,
        }
    }

    // Facade Setter Methods

    /// Sets the oscillator's pitch based on a MIDI note number.
    ///
    /// # Panics
    /// Panics if `pitch_midi` is greater than 127.
    pub fn set_oscillator_pitch(&mut self, pitch_midi: u8) {
        assert!(pitch_midi < 128, "MIDI pitch must be 0-127.");
        self.pitch_midi = pitch_midi;
        let frequency = self.tables.midi_to_hz[pitch_midi as usize];
        self.current_oscillator_mut().set_frequency(frequency);
    }

    /// Selects which of the four pre-built oscillators is active.
    pub fn set_oscillator_waveform(&mut self, waveform: Waveform) {
        self.current_waveform = waveform;
    }

    /// Gets the current MIDI pitch of the voice.
    pub fn pitch_midi(&self) -> u8 {
        self.pitch_midi
    }

    /// Gets the current frequency of the oscillator.
    pub fn oscillator_frequency(&self) -> f64 {
        self.pitch_frequency
    }

    /// Sets the stereo pan position.
    ///
    /// * `pan_position` - a value from -1.0 (full left) to 1.0 (full right).
    ///
    /// # Panics
    /// Panics if `pan_position` is outside `[-1.0, 1.0]`.
    pub fn set_pan_position(&mut self, pan_position: f64) {
        assert!(
            (-1.0..=1.0).contains(&pan_position),
            "Pan position must be between -1.0 and 1.0."
        );
        // Apply the Pan Law
        self.pan_position = pan_position;
        let index = ((pan_position + 1.0) * self.pan_index_scalar) as usize;
        self.left_gain = self.tables.cosine[index];
        self.right_gain = self.tables.sine[index];
    }

    /// Sets the depth of LFO pan modulation.
    pub fn set_pan_depth(&mut self, pan_depth: f64) {
        self.pan_depth = pan_depth;
    }

    // Amp Envelope:
    pub fn set_amp_envelope_attack_time(&mut self, seconds: f64) {
        self.amp_envelope.set_attack_time(seconds);
    }
    pub fn set_amp_envelope_decay_time(&mut self, seconds: f64) {
        self.amp_envelope.set_decay_time(seconds);
    }
    pub fn set_amp_envelope_sustain_level(&mut self, level: f64) {
        self.amp_envelope.set_sustain_level(level);
    }
    pub fn set_amp_envelope_release_time(&mut self, seconds: f64) {
        self.amp_envelope.set_release_time(seconds);
    }
    pub fn set_amp_envelope(
        &mut self,
        attack_time: f64,
        decay_time: f64,
        sustain_level: f64,
        release_time: f64,
    ) {
        self.amp_envelope
            .set_envelope(attack_time, decay_time, sustain_level, release_time);
    }

    // Filter Envelope:
    pub fn set_filter_envelope_attack_time(&mut self, seconds: f64) {
        self.filter_envelope.set_attack_time(seconds);
    }
    pub fn set_filter_envelope_decay_time(&mut self, seconds: f64) {
        self.filter_envelope.set_decay_time(seconds);
    }
    pub fn set_filter_envelope_sustain_level(&mut self, level: f64) {
        self.filter_envelope.set_sustain_level(level);
    }
    pub fn set_filter_envelope_release_time(&mut self, seconds: f64) {
        self.filter_envelope.set_release_time(seconds);
    }
    pub fn set_filter_envelope(
        &mut self,
        attack_time: f64,
        decay_time: f64,
        sustain_level: f64,
        release_time: f64,
    ) {
        self.filter_envelope
            .set_envelope(attack_time, decay_time, sustain_level, release_time);
    }

    /// Sets the parameters for the resonant low-pass filter.
    ///
    /// * `frequency` - the cutoff frequency in Hz.
    /// * `resonance` - the resonance (Q) factor.
    /// * `filter_mod_range` - the range of cutoff modulation from the filter
    ///   envelope, in Hz.
    pub fn set_filter_parameters(&mut self, frequency: f64, resonance: f64, filter_mod_range: f64) {
        self.filter_cutoff = frequency;
        self.filter_resonance = resonance;
        self.filter_mod_range = filter_mod_range;
        self.filter.set_parameters(frequency, resonance);
    }

    /// Sets the pre- and post-filter gain levels.
    ///
    /// * `pre_filter_gain_db` - gain before the filter, in decibels.
    /// * `post_filter_gain_db` - gain after the filter, in decibels.
    pub fn set_filter_gain_staging(&mut self, pre_filter_gain_db: f64, post_filter_gain_db: f64) {
        self.pre_filter_mult = 10.0_f64.powf(pre_filter_gain_db / 20.0);
        self.post_filter_mult = 10.0_f64.powf(post_filter_gain_db / 20.0);
    }

    /// Sets the velocity multiplier for the voice's amplitude (0.0 to 1.0).
    ///
    /// # Panics
    /// Panics if `velocity_mult` is outside `[0.0, 1.0]`.
    pub fn set_velocity(&mut self, velocity_mult: f64) {
        assert!(
            (0.0..=1.0).contains(&velocity_mult),
            "Velocity multiplier must be between 0.0 and 1.0."
        );
        self.velocity_mult = velocity_mult;
    }

    /// Triggers the note-on phase for the voice's envelopes.
    pub fn note_on(&mut self) {
        self.amp_envelope.note_on();
        self.filter_envelope.note_on();
    }

    /// Triggers the note-off phase for the voice's envelopes.
    pub fn note_off(&mut self) {
        self.amp_envelope.note_off();
        self.filter_envelope.note_off();
    }

    /// Records when the note started (used by the voice-stealing algorithm).
    pub fn set_note_on_time(&mut self, time: u64) {
        self.note_on_time = time;
    }

    /// Gets the note-on timestamp used by the voice-stealing algorithm.
    pub fn note_on_time(&self) -> u64 {
        self.note_on_time
    }

    /// Checks if the voice is currently active.
    ///
    /// Returns `true` if the amplitude envelope is not in the IDLE stage.
    pub fn is_active(&self) -> bool {
        self.amp_envelope.stage() != Stage::Idle
    }

    /// Checks if the voice is in the attack/decay/sustain stage. Useful for rendering.
    ///
    /// Returns `true` if the amplitude envelope is not in the IDLE or RELEASE stage.
    pub fn is_active_no_release(&self) -> bool {
        (self.amp_envelope.stage() != Stage::Idle) & (self.amp_envelope.stage() != Stage::Release)
    }

    /// Processes a block of audio while recording per-stage execution times.
    /// Kept as a separate, duplicated body (as in the Java original) so the
    /// uninstrumented hot path carries zero timing overhead.
    ///
    /// * `_lfo_buffer` - the LFO signal for modulation (kept for signature
    ///   parity with the Java version, which also does not consume it here).
    /// * `stereo_output_buffer` - interleaved L/R buffer of `block_size * 2`.
    /// * `timings` - map accumulating the execution time of each stage in
    ///   nanoseconds.
    pub fn process_block_instrumented(
        &mut self,
        _lfo_buffer: &[f64],
        stereo_output_buffer: &mut [f64],
        block_size: usize,
        timings: &mut HashMap<&'static str, u64>,
    ) {
        use std::time::Instant;

        // Oscillator
        let mut start = Instant::now();
        let osc: &mut dyn Oscillator = match self.current_waveform {
            Waveform::Sine => &mut self.sine,
            Waveform::Saw => &mut self.saw,
            Waveform::Triangle => &mut self.triangle,
            Waveform::Square => &mut self.square,
        };
        osc.process_block(None, &mut self.oscillator_output_buffer, block_size);
        *timings.entry("Oscillator").or_insert(0) += start.elapsed().as_nanos() as u64;

        // Filter Envelope
        start = Instant::now();
        self.filter_envelope.process_block(
            None,
            &mut self.filter_envelope_output_buffer,
            block_size,
        );
        *timings.entry("Filter Envelope").or_insert(0) += start.elapsed().as_nanos() as u64;

        // Pre-Filter Gain
        start = Instant::now();
        for sample in self.oscillator_output_buffer.iter_mut().take(block_size) {
            *sample *= self.pre_filter_mult;
        }
        *timings.entry("Pre-Filter Gain").or_insert(0) += start.elapsed().as_nanos() as u64;

        // Filter Parameter Calculation
        start = Instant::now();
        let filter_env_value = self.filter_envelope_output_buffer[0];
        let final_cutoff = self.filter_cutoff + (filter_env_value * self.filter_mod_range);
        self.filter
            .set_parameters(final_cutoff, self.filter_resonance);
        *timings.entry("Filter Params").or_insert(0) += start.elapsed().as_nanos() as u64;

        // Filtering
        start = Instant::now();
        self.filter.process_block(
            Some(&self.oscillator_output_buffer),
            &mut self.filter_output_buffer,
            block_size,
        );
        *timings.entry("Filter").or_insert(0) += start.elapsed().as_nanos() as u64;

        // Amplitude Envelope Processing
        start = Instant::now();
        self.amp_envelope.process_block(
            Some(&self.filter_output_buffer),
            &mut self.amp_envelope_output_buffer,
            block_size,
        );
        *timings.entry("Amp Envelope").or_insert(0) += start.elapsed().as_nanos() as u64;

        // Stereo Panning & Output
        start = Instant::now();
        for i in 0..block_size {
            let mono_sample =
                self.amp_envelope_output_buffer[i] * self.velocity_mult * self.post_filter_mult;
            stereo_output_buffer[i * 2] = mono_sample * self.left_gain;
            stereo_output_buffer[i * 2 + 1] = mono_sample * self.right_gain;
        }
        *timings.entry("Panning").or_insert(0) += start.elapsed().as_nanos() as u64;
    }
}

impl AudioComponent for Voice {
    /// Renders one block of this voice into an interleaved stereo buffer.
    ///
    /// * `_input_buffer` - unused; present for interface consistency.
    /// * `output_buffer` - interleaved stereo buffer of `block_size * 2` samples.
    fn process_block(
        &mut self,
        _input_buffer: Option<&[f64]>,
        output_buffer: &mut [f64],
        block_size: usize,
    ) {
        // Populate base audio component buffers
        let osc: &mut dyn Oscillator = match self.current_waveform {
            Waveform::Sine => &mut self.sine,
            Waveform::Saw => &mut self.saw,
            Waveform::Triangle => &mut self.triangle,
            Waveform::Square => &mut self.square,
        };
        osc.process_block(None, &mut self.oscillator_output_buffer, block_size);
        self.filter_envelope.process_block(
            None,
            &mut self.filter_envelope_output_buffer,
            block_size,
        );

        // Apply Pre-Filter Gain Staging:
        for sample in self.oscillator_output_buffer.iter_mut().take(block_size) {
            *sample *= self.pre_filter_mult;
        }

        // Set Filter Parameters
        let filter_env_value = self.filter_envelope_output_buffer[0];
        let final_cutoff = self.filter_cutoff + (filter_env_value * self.filter_mod_range);
        self.filter
            .set_parameters(final_cutoff, self.filter_resonance);

        // Apply Filter then Amp Env Processing
        self.filter.process_block(
            Some(&self.oscillator_output_buffer),
            &mut self.filter_output_buffer,
            block_size,
        );
        self.amp_envelope.process_block(
            Some(&self.filter_output_buffer),
            &mut self.amp_envelope_output_buffer,
            block_size,
        );

        // Conversion from Mono sample to Stereo, applies panning
        for i in 0..block_size {
            let mono_sample =
                self.amp_envelope_output_buffer[i] * self.velocity_mult * self.post_filter_mult;
            output_buffer[i * 2] = mono_sample * self.left_gain;
            output_buffer[i * 2 + 1] = mono_sample * self.right_gain;
        }
    }
}
