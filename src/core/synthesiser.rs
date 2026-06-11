//! Port of `synth.core.Synthesiser`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::time::Instant;

use crate::components::oscillators::oscillator::Oscillator;
use crate::components::oscillators::saw_oscillator::SawOscillator;
use crate::components::oscillators::sine_oscillator::SineOscillator;
use crate::components::oscillators::square_oscillator::SquareOscillator;
use crate::components::oscillators::triangle_oscillator::TriangleOscillator;
use crate::core::audio_component::AudioComponent;
use crate::core::voice::Voice;

/// The available oscillator/LFO waveforms (port of `Synthesiser.Waveform`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    Sine,
    Saw,
    Triangle,
    Square,
}

/// The main synthesiser class that manages and processes multiple voices.
/// It acts as a facade for controlling all voice parameters and generating
/// the final audio output.
///
/// Threading note: the Java original marks all master parameters `volatile`,
/// guards the voice bank with `synchronized (voices)` and defers parameter
/// propagation to the audio thread via per-group `AtomicBoolean` dirty flags.
/// This port keeps the dirty-flag structure (the audio thread still syncs
/// changed parameter groups to the voices at the start of each block) but
/// exposes a `&mut self` API; stage 2/3 should wrap the synthesiser in a lock
/// (e.g. `Mutex`) or channel to replicate the Java UI/MIDI-thread access.
pub struct Synthesiser {
    // Control all the voices.
    voices: Vec<Voice>,
    sample_rate: f64,

    // Master Configs (synth-wide settings)
    // Oscillator
    waveform: Waveform,

    // Filter
    filter_cutoff: f64,
    filter_resonance: f64,
    filter_mod_range: f64,

    // Filter Envelope
    filter_attack_time: f64,
    filter_decay_time: f64,
    filter_sustain_level: f64,
    filter_release_time: f64,

    // Amp Envelope
    amp_attack_time: f64,
    amp_decay_time: f64,
    amp_sustain_level: f64,
    amp_release_time: f64,

    // Gain Staging
    pre_filter_gain_db: f64,
    post_filter_gain_db: f64,
    voice_sum_attenuation: f64,
    volume_attenuation: f64,
    master_volume_scalar: f64,

    // LFO (four pre-built oscillators; `lfo_selected` is the Java `this.LFO` reference)
    sine_lfo: SineOscillator,
    saw_lfo: SawOscillator,
    triangle_lfo: TriangleOscillator,
    square_lfo: SquareOscillator,
    lfo_selected: Waveform,
    lfo_waveform: Waveform,
    lfo_frequency: f64,
    lfo_position: f64,

    // Panning
    pan_depth: f64,

    // Granular dirty flags: setters set per-group flag, audio thread clears after syncing to voices
    waveform_dirty: AtomicBool,
    filter_dirty: AtomicBool,
    filter_env_dirty: AtomicBool,
    amp_env_dirty: AtomicBool,
    gain_dirty: AtomicBool,
    pan_dirty: AtomicBool,

    // Output Buffers
    block_size: usize,
    voice_output_buffer: Vec<f64>,
    lfo_output_buffer: Vec<f64>,

    // Monotonic note-on clock (replaces Java's System.nanoTime(); strictly
    // increasing, so the "steal the oldest voice" comparison is identical).
    note_clock: u64,
}

impl Synthesiser {
    /// Constructs a new Synthesiser with a specified number of voices.
    ///
    /// * `no_voices` - the number of voices. Must be positive.
    /// * `sample_rate` - the audio sample rate. Must be greater than 40 Hz.
    /// * `block_size` - the number of frames per processing block.
    ///
    /// # Panics
    /// Panics if `no_voices` is zero or `sample_rate <= 40.0`.
    pub fn new(no_voices: usize, sample_rate: f64, block_size: usize) -> Self {
        assert!(no_voices > 0, "Number of voices must be positive.");
        assert!(
            sample_rate > 40.0,
            "Sample rate must be greater than 40 Hz."
        );

        let voice_sum_attenuation = 1.0 / (no_voices as f64).sqrt();

        // Populate voice bank
        let voices: Vec<Voice> = (0..no_voices)
            .map(|_| Voice::new(Waveform::Sine, 0.0, sample_rate, block_size))
            .collect();

        let mut synth = Synthesiser {
            voices,
            sample_rate,
            waveform: Waveform::Sine,
            // Initialise the filter parameters to 1 so voice.set_filter_parameters
            // doesn't panic on construction.
            filter_cutoff: 1.0,
            filter_resonance: 1.0,
            filter_mod_range: 1.0,
            filter_attack_time: 0.0,
            filter_decay_time: 0.0,
            filter_sustain_level: 0.0,
            filter_release_time: 0.0,
            amp_attack_time: 0.0,
            amp_decay_time: 0.0,
            amp_sustain_level: 0.0,
            amp_release_time: 0.0,
            pre_filter_gain_db: 0.0,
            post_filter_gain_db: 0.0,
            voice_sum_attenuation,
            volume_attenuation: voice_sum_attenuation,
            master_volume_scalar: 1.0,
            sine_lfo: SineOscillator::new(sample_rate),
            saw_lfo: SawOscillator::new(sample_rate),
            triangle_lfo: TriangleOscillator::new(sample_rate),
            square_lfo: SquareOscillator::new(sample_rate),
            lfo_selected: Waveform::Sine,
            lfo_waveform: Waveform::Sine,
            lfo_frequency: 0.0,
            lfo_position: 0.0,
            pan_depth: 0.0,
            waveform_dirty: AtomicBool::new(false),
            filter_dirty: AtomicBool::new(false),
            filter_env_dirty: AtomicBool::new(false),
            amp_env_dirty: AtomicBool::new(false),
            gain_dirty: AtomicBool::new(false),
            pan_dirty: AtomicBool::new(false),
            block_size,
            voice_output_buffer: vec![0.0; block_size * 2],
            lfo_output_buffer: vec![0.0; block_size],
            note_clock: 0,
        };

        // Default Synth Patch
        synth.load_patch(
            Waveform::Square, // Synth Waveform
            1000.0,           // filter_cutoff
            3.0,              // filter_resonance
            2000.0,           // filter_mod_range (Hz)
            0.01,             // filter_attack_time
            0.3,              // filter_decay_time
            0.5,              // filter_sustain_level
            0.1,              // filter_release_time
            0.005,            // amp_attack_time
            0.1,              // amp_decay_time
            0.4,              // amp_sustain_level
            0.4,              // amp_release_time
            -3.0,             // Pre Filter Gain (dB)
            0.0,              // Post Filter Gain (dB)
            Waveform::Sine,   // LFO Waveform
            1.0,              // LFO Frequency
            0.4,              // Pan Depth
        );

        synth
    }

    //  --- Setters ---

    /// Updates the waveform for the Low-Frequency Oscillator (LFO).
    pub fn set_lfo_waveform(&mut self, lfo_waveform: Waveform) {
        if self.lfo_waveform != lfo_waveform {
            self.lfo_waveform = lfo_waveform;
        }
    }

    /// Sets the main oscillator waveform. The change is deferred and applied
    /// to all voices by the audio thread at the start of the next processing block.
    pub fn set_oscillator_waveform(&mut self, waveform: Waveform) {
        if self.waveform != waveform {
            self.waveform = waveform;
            self.waveform_dirty.store(true, AtomicOrdering::SeqCst);
        }
    }

    /// Applies the current master settings to the voice at `index`
    /// (the Java `setVoiceParams(Voice)`).
    fn set_voice_params_at(&mut self, index: usize) {
        let waveform = self.waveform;
        let (aa, ad, asl, ar) = (
            self.amp_attack_time,
            self.amp_decay_time,
            self.amp_sustain_level,
            self.amp_release_time,
        );
        let (fa, fd, fsl, fr) = (
            self.filter_attack_time,
            self.filter_decay_time,
            self.filter_sustain_level,
            self.filter_release_time,
        );
        let (fc, fres, fmr) = (
            self.filter_cutoff,
            self.filter_resonance,
            self.filter_mod_range,
        );
        let (pre_db, post_db) = (self.pre_filter_gain_db, self.post_filter_gain_db);
        let pan_depth = self.pan_depth;

        let voice = &mut self.voices[index];
        voice.set_oscillator_waveform(waveform);
        voice.set_amp_envelope(aa, ad, asl, ar);
        voice.set_filter_envelope(fa, fd, fsl, fr);
        voice.set_filter_parameters(fc, fres, fmr);
        voice.set_filter_gain_staging(pre_db, post_db);
        voice.set_pan_depth(pan_depth);
    }

    /// Sets the filter cutoff, clamped to `[20 Hz, Nyquist)`.
    pub fn set_filter_cutoff(&mut self, cutoff: f64) {
        let nyquist_limit = (self.sample_rate / 2.0) - 1.0;
        let max_cutoff = nyquist_limit.next_down();
        let clamped = cutoff.clamp(20.0, max_cutoff);
        if self.filter_cutoff.total_cmp(&clamped) != Ordering::Equal {
            self.filter_cutoff = clamped;
            self.filter_dirty.store(true, AtomicOrdering::SeqCst);
        }
    }

    /// Sets the filter resonance (Q), clamped to `[1.0, 20.0]`.
    pub fn set_filter_resonance(&mut self, resonance: f64) {
        let clamped = resonance.clamp(1.0, 20.0);
        if self.filter_resonance.total_cmp(&clamped) != Ordering::Equal {
            self.filter_resonance = clamped;
            self.filter_dirty.store(true, AtomicOrdering::SeqCst);
        }
    }

    /// Sets the filter-envelope modulation range in Hz, clamped so that
    /// `cutoff + mod_range` stays below Nyquist.
    pub fn set_filter_mod_range(&mut self, mod_range: f64) {
        let nyquist_limit = (self.sample_rate / 2.0) - 1.0;
        let max_mod_range = (nyquist_limit.next_down() - self.filter_cutoff).max(0.0);
        let clamped = mod_range.clamp(0.0, max_mod_range);
        if self.filter_mod_range.total_cmp(&clamped) != Ordering::Equal {
            self.filter_mod_range = clamped;
            self.filter_dirty.store(true, AtomicOrdering::SeqCst);
        }
    }

    /// Sets the filter envelope attack time in seconds (clamped to >= 0).
    pub fn set_filter_attack_time(&mut self, seconds: f64) {
        let clamped = seconds.max(0.0);
        if self.filter_attack_time.total_cmp(&clamped) != Ordering::Equal {
            self.filter_attack_time = clamped;
            self.filter_env_dirty.store(true, AtomicOrdering::SeqCst);
        }
    }

    /// Sets the filter envelope decay time in seconds (clamped to >= 0).
    pub fn set_filter_decay_time(&mut self, seconds: f64) {
        let clamped = seconds.max(0.0);
        if self.filter_decay_time.total_cmp(&clamped) != Ordering::Equal {
            self.filter_decay_time = clamped;
            self.filter_env_dirty.store(true, AtomicOrdering::SeqCst);
        }
    }

    /// Sets the filter envelope sustain level (clamped to `[0.0, 1.0]`).
    pub fn set_filter_sustain_level(&mut self, level: f64) {
        let clamped = level.clamp(0.0, 1.0);
        if self.filter_sustain_level.total_cmp(&clamped) != Ordering::Equal {
            self.filter_sustain_level = clamped;
            self.filter_env_dirty.store(true, AtomicOrdering::SeqCst);
        }
    }

    /// Sets the filter envelope release time in seconds (clamped to >= 0).
    pub fn set_filter_release_time(&mut self, seconds: f64) {
        let clamped = seconds.max(0.0);
        if self.filter_release_time.total_cmp(&clamped) != Ordering::Equal {
            self.filter_release_time = clamped;
            self.filter_env_dirty.store(true, AtomicOrdering::SeqCst);
        }
    }

    /// Sets the amplitude envelope attack time in seconds (clamped to >= 0).
    pub fn set_amp_attack_time(&mut self, seconds: f64) {
        let clamped = seconds.max(0.0);
        if self.amp_attack_time.total_cmp(&clamped) != Ordering::Equal {
            self.amp_attack_time = clamped;
            self.amp_env_dirty.store(true, AtomicOrdering::SeqCst);
        }
    }

    /// Sets the amplitude envelope decay time in seconds (clamped to >= 0).
    pub fn set_amp_decay_time(&mut self, seconds: f64) {
        let clamped = seconds.max(0.0);
        if self.amp_decay_time.total_cmp(&clamped) != Ordering::Equal {
            self.amp_decay_time = clamped;
            self.amp_env_dirty.store(true, AtomicOrdering::SeqCst);
        }
    }

    /// Sets the amplitude envelope sustain level (clamped to `[0.0, 1.0]`).
    pub fn set_amp_sustain_level(&mut self, level: f64) {
        let clamped = level.clamp(0.0, 1.0);
        if self.amp_sustain_level.total_cmp(&clamped) != Ordering::Equal {
            self.amp_sustain_level = clamped;
            self.amp_env_dirty.store(true, AtomicOrdering::SeqCst);
        }
    }

    /// Sets the amplitude envelope release time in seconds (clamped to >= 0).
    pub fn set_amp_release_time(&mut self, seconds: f64) {
        let clamped = seconds.max(0.0);
        if self.amp_release_time.total_cmp(&clamped) != Ordering::Equal {
            self.amp_release_time = clamped;
            self.amp_env_dirty.store(true, AtomicOrdering::SeqCst);
        }
    }

    /// Sets the gain applied before the filter, in decibels.
    pub fn set_pre_filter_gain_db(&mut self, db: f64) {
        if self.pre_filter_gain_db.total_cmp(&db) != Ordering::Equal {
            self.pre_filter_gain_db = db;
            self.gain_dirty.store(true, AtomicOrdering::SeqCst);
        }
    }

    /// Sets the gain applied after the filter, in decibels.
    pub fn set_post_filter_gain_db(&mut self, db: f64) {
        if self.post_filter_gain_db.total_cmp(&db) != Ordering::Equal {
            self.post_filter_gain_db = db;
            self.gain_dirty.store(true, AtomicOrdering::SeqCst);
        }
    }

    /// Sets the LFO frequency in Hz (clamped to >= 0).
    pub fn set_lfo_frequency(&mut self, frequency: f64) {
        self.lfo_frequency = frequency.max(0.0);
    }

    /// Sets the depth of the stereo panning effect (clamped to `[0.0, 1.0]`).
    pub fn set_pan_depth(&mut self, depth: f64) {
        let clamped = depth.clamp(0.0, 1.0);
        if self.pan_depth.total_cmp(&clamped) != Ordering::Equal {
            self.pan_depth = clamped;
            self.pan_dirty.store(true, AtomicOrdering::SeqCst);
        }
    }

    /// Sets the master volume scalar applied on top of the per-voice
    /// `1/sqrt(no_voices)` attenuation.
    pub fn set_master_volume(&mut self, volume_scalar: f64) {
        self.master_volume_scalar = volume_scalar;
        self.volume_attenuation = self.voice_sum_attenuation * volume_scalar;
    }

    /// Applies all current patch settings to all voices. Hook for a potential
    /// future patch-loading system.
    pub fn apply_patch(&mut self) {
        self.set_oscillator_waveform(self.waveform);
        self.set_lfo_waveform(self.lfo_waveform);
        for i in 0..self.voices.len() {
            self.set_voice_params_at(i);
        }
        self.set_lfo_frequency(self.lfo_frequency);
    }

    //  --- Getters ---
    pub fn waveform(&self) -> Waveform {
        self.waveform
    }
    pub fn amp_attack_time(&self) -> f64 {
        self.amp_attack_time
    }
    pub fn amp_decay_time(&self) -> f64 {
        self.amp_decay_time
    }
    pub fn amp_sustain_level(&self) -> f64 {
        self.amp_sustain_level
    }
    pub fn amp_release_time(&self) -> f64 {
        self.amp_release_time
    }
    pub fn filter_cutoff(&self) -> f64 {
        self.filter_cutoff
    }
    pub fn filter_resonance(&self) -> f64 {
        self.filter_resonance
    }
    pub fn filter_mod_range(&self) -> f64 {
        self.filter_mod_range
    }
    pub fn filter_attack_time(&self) -> f64 {
        self.filter_attack_time
    }
    pub fn filter_decay_time(&self) -> f64 {
        self.filter_decay_time
    }
    pub fn filter_sustain_level(&self) -> f64 {
        self.filter_sustain_level
    }
    pub fn filter_release_time(&self) -> f64 {
        self.filter_release_time
    }
    pub fn pre_filter_gain_db(&self) -> f64 {
        self.pre_filter_gain_db
    }
    pub fn post_filter_gain_db(&self) -> f64 {
        self.post_filter_gain_db
    }
    pub fn lfo_waveform(&self) -> Waveform {
        self.lfo_waveform
    }
    pub fn lfo_frequency(&self) -> f64 {
        self.lfo_frequency
    }
    pub fn pan_depth(&self) -> f64 {
        self.pan_depth
    }
    pub fn master_volume_scalar(&self) -> f64 {
        self.master_volume_scalar
    }
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// Fills the provided array with the MIDI notes currently in their
    /// attack/decay/sustain phase.
    ///
    /// Returns the number of active notes written to the array.
    pub fn get_active_notes(&self, active_notes: &mut [u8]) -> usize {
        let mut count = 0;
        for voice in &self.voices {
            if count >= active_notes.len() {
                break;
            }
            if voice.is_active_no_release() {
                active_notes[count] = voice.pitch_midi();
                count += 1;
            }
        }
        count
    }

    /// Returns the number of voices that are not [`Idle`](crate::components::envelope::Stage::Idle),
    /// i.e. currently in attack/decay/sustain/release and being processed
    /// every block.
    ///
    /// Unlike [`get_active_notes`](Self::get_active_notes), this includes
    /// voices in their release tail. A healthy voice pool returns to `0`
    /// shortly after the last note's release completes; a value that never
    /// drops back to `0` (or that grows monotonically under sustained MIDI
    /// traffic) indicates a voice-lifecycle leak.
    pub fn active_voice_count(&self) -> usize {
        self.voices.iter().filter(|voice| voice.is_active()).count()
    }

    /// Gets the current stereo pan position based on the LFO.
    ///
    /// Returns the pan position, ranging from -1.0 (left) to 1.0 (right).
    pub fn get_pan_position(&self) -> f64 {
        self.lfo_position * self.pan_depth
    }

    // --- Synth Control/Processing Methods ---

    /// Loads a new patch with the specified parameters, reconfiguring the
    /// entire synthesiser. See the individual setters for parameter ranges.
    #[allow(clippy::too_many_arguments)]
    pub fn load_patch(
        &mut self,
        waveform: Waveform,
        filter_cutoff: f64,
        filter_resonance: f64,
        filter_mod_range: f64,
        filter_attack_time: f64,
        filter_decay_time: f64,
        filter_sustain_level: f64,
        filter_release_time: f64,
        amp_attack_time: f64,
        amp_decay_time: f64,
        amp_sustain_level: f64,
        amp_release_time: f64,
        pre_filter_gain_db: f64,
        post_filter_gain_db: f64,
        lfo_waveform: Waveform,
        lfo_frequency: f64,
        pan_depth: f64,
    ) {
        // Store the master settings for the synth
        self.set_oscillator_waveform(waveform);
        self.set_lfo_waveform(lfo_waveform);
        self.set_filter_cutoff(filter_cutoff);
        self.set_filter_resonance(filter_resonance);
        self.set_filter_mod_range(filter_mod_range);
        self.set_filter_attack_time(filter_attack_time);
        self.set_filter_decay_time(filter_decay_time);
        self.set_filter_sustain_level(filter_sustain_level);
        self.set_filter_release_time(filter_release_time);
        self.set_amp_attack_time(amp_attack_time);
        self.set_amp_decay_time(amp_decay_time);
        self.set_amp_sustain_level(amp_sustain_level);
        self.set_amp_release_time(amp_release_time);
        self.set_pre_filter_gain_db(pre_filter_gain_db);
        self.set_post_filter_gain_db(post_filter_gain_db);
        self.set_lfo_frequency(lfo_frequency);
        self.set_pan_depth(pan_depth);
    }

    /// Triggers a note-on event for a given MIDI pitch and velocity.
    /// It finds an available voice (or steals the oldest) and assigns it to
    /// play the note.
    ///
    /// * `pitch_midi` - the MIDI pitch of the note (0-127).
    /// * `velocity` - the velocity of the note (0.0 to 1.0).
    ///
    /// # Panics
    /// Panics if `velocity` is outside `[0.0, 1.0]`.
    pub fn note_on(&mut self, pitch_midi: u8, velocity: f64) {
        assert!(
            (0.0..=1.0).contains(&velocity),
            "Velocity must be between 0.0 and 1.0."
        );
        // Check if note is already being played and switch it off if it is
        self.note_off(pitch_midi);

        // Find an inactive voice
        let mut target_index = self.voices.iter().position(|voice| !voice.is_active());

        // If all voices are active, find the oldest one to steal
        if target_index.is_none() {
            let mut oldest = 0;
            for i in 1..self.voices.len() {
                if self.voices[i].note_on_time() < self.voices[oldest].note_on_time() {
                    oldest = i;
                }
            }
            target_index = Some(oldest);
        }
        let target_index = target_index.expect("voice bank is never empty");

        // Apply Settings to Target Voice
        self.note_clock += 1;
        let note_on_time = self.note_clock;
        let pan_position = self.get_pan_position();

        self.voices[target_index].set_oscillator_pitch(pitch_midi);
        self.voices[target_index].set_velocity(velocity);
        self.set_voice_params_at(target_index);
        let target_voice = &mut self.voices[target_index];
        target_voice.set_pan_position(pan_position);
        target_voice.set_note_on_time(note_on_time);
        target_voice.note_on();
    }

    /// Triggers a note-off event for a given MIDI pitch.
    pub fn note_off(&mut self, pitch_midi: u8) {
        for voice in &mut self.voices {
            if voice.is_active() && voice.pitch_midi() == pitch_midi {
                voice.note_off();
            }
        }
    }

    /// Syncs LFO oscillator selection and frequency from the master fields.
    /// Must only be called from the audio thread.
    fn sync_lfo(&mut self) {
        self.lfo_selected = self.lfo_waveform;
        let frequency = self.lfo_frequency;
        self.current_lfo_mut().set_frequency(frequency);
    }

    /// Returns the currently selected LFO (the Java `this.LFO` reference).
    fn current_lfo_mut(&mut self) -> &mut dyn Oscillator {
        match self.lfo_selected {
            Waveform::Sine => &mut self.sine_lfo,
            Waveform::Saw => &mut self.saw_lfo,
            Waveform::Triangle => &mut self.triangle_lfo,
            Waveform::Square => &mut self.square_lfo,
        }
    }

    /// Atomically reads and clears dirty flags, then applies changed parameter
    /// groups to all voices.
    fn sync_dirty_params_to_voices(&mut self) {
        let wf = self.waveform_dirty.swap(false, AtomicOrdering::SeqCst);
        let fi = self.filter_dirty.swap(false, AtomicOrdering::SeqCst);
        let fe = self.filter_env_dirty.swap(false, AtomicOrdering::SeqCst);
        let ae = self.amp_env_dirty.swap(false, AtomicOrdering::SeqCst);
        let ga = self.gain_dirty.swap(false, AtomicOrdering::SeqCst);
        let pa = self.pan_dirty.swap(false, AtomicOrdering::SeqCst);

        if wf || fi || fe || ae || ga || pa {
            let wf_snap = self.waveform;
            let (fc_snap, fr_snap, fmr_snap) = (
                self.filter_cutoff,
                self.filter_resonance,
                self.filter_mod_range,
            );
            let (fa_snap, fd_snap, fs_snap, frt_snap) = (
                self.filter_attack_time,
                self.filter_decay_time,
                self.filter_sustain_level,
                self.filter_release_time,
            );
            let (aa_snap, ad_snap, as_snap, ar_snap) = (
                self.amp_attack_time,
                self.amp_decay_time,
                self.amp_sustain_level,
                self.amp_release_time,
            );
            let (pfg_snap, pfg_post_snap) = (self.pre_filter_gain_db, self.post_filter_gain_db);
            let pd_snap = self.pan_depth;

            for voice in &mut self.voices {
                if wf {
                    voice.set_oscillator_waveform(wf_snap);
                }
                if fi {
                    voice.set_filter_parameters(fc_snap, fr_snap, fmr_snap);
                }
                if fe {
                    voice.set_filter_envelope(fa_snap, fd_snap, fs_snap, frt_snap);
                }
                if ae {
                    voice.set_amp_envelope(aa_snap, ad_snap, as_snap, ar_snap);
                }
                if ga {
                    voice.set_filter_gain_staging(pfg_snap, pfg_post_snap);
                }
                if pa {
                    voice.set_pan_depth(pd_snap);
                }
            }
        }
    }

    /// Processes one block of audio samples for all active voices.
    ///
    /// `stereo_output_buffer` is an interleaved L/R buffer; the first
    /// `block_size * 2` samples are written (the whole buffer is zeroed first,
    /// matching the Java version).
    pub fn process_block(&mut self, stereo_output_buffer: &mut [f64]) {
        // Clear the output buffer
        stereo_output_buffer.fill(0.0);

        self.sync_lfo();

        // Populate LFO buffer
        let block_size = self.block_size;
        {
            let lfo: &mut dyn Oscillator = match self.lfo_selected {
                Waveform::Sine => &mut self.sine_lfo,
                Waveform::Saw => &mut self.saw_lfo,
                Waveform::Triangle => &mut self.triangle_lfo,
                Waveform::Square => &mut self.square_lfo,
            };
            lfo.process_block(None, &mut self.lfo_output_buffer, block_size);
        }

        let vol = self.volume_attenuation;

        // Voice Processing and Mixing
        self.sync_dirty_params_to_voices();
        for voice in &mut self.voices {
            if voice.is_active() {
                // If the voice is active, process its block and sum it into the output buffer.
                voice.process_block(None, &mut self.voice_output_buffer, block_size);
                for (out, voice_sample) in stereo_output_buffer
                    .iter_mut()
                    .zip(&self.voice_output_buffer)
                {
                    *out += voice_sample * vol;
                }
            }
        }

        // Update LFO position once per block (last sample)
        self.lfo_position = self.lfo_output_buffer[block_size - 1];

        // Hard Clipping
        for sample in stereo_output_buffer.iter_mut().take(block_size * 2) {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }

    /// Processes one block of audio samples and returns performance timings.
    ///
    /// Returns a map containing the total time taken for each processing stage
    /// in nanoseconds.
    pub fn process_block_instrumented(
        &mut self,
        stereo_output_buffer: &mut [f64],
    ) -> HashMap<&'static str, u64> {
        let mut timings: HashMap<&'static str, u64> = HashMap::new();

        // Clear the output buffer
        stereo_output_buffer.fill(0.0);

        self.sync_lfo();

        // Populate LFO buffer
        let block_size = self.block_size;
        let mut start = Instant::now();
        {
            let lfo: &mut dyn Oscillator = match self.lfo_selected {
                Waveform::Sine => &mut self.sine_lfo,
                Waveform::Saw => &mut self.saw_lfo,
                Waveform::Triangle => &mut self.triangle_lfo,
                Waveform::Square => &mut self.square_lfo,
            };
            lfo.process_block(None, &mut self.lfo_output_buffer, block_size);
        }
        *timings.entry("LFO").or_insert(0) += start.elapsed().as_nanos() as u64;

        let vol = self.volume_attenuation;

        // Voice Processing and Mixing
        start = Instant::now();
        self.sync_dirty_params_to_voices();
        for voice in &mut self.voices {
            if voice.is_active() {
                voice.process_block_instrumented(
                    &self.lfo_output_buffer,
                    &mut self.voice_output_buffer,
                    block_size,
                    &mut timings,
                );
                for (out, voice_sample) in stereo_output_buffer
                    .iter_mut()
                    .zip(&self.voice_output_buffer)
                {
                    *out += voice_sample * vol;
                }
            }
        }
        *timings.entry("Voice Processing & Mix").or_insert(0) += start.elapsed().as_nanos() as u64;

        // Update LFO position once per block (last sample)
        self.lfo_position = self.lfo_output_buffer[block_size - 1];

        // Hard Clipping
        start = Instant::now();
        for sample in stereo_output_buffer.iter_mut().take(block_size * 2) {
            *sample = sample.clamp(-1.0, 1.0);
        }
        *timings.entry("Hard Clipping").or_insert(0) += start.elapsed().as_nanos() as u64;

        timings
    }
}
