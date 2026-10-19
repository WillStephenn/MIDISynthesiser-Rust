//! Direct unit tests for `core::voice::Voice`.
//!
//! `Voice` is the engine's lower-level public API (see the testing
//! philosophy): unit-testing it directly, independent of `Synthesiser`'s
//! voice-allocation logic, is encouraged. These tests exercise note
//! lifecycle, waveform/pitch handling (pitch is applied to every oscillator, so
//! a waveform switch keeps the pitch), pan, gain staging, and the
//! amp/filter envelope facade methods, all observed via `process_block`'s
//! rendered output and the `is_active*`/getter public API.

use std::collections::HashMap;

use midi_synthesiser::core::audio_component::AudioComponent;
use midi_synthesiser::core::synthesiser::Waveform;
use midi_synthesiser::core::voice::Voice;
use midi_synthesiser::utils::audio_constants::{BLOCK_SIZE, SAMPLE_RATE};
use midi_synthesiser::utils::lookup_tables;

const SR: f64 = SAMPLE_RATE;

/// Asserts every sample in `buffer` is finite, as required for any rendered
/// buffer.
fn assert_all_finite(buffer: &[f64], context: &str) {
    for (i, &sample) in buffer.iter().enumerate() {
        assert!(
            sample.is_finite(),
            "non-finite sample at index {i} ({context}): {sample}"
        );
    }
}

/// Builds a voice with audible gain staging (0 dB pre/post) so
/// `process_block` produces non-zero output once a note is on. Fresh voices
/// default both gain stages to a multiplier of 0.0 (silence) until a patch is
/// applied.
fn audible_voice(waveform: Waveform, pitch_frequency: f64) -> Voice {
    let mut voice = Voice::new(waveform, pitch_frequency, SR, BLOCK_SIZE);
    voice.set_filter_gain_staging(0.0, 0.0); // 0 dB => multiplier 1.0
    voice
}

// --- Note lifecycle ---

/// `note_on`/`note_off`/`is_active`/`is_active_no_release` together describe
/// the voice lifecycle the voice pool relies on: a freshly constructed voice
/// is idle; `note_on` makes it active (and "active, not releasing"); a fast
/// `note_off` enters the release tail (`is_active` true, `is_active_no_release`
/// false); and once the release ramp completes the voice returns fully to
/// idle.
#[test]
fn note_lifecycle_trigger_release_and_return_to_idle() {
    let mut voice = audible_voice(Waveform::Sine, 220.0);
    voice.set_amp_envelope(0.0, 0.0, 1.0, 0.0); // instant attack/decay, full sustain, instant release
    voice.set_filter_envelope(0.0, 0.0, 1.0, 0.0);

    assert!(!voice.is_active(), "fresh voice should be idle");
    assert!(!voice.is_active_no_release(), "fresh voice should be idle");

    voice.note_on();
    assert!(voice.is_active(), "voice should be active after note_on");
    assert!(
        voice.is_active_no_release(),
        "voice should not be in release immediately after note_on"
    );

    // With a 0-second release time, note_off drops the envelope straight to
    // Idle (see Envelope::note_off), so the voice becomes fully inactive.
    voice.note_off();
    assert!(
        !voice.is_active(),
        "voice with instant release should return to idle immediately on note_off"
    );
    assert!(!voice.is_active_no_release());
}

/// A voice released with a non-zero release time enters the release tail
/// (`is_active` true, `is_active_no_release` false) and only becomes fully
/// idle once the release ramp completes.
#[test]
fn note_off_with_release_time_enters_release_then_idle() {
    let mut voice = audible_voice(Waveform::Sine, 220.0);
    let release = 0.01; // short but non-zero
    voice.set_amp_envelope(0.0, 0.0, 1.0, release);
    voice.set_filter_envelope(0.0, 0.0, 1.0, release);

    voice.note_on();
    let mut buffer = vec![0.0_f64; BLOCK_SIZE * 2];
    // Render one block so the envelope reaches Sustain (instant attack/decay).
    voice.process_block(None, &mut buffer, BLOCK_SIZE);
    assert!(voice.is_active_no_release());

    voice.note_off();
    assert!(
        voice.is_active(),
        "voice with a non-zero release time should still be active right after note_off"
    );
    assert!(
        !voice.is_active_no_release(),
        "a releasing voice is not 'active, no release'"
    );

    // Render enough blocks for the release ramp to complete.
    let release_samples = (release * SR).ceil() as usize;
    let release_blocks = release_samples.div_ceil(BLOCK_SIZE) + 1;
    for _ in 0..release_blocks {
        voice.process_block(None, &mut buffer, BLOCK_SIZE);
        assert_all_finite(&buffer, "release tail");
    }
    assert!(
        !voice.is_active(),
        "voice should return to idle once the release ramp completes"
    );
}

// --- oscillator_frequency / set_oscillator_pitch ---

/// `oscillator_frequency()` reflects the *construction-time* pitch frequency
/// only -- `set_oscillator_pitch` (which retargets the lookup-table phase
/// increment of the selected oscillator) does not update it. This documents
/// the getter's actual contract: it is not "the current pitch of the
/// oscillator", it is "the pitch the voice was constructed with".
#[test]
fn oscillator_frequency_reflects_construction_pitch_not_later_set_oscillator_pitch() {
    let construction_freq = lookup_tables::tables().midi_to_hz[57]; // A3, 220 Hz
    let mut voice = Voice::new(Waveform::Sine, construction_freq, SR, BLOCK_SIZE);
    assert_eq!(voice.oscillator_frequency(), construction_freq);

    // Retarget to a different MIDI pitch (A4, 440 Hz).
    voice.set_oscillator_pitch(69);
    assert_eq!(voice.pitch_midi(), 69);
    assert_eq!(
        voice.oscillator_frequency(),
        construction_freq,
        "oscillator_frequency() must remain the construction-time pitch \
         even after set_oscillator_pitch retargets the active oscillator"
    );
}

/// `set_oscillator_pitch` tunes all four oscillators, so switching the
/// active waveform afterwards plays at the voice's pitch rather than at the
/// untuned 0 Hz (flat DC) or a stale pitch from an earlier note.
///
/// Strategy: a voice built with Sine @ 220 Hz is retargeted to 440 Hz and
/// then switched to Saw; its output must match a fresh Saw voice built at
/// 440 Hz (both saw oscillators start at phase 0), and the same must hold
/// for Triangle when the pitch is set after the switch.
#[test]
fn set_oscillator_pitch_tunes_every_oscillator() {
    let a3 = lookup_tables::tables().midi_to_hz[57]; // 220 Hz
    let a4_midi = 69; // 440 Hz
    let a4 = lookup_tables::tables().midi_to_hz[a4_midi as usize];

    let mut voice_a = audible_voice(Waveform::Sine, a3);
    voice_a.set_oscillator_pitch(a4_midi);
    voice_a.set_oscillator_waveform(Waveform::Saw);
    let mut voice_b = audible_voice(Waveform::Saw, a4);

    let mut voice_e = audible_voice(Waveform::Sine, a3);
    voice_e.set_oscillator_waveform(Waveform::Triangle);
    voice_e.set_oscillator_pitch(a4_midi);
    let mut voice_f = audible_voice(Waveform::Triangle, a4);

    let mut outs = Vec::new();
    for voice in [&mut voice_a, &mut voice_b, &mut voice_e, &mut voice_f] {
        voice.note_on();
        let mut out = vec![0.0_f64; BLOCK_SIZE * 2];
        voice.process_block(None, &mut out, BLOCK_SIZE);
        assert_all_finite(&out, "retuned voice");
        outs.push(out);
    }
    assert_eq!(
        outs[0], outs[1],
        "pitch set while Sine was selected must carry to Saw"
    );
    assert_eq!(
        outs[2], outs[3],
        "pitch set while Triangle was selected must match a fresh Triangle voice"
    );
}

/// Counts sign changes of the left channel of an interleaved stereo buffer.
fn zero_crossings_left(stereo: &[f64]) -> usize {
    let mut crossings = 0;
    let mut prev = stereo[0] >= 0.0;
    for frame in stereo.chunks_exact(2).skip(1) {
        let cur = frame[0] >= 0.0;
        if cur != prev {
            crossings += 1;
        }
        prev = cur;
    }
    crossings
}

/// Holding a note and switching waveform must keep sounding a tone at the
/// held pitch (not DC, not silence, not a stale pitch), including switching
/// back to a waveform that played a different note earlier on this voice.
#[test]
fn waveform_switch_during_held_note_keeps_sounding_at_pitch() {
    const SECONDS: f64 = 0.25;
    let blocks = (SECONDS * SR / BLOCK_SIZE as f64) as usize;
    let expected_crossings = 2.0 * 440.0 * (blocks * BLOCK_SIZE) as f64 / SR;

    let waveforms = [
        Waveform::Sine,
        Waveform::Saw,
        Waveform::Triangle,
        Waveform::Square,
    ];
    for target in waveforms {
        let mut voice = audible_voice(Waveform::Square, 0.0);
        voice.set_amp_envelope(0.0, 0.0, 1.0, 0.1);
        // An earlier note on this voice leaves a stale pitch on the
        // oscillators when only the current one is tuned.
        voice.set_oscillator_pitch(48);
        voice.set_oscillator_waveform(target);
        voice.set_oscillator_pitch(48);
        voice.set_oscillator_waveform(Waveform::Square);

        voice.set_oscillator_pitch(69); // A4, 440 Hz
        voice.note_on();
        let mut out = vec![0.0_f64; BLOCK_SIZE * 2];
        voice.process_block(None, &mut out, BLOCK_SIZE);
        voice.set_oscillator_waveform(target);

        let mut rendered = Vec::new();
        for _ in 0..blocks {
            voice.process_block(None, &mut out, BLOCK_SIZE);
            assert_all_finite(&out, "after waveform switch");
            rendered.extend_from_slice(&out);
        }
        let left: Vec<f64> = rendered.chunks_exact(2).map(|f| f[0]).collect();
        let mean = left.iter().sum::<f64>() / left.len() as f64;
        let ac_rms =
            (left.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / left.len() as f64).sqrt();
        assert!(
            ac_rms > 0.05,
            "{target:?}: output after the switch is DC/silent (ac rms {ac_rms})"
        );
        let crossings = zero_crossings_left(&rendered) as f64;
        assert!(
            (crossings - expected_crossings).abs() <= expected_crossings * 0.05 + 2.0,
            "{target:?}: {crossings} zero crossings, expected about {expected_crossings} for 440 Hz"
        );
    }
}

// --- Waveform coverage in process_block ---

/// `process_block` renders finite, non-panicking output for every waveform
/// the voice can be configured with (covering each arm of the internal
/// oscillator-selection match).
#[test]
fn process_block_renders_for_every_waveform() {
    let waveforms = [
        Waveform::Sine,
        Waveform::Saw,
        Waveform::Triangle,
        Waveform::Square,
    ];

    for waveform in waveforms {
        let mut voice = audible_voice(waveform, 220.0);
        voice.set_amp_envelope(0.0, 0.0, 1.0, 0.1);
        voice.note_on();

        let mut buffer = vec![0.0_f64; BLOCK_SIZE * 2];
        voice.process_block(None, &mut buffer, BLOCK_SIZE);

        assert_all_finite(&buffer, &format!("{waveform:?} oscillator"));
        assert!(
            buffer.iter().any(|&s| s != 0.0),
            "{waveform:?} voice should produce non-silent output once active"
        );
    }
}

// --- Panning ---

/// `set_pan_position` redistributes the mono signal between the interleaved
/// stereo output channels according to an equal-power pan law: full left
/// silences the right channel (and vice versa), while centre keeps both
/// channels equal.
#[test]
fn pan_position_redistributes_stereo_channels() {
    fn channel_energy(buffer: &[f64]) -> (f64, f64) {
        let mut left = 0.0;
        let mut right = 0.0;
        for chunk in buffer.chunks_exact(2) {
            left += chunk[0] * chunk[0];
            right += chunk[1] * chunk[1];
        }
        (left, right)
    }

    // Centre pan: equal energy in both channels.
    let mut centre = audible_voice(Waveform::Square, 220.0);
    centre.set_amp_envelope(0.0, 0.0, 1.0, 0.1);
    centre.set_pan_position(0.0);
    centre.note_on();
    let mut centre_buf = vec![0.0_f64; BLOCK_SIZE * 2];
    centre.process_block(None, &mut centre_buf, BLOCK_SIZE);
    let (l, r) = channel_energy(&centre_buf);
    assert!(
        l > 0.0 && r > 0.0,
        "centre pan should sound on both channels"
    );
    assert!(
        (l - r).abs() < 1e-9,
        "centre pan should split energy equally: left={l}, right={r}"
    );

    // Full left: right channel must be (effectively) silent compared to the
    // left. The equal-power pan law derives gains from the lookup tables, so
    // the "off" channel may carry a floating-point residue on the order of
    // the sine/cosine table's quantization rather than being bit-exact zero.
    let mut left = audible_voice(Waveform::Square, 220.0);
    left.set_amp_envelope(0.0, 0.0, 1.0, 0.1);
    left.set_pan_position(-1.0);
    left.note_on();
    let mut left_buf = vec![0.0_f64; BLOCK_SIZE * 2];
    left.process_block(None, &mut left_buf, BLOCK_SIZE);
    let (l, r) = channel_energy(&left_buf);
    assert!(l > 0.0, "full-left pan should sound on the left channel");
    assert!(
        r < l * 1e-6,
        "full-left pan should leave the right channel negligible relative \
         to the left: left={l}, right={r}"
    );

    // Full right: left channel must be (effectively) silent.
    let mut right = audible_voice(Waveform::Square, 220.0);
    right.set_amp_envelope(0.0, 0.0, 1.0, 0.1);
    right.set_pan_position(1.0);
    right.note_on();
    let mut right_buf = vec![0.0_f64; BLOCK_SIZE * 2];
    right.process_block(None, &mut right_buf, BLOCK_SIZE);
    let (l, r) = channel_energy(&right_buf);
    assert!(r > 0.0, "full-right pan should sound on the right channel");
    assert!(
        l < r * 1e-6,
        "full-right pan should leave the left channel negligible relative \
         to the right: left={l}, right={r}"
    );
}

/// `set_pan_position` panics for positions outside `[-1.0, 1.0]`, as
/// documented.
#[test]
#[should_panic(expected = "Pan position must be between -1.0 and 1.0.")]
fn set_pan_position_out_of_range_panics() {
    let mut voice = Voice::new(Waveform::Sine, 220.0, SR, BLOCK_SIZE);
    voice.set_pan_position(1.5);
}

// --- Velocity ---

/// `set_velocity` scales the rendered amplitude: a half-velocity note
/// produces roughly half the peak amplitude of a full-velocity note (all
/// other parameters equal).
#[test]
fn velocity_scales_output_amplitude() {
    fn peak_abs(voice: &mut Voice, blocks: usize) -> f64 {
        let mut buffer = vec![0.0_f64; BLOCK_SIZE * 2];
        let mut peak = 0.0_f64;
        for _ in 0..blocks {
            voice.process_block(None, &mut buffer, BLOCK_SIZE);
            for &s in &buffer {
                peak = peak.max(s.abs());
            }
        }
        peak
    }

    let mut full = audible_voice(Waveform::Square, 220.0);
    full.set_amp_envelope(0.0, 0.0, 1.0, 0.1);
    full.set_velocity(1.0);
    full.note_on();
    let full_peak = peak_abs(&mut full, 4);

    let mut half = audible_voice(Waveform::Square, 220.0);
    half.set_amp_envelope(0.0, 0.0, 1.0, 0.1);
    half.set_velocity(0.5);
    half.note_on();
    let half_peak = peak_abs(&mut half, 4);

    assert!(full_peak > 0.0, "full-velocity voice should be audible");
    assert!(
        (half_peak - full_peak * 0.5).abs() < 1e-9,
        "half velocity should scale peak amplitude by 0.5: full={full_peak}, half={half_peak}"
    );
}

/// `set_velocity` panics for values outside `[0.0, 1.0]`, as documented.
#[test]
#[should_panic(expected = "Velocity multiplier must be between 0.0 and 1.0.")]
fn set_velocity_out_of_range_panics() {
    let mut voice = Voice::new(Waveform::Sine, 220.0, SR, BLOCK_SIZE);
    voice.set_velocity(1.5);
}

// --- Amp envelope individual setters ---

/// The four individual amp-envelope setters (`set_amp_envelope_attack_time`,
/// `..._decay_time`, `..._sustain_level`, `..._release_time`) reach the
/// rendered output exactly like the combined `set_amp_envelope`: a
/// zero-attack envelope reaches its sustain level on the very first sample,
/// while a long attack is still ramping after the first block.
#[test]
fn amp_envelope_individual_setters_reach_rendered_output() {
    // Zero attack/decay, full sustain: amplitude should be at its maximum
    // for the whole first block.
    let mut instant = audible_voice(Waveform::Square, 220.0);
    instant.set_amp_envelope_attack_time(0.0);
    instant.set_amp_envelope_decay_time(0.0);
    instant.set_amp_envelope_sustain_level(1.0);
    instant.set_amp_envelope_release_time(0.1);
    instant.note_on();
    let mut instant_buf = vec![0.0_f64; BLOCK_SIZE * 2];
    instant.process_block(None, &mut instant_buf, BLOCK_SIZE);
    let instant_peak = instant_buf.iter().fold(0.0_f64, |m, &s| m.max(s.abs()));

    // A long attack: after a single (small) block, amplitude should still be
    // ramping up and therefore well below the eventual maximum.
    let mut ramping = audible_voice(Waveform::Square, 220.0);
    ramping.set_amp_envelope_attack_time(1.0); // 1 second attack
    ramping.set_amp_envelope_decay_time(0.0);
    ramping.set_amp_envelope_sustain_level(1.0);
    ramping.set_amp_envelope_release_time(0.1);
    ramping.note_on();
    let mut ramping_buf = vec![0.0_f64; BLOCK_SIZE * 2];
    ramping.process_block(None, &mut ramping_buf, BLOCK_SIZE);
    let ramping_peak = ramping_buf.iter().fold(0.0_f64, |m, &s| m.max(s.abs()));

    assert!(
        instant_peak > 0.0,
        "instant-attack voice should be audible immediately"
    );
    assert!(
        ramping_peak < instant_peak,
        "a long attack should still be ramping after one block, well below \
         the instant-attack peak: ramping={ramping_peak}, instant={instant_peak}"
    );
}

// --- Filter envelope modulation ---

/// The filter envelope modulates the filter cutoff (`final_cutoff =
/// filter_cutoff + filter_env_value * filter_mod_range`). With a low base
/// cutoff, a wide modulation range, an instant filter-envelope attack and a
/// decay back down to a zero sustain, the very first block (filter envelope
/// near its peak, cutoff swept high) must contain noticeably more
/// high-frequency energy than a later block once the filter envelope has
/// decayed back to its zero sustain level (cutoff back at its low base
/// value).
///
/// High-frequency content is approximated by the sum of squared first
/// differences between consecutive samples -- a standard "brightness" proxy
/// that is robust to absolute amplitude (DSP test guidance: assert signal
/// properties with tolerances, not bit-exact buffers).
#[test]
fn filter_envelope_modulation_changes_output_brightness_over_time() {
    fn brightness(buffer: &[f64]) -> f64 {
        buffer
            .chunks_exact(2)
            .map(|c| c[0]) // left channel mono samples
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| (w[1] - w[0]).powi(2))
            .sum()
    }

    let nyquist_limit = (SR / 2.0) - 1.0;
    let base_cutoff = 80.0;
    let mod_range = nyquist_limit - base_cutoff - 100.0; // stay safely below Nyquist at full modulation

    let mut voice = audible_voice(Waveform::Square, 220.0);
    voice.set_filter_parameters(base_cutoff, 1.0, mod_range);
    // Amp envelope: stay at full sustain throughout so the brightness
    // difference is attributable to the filter, not the amp envelope.
    voice.set_amp_envelope(0.0, 0.0, 1.0, 0.1);
    // Filter envelope: instant attack (jumps to 1.0 on sample 0), short
    // decay down to a zero sustain.
    let filter_decay = 0.01;
    voice.set_filter_envelope_attack_time(0.0);
    voice.set_filter_envelope_decay_time(filter_decay);
    voice.set_filter_envelope_sustain_level(0.0);
    voice.set_filter_envelope_release_time(0.1);

    voice.note_on();

    let mut first_block = vec![0.0_f64; BLOCK_SIZE * 2];
    voice.process_block(None, &mut first_block, BLOCK_SIZE);
    assert_all_finite(&first_block, "first block (bright)");

    // Render enough additional blocks for the filter envelope's decay stage
    // to fully complete (reach its zero sustain level).
    let decay_samples = (filter_decay * SR).ceil() as usize;
    let decay_blocks = decay_samples.div_ceil(BLOCK_SIZE) + 2;
    let mut later_block = vec![0.0_f64; BLOCK_SIZE * 2];
    for _ in 0..decay_blocks {
        voice.process_block(None, &mut later_block, BLOCK_SIZE);
        assert_all_finite(&later_block, "decayed block (dark)");
    }

    let bright = brightness(&first_block);
    let dark = brightness(&later_block);

    assert!(
        bright > dark,
        "with the filter envelope at its peak the cutoff is swept high and \
         the output should contain more high-frequency energy than once the \
         envelope has decayed to its zero sustain level: bright={bright}, dark={dark}"
    );
}

// --- process_block_instrumented ---

/// `process_block_instrumented` is a duplicated, timing-instrumented copy of
/// `process_block`'s body. It must (a) record a timing entry for every
/// processing stage and (b) produce the same audio output as `process_block`
/// given identical voice state and input -- for every oscillator waveform
/// (each is a separate match arm in both methods).
#[test]
fn process_block_instrumented_matches_process_block_and_records_all_stages() {
    fn build_voice(waveform: Waveform) -> Voice {
        let mut voice = audible_voice(waveform, 220.0);
        voice.set_filter_parameters(500.0, 2.0, 1000.0);
        voice.set_amp_envelope(0.001, 0.05, 0.5, 0.1);
        voice.set_filter_envelope(0.001, 0.05, 0.5, 0.1);
        voice.note_on();
        voice
    }

    for waveform in [
        Waveform::Sine,
        Waveform::Saw,
        Waveform::Triangle,
        Waveform::Square,
    ] {
        let mut plain = build_voice(waveform);
        let mut instrumented = build_voice(waveform);

        let mut plain_buf = vec![0.0_f64; BLOCK_SIZE * 2];
        let mut instrumented_buf = vec![0.0_f64; BLOCK_SIZE * 2];
        let lfo_buffer = vec![0.0_f64; BLOCK_SIZE];

        // Render a few blocks identically through both paths.
        for _ in 0..3 {
            plain.process_block(None, &mut plain_buf, BLOCK_SIZE);

            let mut timings: HashMap<&'static str, u64> = HashMap::new();
            instrumented.process_block_instrumented(
                &lfo_buffer,
                &mut instrumented_buf,
                BLOCK_SIZE,
                &mut timings,
            );

            assert_all_finite(&plain_buf, &format!("{waveform:?} process_block"));
            assert_all_finite(
                &instrumented_buf,
                &format!("{waveform:?} process_block_instrumented"),
            );
            assert_eq!(
                plain_buf, instrumented_buf,
                "{waveform:?}: process_block_instrumented must render identical \
                 output to process_block"
            );

            for stage in [
                "Oscillator",
                "Filter Envelope",
                "Pre-Filter Gain",
                "Filter Params",
                "Filter",
                "Amp Envelope",
                "Panning",
            ] {
                assert!(
                    timings.contains_key(stage),
                    "{waveform:?}: process_block_instrumented should record a \
                     timing entry for {stage:?}"
                );
            }
        }
    }
}

// --- Note-on time bookkeeping ---

/// `set_note_on_time`/`note_on_time` are a plain round-trip used by the
/// voice-stealing algorithm to find the oldest active voice.
#[test]
fn note_on_time_round_trips() {
    let mut voice = Voice::new(Waveform::Sine, 220.0, SR, BLOCK_SIZE);
    assert_eq!(voice.note_on_time(), 0);
    voice.set_note_on_time(42);
    assert_eq!(voice.note_on_time(), 42);
}

/// `set_oscillator_pitch` panics for MIDI pitches >= 128, as documented.
#[test]
#[should_panic(expected = "MIDI pitch must be 0-127.")]
fn set_oscillator_pitch_out_of_range_panics() {
    let mut voice = Voice::new(Waveform::Sine, 220.0, SR, BLOCK_SIZE);
    voice.set_oscillator_pitch(128);
}

/// `Voice::new` panics on a negative initial pitch frequency, as documented.
#[test]
#[should_panic(expected = "Initial pitch frequency cannot be negative.")]
fn voice_new_negative_pitch_frequency_panics() {
    Voice::new(Waveform::Sine, -1.0, SR, BLOCK_SIZE);
}
