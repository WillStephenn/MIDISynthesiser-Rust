//! Port of `synth.components.Envelope`.

use crate::core::audio_component::AudioComponent;

/// The different stages of the envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Represents an ADSR (Attack, Decay, Sustain, Release) envelope generator.
/// This component modulates the amplitude of an audio signal over time.
pub struct Envelope {
    // State Variables
    current_stage: Stage,
    current_multiplier: f64,
    attack_increment: f64,
    decay_increment: f64,
    release_increment: f64,

    // Pre Computed Constant
    sample_rate_reciprocal: f64,

    // Settings
    // All timings are measured in seconds.
    attack_time: f64,
    decay_time: f64,
    sustain_level: f64,
    release_time: f64,
}

impl Envelope {
    /// Constructs an Envelope with a given sample rate.
    ///
    /// # Panics
    /// Panics if `sample_rate` is not positive (mirrors the Java
    /// `IllegalArgumentException`).
    pub fn new(sample_rate: f64) -> Self {
        assert!(sample_rate > 0.0, "Sample rate must be positive.");
        let mut envelope = Envelope {
            current_stage: Stage::Idle,
            current_multiplier: 0.0,
            attack_increment: 0.0,
            decay_increment: 0.0,
            release_increment: 0.0,
            sample_rate_reciprocal: 1.0 / sample_rate,
            attack_time: 0.0,
            decay_time: 0.0,
            sustain_level: 0.0,
            release_time: 0.0,
        };

        // Default Envelope patch
        envelope.set_envelope(2.0, 2.0, 0.5, 3.0);
        envelope
    }

    /// Sets the current stage of the envelope.
    pub fn set_stage(&mut self, new_stage: Stage) {
        self.current_stage = new_stage;
    }

    /// Gets the current stage of the envelope.
    pub fn stage(&self) -> Stage {
        self.current_stage
    }

    /// Sets the attack time of the envelope.
    ///
    /// # Panics
    /// Panics if `seconds` is negative.
    pub fn set_attack_time(&mut self, seconds: f64) {
        assert!(seconds >= 0.0, "Attack time cannot be negative.");
        self.attack_time = seconds;
        if self.attack_time == 0.0 {
            self.attack_increment = 1.0;
        } else {
            self.attack_increment = self.sample_rate_reciprocal / self.attack_time;
        }
    }

    /// Sets the decay time of the envelope.
    ///
    /// # Panics
    /// Panics if `seconds` is negative.
    pub fn set_decay_time(&mut self, seconds: f64) {
        assert!(seconds >= 0.0, "Decay time cannot be negative.");
        self.decay_time = seconds;
        if self.decay_time == 0.0 {
            self.decay_increment = 1.0 - self.sustain_level;
        } else {
            self.decay_increment =
                (1.0 - self.sustain_level) * self.sample_rate_reciprocal / self.decay_time;
        }
    }

    /// Sets the sustain level of the envelope (0.0 to 1.0).
    ///
    /// # Panics
    /// Panics if `level` is outside `[0.0, 1.0]`.
    pub fn set_sustain_level(&mut self, level: f64) {
        assert!(
            (0.0..=1.0).contains(&level),
            "Sustain level must be between 0.0 and 1.0."
        );
        self.sustain_level = level;
    }

    /// Sets the release time of the envelope.
    ///
    /// # Panics
    /// Panics if `seconds` is negative.
    pub fn set_release_time(&mut self, seconds: f64) {
        assert!(seconds >= 0.0, "Release time cannot be negative.");
        self.release_time = seconds;
    }

    /// Sets all parameters of the envelope.
    ///
    /// Note the ordering: sustain is applied before decay so `decay_increment`
    /// is derived from the new sustain level (matches Java).
    /// `release_increment` is not derived here -- it is computed from
    /// `current_multiplier` when [`note_off`](Self::note_off) fires, so the
    /// release ramp always matches the level the envelope was at when
    /// released.
    pub fn set_envelope(
        &mut self,
        attack_time: f64,
        decay_time: f64,
        sustain_level: f64,
        release_time: f64,
    ) {
        // Set times and level using their respective setters to trigger calculations
        self.set_attack_time(attack_time);
        self.set_sustain_level(sustain_level);
        self.set_decay_time(decay_time);
        self.set_release_time(release_time);
    }

    /// Triggers the attack phase of the envelope when a note is played.
    pub fn note_on(&mut self) {
        self.current_stage = Stage::Attack;
        self.current_multiplier = 0.0;
    }

    /// Triggers the release phase of the envelope when a note is released.
    ///
    /// The release ramp is derived from `current_multiplier` *at the moment
    /// of release* (not from `sustain_level`), so a note released early
    /// (during Attack or Decay, before reaching Sustain) still fades to
    /// silence over `release_time` seconds. This also guarantees
    /// `release_increment > 0` whenever there is anything to release, so the
    /// envelope always reaches `Idle` -- previously, releasing during
    /// Attack/Decay with `sustain_level == 0.0` produced a `release_increment`
    /// of exactly `0.0`, leaving the envelope (and its voice) stuck in
    /// `Release` forever.
    pub fn note_off(&mut self) {
        if self.current_multiplier <= 0.0 || self.release_time == 0.0 {
            // Nothing left to release, or an instant release: drop straight to Idle.
            self.current_multiplier = 0.0;
            self.release_increment = 0.0;
            self.current_stage = Stage::Idle;
        } else {
            self.release_increment =
                self.current_multiplier * self.sample_rate_reciprocal / self.release_time;
            self.current_stage = Stage::Release;
        }
    }
}

impl AudioComponent for Envelope {
    /// Processes a block of audio, applying the envelope to each sample.
    ///
    /// When `input_buffer` is `None` the raw envelope value is written to the
    /// output (the envelope acts as a modulation source).
    fn process_block(
        &mut self,
        input_buffer: Option<&[f64]>,
        output_buffer: &mut [f64],
        block_size: usize,
    ) {
        for i in 0..block_size {
            match self.current_stage {
                Stage::Idle => {
                    self.current_multiplier = 0.0;
                }
                Stage::Attack => {
                    self.current_multiplier += self.attack_increment;
                    if self.current_multiplier >= 1.0 {
                        self.current_multiplier = 1.0;
                        self.set_stage(Stage::Decay);
                    }
                }
                Stage::Decay => {
                    self.current_multiplier -= self.decay_increment;
                    if self.current_multiplier <= self.sustain_level {
                        self.current_multiplier = self.sustain_level;
                        self.set_stage(Stage::Sustain);
                    }
                }
                Stage::Sustain => {
                    self.current_multiplier = self.sustain_level;
                }
                Stage::Release => {
                    self.current_multiplier -= self.release_increment;
                    if self.current_multiplier <= 0.0 {
                        self.current_multiplier = 0.0;
                        self.set_stage(Stage::Idle);
                    }
                }
            }
            match input_buffer {
                None => output_buffer[i] = self.current_multiplier,
                Some(input) => output_buffer[i] = input[i] * self.current_multiplier,
            }
        }
    }
}
