//! Global audio engine constants (port of `synth.utils.AudioConstants`).
//!
//! The Java original is a constants-only interface; in Rust these are plain
//! `pub const` items in a module.

/// The audio sample rate in Hz.
pub const SAMPLE_RATE: f64 = 44100.0;

/// The number of frames processed per engine block.
pub const BLOCK_SIZE: usize = 32;

/// The size of the output device buffer, in frames.
///
/// Currently unused: `cpal` is configured with its default buffer size (see
/// `utils::audio_device_connector`), and the engine always renders exactly
/// `BLOCK_SIZE` frames per call to `Synthesiser::process_block`. Kept for
/// future use (an explicit device buffer size) and validated at startup by
/// `utils::engine_config` (must be `>= BLOCK_SIZE`) so it stays usable.
pub const BUFFER_SIZE: usize = BLOCK_SIZE;

/// The number of polyphonic voices in the synthesiser.
pub const NUMBER_OF_VOICES: usize = 32;

/// The number of entries in each waveform/filter lookup table.
/// Must be a power of two (the oscillators rely on bitmask phase wrapping).
pub const LOOKUP_TABLE_SIZE: usize = 16384 * 2;

/// How often (in seconds) the host application rescans for audio/MIDI devices.
/// (Unused by the engine itself; kept for stages 2/3.)
pub const DEVICE_SCAN_INTERVAL_SECONDS: f64 = 3.0;
