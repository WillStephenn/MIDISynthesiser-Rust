//! Handles playback of MIDI files, sending MIDI events to a synthesiser
//! (port of `synth.midi.MidiFilePlayer`, using `midly` in place of the
//! `javax.sound.midi` `Sequencer`).
//!
//! Java's `Sequencer` played the file asynchronously on its own thread and
//! returned a handle the caller could stop or poll. This port replicates that:
//! [`MidiFilePlayer::play_midi_file`] parses the file up front (returning
//! `None` on any error, as the Java method returned `null`), then spawns a
//! playback thread that walks the merged, tempo-mapped event list in real
//! time and feeds a [`MidiInputHandler`]. The returned [`MidiFilePlayback`]
//! handle is the `Sequencer` stand-in: poll it with
//! [`is_running`](MidiFilePlayback::is_running), halt it with
//! [`stop`](MidiFilePlayback::stop), or block with
//! [`wait`](MidiFilePlayback::wait). Dropping the handle does NOT stop
//! playback (matching the fire-and-forget Java behaviour).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};

use crate::core::synthesiser::Synthesiser;
use crate::midi::midi_input_handler::MidiInputHandler;

/// The default MIDI tempo: 500,000 microseconds per quarter note (120 BPM).
const DEFAULT_TEMPO_US_PER_QN: f64 = 500_000.0;

/// An owned, playback-relevant MIDI event at an absolute tick position.
#[derive(Debug, Clone, Copy)]
enum FileEvent {
    NoteOn { key: u8, vel: u8 },
    NoteOff { key: u8 },
    Controller { controller: u8, value: u8 },
    Tempo { us_per_qn: u32 },
}

/// A handle to an in-progress MIDI file playback (the `Sequencer` equivalent).
pub struct MidiFilePlayback {
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MidiFilePlayback {
    /// Returns `true` while the playback thread is still delivering events.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Asks the playback thread to stop after the event it is currently
    /// waiting on. Any notes still sounding are released (note-off) so no
    /// voices are left hanging.
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    /// Blocks until playback finishes (or is stopped).
    pub fn wait(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Handles playback of MIDI files, sending MIDI events to a synthesiser.
pub struct MidiFilePlayer {
    synth: Arc<Mutex<Synthesiser>>,
}

impl MidiFilePlayer {
    /// Constructs a `MidiFilePlayer`.
    pub fn new(synth: Arc<Mutex<Synthesiser>>) -> Self {
        Self { synth }
    }

    /// Plays a MIDI file from the given file path.
    ///
    /// Playback starts immediately on a background thread. Returns the
    /// [`MidiFilePlayback`] handle used for playback, or `None` if an error
    /// occurs (file missing, unreadable, or not valid MIDI).
    pub fn play_midi_file(&self, file_path: &str) -> Option<MidiFilePlayback> {
        if file_path.trim().is_empty() {
            eprintln!("Error: MIDI file path is null or empty.");
            return None;
        }

        // Load MIDI file
        let bytes = match std::fs::read(file_path) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("Error: MIDI file not found at {file_path} ({e})");
                return None;
            }
        };
        let smf = match Smf::parse(&bytes) {
            Ok(smf) => smf,
            Err(e) => {
                eprintln!("Error playing MIDI file: {e}");
                return None;
            }
        };

        let (events, us_per_tick, tempo_scalable) = extract_events(&smf);

        // Connect the playback thread to the synthesiser via the same
        // handler the live MIDI input path uses (as in the Java original).
        let handler = MidiInputHandler::new(Arc::clone(&self.synth));

        println!("Playing MIDI file: {file_path}");

        let running = Arc::new(AtomicBool::new(true));
        let thread_running = Arc::clone(&running);
        let handle = std::thread::Builder::new()
            .name("midi-file-player".into())
            .spawn(move || {
                play_events(
                    &events,
                    us_per_tick,
                    tempo_scalable,
                    &handler,
                    &thread_running,
                );
                thread_running.store(false, Ordering::Relaxed);
            })
            .ok()?;

        Some(MidiFilePlayback {
            running,
            handle: Some(handle),
        })
    }
}

/// Merges all tracks of the file into a single absolute-tick-ordered event
/// list, and computes the initial tick duration.
///
/// Returns `(events, initial_us_per_tick, tempo_scalable)`, where
/// `tempo_scalable` is `true` for metrical (ticks-per-quarter-note) timing,
/// in which case tempo meta events rescale the tick duration during playback.
fn extract_events(smf: &Smf) -> (Vec<(u64, FileEvent)>, f64, bool) {
    let (us_per_tick, tempo_scalable) = match smf.header.timing {
        Timing::Metrical(ticks_per_qn) => {
            (DEFAULT_TEMPO_US_PER_QN / ticks_per_qn.as_int() as f64, true)
        }
        Timing::Timecode(fps, subframe) => {
            (1_000_000.0 / (fps.as_f32() as f64 * subframe as f64), false)
        }
    };

    let mut events: Vec<(u64, FileEvent)> = Vec::new();
    for track in &smf.tracks {
        let mut tick: u64 = 0;
        for event in track {
            tick += event.delta.as_int() as u64;
            match event.kind {
                TrackEventKind::Midi { message, .. } => match message {
                    MidiMessage::NoteOn { key, vel } => events.push((
                        tick,
                        FileEvent::NoteOn {
                            key: key.as_int(),
                            vel: vel.as_int(),
                        },
                    )),
                    MidiMessage::NoteOff { key, .. } => {
                        events.push((tick, FileEvent::NoteOff { key: key.as_int() }))
                    }
                    MidiMessage::Controller { controller, value } => events.push((
                        tick,
                        FileEvent::Controller {
                            controller: controller.as_int(),
                            value: value.as_int(),
                        },
                    )),
                    _ => {}
                },
                TrackEventKind::Meta(MetaMessage::Tempo(us_per_qn)) => events.push((
                    tick,
                    FileEvent::Tempo {
                        us_per_qn: us_per_qn.as_int(),
                    },
                )),
                _ => {}
            }
        }
    }
    // Stable sort keeps same-tick events in track order (tempo track first).
    events.sort_by_key(|(tick, _)| *tick);
    (events, us_per_tick, tempo_scalable)
}

/// Walks the merged event list in real time, dispatching each event to the
/// handler. Checks the running flag between events so `stop()` takes effect
/// promptly, and releases any still-sounding notes on exit.
fn play_events(
    events: &[(u64, FileEvent)],
    initial_us_per_tick: f64,
    tempo_scalable: bool,
    handler: &MidiInputHandler,
    running: &AtomicBool,
) {
    let mut us_per_tick = initial_us_per_tick;
    let ticks_per_qn = if tempo_scalable {
        DEFAULT_TEMPO_US_PER_QN / initial_us_per_tick
    } else {
        0.0
    };

    let start = Instant::now();
    let mut elapsed_us: f64 = 0.0;
    let mut last_tick: u64 = 0;
    // Tracks which notes are currently sounding so they can be released
    // if playback is stopped early.
    let mut notes_on = [false; 128];

    for &(tick, event) in events {
        elapsed_us += (tick - last_tick) as f64 * us_per_tick;
        last_tick = tick;

        // Sleep in short slices so stop() takes effect promptly.
        let target = Duration::from_micros(elapsed_us as u64);
        loop {
            if !running.load(Ordering::Relaxed) {
                release_all(handler, &notes_on);
                return;
            }
            let now = start.elapsed();
            if now >= target {
                break;
            }
            std::thread::sleep((target - now).min(Duration::from_millis(50)));
        }

        match event {
            FileEvent::NoteOn { key, vel } => {
                handler.note_on(key, vel);
                notes_on[key as usize] = vel > 0;
            }
            FileEvent::NoteOff { key } => {
                handler.note_off(key);
                notes_on[key as usize] = false;
            }
            FileEvent::Controller { controller, value } => {
                handler.control_change(controller, value);
            }
            FileEvent::Tempo { us_per_qn } => {
                if tempo_scalable {
                    us_per_tick = us_per_qn as f64 / ticks_per_qn;
                }
            }
        }
    }
}

/// Sends a note-off for every note currently sounding.
fn release_all(handler: &MidiInputHandler, notes_on: &[bool; 128]) {
    for (pitch, on) in notes_on.iter().enumerate() {
        if *on {
            handler.note_off(pitch as u8);
        }
    }
}
