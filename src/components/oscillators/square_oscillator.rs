//! Port of `synth.components.oscillators.SquareOscillator`.

use crate::components::oscillators::oscillator::{Oscillator, OscillatorCore};
use crate::core::audio_component::AudioComponent;
use crate::utils::lookup_tables;

/// A square-wave oscillator backed by the pre-computed square lookup table.
pub struct SquareOscillator {
    core: OscillatorCore,
    /// Cached `&'static` reference to the square table (resolved once at construction).
    table: &'static [f64],
}

impl SquareOscillator {
    /// Constructs a SquareOscillator with a given sample rate.
    ///
    /// # Panics
    /// Panics if `sample_rate` is not positive.
    pub fn new(sample_rate: f64) -> Self {
        SquareOscillator {
            core: OscillatorCore::new(sample_rate),
            table: &lookup_tables::tables().square,
        }
    }
}

impl Oscillator for SquareOscillator {
    fn set_frequency(&mut self, frequency: f64) {
        self.core.set_frequency(frequency);
    }
}

impl AudioComponent for SquareOscillator {
    /// Fills the output buffer with a block of generated samples.
    /// The input buffer is ignored as oscillators are sound generators.
    fn process_block(
        &mut self,
        _input_buffer: Option<&[f64]>,
        output_buffer: &mut [f64],
        block_size: usize,
    ) {
        self.core
            .process_table_block(self.table, output_buffer, block_size);
    }
}
