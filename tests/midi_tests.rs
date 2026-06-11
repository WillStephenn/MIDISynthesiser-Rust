//! Integration tests for MIDI input handling and file playback.
//!
//! Tests verify the behaviour of `MidiInputHandler` receiving raw MIDI byte
//! sequences and the observable state changes in `Synthesiser`, plus MIDI file
//! playback via `MidiFilePlayer`. All tests are behaviour-driven, using public
//! APIs only.

use midi_synthesiser::core::synthesiser::{Synthesiser, Waveform};
use midi_synthesiser::midi::midi_file_player::MidiFilePlayer;
use midi_synthesiser::midi::midi_input_handler::MidiInputHandler;
use midi_synthesiser::utils::audio_constants::{BLOCK_SIZE, NUMBER_OF_VOICES, SAMPLE_RATE};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// --- Helper Functions ---

/// Constructs a fresh synthesiser with default configuration.
fn fresh_synth() -> Arc<Mutex<Synthesiser>> {
    Arc::new(Mutex::new(Synthesiser::new(
        NUMBER_OF_VOICES,
        SAMPLE_RATE,
        BLOCK_SIZE,
    )))
}

/// Extracts active notes from the synthesiser into a Vec for easier testing.
fn get_active_notes(synth: &Arc<Mutex<Synthesiser>>) -> Vec<u8> {
    let synth_guard = synth.lock().unwrap();
    let mut notes = vec![0u8; NUMBER_OF_VOICES];
    let count = synth_guard.get_active_notes(&mut notes);
    notes.truncate(count);
    notes.sort_unstable();
    notes
}

/// Sends a raw MIDI byte sequence through the handler.
fn send_midi(handler: &MidiInputHandler, bytes: &[u8]) {
    handler.send(bytes);
}

// --- Note On/Off Tests ---

#[test]
fn note_on_adds_note_to_active() {
    let synth = fresh_synth();
    let handler = MidiInputHandler::new(synth.clone());

    // MIDI note on: status=0x90, pitch=60, velocity=100
    send_midi(&handler, &[0x90, 60, 100]);

    let active = get_active_notes(&synth);
    assert_eq!(active, vec![60], "note-on should add note to active list");
}

#[test]
fn note_off_removes_note_from_active() {
    let synth = fresh_synth();
    let handler = MidiInputHandler::new(synth.clone());

    // Turn note on
    send_midi(&handler, &[0x90, 60, 100]);
    assert!(
        get_active_notes(&synth).contains(&60),
        "note-on should make note active"
    );

    // Turn note off: status=0x80, pitch=60
    send_midi(&handler, &[0x80, 60, 0]);

    // Note release phase is immediate in the default patch (0.4s), but we're
    // checking the public API definition: is_active_no_release() excludes
    // release-stage notes. So a note-off transitions to release, removing it
    // from get_active_notes.
    let active = get_active_notes(&synth);
    assert!(
        !active.contains(&60),
        "note-off should remove note from active"
    );
}

#[test]
fn note_on_with_zero_velocity_is_note_off() {
    let synth = fresh_synth();
    let handler = MidiInputHandler::new(synth.clone());

    // Turn note on
    send_midi(&handler, &[0x90, 60, 100]);
    assert!(
        get_active_notes(&synth).contains(&60),
        "note-on with velocity 100 should make note active"
    );

    // Note on with velocity 0: should behave as note off
    send_midi(&handler, &[0x90, 60, 0]);

    let active = get_active_notes(&synth);
    assert!(
        !active.contains(&60),
        "note-on with velocity 0 should be treated as note-off"
    );
}

#[test]
fn multiple_notes_all_active() {
    let synth = fresh_synth();
    let handler = MidiInputHandler::new(synth.clone());

    // Turn on multiple notes
    send_midi(&handler, &[0x90, 36, 80]);
    send_midi(&handler, &[0x90, 48, 90]);
    send_midi(&handler, &[0x90, 60, 100]);

    let active = get_active_notes(&synth);
    assert_eq!(
        active.len(),
        3,
        "three notes should be active after three note-ons"
    );
    assert_eq!(active, vec![36, 48, 60], "all three notes should be active");
}

#[test]
fn retrigger_held_note() {
    let synth = fresh_synth();
    let handler = MidiInputHandler::new(synth.clone());

    // Turn on note 60
    send_midi(&handler, &[0x90, 60, 80]);
    assert_eq!(get_active_notes(&synth), vec![60]);

    // Retrigger the same note with different velocity (note_on without note-off first)
    send_midi(&handler, &[0x90, 60, 100]);
    assert_eq!(
        get_active_notes(&synth),
        vec![60],
        "retringing a note should keep it in active list"
    );
}

#[test]
fn note_on_different_midi_channels() {
    let synth = fresh_synth();
    let handler = MidiInputHandler::new(synth.clone());

    // MIDI status bytes include channel in the lower nibble.
    // Channel 0 (status = 0x90), channel 1 (status = 0x91), ..., channel 15 (status = 0x9F)
    // The handler's send() masks with 0xF0, so all channels are treated identically.
    send_midi(&handler, &[0x90, 60, 100]); // Channel 0
    send_midi(&handler, &[0x91, 61, 100]); // Channel 1
    send_midi(&handler, &[0x9F, 62, 100]); // Channel 15

    let active = get_active_notes(&synth);
    assert_eq!(
        active.len(),
        3,
        "notes on different MIDI channels should all be active"
    );
}

#[test]
fn boundary_pitches() {
    let synth = fresh_synth();
    let handler = MidiInputHandler::new(synth.clone());

    // Pitch 0 and 127 are MIDI boundaries
    send_midi(&handler, &[0x90, 0, 100]);
    send_midi(&handler, &[0x90, 127, 100]);

    let active = get_active_notes(&synth);
    assert!(active.contains(&0), "pitch 0 should be valid");
    assert!(active.contains(&127), "pitch 127 should be valid");
}

#[test]
fn malformed_messages_do_not_panic() {
    let synth = fresh_synth();
    let handler = MidiInputHandler::new(synth.clone());

    // Empty message
    handler.send(&[]);
    // Too short (< 3 bytes)
    handler.send(&[0x90, 60]);

    // A message with extra bytes still processes the first 3 correctly
    handler.send(&[0x90, 60, 100, 200, 255]); // Extra bytes ignored, but first 3 are valid

    // Verify synth is still in usable state and that the above note was triggered
    send_midi(&handler, &[0x90, 48, 100]);
    let active = get_active_notes(&synth);
    assert!(
        active.contains(&48) && active.contains(&60),
        "malformed messages should not crash; note 48 and 60 should both be active"
    );
}

// --- CC (Control Change) Tests ---

/// Table-driven CC mapping tests.
///
/// Each row is: (cc_number, value_byte, getter_name, expected_value, tolerance)
/// Expected values account for spec conversions and documented clamping.
#[test]
fn control_change_mapping_table() {
    type CCTestCase = (u8, u8, fn(&Synthesiser) -> f64, f64, f64);

    let test_cases: Vec<CCTestCase> = vec![
        // Filter envelope controls (scaled = value / 127)
        // CC1: filter attack = scaled * 10 seconds
        (1, 0, |s| s.filter_attack_time(), 0.0, 1e-9),
        (1, 127, |s| s.filter_attack_time(), 10.0, 1e-9),
        (
            1,
            64,
            |s| s.filter_attack_time(),
            (64.0 / 127.0) * 10.0,
            1e-9,
        ),
        // CC2: filter decay = scaled * 10 seconds
        (2, 0, |s| s.filter_decay_time(), 0.0, 1e-9),
        (2, 127, |s| s.filter_decay_time(), 10.0, 1e-9),
        (
            2,
            64,
            |s| s.filter_decay_time(),
            (64.0 / 127.0) * 10.0,
            1e-9,
        ),
        // CC3: filter sustain = scaled (clamped [0, 1])
        (3, 0, |s| s.filter_sustain_level(), 0.0, 1e-9),
        (3, 127, |s| s.filter_sustain_level(), 1.0, 1e-9),
        (3, 64, |s| s.filter_sustain_level(), 64.0 / 127.0, 1e-9),
        // CC4: filter release = scaled * 10 seconds
        (4, 0, |s| s.filter_release_time(), 0.0, 1e-9),
        (4, 127, |s| s.filter_release_time(), 10.0, 1e-9),
        (
            4,
            64,
            |s| s.filter_release_time(),
            (64.0 / 127.0) * 10.0,
            1e-9,
        ),
        // Amplitude envelope controls
        // CC5: amp attack = scaled * 10 seconds
        (5, 0, |s| s.amp_attack_time(), 0.0, 1e-9),
        (5, 127, |s| s.amp_attack_time(), 10.0, 1e-9),
        (5, 64, |s| s.amp_attack_time(), (64.0 / 127.0) * 10.0, 1e-9),
        // CC6: amp release = scaled * 10 seconds (note the unusual order: 5=attack, 8=decay, 6=release)
        (6, 0, |s| s.amp_release_time(), 0.0, 1e-9),
        (6, 127, |s| s.amp_release_time(), 10.0, 1e-9),
        (6, 64, |s| s.amp_release_time(), (64.0 / 127.0) * 10.0, 1e-9),
        // CC7: amp sustain = scaled (clamped [0, 1])
        (7, 0, |s| s.amp_sustain_level(), 0.0, 1e-9),
        (7, 127, |s| s.amp_sustain_level(), 1.0, 1e-9),
        (7, 64, |s| s.amp_sustain_level(), 64.0 / 127.0, 1e-9),
        // CC8: amp decay = scaled * 10 seconds
        (8, 0, |s| s.amp_decay_time(), 0.0, 1e-9),
        (8, 127, |s| s.amp_decay_time(), 10.0, 1e-9),
        (8, 64, |s| s.amp_decay_time(), (64.0 / 127.0) * 10.0, 1e-9),
        // CC9: master volume = scaled (clamped [0, 1])
        (9, 0, |s| s.master_volume_scalar(), 0.0, 1e-9),
        (9, 127, |s| s.master_volume_scalar(), 1.0, 1e-9),
        (9, 64, |s| s.master_volume_scalar(), 64.0 / 127.0, 1e-9),
        // CC10: cutoff frequency (logarithmic: 20 * (20000/20)^scaled Hz)
        // log scale from 20 Hz to 20 kHz
        (10, 0, |s| s.filter_cutoff(), 20.0, 0.1),
        (10, 127, |s| s.filter_cutoff(), 20000.0, 1.0),
        (
            10,
            64,
            |s| s.filter_cutoff(),
            20.0 * (1000.0_f64).powf(64.0 / 127.0),
            1.0,
        ),
        // CC11: resonance = 1 + scaled * 14 (clamped [1, 20])
        (11, 0, |s| s.filter_resonance(), 1.0, 1e-9),
        (11, 127, |s| s.filter_resonance(), 15.0, 1e-9), // 1 + 1.0 * 14
        (
            11,
            64,
            |s| s.filter_resonance(),
            1.0 + (64.0 / 127.0) * 14.0,
            1e-9,
        ),
        // CC12: mod range = scaled * 10000 Hz (note: will clamp to below Nyquist)
        (12, 0, |s| s.filter_mod_range(), 0.0, 1e-9),
        (12, 127, |s| s.filter_mod_range(), 10000.0, 1.0), // May clamp to below Nyquist-cutoff
        // CC14: pre-filter gain = scaled * 48 - 24 dB (range [-24, 24] dB)
        (14, 0, |s| s.pre_filter_gain_db(), -24.0, 1e-9),
        (14, 127, |s| s.pre_filter_gain_db(), 24.0, 1e-9),
        (
            14,
            64,
            |s| s.pre_filter_gain_db(),
            (64.0 / 127.0) * 48.0 - 24.0,
            1e-9,
        ),
        // CC15: post-filter gain = scaled * 48 - 24 dB (range [-24, 24] dB)
        (15, 0, |s| s.post_filter_gain_db(), -24.0, 1e-9),
        (15, 127, |s| s.post_filter_gain_db(), 24.0, 1e-9),
        (
            15,
            64,
            |s| s.post_filter_gain_db(),
            (64.0 / 127.0) * 48.0 - 24.0,
            1e-9,
        ),
        // CC16: pan depth = scaled (clamped [0, 1])
        (16, 0, |s| s.pan_depth(), 0.0, 1e-9),
        (16, 127, |s| s.pan_depth(), 1.0, 1e-9),
        (16, 64, |s| s.pan_depth(), 64.0 / 127.0, 1e-9),
        // CC32: LFO frequency = 0.1 + scaled * 9.9 Hz (range [0.1, 10.0])
        (32, 0, |s| s.lfo_frequency(), 0.1, 1e-9),
        (32, 127, |s| s.lfo_frequency(), 10.0, 1e-9),
        (
            32,
            64,
            |s| s.lfo_frequency(),
            0.1 + (64.0 / 127.0) * 9.9,
            1e-9,
        ),
    ];

    for (cc, value_byte, getter, expected, tolerance) in test_cases {
        let synth = fresh_synth();
        let handler = MidiInputHandler::new(synth.clone());

        // Send control change: status=0xB0 (CC on channel 0), controller, value
        send_midi(&handler, &[0xB0, cc, value_byte]);

        let synth_guard = synth.lock().unwrap();
        let actual = getter(&synth_guard);
        let diff = (actual - expected).abs();
        assert!(
            diff <= tolerance,
            "CC {} with value {} should set to {}, got {} (diff: {})",
            cc,
            value_byte,
            expected,
            actual,
            diff
        );
    }
}

#[test]
fn oscillator_waveform_cc13_quartiles() {
    let test_cases = vec![
        (0, Waveform::Sine),      // 0-31 => Sine
        (31, Waveform::Sine),     // 0-31 => Sine (boundary)
        (32, Waveform::Saw),      // 32-63 => Saw
        (63, Waveform::Saw),      // 32-63 => Saw (boundary)
        (64, Waveform::Triangle), // 64-95 => Triangle
        (95, Waveform::Triangle), // 64-95 => Triangle (boundary)
        (96, Waveform::Square),   // 96-127 => Square
        (127, Waveform::Square),  // 96-127 => Square
    ];

    for (value_byte, expected_waveform) in test_cases {
        let synth = fresh_synth();
        let handler = MidiInputHandler::new(synth.clone());

        // CC13: oscillator waveform
        send_midi(&handler, &[0xB0, 13, value_byte]);

        let synth_guard = synth.lock().unwrap();
        assert_eq!(
            synth_guard.waveform(),
            expected_waveform,
            "CC 13 value {} should set waveform to {:?}",
            value_byte,
            expected_waveform
        );
    }
}

#[test]
fn lfo_waveform_cc17_quartiles() {
    let test_cases = vec![
        (0, Waveform::Sine), // 0-31 => Sine
        (31, Waveform::Sine),
        (32, Waveform::Saw), // 32-63 => Saw
        (63, Waveform::Saw),
        (64, Waveform::Triangle), // 64-95 => Triangle
        (95, Waveform::Triangle),
        (96, Waveform::Square), // 96-127 => Square
        (127, Waveform::Square),
    ];

    for (value_byte, expected_waveform) in test_cases {
        let synth = fresh_synth();
        let handler = MidiInputHandler::new(synth.clone());

        // CC17: LFO waveform
        send_midi(&handler, &[0xB0, 17, value_byte]);

        let synth_guard = synth.lock().unwrap();
        assert_eq!(
            synth_guard.lfo_waveform(),
            expected_waveform,
            "CC 17 value {} should set LFO waveform to {:?}",
            value_byte,
            expected_waveform
        );
    }
}

#[test]
fn unrecognized_cc_does_not_change_state() {
    let synth = fresh_synth();
    let handler = MidiInputHandler::new(synth.clone());

    let synth_guard = synth.lock().unwrap();
    let original_cutoff = synth_guard.filter_cutoff();
    let original_volume = synth_guard.master_volume_scalar();
    drop(synth_guard);

    // Send unrecognized CC (e.g., CC 100, which is not in the map)
    send_midi(&handler, &[0xB0, 100, 127]);

    let synth_guard = synth.lock().unwrap();
    assert_eq!(synth_guard.filter_cutoff(), original_cutoff);
    assert_eq!(synth_guard.master_volume_scalar(), original_volume);
}

#[test]
fn control_change_callback_fires_for_recognized_cc() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let synth = fresh_synth();
    let callback_fired = Arc::new(AtomicBool::new(false));
    let callback_fired_clone = callback_fired.clone();

    let handler = MidiInputHandler::with_control_change_callback(
        synth.clone(),
        Box::new(move || {
            callback_fired_clone.store(true, Ordering::SeqCst);
        }),
    );

    // Reset flag and send a recognized CC
    callback_fired.store(false, Ordering::SeqCst);
    send_midi(&handler, &[0xB0, 9, 100]); // CC9 is recognized (master volume)
    assert!(
        callback_fired.load(Ordering::SeqCst),
        "callback should fire for recognized CC"
    );

    // Reset flag and send an unrecognized CC
    callback_fired.store(false, Ordering::SeqCst);
    send_midi(&handler, &[0xB0, 100, 100]); // CC100 is not recognized
    assert!(
        !callback_fired.load(Ordering::SeqCst),
        "callback should NOT fire for unrecognized CC"
    );
}

// --- MIDI File Playback Tests ---

/// Polls `cond` every 5 ms until it returns true or `timeout` elapses.
/// Returns whether the condition was met — assert on the result.
fn wait_for(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    cond()
}

/// Generates a minimal MIDI file (in memory as bytes) with two notes.
/// Returns the byte buffer in Standard MIDI File format.
fn generate_minimal_midi_file() -> Vec<u8> {
    // Hand-assembled minimal MIDI file:
    // - File header (MThd): format 0, 1 track, 480 ticks per quarter note
    // - Track header (MTrk): track data
    // - Note on events: C4 (pitch 60) at tick 0, D4 (pitch 62) at tick 480
    // - Note off events: C4 at tick 480, D4 at tick 960
    // - End of track

    let mut file = Vec::new();

    // MThd header: "MThd", length 6, format 0, 1 track, 480 ticks/qn
    file.extend_from_slice(b"MThd");
    file.extend_from_slice(&[0, 0, 0, 6]); // Header length = 6
    file.extend_from_slice(&[0, 0]); // Format 0
    file.extend_from_slice(&[0, 1]); // 1 track
    file.extend_from_slice(&[1, 224]); // 480 ticks per quarter note

    // MTrk header
    let mut track = vec![
        0x00, // Delta time 0
        0x90, // Note On, channel 0
        60,   // Pitch
        100,  // Velocity
    ];

    // Event 2: Note On D4 (pitch 62), velocity 90, at delta time 480 (one quarter note)
    track.extend_from_slice(&[0x83, 0x60]); // Variable-length delta time: 480
    track.push(0x90); // Note On, channel 0
    track.push(62); // Pitch
    track.push(90); // Velocity

    // Event 3: Note Off C4, at delta time 480
    track.extend_from_slice(&[0x83, 0x60]); // Delta time 480
    track.push(0x80); // Note Off, channel 0
    track.push(60); // Pitch
    track.push(0); // Velocity (ignored)

    // Event 4: Note Off D4, at delta time 480
    track.extend_from_slice(&[0x83, 0x60]); // Delta time 480
    track.push(0x80); // Note Off, channel 0
    track.push(62); // Pitch
    track.push(0);

    // End of Track meta event
    track.push(0x00); // Delta time 0
    track.push(0xFF); // Meta event
    track.push(0x2F); // End of Track
    track.push(0x00); // Length 0

    // MTrk header: "MTrk", length
    file.extend_from_slice(b"MTrk");
    let track_len = track.len() as u32;
    file.extend_from_slice(&track_len.to_be_bytes());
    file.extend_from_slice(&track);

    file
}

#[test]
fn midi_file_player_returns_some_for_valid_file() {
    let synth = fresh_synth();
    let player = MidiFilePlayer::new(synth);

    // Create a temporary MIDI file
    let midi_bytes = generate_minimal_midi_file();
    let temp_path = "/tmp/test_valid.mid";
    std::fs::write(temp_path, &midi_bytes).expect("Failed to write MIDI file");

    let result = player.play_midi_file(temp_path);
    assert!(
        result.is_some(),
        "play_midi_file should return Some for a valid MIDI file"
    );

    // Clean up
    let _ = std::fs::remove_file(temp_path);
}

#[test]
fn midi_file_player_returns_none_for_nonexistent_file() {
    let synth = fresh_synth();
    let player = MidiFilePlayer::new(synth);

    let result = player.play_midi_file("/tmp/nonexistent_file_12345.mid");
    assert!(
        result.is_none(),
        "play_midi_file should return None for a nonexistent file"
    );
}

#[test]
fn midi_file_player_returns_none_for_garbage_file() {
    let synth = fresh_synth();
    let player = MidiFilePlayer::new(synth);

    let temp_path = "/tmp/test_garbage.mid";
    std::fs::write(temp_path, b"This is not a valid MIDI file").expect("Failed to write file");

    let result = player.play_midi_file(temp_path);
    assert!(
        result.is_none(),
        "play_midi_file should return None for invalid MIDI"
    );

    // Clean up
    let _ = std::fs::remove_file(temp_path);
}

#[test]
fn midi_file_player_empty_path_returns_none() {
    let synth = fresh_synth();
    let player = MidiFilePlayer::new(synth);

    let result = player.play_midi_file("");
    assert!(
        result.is_none(),
        "play_midi_file should return None for empty path"
    );
}

#[test]
fn midi_file_playback_completion() {
    let synth = fresh_synth();
    let player = MidiFilePlayer::new(synth.clone());

    // Create and write a minimal MIDI file
    let midi_bytes = generate_minimal_midi_file();
    let temp_path = "/tmp/test_playback.mid";
    std::fs::write(temp_path, &midi_bytes).expect("Failed to write MIDI file");

    let playback = player
        .play_midi_file(temp_path)
        .expect("Failed to start playback");

    // Verify playback is running
    assert!(
        playback.is_running(),
        "playback should be running immediately after starting"
    );

    // Wait for playback to complete (generous timeout)
    // The file has notes at 0ms, 500ms, 1000ms, 1500ms (at 480 ticks/qn and default 120 BPM)
    // Plus release time. Allow 5 seconds for safety.
    playback.wait();

    // After completion, playback should no longer be running
    // (Note: the atomic flag is set to false after the thread exits)
    // We don't re-check is_running here because the handle is consumed by wait().
    // Instead, we verify that the wait() completed without hanging.

    // Clean up
    let _ = std::fs::remove_file(temp_path);
}

#[test]
fn midi_file_playback_stop() {
    let synth = fresh_synth();
    let player = MidiFilePlayer::new(synth.clone());

    let midi_bytes = generate_minimal_midi_file();
    let temp_path = "/tmp/test_stop.mid";
    std::fs::write(temp_path, &midi_bytes).expect("Failed to write MIDI file");

    let playback = player
        .play_midi_file(temp_path)
        .expect("Failed to start playback");

    // The generated file plays for ~1.5 s, so it must still be running here.
    assert!(playback.is_running(), "playback should be running");

    // Stop playback and wait for the thread to notice the flag and exit.
    playback.stop();
    assert!(
        wait_for(Duration::from_secs(2), || !playback.is_running()),
        "playback should stop after stop()"
    );

    // Clean up
    let _ = std::fs::remove_file(temp_path);
}

#[test]
fn midi_file_playback_feeds_notes_through_handler() {
    let synth = fresh_synth();
    let player = MidiFilePlayer::new(synth.clone());

    let midi_bytes = generate_minimal_midi_file();
    let temp_path = "/tmp/test_notes.mid";
    std::fs::write(temp_path, &midi_bytes).expect("Failed to write MIDI file");

    let playback = player
        .play_midi_file(temp_path)
        .expect("Failed to start playback");

    // The first note-on is at tick 0 and is held for ~0.5 s; poll until it lands.
    assert!(
        wait_for(Duration::from_secs(2), || !get_active_notes(&synth)
            .is_empty()),
        "at least one note should have been triggered by playback"
    );

    // Stop and clean up
    playback.stop();
    playback.wait();
    let _ = std::fs::remove_file(temp_path);
}
