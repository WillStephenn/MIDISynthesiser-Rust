//! Direct unit tests for `core::voice::Voice`.
//!
//! `Voice` is the engine's lower-level public API (see the testing
//! philosophy): unit-testing it directly, independent of `Synthesiser`'s
//! voice-allocation logic, is encouraged. These tests exercise note
//! lifecycle, waveform/pitch handling (including the documented "only the
//! selected oscillator is repointed" quirk), pan, gain staging, and the
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

/// Documents the quirk called out in the voice design notes: switching the
/// active waveform with `set_oscillator_waveform` does *not* retroactively
/// apply the most recent `set_oscillator_pitch` to the newly-selected
/// oscillator -- each of the four pre-built oscillators keeps whatever
/// frequency it last had.
///
/// Strategy: a voice is constructed with Sine selected at 220 Hz (so the saw
/// oscillator is left at its constructor default of 0 Hz). After retargeting
/// the pitch (which only updates the *sine* oscillator) and switching to Saw,
/// the voice's rendered output must be identical to a fresh voice constructed
/// directly with Saw at 0 Hz (both saw oscillators are at 0 Hz, untouched) --
/// and different from a fresh voice constructed with Saw at 220 Hz.
#[test]
fn set_oscillator_pitch_only_updates_the_selected_oscillator() {
    let a3 = lookup_tables::tables().midi_to_hz[57]; // 220 Hz
    let a4_midi = 69; // 440 Hz

    // Voice A: built with Sine @ 220 Hz, then retargeted to 440 Hz (still
    // Sine), then switched to Saw without ever calling set_oscillator_pitch
    // while Saw is selected.
    let mut voice_a = audible_voice(Waveform::Sine, a3);
    voice_a.set_oscillator_pitch(a4_midi);
    voice_a.set_oscillator_waveform(Waveform::Saw);

    // Voice B: built directly with Saw @ 0 Hz (the constructor default for
    // every oscillator other than the one matching the constructor's
    // waveform argument).
    let mut voice_b = audible_voice(Waveform::Saw, 0.0);

    // Voice C: built directly with Saw @ 220 Hz, for contrast.
    let mut voice_c = audible_voice(Waveform::Saw, a3);

    for voice in [&mut voice_a, &mut voice_b, &mut voice_c] {
        voice.note_on();
    }

    let mut out_a = vec![0.0_f64; BLOCK_SIZE * 2];
    let mut out_b = vec![0.0_f64; BLOCK_SIZE * 2];
    let mut out_c = vec![0.0_f64; BLOCK_SIZE * 2];
    voice_a.process_block(None, &mut out_a, BLOCK_SIZE);
    voice_b.process_block(None, &mut out_b, BLOCK_SIZE);
    voice_c.process_block(None, &mut out_c, BLOCK_SIZE);

    assert_all_finite(&out_a, "voice A (retargeted then switched to saw)");
    assert_all_finite(&out_b, "voice B (fresh saw @ 0 Hz)");
    assert_all_finite(&out_c, "voice C (fresh saw @ 220 Hz)");

    assert_eq!(
        out_a, out_b,
        "switching to Saw after set_oscillator_pitch (while Sine was \
         selected) must leave the saw oscillator at its untouched 0 Hz \
         default, matching a voice constructed directly with Saw @ 0 Hz"
    );
    assert_ne!(
        out_a, out_c,
        "the retargeted-then-switched voice must NOT match a voice whose \
         saw oscillator was actually given the 220 Hz pitch"
    );

    // A second, independent pair documents the other side of the quirk:
    // calling set_oscillator_pitch *while Triangle is selected* DOES
    // retarget the now-selected triangle oscillator, matching a fresh voice
    // constructed directly with Triangle at the retargeted frequency.
    let a4 = lookup_tables::tables().midi_to_hz[a4_midi as usize];
    let mut voice_e = audible_voice(Waveform::Sine, a3);
    voice_e.set_oscillator_waveform(Waveform::Triangle);
    voice_e.set_oscillator_pitch(a4_midi);
    voice_e.note_on();

    let mut voice_f = audible_voice(Waveform::Triangle, a4);
    voice_f.note_on();

    let mut out_e = vec![0.0_f64; BLOCK_SIZE * 2];
    let mut out_f = vec![0.0_f64; BLOCK_SIZE * 2];
    voice_e.process_block(None, &mut out_e, BLOCK_SIZE);
    voice_f.process_block(None, &mut out_f, BLOCK_SIZE);

    assert_all_finite(&out_e, "voice E (switched to triangle, retargeted)");
    assert_eq!(
        out_e, out_f,
        "calling set_oscillator_pitch while Triangle is selected must \
         retarget the triangle oscillator, matching a voice constructed \
         directly with Triangle at the retargeted frequency"
    );
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
