//! Sanity tests for the ported DSP engine.

use midi_synthesiser::components::envelope::{Envelope, Stage};
use midi_synthesiser::components::filters::resonant_low_pass_filter::ResonantLowPassFilter;
use midi_synthesiser::components::oscillators::oscillator::Oscillator;
use midi_synthesiser::components::oscillators::saw_oscillator::SawOscillator;
use midi_synthesiser::components::oscillators::sine_oscillator::SineOscillator;
use midi_synthesiser::components::oscillators::square_oscillator::SquareOscillator;
use midi_synthesiser::components::oscillators::triangle_oscillator::TriangleOscillator;
use midi_synthesiser::core::audio_component::AudioComponent;
use midi_synthesiser::core::synthesiser::{Synthesiser, Waveform};
use midi_synthesiser::utils::audio_constants::{BLOCK_SIZE, NUMBER_OF_VOICES, SAMPLE_RATE};

const SR: f64 = SAMPLE_RATE;

// --- Envelope ---

#[test]
fn envelope_stage_progression() {
    let mut env = Envelope::new(SR);
    // 10ms attack, 10ms decay, 0.5 sustain, 10ms release => 441 samples per phase
    env.set_envelope(0.01, 0.01, 0.5, 0.01);
    assert_eq!(env.stage(), Stage::Idle);

    env.note_on();
    assert_eq!(env.stage(), Stage::Attack);

    let mut buffer = [0.0_f64; 256];

    // One block (256 samples) is not enough to finish the 441-sample attack
    env.process_block(None, &mut buffer, 256);
    assert_eq!(env.stage(), Stage::Attack);
    assert!(buffer[255] > 0.0 && buffer[255] < 1.0);

    // After the attack completes the envelope must pass through DECAY...
    env.process_block(None, &mut buffer, 256);
    assert_eq!(env.stage(), Stage::Decay);

    // ...and settle at SUSTAIN with the configured level.
    for _ in 0..4 {
        env.process_block(None, &mut buffer, 256);
    }
    assert_eq!(env.stage(), Stage::Sustain);
    assert_eq!(buffer[255], 0.5);

    // Note off enters RELEASE and ramps down to IDLE at 0.0.
    env.note_off();
    assert_eq!(env.stage(), Stage::Release);
    for _ in 0..4 {
        env.process_block(None, &mut buffer, 256);
    }
    assert_eq!(env.stage(), Stage::Idle);
    assert_eq!(buffer[255], 0.0);
}

#[test]
fn envelope_zero_attack_jumps_to_full_level() {
    let mut env = Envelope::new(SR);
    env.set_envelope(0.0, 0.1, 0.5, 0.1);
    env.note_on();
    let mut buffer = [0.0_f64; 4];
    env.process_block(None, &mut buffer, 4);
    // First sample hits 1.0 immediately, then decay begins
    assert_eq!(buffer[0], 1.0);
    assert_eq!(env.stage(), Stage::Decay);
}

// --- Oscillators ---

#[test]
fn oscillator_output_bounds() {
    let mut buffer = [0.0_f64; 1024];
    let mut oscillators: Vec<Box<dyn Oscillator>> = vec![
        Box::new(SineOscillator::new(SR)),
        Box::new(SawOscillator::new(SR)),
        Box::new(SquareOscillator::new(SR)),
        Box::new(TriangleOscillator::new(SR)),
    ];
    for osc in oscillators.iter_mut() {
        osc.set_frequency(440.0);
        osc.process_block(None, &mut buffer, 1024);
        for &sample in &buffer {
            assert!(
                (-1.0..=1.0).contains(&sample),
                "oscillator output out of bounds: {sample}"
            );
        }
    }
}

#[test]
fn sine_oscillator_periodicity() {
    // Choose a frequency whose phase increment is an exact integer (64),
    // giving an exact period of TABLE_SIZE / 64 = 512 samples.
    let table_size = midi_synthesiser::utils::lookup_tables::TABLE_SIZE as f64;
    let frequency = SR * 64.0 / table_size;

    let mut osc = SineOscillator::new(SR);
    osc.set_frequency(frequency);

    let mut buffer = [0.0_f64; 2048];
    osc.process_block(None, &mut buffer, 2048);

    for i in 0..1536 {
        assert_eq!(
            buffer[i],
            buffer[i + 512],
            "sine output not periodic at sample {i}"
        );
    }
    // A full-cycle sine should actually swing close to +/-1
    let max = buffer.iter().cloned().fold(f64::MIN, f64::max);
    let min = buffer.iter().cloned().fold(f64::MAX, f64::min);
    assert!(max > 0.99 && min < -0.99);
}

// --- Filter ---

#[test]
fn filter_stable_at_extreme_cutoff_and_resonance() {
    let nyquist_limit = (SR / 2.0) - 1.0;
    let extremes = [
        (20.0, 1.0),                 // lowest sensible cutoff, no resonance
        (20.0, 20.0),                // lowest cutoff, max resonance
        (nyquist_limit - 1.0, 1.0),  // near-Nyquist cutoff
        (nyquist_limit - 1.0, 20.0), // near-Nyquist cutoff, max resonance
    ];

    for (cutoff, resonance) in extremes {
        let mut filter = ResonantLowPassFilter::new(SR);
        filter.set_parameters(cutoff, resonance);

        // Drive the filter hard with a full-scale square wave for many blocks.
        let mut osc = SquareOscillator::new(SR);
        osc.set_frequency(220.0);
        let mut input = [0.0_f64; 256];
        let mut output = [0.0_f64; 256];
        for _ in 0..200 {
            osc.process_block(None, &mut input, 256);
            filter.process_block(Some(&input), &mut output, 256);
            for &sample in &output {
                assert!(
                    sample.is_finite(),
                    "filter blew up at cutoff={cutoff}, q={resonance}"
                );
                assert!(
                    sample.abs() < 100.0,
                    "filter output unbounded at cutoff={cutoff}, q={resonance}: {sample}"
                );
            }
        }
    }
}

// --- Synthesiser / voice stealing ---

#[test]
fn voice_stealing_replaces_oldest_note() {
    let mut synth = Synthesiser::new(NUMBER_OF_VOICES, SR, BLOCK_SIZE);

    // Fill every voice. Note 60 is the oldest.
    for i in 0..NUMBER_OF_VOICES {
        synth.note_on(60 + i as u8, 1.0);
    }
    let mut notes = [0_u8; NUMBER_OF_VOICES];
    let count = synth.get_active_notes(&mut notes);
    assert_eq!(count, NUMBER_OF_VOICES);

    // One more note than the pool holds must steal the oldest voice (note 60).
    synth.note_on(100, 1.0);
    let count = synth.get_active_notes(&mut notes);
    assert_eq!(count, NUMBER_OF_VOICES);
    let active = &notes[..count];
    assert!(active.contains(&100), "new note not active");
    assert!(!active.contains(&60), "oldest note was not stolen");
    assert!(active.contains(&61), "newer note was wrongly stolen");

    // The next steal takes note 61, the new oldest.
    synth.note_on(101, 1.0);
    let count = synth.get_active_notes(&mut notes);
    let active = &notes[..count];
    assert!(!active.contains(&61));
    assert!(active.contains(&100) && active.contains(&101));
}

#[test]
fn note_off_releases_voice_and_renders_silence_when_idle() {
    let mut synth = Synthesiser::new(NUMBER_OF_VOICES, SR, BLOCK_SIZE);
    synth.set_amp_release_time(0.001); // fast release so the voice frees quickly

    let mut buffer = vec![0.0_f64; BLOCK_SIZE * 2];

    // Silence before any note
    synth.process_block(&mut buffer);
    assert!(buffer.iter().all(|&s| s == 0.0));

    synth.note_on(64, 1.0);
    synth.process_block(&mut buffer);
    assert!(
        buffer.iter().any(|&s| s != 0.0),
        "active note produced no output"
    );
    // Output must respect the hard clipper
    assert!(buffer.iter().all(|&s| (-1.0..=1.0).contains(&s)));

    synth.note_off(64);
    let mut notes = [0_u8; NUMBER_OF_VOICES];
    assert_eq!(
        synth.get_active_notes(&mut notes),
        0,
        "released note still counted as active (no-release)"
    );

    // After the release completes, the synth is silent again.
    for _ in 0..50 {
        synth.process_block(&mut buffer);
    }
    assert!(buffer.iter().all(|&s| s == 0.0));
}

#[test]
fn retriggering_same_note_does_not_consume_extra_voices() {
    let mut synth = Synthesiser::new(NUMBER_OF_VOICES, SR, BLOCK_SIZE);
    // note_on for an already-sounding pitch first releases it (Java behaviour)
    synth.note_on(60, 1.0);
    synth.note_on(60, 1.0);
    let mut notes = [0_u8; NUMBER_OF_VOICES];
    assert_eq!(synth.get_active_notes(&mut notes), 1);
    assert_eq!(notes[0], 60);
}

#[test]
fn waveform_parameter_reaches_default_patch() {
    let synth = Synthesiser::new(NUMBER_OF_VOICES, SR, BLOCK_SIZE);
    // The default patch loads a SQUARE wave with a 1 kHz cutoff (see Java constructor)
    assert_eq!(synth.waveform(), Waveform::Square);
    assert_eq!(synth.filter_cutoff(), 1000.0);
    assert_eq!(synth.filter_resonance(), 3.0);
    assert_eq!(synth.lfo_waveform(), Waveform::Sine);
}

// --- Construction parameters ---

/// `Synthesiser::new` records the sample rate and block size it was
/// constructed with, queryable via `sample_rate()`/`block_size()`. Hosts
/// (and `process_block_instrumented`, which derives its block size from
/// `self.block_size`) rely on these matching the constructor arguments.
#[test]
fn synthesiser_records_sample_rate_and_block_size_from_construction() {
    // Exercise more than one configuration so this isn't just restating the
    // single value used elsewhere in this file.
    for (sample_rate, block_size) in [(SR, BLOCK_SIZE), (SR * 2.0, BLOCK_SIZE * 2)] {
        let synth = Synthesiser::new(NUMBER_OF_VOICES, sample_rate, block_size);
        assert_eq!(synth.sample_rate(), sample_rate);
        assert_eq!(synth.block_size(), block_size);
    }
}

// --- get_active_notes ---

/// `get_active_notes` must never write past the end of the caller-provided
/// buffer: when the buffer is smaller than the number of active voices, it
/// fills the buffer completely and returns its length, rather than
/// panicking or silently dropping the bound.
#[test]
fn get_active_notes_truncates_to_a_buffer_smaller_than_the_voice_pool() {
    let mut synth = Synthesiser::new(NUMBER_OF_VOICES, SR, BLOCK_SIZE);
    for i in 0..NUMBER_OF_VOICES {
        synth.note_on(36 + i as u8, 1.0);
    }

    // A buffer with room for only half the active voices.
    let mut small = vec![0_u8; NUMBER_OF_VOICES / 2];
    let count = synth.get_active_notes(&mut small);
    assert_eq!(
        count,
        small.len(),
        "get_active_notes should fill (and report) exactly the buffer's \
         length when there are more active voices than space"
    );

    // A zero-length buffer must report zero without panicking.
    let mut empty: [u8; 0] = [];
    assert_eq!(synth.get_active_notes(&mut empty), 0);
}

// --- LFO waveform selection ---

/// Selecting each LFO waveform must (a) be reflected by `lfo_waveform()`,
/// (b) keep `process_block` rendering finite output, and (c) drive
/// `get_pan_position()` to a value within the pan range derived from the
/// configured `pan_depth` -- exercising every arm of the internal
/// LFO-selection match in both `sync_lfo`/`current_lfo_mut` and
/// `process_block`.
#[test]
fn lfo_waveform_selection_drives_pan_position_for_every_waveform() {
    for waveform in [
        Waveform::Sine,
        Waveform::Saw,
        Waveform::Triangle,
        Waveform::Square,
    ] {
        let mut synth = Synthesiser::new(NUMBER_OF_VOICES, SR, BLOCK_SIZE);
        synth.set_lfo_waveform(waveform);
        synth.set_lfo_frequency(2.0);
        assert_eq!(synth.lfo_waveform(), waveform);

        synth.note_on(60, 1.0);
        let mut buffer = vec![0.0_f64; BLOCK_SIZE * 2];
        let pan_depth = synth.pan_depth();

        for _ in 0..8 {
            synth.process_block(&mut buffer);
            assert!(
                buffer.iter().all(|&s| s.is_finite()),
                "{waveform:?} LFO: output must stay finite"
            );

            let pan = synth.get_pan_position();
            assert!(
                (-pan_depth..=pan_depth).contains(&pan) || pan.abs() <= pan_depth + 1e-9,
                "{waveform:?} LFO: pan position {pan} must stay within \
                 +/-pan_depth ({pan_depth})"
            );
        }
    }
}

// --- Re-syncing changed parameters after construction ---

/// Parameter setters in the filter-envelope, amp-envelope, gain-staging and
/// pan-depth groups each set their own dirty flag; `process_block` must sync
/// *all* changed groups to the voices on the next call, even when several
/// groups change between renders. This is checked both for an audible effect
/// (a large pre-filter gain reduction measurably quietens the output) and for
/// overall engine health (output stays finite with every group dirty at once).
#[test]
fn parameter_changes_in_every_dirty_group_resync_on_next_block() {
    let mut buffer = vec![0.0_f64; BLOCK_SIZE * 2];

    // Baseline: default patch, render a few blocks of a held note.
    let mut baseline = Synthesiser::new(NUMBER_OF_VOICES, SR, BLOCK_SIZE);
    baseline.note_on(60, 1.0);
    let mut baseline_peak = 0.0_f64;
    for _ in 0..8 {
        baseline.process_block(&mut buffer);
        for &s in &buffer {
            baseline_peak = baseline_peak.max(s.abs());
        }
    }
    assert!(baseline_peak > 0.0, "baseline patch should be audible");

    // Quietened: after the first block (which syncs the construction-time
    // dirty flags), change one parameter from each remaining group --
    // filter envelope, amp envelope, gain staging, and pan depth -- then
    // render. All four groups' dirty flags must be applied together.
    let mut quiet = Synthesiser::new(NUMBER_OF_VOICES, SR, BLOCK_SIZE);
    quiet.note_on(60, 1.0);
    quiet.process_block(&mut buffer); // clear construction-time dirty flags

    quiet.set_filter_attack_time(quiet.filter_attack_time() + 0.05);
    quiet.set_amp_attack_time(quiet.amp_attack_time() + 0.05);
    quiet.set_pan_depth((quiet.pan_depth() + 0.2).min(1.0));
    // A large gain cut should produce a measurable amplitude drop.
    quiet.set_pre_filter_gain_db(quiet.pre_filter_gain_db() - 40.0);

    let mut quiet_peak = 0.0_f64;
    for _ in 0..8 {
        quiet.process_block(&mut buffer);
        assert!(
            buffer.iter().all(|&s| s.is_finite()),
            "output must stay finite when every dirty-flag group changes together"
        );
        for &s in &buffer {
            quiet_peak = quiet_peak.max(s.abs());
        }
    }

    assert!(
        quiet_peak < baseline_peak * 0.5,
        "a 40 dB pre-filter gain cut applied via the gain-dirty group should \
         substantially reduce peak output: baseline={baseline_peak}, quiet={quiet_peak}"
    );
}

/// Each of the filter-envelope, amp-envelope, gain-staging and pan-depth
/// dirty-flag groups must be synced to the voices independently of the
/// others: changing only one group between renders must not require any of
/// the other groups to also be dirty. Rendered output stays finite
/// throughout every combination.
#[test]
fn each_dirty_group_resyncs_independently_of_the_others() {
    let mut synth = Synthesiser::new(NUMBER_OF_VOICES, SR, BLOCK_SIZE);
    let mut buffer = vec![0.0_f64; BLOCK_SIZE * 2];
    synth.note_on(60, 1.0);

    // Clear the construction-time dirty flags.
    synth.process_block(&mut buffer);

    // One render with nothing dirty.
    synth.process_block(&mut buffer);
    assert!(buffer.iter().all(|&s| s.is_finite()));

    // Filter envelope only.
    synth.set_filter_attack_time(synth.filter_attack_time() + 0.01);
    synth.process_block(&mut buffer);
    assert!(buffer.iter().all(|&s| s.is_finite()));

    // Amp envelope only.
    synth.set_amp_decay_time(synth.amp_decay_time() + 0.01);
    synth.process_block(&mut buffer);
    assert!(buffer.iter().all(|&s| s.is_finite()));

    // Gain staging only.
    synth.set_post_filter_gain_db(synth.post_filter_gain_db() + 1.0);
    synth.process_block(&mut buffer);
    assert!(buffer.iter().all(|&s| s.is_finite()));

    // Pan depth only.
    synth.set_pan_depth((synth.pan_depth() - 0.1).max(0.0));
    synth.process_block(&mut buffer);
    assert!(buffer.iter().all(|&s| s.is_finite()));

    // A final render with nothing dirty again.
    synth.process_block(&mut buffer);
    assert!(buffer.iter().all(|&s| s.is_finite()));
    assert!(
        buffer.iter().any(|&s| s != 0.0),
        "the held note should still be audible after independent group syncs"
    );
}

// --- apply_patch ---

/// `apply_patch` re-applies the synth's current master settings to every
/// voice. Calling it after a parameter change (and after the normal
/// dirty-flag sync has already run) must be harmless: the synth's reported
/// parameters are unchanged and audio continues to render correctly.
#[test]
fn apply_patch_reapplies_current_settings_without_changing_them() {
    let mut synth = Synthesiser::new(NUMBER_OF_VOICES, SR, BLOCK_SIZE);
    synth.set_oscillator_waveform(Waveform::Triangle);
    synth.set_lfo_waveform(Waveform::Square);
    synth.set_filter_cutoff(1234.0);

    let mut buffer = vec![0.0_f64; BLOCK_SIZE * 2];
    synth.note_on(60, 1.0);
    synth.process_block(&mut buffer); // normal sync

    let waveform_before = synth.waveform();
    let lfo_waveform_before = synth.lfo_waveform();
    let cutoff_before = synth.filter_cutoff();
    let lfo_frequency_before = synth.lfo_frequency();

    synth.apply_patch();

    assert_eq!(synth.waveform(), waveform_before);
    assert_eq!(synth.lfo_waveform(), lfo_waveform_before);
    assert_eq!(synth.filter_cutoff(), cutoff_before);
    assert_eq!(synth.lfo_frequency(), lfo_frequency_before);

    // Audio rendering must remain healthy after apply_patch.
    for _ in 0..4 {
        synth.process_block(&mut buffer);
        assert!(
            buffer.iter().all(|&s| s.is_finite()),
            "output must stay finite after apply_patch"
        );
    }
    assert!(
        buffer.iter().any(|&s| s != 0.0),
        "the held note should still be audible after apply_patch"
    );
}

// --- process_block_instrumented (Synthesiser) ---

/// `Synthesiser::process_block_instrumented` is a timing-instrumented
/// counterpart to `process_block`. Given identical synth state, it must (a)
/// produce the same audio output as `process_block` and (b) record timing
/// entries for the LFO, voice processing/mix, and hard-clipping stages, plus
/// every per-voice stage forwarded from `Voice::process_block_instrumented`.
///
/// Run for every LFO waveform, covering each arm of the internal
/// LFO-selection match in both methods.
#[test]
fn synthesiser_process_block_instrumented_matches_process_block_and_records_stages() {
    fn build_synth(lfo_waveform: Waveform) -> Synthesiser {
        let mut synth = Synthesiser::new(NUMBER_OF_VOICES, SR, BLOCK_SIZE);
        synth.set_lfo_waveform(lfo_waveform);
        synth.note_on(60, 1.0);
        synth.note_on(64, 1.0);
        synth
    }

    for lfo_waveform in [
        Waveform::Sine,
        Waveform::Saw,
        Waveform::Triangle,
        Waveform::Square,
    ] {
        let mut plain = build_synth(lfo_waveform);
        let mut instrumented = build_synth(lfo_waveform);

        let mut plain_buf = vec![0.0_f64; BLOCK_SIZE * 2];
        let mut instrumented_buf = vec![0.0_f64; BLOCK_SIZE * 2];

        for _ in 0..3 {
            plain.process_block(&mut plain_buf);
            let timings = instrumented.process_block_instrumented(&mut instrumented_buf);

            assert!(plain_buf.iter().all(|&s| s.is_finite()));
            assert!(instrumented_buf.iter().all(|&s| s.is_finite()));
            assert_eq!(
                plain_buf, instrumented_buf,
                "{lfo_waveform:?} LFO: process_block_instrumented must render \
                 identical output to process_block"
            );

            for stage in [
                "LFO",
                "Voice Processing & Mix",
                "Hard Clipping",
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
                    "{lfo_waveform:?} LFO: synthesiser process_block_instrumented \
                     should record a timing entry for {stage:?}"
                );
            }
        }
    }
}
