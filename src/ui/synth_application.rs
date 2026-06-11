//! The main application shell for the synthesiser's GUI
//! (port of `synth.ui.SynthApplication`).
//!
//! The JavaFX class loaded the FXML layout, created the scene and showed the
//! primary stage; here the equivalent is configuring the eframe native window
//! and handing control to [`SynthUiController`]. The Java close-request hook
//! (`controller.shutdown()`) is unnecessary: dropping the controller when the
//! window closes drops the `cpal::Stream` and `MidiInputConnection`, which
//! stops the audio callback and closes the MIDI port.

use eframe::egui;

use crate::ui::synth_ui_controller::SynthUiController;

/// Launches the GUI and blocks until the window is closed
/// (the equivalent of `Application.launch`).
pub fn launch() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MIDI Synthesiser")
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([1080.0, 680.0]),
        ..Default::default()
    };
    eframe::run_native(
        "MIDI Synthesiser",
        options,
        Box::new(|cc| Ok(Box::new(SynthUiController::new(cc)))),
    )
}
