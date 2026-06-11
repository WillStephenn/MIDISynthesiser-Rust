//! Thin GUI tests for the egui/eframe synthesiser UI (`egui_kittest`).
//!
//! Per `CLAUDE.md`'s testing philosophy, this layer stays small: a smoke
//! test that the controller draws without panicking, a startup-state check
//! against the synthesiser's real defaults, one interaction test that
//! verifies a slider edit reaches the [`Synthesiser`], and a test for the
//! MIDI-CC -> UI resync path. None of these touch real audio/MIDI hardware:
//! the controller is built via
//! [`SynthUiController::without_devices`], a small testing seam added to
//! `src/ui/synth_ui_controller.rs` that skips device enumeration, stream
//! creation and MIDI connection entirely (`cpal::Stream` is `!Send` and is
//! simply never constructed in these tests).

use std::sync::{Arc, Mutex};

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use midi_synthesiser::core::synthesiser::Synthesiser;
use midi_synthesiser::ui::synth_ui_controller::SynthUiController;
use midi_synthesiser::utils::audio_constants::{BLOCK_SIZE, NUMBER_OF_VOICES, SAMPLE_RATE};

/// Builds a fresh [`Synthesiser`] with the production constants, matching
/// what [`SynthUiController::new`] constructs in the real app.
fn new_synth() -> Synthesiser {
    Synthesiser::new(NUMBER_OF_VOICES, SAMPLE_RATE, BLOCK_SIZE)
}

/// Builds a test harness around a [`SynthUiController`] wired to `synth`,
/// with no audio/MIDI devices connected.
fn build_harness(synth: Arc<Mutex<Synthesiser>>) -> Harness<'static, SynthUiController> {
    Harness::builder()
        .with_size(eframe::egui::Vec2::new(1200.0, 800.0))
        .build_eframe(move |cc| SynthUiController::without_devices(synth, &cc.egui_ctx))
}

/// Smoke test: the controller draws a full frame without panicking, with a
/// real [`Synthesiser`] behind the mutex and no devices connected.
#[test]
fn renders_without_panicking() {
    let synth = Arc::new(Mutex::new(new_synth()));
    let mut harness = build_harness(synth);

    harness.run();
}

/// Startup state: the cutoff slider's accessible range and value reflect the
/// synthesiser's actual default patch, not a hardcoded literal.
#[test]
fn cutoff_slider_shows_default_patch_value() {
    let synth = Arc::new(Mutex::new(new_synth()));
    let expected_cutoff = synth.lock().unwrap().filter_cutoff();

    let mut harness = build_harness(synth);
    harness.run();

    let slider = cutoff_slider(&harness);
    let shown_value = slider
        .accesskit_node()
        .numeric_value()
        .expect("slider should report a numeric value");

    assert_eq!(
        shown_value, expected_cutoff,
        "the cutoff slider should display the synthesiser's default cutoff"
    );
}

/// Interaction: clicking on the filter-cutoff slider's rail moves its value,
/// and that edit is pushed through to the [`Synthesiser`] within the same
/// frame.
#[test]
fn dragging_cutoff_slider_updates_synthesiser() {
    let synth = Arc::new(Mutex::new(new_synth()));
    let original_cutoff = synth.lock().unwrap().filter_cutoff();

    let mut harness = build_harness(Arc::clone(&synth));
    harness.run();

    // Click near the right-hand end of the slider's rail, which should set
    // the cutoff far above its default (1000 Hz, well left of centre on the
    // 20..=16000 Hz range).
    let rect = cutoff_slider(&harness).rect();
    let target = eframe::egui::pos2(rect.left() + rect.width() * 0.9, rect.center().y);
    click_at(&harness, target);

    harness.run();

    let new_cutoff = synth.lock().unwrap().filter_cutoff();
    assert!(
        new_cutoff > original_cutoff,
        "clicking near the top of the cutoff slider should raise the cutoff \
         (was {original_cutoff}, now {new_cutoff})"
    );
    assert!(
        (20.0..=16000.0).contains(&new_cutoff),
        "cutoff should stay within its documented range, got {new_cutoff}"
    );
}

/// MIDI sync path: a parameter changed directly on the [`Synthesiser`] (as
/// the MIDI input thread would do via a CC message) is picked up by the UI
/// once [`SynthUiController::request_midi_resync`] is signalled and a frame
/// runs.
#[test]
fn midi_resync_refreshes_cutoff_slider() {
    let synth = Arc::new(Mutex::new(new_synth()));

    let mut harness = build_harness(Arc::clone(&synth));
    harness.run();

    // Simulate the MIDI input thread: mutate the synth directly, then flag
    // that the UI needs to resync, exactly as the CC callback does.
    let midi_driven_cutoff = 5000.0;
    {
        let mut guard = synth.lock().unwrap();
        guard.set_filter_cutoff(midi_driven_cutoff);
    }
    harness.state().request_midi_resync();

    harness.run();

    let slider = cutoff_slider(&harness);
    let shown_value = slider
        .accesskit_node()
        .numeric_value()
        .expect("slider should report a numeric value");

    assert_eq!(
        shown_value, midi_driven_cutoff,
        "after a MIDI resync the cutoff slider should reflect the synth's new value"
    );
}

/// Finds the filter-cutoff slider by its accessible numeric range
/// (20 Hz..=16000 Hz), which is unique among the sliders in the UI. The
/// slider has no visible accessibility label (it uses
/// `Slider::show_value(false)` alongside a separate value-readout label), so
/// the range is the most stable way to identify it without depending on
/// widget ordering.
fn cutoff_slider<'h>(harness: &'h Harness<'static, SynthUiController>) -> egui_kittest::Node<'h> {
    harness
        .root()
        .get_all_by_role(eframe::egui::accesskit::Role::Slider)
        .find(|node| {
            let accesskit = node.accesskit_node();
            accesskit.min_numeric_value() == Some(20.0)
                && accesskit.max_numeric_value() == Some(16000.0)
        })
        .expect("filter cutoff slider should be present")
}

/// Simulates a single click (press + release) at `pos` within the harness,
/// without relying on a particular node already being hovered.
fn click_at(harness: &Harness<'static, SynthUiController>, pos: eframe::egui::Pos2) {
    harness.event(eframe::egui::Event::PointerMoved(pos));
    harness.event(eframe::egui::Event::PointerButton {
        pos,
        button: eframe::egui::PointerButton::Primary,
        pressed: true,
        modifiers: eframe::egui::Modifiers::default(),
    });
    harness.event(eframe::egui::Event::PointerButton {
        pos,
        button: eframe::egui::PointerButton::Primary,
        pressed: false,
        modifiers: eframe::egui::Modifiers::default(),
    });
}
