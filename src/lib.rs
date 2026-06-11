//! Rust port of the Java MIDI synthesiser.
//!
//! The module tree mirrors the Java package layout:
//!
//! - `utils`         -> `synth.utils`         (constants, lookup tables, audio device I/O)
//! - `components`    -> `synth.components`    (envelope, filters, oscillators)
//! - `core`          -> `synth.core`          (audio component trait, voice, synthesiser)
//! - `midi`          -> `synth.midi`          (MIDI device I/O, input handling, file playback)
//! - `visualisation` -> `synth.visualisation` (console ASCII renderer)
//! - `ui`            -> `synth.ui`            (egui/eframe GUI, port of the JavaFX UI)
//!
//! The DSP engine modules (`core`, `components`, the constants/lookup tables
//! in `utils`) have zero non-std dependencies and perform no I/O; all audio is
//! rendered into caller-provided buffers via
//! [`core::synthesiser::Synthesiser::process_block`]. The stage-2 I/O layer
//! (`utils::audio_device_connector`, `midi`, `visualisation`) wires the engine
//! to real devices via `cpal`, `midir` and `midly`.

pub mod components;
pub mod core;
pub mod midi;
pub mod ui;
pub mod utils;
pub mod visualisation;
