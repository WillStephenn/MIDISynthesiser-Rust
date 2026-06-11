//! Port of `synth.core.AudioComponent` (Java interface -> Rust trait).

/// A processing node in the audio graph.
///
/// In the Java original the input buffer could be `null` for components that
/// generate their own signal (oscillators, envelopes in modulation mode); in
/// Rust that is expressed with `Option<&[f64]>`.
pub trait AudioComponent {
    /// Processes a block of audio samples.
    ///
    /// * `input_buffer` - the input signal, or `None` if the component
    ///   generates its own signal (like an oscillator).
    /// * `output_buffer` - the buffer the processed output is written to.
    /// * `block_size` - the number of samples to process in this block.
    fn process_block(
        &mut self,
        input_buffer: Option<&[f64]>,
        output_buffer: &mut [f64],
        block_size: usize,
    );
}
