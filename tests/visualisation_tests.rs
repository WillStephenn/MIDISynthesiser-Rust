//! Tests for the console ASCII renderer (`visualisation::ascii_renderer`).
//!
//! `render_to_string` exists precisely so the rendered frame can be asserted
//! on without a console. These tests check that the formatted block reflects
//! the synthesiser's *current* parameters and active-note state through the
//! public API -- not the exact layout/whitespace of the template, which is
//! free to change.

use midi_synthesiser::core::synthesiser::{Synthesiser, Waveform};
use midi_synthesiser::utils::audio_constants::{BLOCK_SIZE, NUMBER_OF_VOICES, SAMPLE_RATE};
use midi_synthesiser::visualisation::ascii_renderer::render_to_string;

fn fresh_synth() -> Synthesiser {
    Synthesiser::new(NUMBER_OF_VOICES, SAMPLE_RATE, BLOCK_SIZE)
}

/// With no notes active, every one of the 12 piano-key slots must be shown
/// "off" (`[ ]`) and none "on" (`[#]`).
#[test]
fn no_active_notes_renders_all_keys_off() {
    let synth = fresh_synth();
    let frame = render_to_string(&synth);

    assert_eq!(
        frame.matches("[ ]").count(),
        12,
        "all 12 piano-key slots should be rendered off when no notes are active:\n{frame}"
    );
    assert_eq!(
        frame.matches("[#]").count(),
        0,
        "no piano-key slot should be rendered on when no notes are active:\n{frame}"
    );
}

/// Triggering a note must light up exactly the corresponding pitch-class slot
/// (note number mod 12), and nothing else, while the note is in its
/// attack/decay/sustain phase.
#[test]
fn active_note_lights_up_its_pitch_class_slot() {
    let mut synth = fresh_synth();
    // Middle C (60) is pitch class 0 -> the first slot ("C").
    synth.note_on(60, 1.0);

    let frame = render_to_string(&synth);

    assert_eq!(
        frame.matches("[#]").count(),
        1,
        "exactly one piano-key slot should be lit for one active note:\n{frame}"
    );
    assert_eq!(
        frame.matches("[ ]").count(),
        11,
        "the other 11 piano-key slots should remain off:\n{frame}"
    );
}

/// A note that has been released and finished its release tail returns to
/// "off" in the rendered keyboard, even though the voice may briefly still be
/// rendered as `is_active()` during the release ramp -- the renderer uses
/// `is_active_no_release`, so a fully-idle voice never lights its key.
#[test]
fn idle_after_release_renders_key_off_again() {
    let mut synth = fresh_synth();
    synth.set_amp_release_time(0.0); // instant release -> Idle on the next block
    synth.note_on(60, 1.0);
    synth.note_off(60);

    // Drive one block so the envelope settles to Idle.
    let mut buffer = vec![0.0_f64; BLOCK_SIZE * 2];
    synth.process_block(&mut buffer);

    let frame = render_to_string(&synth);
    assert_eq!(
        frame.matches("[#]").count(),
        0,
        "a released, idle note should not be shown as active:\n{frame}"
    );
}

/// Two simultaneous notes that share a pitch class (an octave apart) light
/// only the single shared slot once, while two notes in different pitch
/// classes light two distinct slots.
#[test]
fn multiple_active_notes_light_distinct_or_shared_slots() {
    // Two notes an octave apart share pitch class 0 ("C").
    let mut synth = fresh_synth();
    synth.note_on(48, 1.0); // C3
    synth.note_on(60, 1.0); // C4 (same pitch class)
    let frame = render_to_string(&synth);
    assert_eq!(
        frame.matches("[#]").count(),
        1,
        "notes an octave apart share one pitch-class slot:\n{frame}"
    );

    // A third note in a different pitch class lights a second slot.
    let mut synth = fresh_synth();
    synth.note_on(60, 1.0); // C4 -> pitch class 0
    synth.note_on(64, 1.0); // E4 -> pitch class 4
    let frame = render_to_string(&synth);
    assert_eq!(
        frame.matches("[#]").count(),
        2,
        "notes in different pitch classes light distinct slots:\n{frame}"
    );
}

/// Filling the entire voice pool with notes spanning at least 12 distinct
/// pitch classes lights every slot.
#[test]
fn full_voice_pool_with_all_pitch_classes_lights_every_key() {
    let mut synth = fresh_synth();
    for i in 0..NUMBER_OF_VOICES {
        synth.note_on(48 + i as u8, 1.0);
    }
    let frame = render_to_string(&synth);

    let lit = frame.matches("[#]").count();
    let expected = NUMBER_OF_VOICES.min(12);
    assert_eq!(
        lit, expected,
        "with a full voice pool spanning >= 12 pitches, every reachable \
         pitch-class slot (up to all 12) should be lit:\n{frame}"
    );
}

/// The rendered block must reflect the synth's currently-selected oscillator
/// and LFO waveforms by name.
#[test]
fn waveform_names_are_reflected() {
    let mut synth = fresh_synth();

    synth.set_oscillator_waveform(Waveform::Triangle);
    synth.set_lfo_waveform(Waveform::Saw);
    let frame = render_to_string(&synth);
    assert!(
        frame.contains("TRIANGLE"),
        "oscillator waveform name should appear in the rendered frame:\n{frame}"
    );
    assert!(
        frame.contains("SAW"),
        "LFO waveform name should appear in the rendered frame:\n{frame}"
    );

    synth.set_oscillator_waveform(Waveform::Square);
    synth.set_lfo_waveform(Waveform::Triangle);
    let frame = render_to_string(&synth);
    assert!(frame.contains("SQUARE"), "{frame}");
    assert!(frame.contains("TRIANGLE"), "{frame}");
}

/// Numeric envelope, filter, gain, LFO and pan parameters set via the public
/// setters are reflected in the rendered frame -- the renderer reads the
/// master patch fields directly, so this should be visible without a
/// `process_block` call.
#[test]
fn parameter_changes_are_reflected_in_rendered_frame() {
    let mut synth = fresh_synth();

    synth.set_amp_attack_time(0.25);
    synth.set_amp_sustain_level(0.75);
    synth.set_filter_cutoff(2500.0);
    synth.set_filter_resonance(7.5);
    synth.set_lfo_frequency(3.0);
    synth.set_pan_depth(0.6);

    let frame = render_to_string(&synth);

    // Attack time formatted with two decimal places (e.g. "0.25").
    assert!(
        frame.contains("0.25"),
        "amp attack time should be reflected:\n{frame}"
    );
    // Sustain level rendered as a whole-number percentage.
    assert!(
        frame.contains("75%"),
        "amp sustain level should be reflected as a percentage:\n{frame}"
    );
    // Filter cutoff rendered with no decimal places.
    assert!(
        frame.contains("2500"),
        "filter cutoff should be reflected:\n{frame}"
    );
    // Filter resonance rendered with one decimal place.
    assert!(
        frame.contains("7.5"),
        "filter resonance should be reflected:\n{frame}"
    );
    // LFO frequency rendered with one decimal place.
    assert!(
        frame.contains("3.0"),
        "LFO frequency should be reflected:\n{frame}"
    );
    // Pan depth rendered as a whole-number percentage.
    assert!(
        frame.contains("60%"),
        "pan depth should be reflected as a percentage:\n{frame}"
    );
}

/// `render_to_string` must always produce the fixed set of section headers
/// regardless of synth state, so callers relying on the block's overall shape
/// (e.g. for a fixed-size terminal redraw) are not surprised.
#[test]
fn rendered_frame_contains_expected_section_headers() {
    let synth = fresh_synth();
    let frame = render_to_string(&synth);

    for header in [
        "OSCILLATOR:",
        "AMP ENVELOPE:",
        "FILTER:",
        "LFO:",
        "KEYBOARD:",
    ] {
        assert!(
            frame.contains(header),
            "rendered frame missing expected section header {header:?}:\n{frame}"
        );
    }
}
