//! The egui/eframe graphical user interface (port of `synth.ui`).
//!
//! - [`synth_application`]  -> `SynthApplication` (window shell / launcher)
//! - [`synth_ui_controller`] -> `SynthUIController` (state + panels + device glue)
//! - [`envelope_visualizer`] -> `EnvelopeVisualizer` (interactive ADSR canvas)
//! - [`theme`]               -> `junes_logue.css` (colour palette + egui style)
//!
//! The UI talks only to the [`crate::core::synthesiser::Synthesiser`] API
//! (behind an `Arc<Mutex<_>>`) plus the thin device-connector glue in
//! [`crate::utils::audio_device_connector`] and
//! [`crate::midi::midi_device_connector`]; no DSP or business logic lives in
//! the widgets, keeping the module portable to a future plugin (nih-plug) host.

pub mod envelope_visualizer;
pub mod synth_application;
pub mod synth_ui_controller;
pub mod theme;
