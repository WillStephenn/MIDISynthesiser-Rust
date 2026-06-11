//! Port of the abstract `synth.components.oscillators.Oscillator` base class.
//!
//! Design note: in Java the abstract `Oscillator` class holds the shared state
//! (phase accumulator, frequency, phase increment, sample-rate reciprocal) and
//! the four subclasses differ only by which lookup table they read in
//! `processBlock`. In Rust this is expressed as:
//!
//! * the [`Oscillator`] trait - the public interface (`set_frequency` plus
//!   [`AudioComponent`] processing), mirroring the abstract class surface;
//! * the [`OscillatorCore`] struct - the shared state and table-indexing
//!   logic, embedded by composition in each concrete oscillator.
//!
//! This stays closest to the Java structure (one concrete type per waveform,
//! shared base logic in one place) without inheritance.

use crate::core::audio_component::AudioComponent;
use crate::utils::lookup_tables::TABLE_SIZE;

/// Bitmask used to wrap the integer phase index into the lookup table
/// (`TABLE_SIZE` is a power of two).
pub const PHASE_MASK: usize = TABLE_SIZE - 1;

/// Represents an oscillator, which generates a periodic waveform at a
/// specified frequency.
pub trait Oscillator: AudioComponent {
    /// Sets the frequency of the oscillator.
    ///
    /// # Panics
    /// Panics if `frequency` is negative.
    fn set_frequency(&mut self, frequency: f64);
}

/// Shared oscillator state and logic (the body of the Java abstract class).
pub struct OscillatorCore {
    // Instance Variables
    pub(crate) phase: f64,
    pub(crate) frequency: f64,
    pub(crate) phase_increment: f64,

    // Pre Computed Constant
    pub(crate) sample_rate_reciprocal: f64,
}

impl OscillatorCore {
    /// Constructs the shared oscillator state for a given sample rate.
    ///
    /// # Panics
    /// Panics if `sample_rate` is not positive.
    pub fn new(sample_rate: f64) -> Self {
        assert!(sample_rate > 0.0, "Sample rate must be positive.");
        OscillatorCore {
            phase: 0.0,
            frequency: 0.0,
            phase_increment: 0.0,
            sample_rate_reciprocal: 1.0 / sample_rate,
        }
    }

    /// Advances the phase of the oscillator for the next sample.
    #[inline]
    fn advance_phase(&mut self) {
        self.phase += self.phase_increment;
    }

    /// Sets the frequency of the oscillator.
    ///
    /// # Panics
    /// Panics if `frequency` is negative.
    pub fn set_frequency(&mut self, frequency: f64) {
        assert!(frequency >= 0.0, "Frequency cannot be negative.");
        self.frequency = frequency;
        // Default Phase Increment Equation, override for non-linear oscillators
        self.phase_increment = (TABLE_SIZE as f64 * frequency) * self.sample_rate_reciprocal;
    }

    /// Fills the output buffer with `block_size` samples read from `table`,
    /// advancing the phase accumulator per sample. The integer phase index is
    /// wrapped with [`PHASE_MASK`] to prevent overflow, matching the Java
    /// `(int) phase & phaseMask` trick (`f64 as i32` saturates in Rust just as
    /// Java's narrowing double-to-int conversion does).
    #[inline]
    pub fn process_table_block(&mut self, table: &[f64], output_buffer: &mut [f64], block_size: usize) {
        for sample in output_buffer.iter_mut().take(block_size) {
            let index = (self.phase as i32 as usize) & PHASE_MASK;
            *sample = table[index];
            self.advance_phase();
        }
    }
}
