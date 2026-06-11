//! A MIDI receiver that processes incoming MIDI messages and controls a
//! synthesiser (port of `synth.midi.MidiInputHandler`).
//!
//! It handles Note On and Note Off events to trigger and release voices, and
//! maps Control Change messages onto the synthesiser's parameters.
//!
//! Where the Java class implemented `javax.sound.midi.Receiver` and received
//! decoded `ShortMessage`s, this port receives raw MIDI bytes (as delivered by
//! `midir`) via [`MidiInputHandler::send`] and decodes them itself. The
//! [`MidiFilePlayer`](crate::midi::midi_file_player::MidiFilePlayer) reuses
//! the same handler through the typed [`note_on`](MidiInputHandler::note_on) /
//! [`note_off`](MidiInputHandler::note_off) /
//! [`control_change`](MidiInputHandler::control_change) entry points.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::core::synthesiser::{Synthesiser, Waveform};

/// MIDI status nibble for Note Off messages.
const NOTE_OFF: u8 = 0x80;
/// MIDI status nibble for Note On messages.
const NOTE_ON: u8 = 0x90;
/// MIDI status nibble for Control Change messages.
const CONTROL_CHANGE: u8 = 0xB0;

/// Optional callback invoked after a recognised MIDI CC message is processed
/// (used by the UI layer to coalesce parameter readout refreshes).
pub type ControlChangeCallback = Box<dyn Fn() + Send + Sync + 'static>;

/// A MIDI receiver that processes incoming MIDI messages and controls a
/// synthesiser.
pub struct MidiInputHandler {
    synth: Arc<Mutex<Synthesiser>>,
    on_control_change: Option<ControlChangeCallback>,
}

impl MidiInputHandler {
    /// Constructs a `MidiInputHandler`.
    pub fn new(synth: Arc<Mutex<Synthesiser>>) -> Self {
        Self {
            synth,
            on_control_change: None,
        }
    }

    /// Constructs a `MidiInputHandler` with a control change callback,
    /// invoked after each recognised CC message is processed.
    pub fn with_control_change_callback(
        synth: Arc<Mutex<Synthesiser>>,
        on_control_change: ControlChangeCallback,
    ) -> Self {
        Self {
            synth,
            on_control_change: Some(on_control_change),
        }
    }

    /// Processes an incoming raw MIDI message, sending the control signals to
    /// the synthesiser (equivalent of the Java `Receiver.send`).
    ///
    /// Only channel voice messages (Note On/Off, Control Change) are handled;
    /// everything else is ignored, as in the Java original.
    pub fn send(&self, message: &[u8]) {
        if message.len() < 3 {
            return;
        }
        let command = message[0] & 0xF0;
        let data1 = message[1] & 0x7F;
        let data2 = message[2] & 0x7F;

        match command {
            NOTE_ON => self.note_on(data1, data2),
            NOTE_OFF => self.note_off(data1),
            CONTROL_CHANGE => self.control_change(data1, data2),
            _ => {}
        }
    }

    /// Handles a Note On message. A velocity byte of 0 is treated as a
    /// Note Off, per the MIDI spec (and the Java original).
    pub fn note_on(&self, pitch: u8, velocity_byte: u8) {
        // Converts the velocity byte to a scalar, as in the Java original.
        let velocity = velocity_byte as f64 / 127.0;
        if velocity > 0.0 {
            self.lock_synth().note_on(pitch, velocity);
        } else {
            self.lock_synth().note_off(pitch);
        }
    }

    /// Handles a Note Off message.
    pub fn note_off(&self, pitch: u8) {
        self.lock_synth().note_off(pitch);
    }

    /// Handles a Control Change message, routing the controller number to the
    /// matching synthesiser parameter (same CC map as the Java original).
    pub fn control_change(&self, controller: u8, value: u8) {
        let scaled_value = value as f64 / 127.0;
        let mut synth = self.lock_synth();

        // Parameter control switch:
        let mut handled = true;
        match controller {
            // --- OSCILLATOR CONTROLS ---
            32 => {
                // Modulation Wheel, LFO Frequency
                synth.set_lfo_frequency(0.1 + (scaled_value * 9.9));
            }
            13 => {
                // Oscillator Waveform
                synth.set_oscillator_waveform(waveform_from_cc(value));
            }
            17 => {
                // LFO Waveform
                synth.set_lfo_waveform(waveform_from_cc(value));
            }

            // --- FILTER CONTROLS ---
            10 => {
                // Freq Cutoff: logarithmic mapping
                let min_freq = 20.0;
                let max_freq = 20000.0;
                let new_cutoff = min_freq * f64::powf(max_freq / min_freq, scaled_value);
                synth.set_filter_cutoff(new_cutoff);
            }
            11 => {
                // Resonance
                synth.set_filter_resonance(1.0 + (scaled_value * 14.0));
            }
            12 => {
                // Filter Mod Range, from 0 to 10KHz
                synth.set_filter_mod_range(scaled_value * 10000.0);
            }

            // --- FILTER ENVELOPE ---
            1 => synth.set_filter_attack_time(scaled_value * 10.0),
            2 => synth.set_filter_decay_time(scaled_value * 10.0),
            3 => synth.set_filter_sustain_level(scaled_value),
            4 => synth.set_filter_release_time(scaled_value * 10.0),

            // --- AMPLITUDE ENVELOPE ---
            5 => synth.set_amp_attack_time(scaled_value * 10.0),
            6 => synth.set_amp_release_time(scaled_value * 10.0),
            7 => synth.set_amp_sustain_level(scaled_value),
            8 => synth.set_amp_decay_time(scaled_value * 10.0),

            // --- GAIN & PANNING ---
            9 => {
                // Master Volume
                synth.set_master_volume(scaled_value);
            }
            14 => {
                // Pre-Filter Gain: ranges -24dB to +24dB
                synth.set_pre_filter_gain_db((scaled_value * 48.0) - 24.0);
            }
            15 => {
                // Post-Filter Gain: ranges -24dB to +24dB
                synth.set_post_filter_gain_db((scaled_value * 48.0) - 24.0);
            }
            16 => {
                // Pan Depth
                synth.set_pan_depth(scaled_value);
            }
            _ => handled = false,
        }
        drop(synth);

        if handled && let Some(callback) = &self.on_control_change {
            callback();
        }
    }

    /// Locks the shared synthesiser, recovering from a poisoned lock so a
    /// panic elsewhere cannot silence the MIDI path.
    fn lock_synth(&self) -> MutexGuard<'_, Synthesiser> {
        match self.synth.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// Maps a 0-127 CC value to a waveform by quartile, as in the Java original.
fn waveform_from_cc(value: u8) -> Waveform {
    match value {
        0..=31 => Waveform::Sine,
        32..=63 => Waveform::Saw,
        64..=95 => Waveform::Triangle,
        _ => Waveform::Square,
    }
}
