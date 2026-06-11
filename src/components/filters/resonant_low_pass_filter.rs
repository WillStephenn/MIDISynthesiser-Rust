//! Port of `synth.components.filters.ResonantLowPassFilter`.

use crate::components::filters::filter::Filter;
use crate::core::audio_component::AudioComponent;
use crate::utils::lookup_tables::{self, LookupTables, RESONANCE_STEPS, TABLE_SIZE};

/// Implements a resonant low-pass filter using the Topology-Preserving Transform (TPT)
/// State-Variable Filter (SVF) design by Vadim Zavalishin.
pub struct ResonantLowPassFilter {
    sample_rate: f64,

    // Filter's internal memory variables
    integrator1: f64,
    integrator2: f64,

    // Constant Pre-Computed Constants
    cutoff_scalar: f64,
    resonance_scalar: f64,
    nyquist_limit: f64,

    // Cached coefficients
    a1: f64,
    a2: f64,
    a3: f64,
    prev_cutoff_index: i32,
    prev_resonance_index: i32,

    // Cached &'static reference so the audio path never touches the LazyLock.
    tables: &'static LookupTables,
}

impl ResonantLowPassFilter {
    /// Constructs a ResonantLowPassFilter with a given sample rate.
    ///
    /// # Panics
    /// Panics if `sample_rate` is not positive.
    pub fn new(sample_rate: f64) -> Self {
        assert!(sample_rate > 0.0, "Sample rate must be positive.");
        let mut filter = ResonantLowPassFilter {
            sample_rate,
            integrator1: 0.0,
            integrator2: 0.0,
            cutoff_scalar: TABLE_SIZE as f64 / sample_rate,
            resonance_scalar: (RESONANCE_STEPS - 1) as f64 / 19.0, // Resonance ranges from 1 to 20
            nyquist_limit: (sample_rate / 2.0) - 1.0,
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            prev_cutoff_index: -1,
            prev_resonance_index: -1,
            tables: lookup_tables::tables(),
        };
        filter.set_parameters(1000.0, 1.0);
        filter
    }

    /// Sets the cutoff frequency and resonance (Q) of the filter.
    ///
    /// This method checks if the parameters have changed enough to warrant
    /// fetching new coefficients from the lookup tables.
    ///
    /// * `cutoff_frequency` - the cutoff frequency in Hz. Must be positive and
    ///   below the Nyquist frequency.
    /// * `resonance_q` - the resonance factor (Q). Must be a positive value.
    ///
    /// # Panics
    /// Panics if the parameters are out of range (mirrors the Java
    /// `IllegalArgumentException`s).
    pub fn set_parameters(&mut self, cutoff_frequency: f64, resonance_q: f64) {
        assert!(
            cutoff_frequency > 0.0 && cutoff_frequency < self.nyquist_limit,
            "Cutoff frequency must be positive and below the Nyquist frequency."
        );
        assert!(resonance_q > 0.0, "Resonance (Q) must be positive.");

        // Calculate the index for the cutoff & resonance
        let target_cutoff_index = (cutoff_frequency * self.cutoff_scalar) as i32;
        let target_resonance_index = ((resonance_q - 1.0) * self.resonance_scalar) as i32;

        // Compare to cached values and fetch new coefficients from the LUTs
        if target_cutoff_index != self.prev_cutoff_index
            || target_resonance_index != self.prev_resonance_index
        {
            self.prev_cutoff_index = target_cutoff_index;
            self.prev_resonance_index = target_resonance_index;
            let cutoff_index = target_cutoff_index as usize;
            let resonance_index = target_resonance_index as usize;
            self.a1 = self.tables.a1_table[cutoff_index][resonance_index];
            self.a2 = self.tables.a2_table[cutoff_index][resonance_index];
            self.a3 = self.tables.a3_table[cutoff_index][resonance_index];
        }
    }
}

impl Filter for ResonantLowPassFilter {
    fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
}

impl AudioComponent for ResonantLowPassFilter {
    /// Processes a block of audio through the TPT SVF low-pass stage.
    fn process_block(
        &mut self,
        input_buffer: Option<&[f64]>,
        output_buffer: &mut [f64],
        block_size: usize,
    ) {
        let input = input_buffer.expect("ResonantLowPassFilter requires an input buffer");
        for i in 0..block_size {
            // The TPT State-Variable Filter Algorithm (unchanged)
            let v3 = input[i] - self.integrator2;
            let v1 = self.a1 * self.integrator1 + self.a2 * v3;
            let v2 = self.integrator2 + self.a2 * self.integrator1 + self.a3 * v3;

            self.integrator1 = 2.0 * v1 - self.integrator1;
            self.integrator2 = 2.0 * v2 - self.integrator2;

            output_buffer[i] = v2;
        }
    }
}
