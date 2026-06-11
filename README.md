# MIDI Synthesiser Rust Port

This is a Rust port of the polyphonic MIDI synthesiser originally written in Java.

The original project can be found at [WillStephenn/MIDISynthesiser](https://github.com/WillStephenn/MIDISynthesiser).

This Rust port was initially ported by **claude fable 5** to bring the synthesiser into a more appropriate language for real-time, low-latency audio processing.

## Performance Comparison

Migrating the synthesiser from Java to Rust has resulted in dramatic performance improvements, reducing the total processing time by **~45%** under identical test bench conditions.

### Benchmark Configurations
* **Voices**: 8 active voices
* **Workload**: 2000 audio blocks processed

### Results Overview
| Language | Total Processing Time (Standard) | Total Processing Time (Contention) |
|---|---|---|
| **Java** | 93 ms | 93 ms |
| **Rust** | **51 ms** (~45% faster) | **51 ms** (~45% faster) |

---

### Detailed Stage Breakdown (Average Time Per Stage in Microseconds)

#### 1. Standard Test

| Processing Stage | Java Output (µs) | Rust Output (µs) | Difference |
| :--- | :---: | :---: | :---: |
| **Voice Processing & Mix** | 42 | 24 | -18 µs (-43%) |
| **Filter** | 12 | 12 | 0 µs (0%) |
| **Amp Envelope** | 5 | 2 | -3 µs (-60%) |
| **Filter Envelope** | 4 | 1 | -3 µs (-75%) |
| **Oscillator** | 3 | 2 | -1 µs (-33%) |
| **Panning** | 2 | 0 | -2 µs (-100%) |
| **Pre-Filter Gain** | 1 | 0 | -1 µs (-100%) |
| **LFO** | 1 | 0 | -1 µs (-100%) |
| **Hard Clipping** | 1 | 0 | -1 µs (-100%) |
| **Filter Params** | 0 | 0 | 0 µs |

#### 2. Contention Stress Test

| Processing Stage | Java Output (µs) | Rust Output (µs) | Difference |
| :--- | :---: | :---: | :---: |
| **Voice Processing & Mix** | 45 | 22 | -23 µs (-51%) |
| **Filter** | 10 | 11 | +1 µs (+10%) |
| **Oscillator** | 2 | 2 | 0 µs (0%) |
| **Amp Envelope** | 8 | 2 | -6 µs (-75%) |
| **Filter Envelope** | 6 | 1 | -5 µs (-83%) |
| **Panning** | 4 | 0 | -4 µs (-100%) |
| **Pre-Filter Gain** | 1 | 0 | -1 µs (-100%) |
| **Hard Clipping** | 0 | 0 | 0 µs |
| **LFO** | 0 | 0 | 0 µs |
| **Filter Params** | 0 | 0 | 0 µs |
