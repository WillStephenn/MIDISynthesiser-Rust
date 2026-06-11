//! Controller for the main synthesiser user interface
//! (port of `synth.ui.SynthUIController`).
//!
//! Manages the initialisation of the synth, audio/MIDI devices, and binds UI
//! controls to the synthesiser's parameters. Where the JavaFX controller
//! wired listeners to FXML-injected widgets, this egui port keeps a
//! [`UiState`] snapshot of every parameter, draws the widgets from it each
//! frame, and pushes any user edits back to the shared [`Synthesiser`] under
//! one brief mutex lock per frame.
//!
//! Threading model:
//! - The GUI thread owns the `cpal::Stream` (which is `!Send`) and the
//!   `MidiInputConnection`; dropping either stops audio / disconnects MIDI,
//!   replacing the Java audio-render thread and `MidiDevice.close()`.
//! - The cpal audio callback locks the synthesiser once per engine block.
//! - The midir callback thread locks the synthesiser per MIDI message; after
//!   each recognised CC it sets an `AtomicBool` and requests a repaint, and
//!   the next frame re-reads the getters into [`UiState`] — the coalesced
//!   equivalent of the Java `midiSyncPending` + `Platform.runLater` sync.
//! - Device rescans run on the GUI thread every
//!   [`DEVICE_SCAN_INTERVAL_SECONDS`] (the Java app used a background
//!   executor + `Platform.runLater`; enumeration is cheap enough to do
//!   inline, and it keeps the cpal/midir host objects on one thread).
//!
//! June's Logue - Modern/Vintage Terracotta Theme

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use midir::MidiInputConnection;

use crate::core::synthesiser::{Synthesiser, Waveform};
use crate::midi::midi_device_connector;
use crate::midi::midi_input_handler::ControlChangeCallback;
use crate::ui::envelope_visualizer::{Adsr, EnvelopeVisualizer};
use crate::ui::theme;
use crate::utils::audio_constants::{
    BLOCK_SIZE, DEVICE_SCAN_INTERVAL_SECONDS, NUMBER_OF_VOICES, SAMPLE_RATE,
};
use crate::utils::audio_device_connector;

/// All four waveforms, for the choice boxes (Java used `Waveform.values()`).
const WAVEFORMS: [Waveform; 4] = [
    Waveform::Sine,
    Waveform::Saw,
    Waveform::Triangle,
    Waveform::Square,
];

/// Display name for a waveform (the Java enum's `toString()`).
fn waveform_label(waveform: Waveform) -> &'static str {
    match waveform {
        Waveform::Sine => "SINE",
        Waveform::Saw => "SAW",
        Waveform::Triangle => "TRIANGLE",
        Waveform::Square => "SQUARE",
    }
}

/// Snapshot of every synthesiser parameter shown in the UI.
///
/// This is the immediate-mode stand-in for the values held by the JavaFX
/// sliders/choice boxes: widgets edit these fields, and the controller pushes
/// them to the synthesiser (user edits) or refreshes them from the
/// synthesiser's getters (MIDI CC sync, startup patch defaults).
#[derive(Debug, Clone, Copy, PartialEq)]
struct UiState {
    // Oscillator & LFO
    waveform: Waveform,
    lfo_waveform: Waveform,
    lfo_frequency: f64,
    // Filter
    filter_cutoff: f64,
    filter_resonance: f64,
    filter_mod_range: f64,
    // Envelopes
    amp_envelope: Adsr,
    filter_envelope: Adsr,
    // Global controls
    master_volume: f64,
    pan_depth: f64,
    pre_filter_gain_db: f64,
    post_filter_gain_db: f64,
}

impl UiState {
    /// Reads the current patch out of the synthesiser
    /// (the Java `syncUIWithSynthSettings`).
    fn read_from(synth: &Synthesiser) -> Self {
        UiState {
            waveform: synth.waveform(),
            lfo_waveform: synth.lfo_waveform(),
            lfo_frequency: synth.lfo_frequency(),
            filter_cutoff: synth.filter_cutoff(),
            filter_resonance: synth.filter_resonance(),
            filter_mod_range: synth.filter_mod_range(),
            amp_envelope: Adsr {
                attack: synth.amp_attack_time(),
                decay: synth.amp_decay_time(),
                sustain: synth.amp_sustain_level(),
                release: synth.amp_release_time(),
            },
            filter_envelope: Adsr {
                attack: synth.filter_attack_time(),
                decay: synth.filter_decay_time(),
                sustain: synth.filter_sustain_level(),
                release: synth.filter_release_time(),
            },
            master_volume: synth.master_volume_scalar(),
            pan_depth: synth.pan_depth(),
            pre_filter_gain_db: synth.pre_filter_gain_db(),
            post_filter_gain_db: synth.post_filter_gain_db(),
        }
    }

    /// Pushes every parameter to the synthesiser. The setters early-out when
    /// a value is unchanged, so calling them all under one brief lock is the
    /// immediate-mode equivalent of the Java per-slider listeners.
    fn write_to(&self, synth: &mut Synthesiser) {
        synth.set_oscillator_waveform(self.waveform);
        synth.set_lfo_waveform(self.lfo_waveform);
        synth.set_lfo_frequency(self.lfo_frequency);
        synth.set_filter_cutoff(self.filter_cutoff);
        synth.set_filter_resonance(self.filter_resonance);
        synth.set_filter_mod_range(self.filter_mod_range);
        synth.set_amp_attack_time(self.amp_envelope.attack);
        synth.set_amp_decay_time(self.amp_envelope.decay);
        synth.set_amp_sustain_level(self.amp_envelope.sustain);
        synth.set_amp_release_time(self.amp_envelope.release);
        synth.set_filter_attack_time(self.filter_envelope.attack);
        synth.set_filter_decay_time(self.filter_envelope.decay);
        synth.set_filter_sustain_level(self.filter_envelope.sustain);
        synth.set_filter_release_time(self.filter_envelope.release);
        synth.set_master_volume(self.master_volume);
        synth.set_pan_depth(self.pan_depth);
        synth.set_pre_filter_gain_db(self.pre_filter_gain_db);
        synth.set_post_filter_gain_db(self.post_filter_gain_db);
    }
}

/// The egui application state: synthesiser, device connections and UI widgets.
pub struct SynthUiController {
    synth: Arc<Mutex<Synthesiser>>,

    // Device management (the `line`/`audioThread`/`midiDevice` of the Java
    // controller; dropping the handles stops the audio/MIDI).
    audio_stream: Option<cpal::Stream>,
    midi_connection: Option<MidiInputConnection<()>>,
    audio_devices: Vec<String>,
    midi_devices: Vec<String>,
    selected_audio_device: Option<String>,
    selected_midi_device: Option<String>,
    last_device_scan: Instant,

    // Coalescing flag for MIDI CC UI sync (the Java `midiSyncPending`).
    midi_sync_pending: Arc<AtomicBool>,

    // UI state
    state: UiState,
    amp_envelope_visualizer: EnvelopeVisualizer,
    filter_envelope_visualizer: EnvelopeVisualizer,
}

impl SynthUiController {
    /// Initialises the controller: creates the synthesiser, applies the
    /// theme, scans devices and auto-connects the first audio/MIDI device
    /// (the Java `initialize` + `setupDeviceSelectors`).
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);

        let synth = Arc::new(Mutex::new(Synthesiser::new(
            NUMBER_OF_VOICES,
            SAMPLE_RATE,
            BLOCK_SIZE,
        )));

        // Show the default patch on startup.
        let state = UiState::read_from(&synth.lock().unwrap_or_else(|p| p.into_inner()));

        let mut controller = SynthUiController {
            synth,
            audio_stream: None,
            midi_connection: None,
            audio_devices: audio_device_connector::get_audio_output_device_list(),
            midi_devices: midi_device_connector::get_midi_devices_list(),
            selected_audio_device: None,
            selected_midi_device: None,
            last_device_scan: Instant::now(),
            midi_sync_pending: Arc::new(AtomicBool::new(false)),
            state,
            amp_envelope_visualizer: EnvelopeVisualizer::default(),
            filter_envelope_visualizer: EnvelopeVisualizer::default(),
        };

        // Select the first device by default to kick things off.
        if let Some(first) = controller.audio_devices.first().cloned() {
            controller.change_audio_device(&first);
        }
        if let Some(first) = controller.midi_devices.first().cloned() {
            controller.change_midi_device(&first, &cc.egui_ctx);
        }

        controller
    }

    /// Changes the active audio output device. The previous stream is
    /// dropped first (stopping its callback), replacing the Java
    /// stop-thread/close-line/reopen dance.
    fn change_audio_device(&mut self, device_name: &str) {
        self.audio_stream = None;
        match audio_device_connector::start_output_stream(device_name, Arc::clone(&self.synth)) {
            Ok(stream) => {
                self.audio_stream = Some(stream);
                self.selected_audio_device = Some(device_name.to_string());
            }
            Err(e) => {
                eprintln!("Failed to open audio device: {e}");
                self.selected_audio_device = None;
            }
        }
    }

    /// Changes the active MIDI input device, registering a CC callback that
    /// schedules a coalesced UI sync (the Java `onMidiControlChange`).
    fn change_midi_device(&mut self, device_name: &str, ctx: &egui::Context) {
        self.midi_connection = None; // close the previous connection

        let pending = Arc::clone(&self.midi_sync_pending);
        let repaint_ctx = ctx.clone();
        let on_control_change: ControlChangeCallback = Box::new(move || {
            // Coalesce: many CCs between frames cause a single getter sync.
            if !pending.swap(true, Ordering::AcqRel) {
                repaint_ctx.request_repaint();
            }
        });

        self.midi_connection = midi_device_connector::connect_to_device_with_callback(
            Arc::clone(&self.synth),
            device_name,
            Some(on_control_change),
        );
        self.selected_midi_device = self
            .midi_connection
            .is_some()
            .then(|| device_name.to_string());
    }

    /// Rescans the device lists, keeping the current selections when the
    /// devices are still present (the Java `refreshDeviceLists`).
    fn refresh_device_lists(&mut self) {
        let new_midi = midi_device_connector::get_midi_devices_list();
        let new_audio = audio_device_connector::get_audio_output_device_list();

        if new_midi != self.midi_devices {
            self.midi_devices = new_midi;
            if let Some(selected) = &self.selected_midi_device
                && !self.midi_devices.contains(selected)
            {
                // Keep the connection handle (midir reports errors itself if
                // the port died); only the dropdown selection is cleared,
                // matching the JavaFX ChoiceBox losing its value.
                self.selected_midi_device = None;
            }
        }
        if new_audio != self.audio_devices {
            self.audio_devices = new_audio;
            if let Some(selected) = &self.selected_audio_device
                && !self.audio_devices.contains(selected)
            {
                self.selected_audio_device = None;
            }
        }
    }

    /// Syncs the UI controls with the current synthesiser settings
    /// (the Java `syncUIWithSynthSettings`, run after MIDI CC changes).
    fn sync_ui_from_synth(&mut self) {
        let guard = self.synth.lock().unwrap_or_else(|p| p.into_inner());
        self.state = UiState::read_from(&guard);
    }

    // --- Panels -----------------------------------------------------------

    /// Header: title, subtitle and the MIDI/audio device dropdowns
    /// (the FXML `<top>` section).
    fn header_panel(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        let frame = egui::Frame::new()
            .fill(theme::CHOCOLATE_COSMOS)
            .inner_margin(egui::Margin::symmetric(30, 16));
        egui::Panel::top("header")
            .frame(frame)
            .show_separator_line(false)
            .show_inside(root, |ui| {
                ui.label(
                    egui::RichText::new("JUNE'S LOGUE")
                        .color(theme::ORANGE_PEEL)
                        .size(36.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new("RUST SYNTHESISER")
                        .color(theme::WHITE_SMOKE)
                        .size(14.0),
                );
                ui.add_space(12.0);

                // Device Selection Row
                let mut new_midi: Option<String> = None;
                let mut new_audio: Option<String> = None;
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(theme::parameter_label("MIDI INPUT").color(theme::ORANGE_PEEL));
                        new_midi = device_choice_box(
                            ui,
                            "midi_device",
                            &self.midi_devices,
                            self.selected_midi_device.as_deref(),
                        );
                    });
                    ui.add_space(24.0);
                    ui.vertical(|ui| {
                        ui.label(
                            theme::parameter_label("AUDIO OUTPUT").color(theme::ORANGE_PEEL),
                        );
                        new_audio = device_choice_box(
                            ui,
                            "audio_device",
                            &self.audio_devices,
                            self.selected_audio_device.as_deref(),
                        );
                    });
                });
                if let Some(name) = new_midi {
                    self.change_midi_device(&name, &ctx);
                }
                if let Some(name) = new_audio {
                    self.change_audio_device(&name);
                }

                ui.add_space(6.0);
            });
    }

    /// Global controls footer: master volume, pan depth and gain staging
    /// (the FXML `<bottom>` section).
    fn global_controls_panel(&mut self, root: &mut egui::Ui) -> bool {
        let mut changed = false;
        let frame = egui::Frame::new()
            .fill(theme::CHOCOLATE_COSMOS)
            .inner_margin(egui::Margin::symmetric(30, 16));
        egui::Panel::bottom("global_controls")
            .frame(frame)
            .show_separator_line(false)
            .show_inside(root, |ui| {
                ui.label(theme::section_header("GLOBAL CONTROLS"));
                section_separator(ui);
                ui.add_space(4.0);
                ui.columns(4, |columns| {
                    changed |= parameter_slider(
                        &mut columns[0],
                        "MASTER VOLUME",
                        &mut self.state.master_volume,
                        0.0..=1.0,
                        |v| format!("{:.0}%", v * 100.0),
                    );
                    changed |= parameter_slider(
                        &mut columns[1],
                        "PAN DEPTH",
                        &mut self.state.pan_depth,
                        0.0..=1.0,
                        |v| format!("{v:.2}"),
                    );
                    changed |= parameter_slider(
                        &mut columns[2],
                        "PRE-FILTER GAIN",
                        &mut self.state.pre_filter_gain_db,
                        -20.0..=20.0,
                        |v| format!("{v:.1} dB"),
                    );
                    changed |= parameter_slider(
                        &mut columns[3],
                        "POST-FILTER GAIN",
                        &mut self.state.post_filter_gain_db,
                        -20.0..=20.0,
                        |v| format!("{v:.1} dB"),
                    );
                });
            });
        changed
    }

    /// Oscillator & LFO section (first column of the FXML `<center>`).
    fn oscillator_section(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.label(theme::section_header("OSCILLATOR"));
        section_separator(ui);
        ui.label(theme::parameter_label("WAVEFORM"));
        changed |= waveform_choice_box(ui, "osc_waveform", &mut self.state.waveform);

        ui.add_space(20.0);
        ui.label(theme::section_header("LFO"));
        section_separator(ui);
        ui.label(theme::parameter_label("LFO WAVEFORM"));
        changed |= waveform_choice_box(ui, "lfo_waveform", &mut self.state.lfo_waveform);
        ui.add_space(8.0);
        changed |= parameter_slider(
            ui,
            "LFO FREQUENCY",
            &mut self.state.lfo_frequency,
            0.1..=20.0,
            |v| format!("{v:.1} Hz"),
        );
        changed
    }

    /// Filter section (second column of the FXML `<center>`).
    fn filter_section(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.label(theme::section_header("FILTER"));
        section_separator(ui);
        changed |= parameter_slider(
            ui,
            "CUTOFF",
            &mut self.state.filter_cutoff,
            20.0..=16000.0,
            |v| format!("{v:.0} Hz"),
        );
        ui.add_space(8.0);
        changed |= parameter_slider(
            ui,
            "RESONANCE",
            &mut self.state.filter_resonance,
            1.0..=20.0,
            |v| format!("{v:.1}"),
        );
        ui.add_space(8.0);
        changed |= parameter_slider(
            ui,
            "MOD RANGE",
            &mut self.state.filter_mod_range,
            0.0..=6000.0,
            |v| format!("{v:.0} Hz"),
        );
        changed
    }

    /// Amp envelope section: visualizer + ADSR sliders
    /// (third column of the FXML `<center>`).
    fn amp_envelope_section(&mut self, ui: &mut egui::Ui) -> bool {
        ui.label(theme::section_header("AMP ENVELOPE"));
        section_separator(ui);
        let mut changed = self
            .amp_envelope_visualizer
            .show(ui, &mut self.state.amp_envelope);
        ui.add_space(8.0);
        changed |= adsr_sliders(ui, &mut self.state.amp_envelope);
        changed
    }

    /// Filter envelope section: visualizer + ADSR sliders
    /// (fourth column of the FXML `<center>`).
    fn filter_envelope_section(&mut self, ui: &mut egui::Ui) -> bool {
        ui.label(theme::section_header("FILTER ENVELOPE"));
        section_separator(ui);
        let mut changed = self
            .filter_envelope_visualizer
            .show(ui, &mut self.state.filter_envelope);
        ui.add_space(8.0);
        changed |= adsr_sliders(ui, &mut self.state.filter_envelope);
        changed
    }
}

impl eframe::App for SynthUiController {
    /// Per-frame update: MIDI-driven state sync, periodic device rescan,
    /// then the four panels; finally any user edits are pushed to the
    /// synthesiser under a single brief lock.
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Coalesced MIDI CC -> UI sync (the Java Platform.runLater body).
        if self.midi_sync_pending.swap(false, Ordering::AcqRel) {
            self.sync_ui_from_synth();
        }

        // Periodic device rescan, and a scheduled repaint so the scan keeps
        // ticking even when no input/MIDI events arrive.
        let scan_interval = Duration::from_secs_f64(DEVICE_SCAN_INTERVAL_SECONDS);
        if self.last_device_scan.elapsed() >= scan_interval {
            self.refresh_device_lists();
            self.last_device_scan = Instant::now();
        }
        root.ctx().request_repaint_after(
            scan_interval.saturating_sub(self.last_device_scan.elapsed()),
        );

        let mut changed = false;

        self.header_panel(root);
        changed |= self.global_controls_panel(root);

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BLACK))
            .show_inside(root, |ui| {
                ui.columns(4, |columns| {
                    // Alternate section backgrounds like the FXML
                    // `.section-box` / `.filter-section-box` styling.
                    changed |= section_box(&mut columns[0], theme::CHOCOLATE_COSMOS, |ui| {
                        self.oscillator_section(ui)
                    });
                    changed |= section_box(&mut columns[1], theme::BLACK, |ui| {
                        self.filter_section(ui)
                    });
                    changed |= section_box(&mut columns[2], theme::CHOCOLATE_COSMOS, |ui| {
                        self.amp_envelope_section(ui)
                    });
                    changed |= section_box(&mut columns[3], theme::BLACK, |ui| {
                        self.filter_envelope_section(ui)
                    });
                });
            });

        // Push user edits to the synthesiser: one short lock per frame, only
        // when something actually changed.
        if changed {
            let mut guard = self.synth.lock().unwrap_or_else(|p| p.into_inner());
            self.state.write_to(&mut guard);
        }
    }
}

// --- Widget helpers --------------------------------------------------------

/// A burnt-orange separator line under a section header
/// (the FXML `<Separator>` styling).
fn section_separator(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 2.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::BURNT_ORANGE);
    ui.add_space(6.0);
}

/// Wraps a section column in a filled frame (the `.section-box` /
/// `.filter-section-box` CSS classes), returning the closure's changed flag.
fn section_box(
    ui: &mut egui::Ui,
    fill: egui::Color32,
    add_contents: impl FnOnce(&mut egui::Ui) -> bool,
) -> bool {
    egui::Frame::new()
        .fill(fill)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            add_contents(ui)
        })
        .inner
}

/// A labelled parameter slider with a live value readout, mirroring the
/// FXML label + value-readout + slider rows. Returns `true` when edited.
fn parameter_slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    format: impl Fn(f64) -> String,
) -> bool {
    ui.horizontal(|ui| {
        ui.label(theme::parameter_label(label));
        ui.label(theme::value_readout(&format(*value)));
    });
    ui.spacing_mut().slider_width = (ui.available_width() - 8.0).max(60.0);
    ui.add(egui::Slider::new(value, range).show_value(false))
        .changed()
}

/// The four ADSR sliders shared by the amp/filter envelope sections.
fn adsr_sliders(ui: &mut egui::Ui, adsr: &mut Adsr) -> bool {
    let mut changed = false;
    changed |= parameter_slider(ui, "ATTACK", &mut adsr.attack, 0.001..=5.0, |v| {
        format!("{v:.3} s")
    });
    ui.add_space(6.0);
    changed |= parameter_slider(ui, "DECAY", &mut adsr.decay, 0.001..=5.0, |v| {
        format!("{v:.3} s")
    });
    ui.add_space(6.0);
    changed |= parameter_slider(ui, "SUSTAIN", &mut adsr.sustain, 0.0..=1.0, |v| {
        format!("{v:.2}")
    });
    ui.add_space(6.0);
    changed |= parameter_slider(ui, "RELEASE", &mut adsr.release, 0.001..=5.0, |v| {
        format!("{v:.3} s")
    });
    changed
}

/// A waveform choice box. Returns `true` when the selection changed.
fn waveform_choice_box(ui: &mut egui::Ui, id: &str, waveform: &mut Waveform) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width().min(220.0))
        .selected_text(waveform_label(*waveform))
        .show_ui(ui, |ui| {
            for candidate in WAVEFORMS {
                changed |= ui
                    .selectable_value(waveform, candidate, waveform_label(candidate))
                    .changed();
            }
        });
    changed
}

/// A device choice box. Returns the newly selected device name, if the user
/// picked a different one this frame.
fn device_choice_box(
    ui: &mut egui::Ui,
    id: &str,
    devices: &[String],
    selected: Option<&str>,
) -> Option<String> {
    let mut clicked: Option<String> = None;
    egui::ComboBox::from_id_salt(id)
        .width(280.0)
        .selected_text(selected.unwrap_or("(none)"))
        .show_ui(ui, |ui| {
            if devices.is_empty() {
                ui.label("No devices found");
            }
            for device in devices {
                let is_selected = Some(device.as_str()) == selected;
                if ui.selectable_label(is_selected, device).clicked() && !is_selected {
                    clicked = Some(device.clone());
                }
            }
        });
    clicked
}
