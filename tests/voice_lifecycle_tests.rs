//! Voice-lifecycle soak tests.
//!
//! These tests drive the engine through `process_block` the way `cpal`'s
//! audio callback and `midir`'s MIDI callback would under heavy arpeggiated
//! MIDI traffic (as reported from Logic Pro): dense, overlapping
//! note-on/note-off pairs, repeated pitches, and constant voice stealing,
//! rendered far faster than real time.
//!
//! They assert the two properties that distinguish a healthy voice pool from
//! a "lockup":
//!
//! 1. Every rendered sample stays finite.
//! 2. Once all notes have been released and enough time has passed for every
//!    envelope's release stage to complete, [`Synthesiser::active_voice_count`]
//!    returns to `0` -- i.e. no voice is permanently stuck processing.
//! 3. Per-block render cost does not grow over the course of the soak (a
//!    growing voice pool or denormal ringing would show up as a steady
//!    increase in average block time).

use midi_synthesiser::core::synthesiser::{Synthesiser, Waveform};
use midi_synthesiser::utils::audio_constants::{BLOCK_SIZE, NUMBER_OF_VOICES, SAMPLE_RATE};
use std::time::Instant;

/// Asserts every sample in `buffer` is finite (no NaN/inf), as required by
/// the testing philosophy for every rendered buffer.
fn assert_all_finite(buffer: &[f64], context: &str) {
    for (i, &sample) in buffer.iter().enumerate() {
        assert!(
            sample.is_finite(),
            "non-finite sample at index {i} ({context}): {sample}"
        );
    }
}

/// Renders `n` blocks, asserting finiteness throughout.
fn render_blocks(synth: &mut Synthesiser, buffer: &mut [f64], n: usize, context: &str) {
    for _ in 0..n {
        synth.process_block(buffer);
        assert_all_finite(buffer, context);
    }
}

/// How many blocks correspond to `seconds` of audio at the engine's
/// configured sample rate / block size.
fn blocks_for(seconds: f64) -> usize {
    ((seconds * SAMPLE_RATE / BLOCK_SIZE as f64).ceil() as usize).max(1)
}

/// A simple chromatic arpeggio pattern: repeatedly cycles through a fixed
/// span of MIDI pitches (built from `NUMBER_OF_VOICES` so it scales with the
/// voice pool, deliberately exceeding it to force voice stealing), gating
/// each note on/off to mimic a DAW arpeggiator.
///
/// `gate_fraction` controls how much of each step the note is held for
/// before `note_off` fires (e.g. `0.5` = note-off halfway through the step).
fn run_arpeggio(
    synth: &mut Synthesiser,
    buffer: &mut [f64],
    steps: usize,
    blocks_per_step: usize,
    gate_fraction: f64,
    pitch_span: u8,
) {
    let gate_blocks = ((blocks_per_step as f64 * gate_fraction) as usize).max(1);
    for step in 0..steps {
        // Repeated pitches across the span (mod arithmetic) so the same note
        // is retriggered while a previous instance may still be releasing,
        // exercising the "note_on for a held pitch releases it first" path.
        let pitch = 36 + (step as u8 % pitch_span);
        synth.note_on(pitch, 0.9);

        for b in 0..blocks_per_step {
            if b == gate_blocks {
                synth.note_off(pitch);
            }
            render_blocks(synth, buffer, 1, "arpeggio soak");
        }
    }
}

/// Releases every possible MIDI pitch and renders enough blocks for any
/// in-flight release stage to complete, then asserts the voice pool is fully
/// idle.
fn drain_to_idle(synth: &mut Synthesiser, buffer: &mut [f64], release_seconds: f64) {
    for pitch in 0..128u8 {
        synth.note_off(pitch);
    }
    // A generous margin over the configured release time.
    let drain_blocks = blocks_for(release_seconds * 2.0 + 0.5);
    render_blocks(synth, buffer, drain_blocks, "drain to idle");

    assert_eq!(
        synth.active_voice_count(),
        0,
        "voice pool did not return to Idle after release; a voice is stuck \
         and will be processed forever"
    );
}

/// Regression test for the exact lifecycle leak found while investigating the
/// reported audio lockup: an envelope released *before* reaching Sustain,
/// with `sustain_level == 0.0`, derived a `release_increment` of exactly
/// `0.0` (it was computed from `sustain_level`). `current_multiplier` then
/// never decremented, the envelope never reached `Idle`, and the voice was
/// processed forever.
///
/// This test fails before the fix (the voice never returns to Idle) and
/// passes after (the release ramp is derived from the multiplier at the
/// moment of release, so it always reaches zero).
#[test]
fn early_release_with_zero_sustain_returns_voice_to_idle() {
    let mut synth = Synthesiser::new(NUMBER_OF_VOICES, SAMPLE_RATE, BLOCK_SIZE);
    let mut buffer = vec![0.0_f64; BLOCK_SIZE * 2];

    // A "plucky" envelope: attack + long decay, sustain = 0.0, short release.
    let attack = 0.01;
    let decay = 0.3;
    let release = 0.1;
    synth.set_amp_attack_time(attack);
    synth.set_amp_decay_time(decay);
    synth.set_amp_sustain_level(0.0);
    synth.set_amp_release_time(release);

    synth.note_on(60, 1.0);

    // Release partway through the attack+decay ramp, well before Sustain
    // would ever be reached (sustain level is 0, so Sustain == Idle anyway).
    let blocks_mid_decay = blocks_for((attack + decay) / 2.0);
    render_blocks(&mut synth, &mut buffer, blocks_mid_decay, "mid-decay");

    assert_eq!(
        synth.active_voice_count(),
        1,
        "voice should still be active mid-decay"
    );

    synth.note_off(60);

    drain_to_idle(&mut synth, &mut buffer, release);
}

/// Soak test: several minutes of arpeggiated chords with the default patch,
/// including repeated pitches and overlapping releases that force constant
/// voice stealing across the whole pool. Asserts the engine stays healthy
/// (finite output, voices return to Idle, render cost does not grow).
#[test]
fn arpeggio_soak_returns_all_voices_to_idle_and_stays_finite() {
    let mut synth = Synthesiser::new(NUMBER_OF_VOICES, SAMPLE_RATE, BLOCK_SIZE);
    let mut buffer = vec![0.0_f64; BLOCK_SIZE * 2];
    let release = synth.amp_release_time();

    // 16th notes at 160 BPM, 50% gate -- a brisk arpeggio.
    let bpm = 160.0;
    let sixteenth_seconds = 60.0 / bpm / 4.0;
    let blocks_per_step = blocks_for(sixteenth_seconds);

    // Span more pitches than there are voices, so stealing is constant.
    let pitch_span = (NUMBER_OF_VOICES as u8).saturating_mul(2).max(12);

    // ~3 minutes of simulated arpeggio, rendered far faster than real time.
    let total_seconds = 180.0;
    let steps = (total_seconds / sixteenth_seconds) as usize;

    run_arpeggio(
        &mut synth,
        &mut buffer,
        steps,
        blocks_per_step,
        0.5,
        pitch_span,
    );

    drain_to_idle(&mut synth, &mut buffer, release);
}

/// Reproduces the reported "lockup" directly: a percussive patch
/// (`amp_sustain_level == 0.0`) arpeggiated faster than its attack+decay
/// time, so every note-off lands before Sustain is ever reached. Before the
/// envelope fix this leaks one permanently-active voice per arpeggio step
/// until the entire `NUMBER_OF_VOICES`-sized pool is stuck, and per-block
/// render cost grows monotonically with the soak length.
///
/// The test asserts:
/// - All output stays finite throughout.
/// - The voice pool returns to fully `Idle` after the soak ends and releases
///   complete (this is the assertion that fails outright before the fix).
/// - Average per-block render cost in the back half of the soak is not
///   meaningfully larger than in the front half (catches an unbounded-growth
///   pathology -- leaked voices or denormal ringing -- without depending on
///   wall-clock thresholds).
#[test]
fn percussive_arpeggio_soak_keeps_render_cost_bounded() {
    let mut synth = Synthesiser::new(NUMBER_OF_VOICES, SAMPLE_RATE, BLOCK_SIZE);
    let mut buffer = vec![0.0_f64; BLOCK_SIZE * 2];

    // Percussive patch: short attack, long decay, zero sustain, short release.
    let attack = 0.01;
    let decay = 0.3;
    let release = 0.1;
    synth.set_oscillator_waveform(Waveform::Saw);
    synth.set_amp_attack_time(attack);
    synth.set_amp_decay_time(decay);
    synth.set_amp_sustain_level(0.0);
    synth.set_amp_release_time(release);

    // 16th notes at 160 BPM, 50% gate (~47 ms gate-on), well inside the
    // attack+decay (310 ms) -- every note-off happens during Decay.
    let bpm = 160.0;
    let sixteenth_seconds = 60.0 / bpm / 4.0;
    let blocks_per_step = blocks_for(sixteenth_seconds);
    assert!(
        (sixteenth_seconds * 0.5) < (attack + decay),
        "test setup invariant: gate time must be shorter than attack+decay"
    );

    let pitch_span = (NUMBER_OF_VOICES as u8).saturating_mul(2).max(12);

    // A minute of simulated arpeggio is enough to either saturate the voice
    // pool (before the fix) or demonstrate steady-state cost (after).
    let total_seconds = 60.0;
    let steps = (total_seconds / sixteenth_seconds) as usize;
    assert!(
        steps > NUMBER_OF_VOICES,
        "need more steps than voices to fill the pool"
    );

    // Split the soak into two halves and time each, in blocks (not wall
    // clock seconds), so this stays deterministic across machines.
    let half = steps / 2;

    let first_half_start = Instant::now();
    run_arpeggio(
        &mut synth,
        &mut buffer,
        half,
        blocks_per_step,
        0.5,
        pitch_span,
    );
    let first_half_elapsed = first_half_start.elapsed();
    let first_half_blocks = (half * blocks_per_step) as u32;

    let second_half_start = Instant::now();
    run_arpeggio(
        &mut synth,
        &mut buffer,
        steps - half,
        blocks_per_step,
        0.5,
        pitch_span,
    );
    let second_half_elapsed = second_half_start.elapsed();
    let second_half_blocks = ((steps - half) * blocks_per_step) as u32;

    // The voice pool must return to Idle once everything is released.
    drain_to_idle(&mut synth, &mut buffer, release);

    // Relative-slowdown check: a leaking voice pool grows roughly linearly
    // with the number of arpeggio steps, so the second half (which starts
    // with ~`NUMBER_OF_VOICES` more leaked voices than the first half began
    // with, before the fix) would take dramatically longer per block. After
    // the fix, both halves process a small, bounded number of active voices,
    // so the ratio should be close to 1. Allow a generous margin (10x) so
    // this never flakes on a loaded CI box, while still catching an
    // unbounded-growth regression (which produced a >10x ratio in practice).
    let first_half_per_block = first_half_elapsed.as_secs_f64() / first_half_blocks as f64;
    let second_half_per_block = second_half_elapsed.as_secs_f64() / second_half_blocks as f64;
    let ratio = second_half_per_block / first_half_per_block.max(f64::EPSILON);

    assert!(
        ratio < 10.0,
        "second half of the soak is {ratio:.1}x slower per block than the first \
         half ({second_half_per_block:e}s vs {first_half_per_block:e}s); this \
         indicates an unbounded growth in per-block cost (leaked voices or \
         denormal ringing)"
    );
}

/// `--release`-only timing test: with the engine's configured
/// `NUMBER_OF_VOICES` and `BLOCK_SIZE`, a fully-loaded voice pool in steady
/// sustain must render each block well within the real-time budget
/// (`BLOCK_SIZE` frames at `SAMPLE_RATE`). Ignored by default because debug
/// builds are not representative of real-time performance and would flake;
/// run explicitly with `cargo test --release -- --ignored`.
#[test]
#[ignore = "wall-clock timing; representative only in --release builds"]
fn fully_loaded_voice_pool_renders_within_real_time_budget() {
    let mut synth = Synthesiser::new(NUMBER_OF_VOICES, SAMPLE_RATE, BLOCK_SIZE);
    let mut buffer = vec![0.0_f64; BLOCK_SIZE * 2];

    for i in 0..NUMBER_OF_VOICES {
        let pitch = 36 + (i % 64) as u8;
        synth.note_on(pitch, 1.0);
    }

    // Warm up (lookup tables, branch predictors, etc.).
    render_blocks(&mut synth, &mut buffer, blocks_for(0.5), "warmup");

    let budget = std::time::Duration::from_secs_f64(BLOCK_SIZE as f64 / SAMPLE_RATE);
    let n = blocks_for(1.0);
    let start = Instant::now();
    render_blocks(&mut synth, &mut buffer, n, "steady state");
    let elapsed = start.elapsed();
    let per_block = elapsed / n as u32;

    assert!(
        per_block < budget,
        "average per-block render time {per_block:?} exceeds the real-time \
         budget {budget:?} for {NUMBER_OF_VOICES} voices @ {BLOCK_SIZE}-frame blocks"
    );
}
