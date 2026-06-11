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
    let mut notes = [0_u8; 16];
    let count = synth.get_active_notes(&mut notes);
    assert_eq!(count, NUMBER_OF_VOICES);

    // A ninth note must steal the oldest voice (note 60).
    synth.note_on(80, 1.0);
    let count = synth.get_active_notes(&mut notes);
    assert_eq!(count, NUMBER_OF_VOICES);
    let active = &notes[..count];
    assert!(active.contains(&80), "new note not active");
    assert!(!active.contains(&60), "oldest note was not stolen");
    assert!(active.contains(&61), "newer note was wrongly stolen");

    // The next steal takes note 61, the new oldest.
    synth.note_on(81, 1.0);
    let count = synth.get_active_notes(&mut notes);
    let active = &notes[..count];
    assert!(!active.contains(&61));
    assert!(active.contains(&80) && active.contains(&81));
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
    let mut notes = [0_u8; 16];
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
    let mut notes = [0_u8; 16];
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
