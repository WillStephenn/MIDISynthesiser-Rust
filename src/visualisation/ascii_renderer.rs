//! Renders the synthesiser's state as a formatted ASCII block in the console
//! (port of `synth.visualisation.AsciiRenderer`).
//!
//! The Java class held a static scratch array and printed directly; this port
//! additionally exposes [`render_to_string`] so callers sharing the
//! synthesiser behind a `Mutex` can format the frame under a brief lock and
//! print it after releasing the lock (keeping the audio thread unblocked).

use crate::core::synthesiser::{Synthesiser, Waveform};
use crate::utils::audio_constants::NUMBER_OF_VOICES;

/// Note names for the one-octave keyboard display.
const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Clears the console screen. This is used to create an animation effect by
/// clearing the previous frame before drawing the new one.
pub fn clear_console() {
    let status = if cfg!(windows) {
        // For Windows, run the 'cls' command
        std::process::Command::new("cmd")
            .args(["/c", "cls"])
            .status()
    } else {
        // For macOS and Linux, run the 'clear' command
        std::process::Command::new("clear").status()
    };
    if let Err(e) = status {
        eprintln!("Error clearing console: {e}");
    }
}

/// Renders the synthesiser's state as a formatted ASCII block, clearing the
/// console first (same behaviour as the Java `render`).
///
/// Callers holding the synthesiser behind a `Mutex` should prefer
/// [`render_to_string`] under the lock and print afterwards.
pub fn render(synth: &Synthesiser) {
    // Clear the console before printing the new state
    clear_console();
    println!("{}", render_to_string(synth));
}

/// Formats the synthesiser's state as the ASCII UI block, without printing.
pub fn render_to_string(synth: &Synthesiser) -> String {
    // Active Notes Rendering
    let mut active_notes = [0u8; NUMBER_OF_VOICES];
    let active_note_count = synth.get_active_notes(&mut active_notes);
    let mut piano_display = String::new();

    for i in 0..NOTE_NAMES.len() as u8 {
        let note_is_active = active_notes[..active_note_count]
            .iter()
            .any(|&note| note % 12 == i);
        // Use a brighter block character for active notes
        piano_display.push_str(if note_is_active { "[#]" } else { "[ ]" });
    }

    format!(
        "\
+================JUNE'S===LOGUE================+
|OSCILLATOR: {:<34}|
+----------------------------------------------+
|AMP ENVELOPE:                (time unit: s)   |
| ATTACK   DECAY    SUSTAIN   RELEASE          |
| {:<8.2} {:<8.2} {:<9} {:<8.2}         |
+----------------------------------------------+
|FILTER:                      (time unit: s)   |
| CUTOFF(Hz)   RESONANCE(Q)   MOD RANGE(Hz)    |
| {:<12.0} {:<6.1}         {:<17.0}|
|                                              |
| ATTACK   DECAY    SUSTAIN   RELEASE          |
| {:<8.2} {:<8.2} {:<9} {:<8.2}         |
|                                              |
| PRE-GAIN(db)      POST-GAIN(db)              |
| {:<17.1} {:<10.1}                 |
+----------------------------------------------+
|LFO:                                          |
| OSCILLATOR: {:<10} FREQUENCY(Hz):{:<8.1}|
|                                              |
| Pan Depth: {:<34}|
+----------------------------------------------+
|KEYBOARD: {:<35}|",
        waveform_name(synth.waveform()),
        synth.amp_attack_time(),
        synth.amp_decay_time(),
        format!("{:.0}%", synth.amp_sustain_level() * 100.0),
        synth.amp_release_time(),
        synth.filter_cutoff(),
        synth.filter_resonance(),
        synth.filter_mod_range(),
        synth.filter_attack_time(),
        synth.filter_decay_time(),
        format!("{:.0}%", synth.filter_sustain_level() * 100.0),
        synth.filter_release_time(),
        synth.pre_filter_gain_db(),
        synth.post_filter_gain_db(),
        waveform_name(synth.lfo_waveform()),
        synth.lfo_frequency(),
        format!("{:.0}%", synth.pan_depth() * 100.0),
        piano_display,
    )
}

/// Formats a waveform like the Java enum's `toString()` (e.g. `SINE`).
fn waveform_name(waveform: Waveform) -> &'static str {
    match waveform {
        Waveform::Sine => "SINE",
        Waveform::Saw => "SAW",
        Waveform::Triangle => "TRIANGLE",
        Waveform::Square => "SQUARE",
    }
}
