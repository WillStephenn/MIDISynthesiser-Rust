//! A utility module for listing and connecting to available MIDI input
//! devices (port of `synth.midi.MidiDeviceConnector`, using `midir` in place
//! of `javax.sound.midi`).

use std::io::Write as _;
use std::sync::{Arc, Mutex};

use midir::{MidiInput, MidiInputConnection};

use crate::core::synthesiser::Synthesiser;
use crate::midi::midi_input_handler::{ControlChangeCallback, MidiInputHandler};

/// Creates a fresh `MidiInput` client handle, or `None` if the MIDI backend
/// is unavailable.
fn midi_input(verbose: bool) -> Option<MidiInput> {
    match MidiInput::new("June's Logue") {
        Ok(input) => Some(input),
        Err(e) => {
            if verbose {
                eprintln!("Could not initialise MIDI input: {e}");
            }
            None
        }
    }
}

/// Returns all available MIDI input device names.
///
/// An input device is one that can send MIDI messages (an input port in
/// `midir` terms). Equivalent to `get_midi_devices_list(false)`
/// (no console output).
pub fn get_midi_devices_list() -> Vec<String> {
    get_midi_devices_list_verbose(false)
}

/// Returns all available MIDI input device names, optionally printing a
/// numbered selection menu to the console.
pub fn get_midi_devices_list_verbose(verbose: bool) -> Vec<String> {
    let mut devices = Vec::new();
    if verbose {
        println!("--- Select MIDI Input Device ---");
    }
    let Some(input) = midi_input(verbose) else {
        return devices;
    };
    let ports = input.ports();
    if ports.is_empty() {
        if verbose {
            println!("No MIDI devices found.");
        }
        return devices;
    }
    let mut i = 1;
    for port in &ports {
        match input.port_name(port) {
            Ok(name) => {
                devices.push(name.clone());
                if verbose {
                    println!("{i}- {name}");
                }
                i += 1;
            }
            Err(e) => {
                if verbose {
                    eprintln!("Skipping MIDI device: {e}");
                }
            }
        }
    }
    if verbose {
        println!("------------------------------------");
    }
    devices
}

/// Prompts the user to select a MIDI input device.
///
/// Returns the name of the selected MIDI device, or `None` if none are
/// found (or stdin is closed).
pub fn prompt_user() -> Option<String> {
    let devices = get_midi_devices_list_verbose(true);
    if devices.is_empty() {
        println!("No MIDI devices available.");
        return None;
    }

    loop {
        print!("Enter the number of the device you want to use: ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => return None, // EOF / read error
            Ok(_) => {}
        }
        match line.trim().parse::<usize>() {
            Ok(input) if (1..=devices.len()).contains(&input) => {
                let selected = devices[input - 1].clone();
                println!("Selecting {selected}");
                return Some(selected);
            }
            Ok(_) => {
                println!(
                    "Invalid number. Please enter a number between 1 and {}.",
                    devices.len()
                );
            }
            Err(_) => println!("Invalid input. Please enter a number."),
        }
    }
}

/// Connects the synthesiser to the first MIDI input device found with the
/// specified name.
///
/// Returns the open [`MidiInputConnection`] if successful, otherwise `None`.
/// The connection stays open (and keeps feeding the synthesiser) until the
/// returned handle is dropped or closed — the equivalent of the Java
/// `MidiDevice` handle the caller had to `close()`.
pub fn connect_to_device(
    synth: Arc<Mutex<Synthesiser>>,
    device_name: &str,
) -> Option<MidiInputConnection<()>> {
    connect_to_device_with_callback(synth, device_name, None)
}

/// Connects the synthesiser to the first MIDI input device found with the
/// specified name, with an optional callback invoked after each MIDI CC
/// message is processed.
pub fn connect_to_device_with_callback(
    synth: Arc<Mutex<Synthesiser>>,
    device_name: &str,
    on_control_change: Option<ControlChangeCallback>,
) -> Option<MidiInputConnection<()>> {
    let input = midi_input(true)?;
    for port in input.ports() {
        let Ok(name) = input.port_name(&port) else {
            continue;
        };
        if name == device_name {
            let handler = match on_control_change {
                Some(callback) => {
                    MidiInputHandler::with_control_change_callback(synth, callback)
                }
                None => MidiInputHandler::new(synth),
            };
            match input.connect(
                &port,
                "junes-logue-input",
                move |_timestamp, message, _| handler.send(message),
                (),
            ) {
                Ok(connection) => {
                    println!("Successfully connected to MIDI device: {device_name}");
                    return Some(connection);
                }
                Err(e) => {
                    eprintln!("Could not open MIDI device '{device_name}': {e}");
                    return None;
                }
            }
        }
    }
    eprintln!("Could not find MIDI device: '{device_name}'");
    None
}
