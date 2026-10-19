//! MIDI I/O layer (port of the `synth.midi` package): device enumeration and
//! connection, live MIDI input handling, and MIDI file playback.

#[cfg(feature = "standalone")]
pub mod midi_device_connector;
#[cfg(feature = "standalone")]
pub mod midi_file_player;
pub mod midi_input_handler;
