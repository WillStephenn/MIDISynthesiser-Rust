//! The main entry point for the application (port of `synth.Main`).
//!
//! Like the Java `Main`, running with no arguments launches the GUI
//! ([`midi_synthesiser::ui::synth_application::launch`]). Passing `--cli` (or
//! any CLI-only option such as `--list-devices`, `--play`, `--audio-device`,
//! `--midi-device` or `--ascii`) instead runs the console front end
//! ([`run_cli`]): list/select an audio output and MIDI input device, run the
//! synthesiser live, optionally play a MIDI file and/or show the ASCII
//! visualisation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use midi_synthesiser::core::synthesiser::Synthesiser;
use midi_synthesiser::midi::{midi_device_connector, midi_file_player::MidiFilePlayer};
use midi_synthesiser::utils::audio_constants::{BLOCK_SIZE, NUMBER_OF_VOICES, SAMPLE_RATE};
use midi_synthesiser::utils::audio_device_connector;
use midi_synthesiser::visualisation::ascii_renderer;

/// Parsed command-line options.
#[derive(Default)]
struct Options {
    cli: bool,
    list_devices: bool,
    audio_device: Option<String>,
    midi_device: Option<String>,
    play_file: Option<String>,
    ascii: bool,
}

impl Options {
    /// Whether any option implies the console front end. With no arguments
    /// (the Java `Main` behaviour) the GUI launches instead.
    fn wants_cli(&self) -> bool {
        self.cli
            || self.list_devices
            || self.ascii
            || self.audio_device.is_some()
            || self.midi_device.is_some()
            || self.play_file.is_some()
    }
}

const USAGE: &str = "\
June's Logue - MIDI Synthesiser

USAGE:
  midi-synthesiser [OPTIONS]

With no options the graphical interface is launched. Any of the options below
runs the command-line interface instead:

OPTIONS:
  --cli                    Run in CLI mode (prompts for devices interactively)
  --list-devices           List audio output and MIDI input devices, then exit
  --audio-device <NAME|N>  Audio output device, by name or 1-based list index
                           (otherwise prompts interactively)
  --midi-device <NAME|N>   MIDI input device, by name or 1-based list index
                           (otherwise prompts interactively)
  --play <FILE>            Play a .mid file through the synthesiser instead of
                           connecting live MIDI input
  --ascii                  Show a live ASCII visualisation of the synth state
  -h, --help               Show this help
";

/// The main method. Parses arguments, then launches the GUI (the default,
/// matching the Java `Main`) or runs the CLI front end.
fn main() {
    let options = match parse_args() {
        Ok(Some(options)) => options,
        Ok(None) => return, // --help
        Err(message) => {
            eprintln!("{message}");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };

    if options.wants_cli() {
        if let Err(e) = run_cli(options) {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    } else if let Err(e) = midi_synthesiser::ui::synth_application::launch() {
        eprintln!("GUI error: {e}");
        std::process::exit(1);
    }
}

/// Parses command-line arguments. Returns `Ok(None)` if help was requested.
fn parse_args() -> Result<Option<Options>, String> {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "--list-devices" | "--list" => options.list_devices = true,
            "--audio-device" => {
                options.audio_device = Some(args.next().ok_or("--audio-device requires a value")?);
            }
            "--midi-device" => {
                options.midi_device = Some(args.next().ok_or("--midi-device requires a value")?);
            }
            "--play" => {
                options.play_file = Some(args.next().ok_or("--play requires a file path")?);
            }
            "--ascii" => options.ascii = true,
            "--cli" => options.cli = true,
            other => return Err(format!("Unknown argument: {other}")),
        }
    }
    Ok(Some(options))
}

/// Runs the synthesiser as a console application: selects devices, starts the
/// audio stream, attaches live MIDI input or plays a MIDI file, and runs
/// until playback finishes or the user presses Enter.
fn run_cli(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    if options.list_devices {
        audio_device_connector::get_audio_output_device_list_verbose(true);
        midi_device_connector::get_midi_devices_list_verbose(true);
        return Ok(());
    }

    // The shared synthesiser: locked once per block by the audio callback,
    // briefly by MIDI handlers and the visualisation (as in the Java app,
    // where UI/MIDI threads called the synth directly under its locks).
    let synth = Arc::new(Mutex::new(Synthesiser::new(
        NUMBER_OF_VOICES,
        SAMPLE_RATE,
        BLOCK_SIZE,
    )));

    // --- Audio output device ---
    let audio_device_name = match &options.audio_device {
        Some(arg) => resolve_device(
            arg,
            &audio_device_connector::get_audio_output_device_list(),
            "audio output",
        )?,
        None => audio_device_connector::prompt_user().ok_or("No audio output device selected")?,
    };
    // The stream renders in the background; audio stops when it is dropped.
    let _stream =
        audio_device_connector::start_output_stream(&audio_device_name, Arc::clone(&synth))?;

    // --- ASCII visualisation (optional) ---
    let visualising = Arc::new(AtomicBool::new(options.ascii));
    let visualiser = options.ascii.then(|| {
        let synth = Arc::clone(&synth);
        let running = Arc::clone(&visualising);
        std::thread::spawn(move || {
            while running.load(Ordering::Relaxed) {
                // Format under a brief lock, print after releasing it so the
                // audio callback is never blocked on console I/O.
                let frame = {
                    let guard = synth.lock().unwrap_or_else(|p| p.into_inner());
                    ascii_renderer::render_to_string(&guard)
                };
                ascii_renderer::clear_console();
                println!("{frame}");
                std::thread::sleep(Duration::from_millis(100));
            }
        })
    });

    // --- MIDI file playback or live MIDI input ---
    if let Some(file_path) = &options.play_file {
        let player = MidiFilePlayer::new(Arc::clone(&synth));
        let playback = player
            .play_midi_file(file_path)
            .ok_or("MIDI file playback failed")?;
        playback.wait();
        // Let release tails ring out before tearing the stream down.
        std::thread::sleep(Duration::from_secs(2));
    } else {
        let midi_device_name = match &options.midi_device {
            Some(arg) => Some(resolve_device(
                arg,
                &midi_device_connector::get_midi_devices_list(),
                "MIDI input",
            )?),
            None => midi_device_connector::prompt_user(),
        };
        // Keep the connection handle alive for the duration of the session.
        let _connection = match midi_device_name {
            Some(name) => midi_device_connector::connect_to_device(Arc::clone(&synth), &name),
            None => {
                println!("Continuing without MIDI input.");
                None
            }
        };

        println!("Synth running. Press Enter to quit.");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }

    visualising.store(false, Ordering::Relaxed);
    if let Some(handle) = visualiser {
        let _ = handle.join();
    }
    Ok(())
}

/// Resolves a device argument that is either a 1-based index into the device
/// list (matching the interactive prompt numbering) or a device name.
fn resolve_device(arg: &str, devices: &[String], kind: &str) -> Result<String, String> {
    if let Ok(index) = arg.parse::<usize>() {
        return devices.get(index.wrapping_sub(1)).cloned().ok_or_else(|| {
            format!(
                "Invalid {kind} device index {index}; {} device(s) available",
                devices.len()
            )
        });
    }
    // Pass names through even if not currently listed; the connectors handle
    // (and report) missing devices with their own fallback behaviour.
    Ok(arg.to_string())
}
