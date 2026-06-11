# MIDI Synthesiser (Rust)

Rust port of the Java/JavaFX polyphonic MIDI synthesiser. Hand-written DSP engine
(oscillators, TPT resonant low-pass filter, dual ADSR envelopes, LFO, lookup tables,
voice stealing), cpal/midir/midly for platform I/O, egui/eframe GUI.

## Architecture constraints

- The engine (`src/core`, `src/components`, `src/utils` except `audio_device_connector.rs`)
  must stay free of I/O dependencies. Long-term goal: wrap the lib as a VST/CLAP plugin via
  nih-plug, reusing the egui GUI through `nih_plug_egui`. Keep that separation intact.
- The audio path must stay allocation-free after startup and lock the shared
  `Mutex<Synthesiser>` at most once per block.
- Engine parameter setters are real-time safe: plain write + dirty flag, synced to voices at
  the start of `process_block`.
- **Engine configuration is validated, not trusted.** The constants in
  `utils::audio_constants` are user-tunable; every structural invariant (e.g. lookup table
  size must be a power of two, voice count ≥ 1, buffer ≥ block) is checked at startup by the
  config validation layer. An invalid value falls back to a documented safe default and
  emits a warning that hosts must surface (GUI banner / CLI stderr) — never a panic, never
  silent corruption. Real-time *feasibility* of a config is machine-dependent and cannot be
  validated statically; that is the soak/performance tests' job.

## Commands

- Build / lint: `cargo build --all-targets`, `cargo clippy --all-targets`, `cargo fmt`
- Test: `cargo test`
- Coverage: `cargo llvm-cov --html --ignore-filename-regex 'src/(main\.rs|ui/|utils/audio_device_connector\.rs|midi/midi_device_connector\.rs)'`
  (requires `cargo install cargo-llvm-cov`)
- Run: `cargo run` (GUI) · `cargo run -- --cli` (also `--list-devices`, `--play <file.mid>`, `--ascii`)
- Performance check: `cargo run --release --example performance_test`

## Testing philosophy

### Test layers

1. **Engine unit tests** — envelope state machine, oscillator signal properties, filter
   stability, voice allocation/stealing/retrigger, parameter clamping. Pure, fast,
   deterministic; no I/O. The engine's lower-level types (`Voice`, `Envelope`, oscillators)
   are public API of the lib — unit-testing them directly is fine and encouraged.
2. **MIDI integration tests** — raw MIDI byte streams through `MidiInputHandler` and
   observable `Synthesiser` state out the other side; the CC map is table-driven. MIDI file
   playback is tested with small generated files.
3. **End-to-end render tests** — a scripted MIDI event sequence driven through
   `process_block` loops, asserting on signal properties of the rendered audio and on voice
   pool health afterwards. This layer includes **soak tests**: simulate minutes of audio
   (rendered much faster than real time) and assert that per-block render cost stays bounded
   and voices return to `Idle`. Soak tests are how we catch voice-lifecycle leaks and
   denormal CPU blowup, the classic real-time audio failure modes.
4. **GUI tests** (`egui_kittest`) — smoke tests (the app draws a frame without panicking,
   key widgets exist) and a small number of interaction tests (slider change reaches the
   synth, MIDI dirty-flag refreshes the UI state). Keep this layer thin; immediate-mode draw
   code churns and snapshot-style assertions on it are brittle.
5. **Not automatically tested** — hardware device I/O (`cpal`/`midir` glue). Keep those
   modules thin and dumb; they are verified manually. Device enumeration smoke tests may run
   but must skip (not fail) when no device exists, so CI stays green.

### DSP test guidance

- Assert **signal properties** with tolerances — peak, RMS, period, monotonic decay,
  silence thresholds — not bit-exact golden buffers. Bit-exactness is only asserted where it
  is a documented contract (e.g. hard clip at ±1.0, silence == exactly 0.0 when idle).
- Every rendered buffer must be checked finite (no NaN/inf). Use the shared test helper
  rather than re-rolling the loop.
- Long-tail behaviour needs explicit tests: after note-off, render until silent and bound
  how long that takes; assert the voice actually returns to `Idle` rather than ringing
  forever at denormal amplitude.
- **Tests parametrise over configuration; they never pin it.** Derive every buffer size,
  note range, and timing from the constants/config so retuning the engine never requires
  touching a test. (This has already bitten once: a hardcoded 16-slot buffer broke when the
  voice count changed to 32.) Behaviours with config-dependent edge cases should be
  exercised at the extremes of the *valid* ranges, not just the current values. Defining
  and enforcing "valid" is the config validation layer's job (see architecture constraints)
  and that layer is itself unit-tested: every invalid value falls back to its safe default
  and produces a warning.

### Coverage

- **Tool:** `cargo-llvm-cov`. Run plain `cargo llvm-cov` (all test targets) — **not**
  `cargo llvm-cov --lib`, which silently excludes everything under `tests/` and undercounts.
- **Scope:** the lib, minus GUI (`src/ui/`), CLI glue (`src/main.rs`), and hardware glue
  (`audio_device_connector.rs`, `midi_device_connector.rs`) — see the command above.
- **Target: ≥ 90 % line coverage on that scope.** Function/region coverage is reported and
  worth glancing at, but only the line figure gates. (Branch coverage requires nightly and
  rewards chasing degenerate clamp arms; not worth it here.)
- Do not chase 100 %: unreachable panic sentinels, `unreachable!()` arms, and trivial
  one-liners do not need tests.

### Behaviour-driven testing (mandatory)

Tests verify **expected behaviour and outcomes**, not implementation details.

Do test:
- Correct outputs for valid inputs, via the public API.
- Edge cases and boundaries: empty/zero-length input, velocity 0 (= note-off), pitch 0/127,
  parameter values at and beyond clamp limits, full voice pool, retrigger of a held note.
- Every error path: each `Err(...)`/`None` return and each documented panic
  (`#[should_panic]`) gets at least one test.
- State transitions observable through the public API (envelope stages, voice lifecycle,
  active-note bookkeeping).

Do NOT test:
- Trivial field accessors, `#[derive(...)]` output, or standard-library behaviour.
- Private internals via reflection-style tricks; if a behaviour can't be observed through
  public API, first ask whether the design needs a seam, then whether the test is needed.
- Assertions that merely restate the implementation (a hardcoded value copied from the
  code). A useful test fails when the implementation is wrong, survives refactors, and
  documents intent another developer can read.

Quality checklist before a test is done:
1. Would it fail if the implementation had a bug?
2. Does it verify *what* the code should do, not *how*?
3. Is it independent of implementation details that change under refactoring?
4. Does it document expected behaviour clearly?
5. Is it deterministic — no sleeps, no reliance on wall-clock timing or device presence?

**Table-driven tests** are preferred when multiple cases share an assertion shape: iterate a
slice of tuples instead of duplicating test functions (the MIDI CC map is the canonical
example).
