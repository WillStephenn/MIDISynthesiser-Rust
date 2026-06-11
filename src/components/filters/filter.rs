//! Port of `synth.components.filters.Filter`.
//!
//! The Java original is an abstract class that only stores the sample rate
//! and re-declares `processBlock`; in Rust this becomes a marker-style trait
//! extending [`AudioComponent`].

use crate::core::audio_component::AudioComponent;

/// Represents a filter that can process an audio signal.
/// This trait provides the basic structure for different filter types.
pub trait Filter: AudioComponent {
    /// The sample rate of the audio system this filter was constructed for.
    fn sample_rate(&self) -> f64;
}
