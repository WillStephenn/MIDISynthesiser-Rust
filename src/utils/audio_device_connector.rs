//! A utility module for listing and connecting to available audio output
//! devices (port of `synth.utils.AudioDeviceConnector`, using `cpal` in place
//! of `javax.sound.sampled`).
//!
//! The Java original handed back a raw `SourceDataLine` and left the render
//! loop to the caller (a dedicated audio thread that rendered fixed 256-frame
//! blocks and wrote 16-bit big-endian PCM to the line). With `cpal` the render
//! loop *is* the device callback, so this module also owns the equivalent of
//! that loop: [`start_output_stream`] builds an output stream whose callback
//! pulls fixed [`BLOCK_SIZE`] frame blocks from the shared
//! [`Synthesiser`] and bridges them onto cpal's variable-sized buffers with a
//! leftover-carrying block buffer.

use std::io::Write as _;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

use crate::core::synthesiser::Synthesiser;
use crate::utils::audio_constants::{BLOCK_SIZE, SAMPLE_RATE};

/// Returns the human-readable name of a cpal output device, if available.
fn device_name(device: &cpal::Device) -> Result<String, cpal::Error> {
    device.description().map(|d| d.name().to_string())
}

/// Returns all available audio output device names.
///
/// An output device is one the host reports as supporting output streams.
/// Equivalent to `get_audio_output_device_list(false)` (no console output).
pub fn get_audio_output_device_list() -> Vec<String> {
    get_audio_output_device_list_verbose(false)
}

/// Returns all available audio output device names, optionally printing a
/// numbered selection menu to the console.
pub fn get_audio_output_device_list_verbose(verbose: bool) -> Vec<String> {
    let mut devices = Vec::new();
    if verbose {
        println!("--- Select Audio Output Device ---");
    }
    let host = cpal::default_host();
    let mut i = 1;
    match host.output_devices() {
        Ok(output_devices) => {
            for device in output_devices {
                match device_name(&device) {
                    Ok(name) => {
                        devices.push(name.clone());
                        if verbose {
                            println!("{i}- {name}");
                        }
                        i += 1;
                    }
                    Err(e) => {
                        if verbose {
                            eprintln!("Skipping audio device: {e}");
                        }
                    }
                }
            }
        }
        Err(e) => {
            if verbose {
                eprintln!("Could not enumerate audio devices: {e}");
            }
        }
    }
    if verbose {
        println!("------------------------------------");
    }
    devices
}

/// Prompts the user to select an audio output device.
///
/// Returns the name of the selected audio device, or `None` if none are
/// found (or stdin is closed).
pub fn prompt_user() -> Option<String> {
    let devices = get_audio_output_device_list_verbose(true);
    if devices.is_empty() {
        println!("No audio output devices available.");
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

/// Gets an output [`cpal::Device`] by name.
///
/// If the named device is not found, it falls back to the system's default
/// output device (mirroring the Java `getOutputLine` fallback semantics).
pub fn get_output_device(device_name: &str) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Ok(output_devices) = host.output_devices() {
        for device in output_devices {
            if self::device_name(&device)
                .map(|n| n == device_name)
                .unwrap_or(false)
            {
                println!("Successfully connected to audio device: {device_name}");
                return Some(device);
            }
        }
    }

    // If the named device wasn't found, try getting the default device
    eprintln!("Audio device '{device_name}' not found or failed to open. Using default device.");
    match host.default_output_device() {
        Some(device) => {
            println!("Successfully connected to default audio device.");
            Some(device)
        }
        None => {
            eprintln!("Could not get default audio device.");
            None
        }
    }
}

/// Builds and starts a stereo output stream on the named device, rendering
/// audio from the shared synthesiser.
///
/// The stream callback locks the synthesiser exactly once per engine block,
/// renders [`BLOCK_SIZE`] frames of interleaved stereo `f64` audio and copies
/// it (converted to the device sample format) into cpal's buffer, carrying any
/// leftover samples over to the next callback. No allocation happens in the
/// callback after startup.
///
/// Requests 44100 Hz stereo; if the device does not support that rate the
/// device's default output configuration is used instead (with a warning that
/// pitch/timing will be off, since the engine always renders at 44100 Hz).
///
/// The returned [`cpal::Stream`] is already playing. Audio stops when the
/// stream is dropped (or paused via [`StreamTrait::pause`]).
pub fn start_output_stream(
    device_name: &str,
    synth: Arc<Mutex<Synthesiser>>,
) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    let device =
        get_output_device(device_name).ok_or("No audio output device available")?;
    start_output_stream_on(&device, synth)
}

/// Builds and starts a stereo output stream on the given device.
/// See [`start_output_stream`].
pub fn start_output_stream_on(
    device: &cpal::Device,
    synth: Arc<Mutex<Synthesiser>>,
) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    let target_rate: cpal::SampleRate = SAMPLE_RATE as cpal::SampleRate;

    // Prefer a stereo config at the engine sample rate, f32 first.
    let mut chosen: Option<cpal::SupportedStreamConfig> = None;
    if let Ok(configs) = device.supported_output_configs() {
        let mut candidates: Vec<_> = configs
            .filter(|c| {
                c.channels() == 2
                    && c.min_sample_rate() <= target_rate
                    && c.max_sample_rate() >= target_rate
            })
            .collect();
        // Prefer f32 output so the f64 engine samples convert losslessly.
        candidates.sort_by_key(|c| match c.sample_format() {
            cpal::SampleFormat::F32 => 0,
            cpal::SampleFormat::I16 => 1,
            cpal::SampleFormat::U16 => 2,
            _ => 3,
        });
        chosen = candidates
            .into_iter()
            .next()
            .map(|c| c.with_sample_rate(target_rate));
    }

    let supported = match chosen {
        Some(c) => c,
        None => {
            let default = device.default_output_config()?;
            eprintln!(
                "Warning: device does not support stereo @ {} Hz; using default config \
                 ({} ch @ {} Hz). The engine renders at {} Hz, so pitch/timing may be off.",
                SAMPLE_RATE as u32,
                default.channels(),
                default.sample_rate(),
                SAMPLE_RATE as u32
            );
            if default.channels() != 2 {
                return Err("Default output device configuration is not stereo".into());
            }
            default
        }
    };

    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.config();

    let stream = match sample_format {
        cpal::SampleFormat::F32 => build_stream::<f32>(device, &config, synth)?,
        cpal::SampleFormat::I16 => build_stream::<i16>(device, &config, synth)?,
        cpal::SampleFormat::U16 => build_stream::<u16>(device, &config, synth)?,
        other => return Err(format!("Unsupported sample format: {other}").into()),
    };
    stream.play()?;
    Ok(stream)
}

/// Builds the output stream for a concrete device sample format.
///
/// The engine's `f64` samples (already clipped to +/-1) are converted via
/// `f32` to the device format. This replaces the Java render thread's manual
/// 16-bit big-endian PCM conversion; cpal performs the equivalent scaling
/// when the device format is integer PCM.
fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    synth: Arc<Mutex<Synthesiser>>,
) -> Result<cpal::Stream, Box<dyn std::error::Error>>
where
    T: SizedSample + FromSample<f32>,
{
    // Leftover-carrying block buffer: the engine always renders exactly
    // BLOCK_SIZE frames; cpal callbacks consume them at whatever size it asks.
    let mut block = vec![0.0f64; BLOCK_SIZE * 2];
    let mut pos = block.len(); // start "empty" so the first callback renders

    let data_fn = move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
        let mut i = 0;
        while i < data.len() {
            if pos >= block.len() {
                // Lock once per engine block, exactly like the Java audio
                // thread's synchronized processBlock call.
                let mut guard = match synth.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                guard.process_block(&mut block);
                pos = 0;
            }
            let n = (block.len() - pos).min(data.len() - i);
            for k in 0..n {
                data[i + k] = T::from_sample(block[pos + k] as f32);
            }
            pos += n;
            i += n;
        }
    };

    let err_fn = |err: cpal::Error| {
        eprintln!("Audio stream error: {err}");
    };

    Ok(device.build_output_stream(*config, data_fn, err_fn, None)?)
}
