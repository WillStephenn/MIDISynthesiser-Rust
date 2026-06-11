//! Port of `synth.tests.PerformanceTest` - a headless benchmark of the engine.
//!
//! Run with: `cargo run --release --example performance_test`
//!
//! The Java contention test mutated the synthesiser from a second thread via
//! volatile fields; in Rust the synthesiser is wrapped in an `Arc<Mutex<_>>`
//! and both threads lock it (stage 2/3 will decide the real-time threading
//! model).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use midi_synthesiser::core::synthesiser::Synthesiser;
use midi_synthesiser::utils::audio_constants;

fn main() {
    run_standard_test();
    println!("\n");
    run_contention_stress_test();
}

fn print_timing_results(total_timings: &HashMap<&'static str, u64>, blocks: u64) {
    println!("\n--- Average Time Per Stage (in microseconds) ---");
    let mut entries: Vec<(&&str, &u64)> = total_timings.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1)); // Sort by time, descending
    for (stage, time_in_nanos) in entries {
        let average_time = time_in_nanos / blocks;
        println!("{:<25}: {} \u{b5}s", stage, average_time / 1000);
    }
    println!("------------------------------------------");
}

fn run_standard_test() {
    // Setup
    let number_of_blocks_to_process: u64 = 2000;
    let mut synth = Synthesiser::new(
        audio_constants::NUMBER_OF_VOICES,
        audio_constants::SAMPLE_RATE,
        audio_constants::BLOCK_SIZE,
    );

    let mut audio_block = vec![0.0_f64; audio_constants::BLOCK_SIZE * 2];
    let mut total_timings: HashMap<&'static str, u64> = HashMap::new();

    // Activate voices
    println!(
        "Activating {} voices for the test...",
        audio_constants::NUMBER_OF_VOICES
    );
    for i in 0..audio_constants::NUMBER_OF_VOICES {
        synth.note_on(60 + i as u8, 1.0);
    }

    // Run processing loop
    println!("Processing {} audio blocks...", number_of_blocks_to_process);
    let mut total_test_time_nanos: u64 = 0;
    for _ in 0..number_of_blocks_to_process {
        let block_start = Instant::now();
        let block_timings = synth.process_block_instrumented(&mut audio_block);
        total_test_time_nanos += block_start.elapsed().as_nanos() as u64;

        // Aggregate timings from this block into the total
        for (key, value) in block_timings {
            *total_timings.entry(key).or_insert(0) += value;
        }
    }
    println!("Processing complete.\n");

    // Print results
    println!("--- Synthesiser Performance Test Results ---");
    println!(
        "Total processing time: {} ms",
        total_test_time_nanos / 1_000_000
    );
    print_timing_results(&total_timings, number_of_blocks_to_process);
}

fn run_contention_stress_test() {
    let number_of_blocks_to_process: u64 = 2000;
    let synth = Arc::new(Mutex::new(Synthesiser::new(
        audio_constants::NUMBER_OF_VOICES,
        audio_constants::SAMPLE_RATE,
        audio_constants::BLOCK_SIZE,
    )));

    let mut audio_block = vec![0.0_f64; audio_constants::BLOCK_SIZE * 2];
    let mut total_timings: HashMap<&'static str, u64> = HashMap::new();

    // Activate voices
    println!("=== Contention Stress Test ===");
    println!("Activating {} voices...", audio_constants::NUMBER_OF_VOICES);
    {
        let mut synth = synth.lock().unwrap();
        for i in 0..audio_constants::NUMBER_OF_VOICES {
            synth.note_on(60 + i as u8, 1.0);
        }
    }

    // Start a setter-spam thread simulating rapid MIDI CC messages
    let running = Arc::new(AtomicBool::new(true));
    let setter_running = Arc::clone(&running);
    let setter_synth = Arc::clone(&synth);
    let setter_thread = std::thread::spawn(move || {
        let mut v = 0.0_f64;
        while setter_running.load(Ordering::Relaxed) {
            v += 0.01;
            if v > 1.0 {
                v = 0.0;
            }
            let mut synth = setter_synth.lock().unwrap();
            synth.set_filter_cutoff(20.0 + v * 19980.0);
            synth.set_filter_resonance(1.0 + v * 14.0);
            synth.set_amp_attack_time(v * 2.0);
            synth.set_amp_release_time(v * 2.0);
            synth.set_pre_filter_gain_db(v * 24.0 - 12.0);
            synth.set_master_volume(0.5 + v * 0.5);
            synth.set_lfo_frequency(0.1 + v * 9.9);
            synth.set_pan_depth(v);
        }
    });

    // Run processing loop under contention
    println!(
        "Processing {} blocks under contention...",
        number_of_blocks_to_process
    );
    let mut total_test_time_nanos: u64 = 0;
    for _ in 0..number_of_blocks_to_process {
        let block_start = Instant::now();
        let block_timings = synth
            .lock()
            .unwrap()
            .process_block_instrumented(&mut audio_block);
        total_test_time_nanos += block_start.elapsed().as_nanos() as u64;
        for (key, value) in block_timings {
            *total_timings.entry(key).or_insert(0) += value;
        }
    }

    running.store(false, Ordering::Relaxed);
    if setter_thread.join().is_err() {
        eprintln!("Setter thread panicked.");
    }
    println!("Contention test complete.\n");

    // Print results
    println!("--- Contention Stress Test Results ---");
    println!(
        "Total processing time: {} ms",
        total_test_time_nanos / 1_000_000
    );
    print_timing_results(&total_timings, number_of_blocks_to_process);
}
