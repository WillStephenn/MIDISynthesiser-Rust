//! Pre-computes expensive math functions on startup in order to optimise
//! real-time performance (port of `synth.utils.LookupTables`).
//!
//! The Java original uses static initialisation; here all tables live in one
//! [`LookupTables`] struct behind a [`LazyLock`] so they are computed exactly
//! once, on first use. Audio components cache `&'static` references to the
//! tables at construction time so the audio path never touches the lock and
//! never allocates.

use std::sync::LazyLock;

use crate::utils::{audio_constants, engine_config};

/// The raw, *unvalidated* [`audio_constants::LOOKUP_TABLE_SIZE`] constant.
///
/// Kept for callers (and tests) that need a compile-time table size for
/// arithmetic such as deriving an exact-period test frequency. The actual
/// [`LookupTables`] (and therefore the oscillators' phase wrapping) are sized
/// from the *validated* [`table_size`], not this constant: if
/// [`audio_constants::LOOKUP_TABLE_SIZE`] is valid (the default and the only
/// value this constant should ever be set to), the two agree.
pub const TABLE_SIZE: usize = audio_constants::LOOKUP_TABLE_SIZE;

/// Returns the number of entries in each waveform table.
///
/// This is the *validated* lookup-table size (see
/// [`crate::utils::engine_config`]), guaranteed to be a power of two: an
/// invalid [`audio_constants::LOOKUP_TABLE_SIZE`] falls back to the
/// documented safe default rather than corrupting the oscillators' bitmask
/// phase wrapping.
pub fn table_size() -> usize {
    engine_config::validated_config().lookup_table_size
}

/// The number of discrete resonance values in the filter coefficient tables.
pub const RESONANCE_STEPS: usize = 128;

/// The set of all pre-computed lookup tables used by the engine.
pub struct LookupTables {
    /// The number of entries in `sine`, `cosine`, `square`, `saw`,
    /// `triangle`, `tan_table`, `a1_table`, `a2_table` and `a3_table` (the
    /// validated lookup-table size; see [`table_size`]).
    pub table_size: usize,
    /// One cycle of a sine wave, `sin(2*pi*i/table_size)`.
    pub sine: Vec<f64>,
    /// One cycle of a cosine wave, `cos(2*pi*i/table_size)`.
    pub cosine: Vec<f64>,
    /// One cycle of a square wave (+1.0 for the first half, -1.0 for the second).
    pub square: Vec<f64>,
    /// One cycle of a saw wave, rising linearly from -1.0 to 1.0.
    pub saw: Vec<f64>,
    /// One cycle of a triangle wave, from -1.0 to 1.0.
    pub triangle: Vec<f64>,
    /// `tan(pi*i/table_size)` for the TPT filter pre-warp (0 to pi).
    pub tan_table: Vec<f64>,
    /// TPT filter coefficient a1, indexed `[cutoff_index][resonance_index]`.
    pub a1_table: Vec<[f64; RESONANCE_STEPS]>,
    /// TPT filter coefficient a2, indexed `[cutoff_index][resonance_index]`.
    pub a2_table: Vec<[f64; RESONANCE_STEPS]>,
    /// TPT filter coefficient a3, indexed `[cutoff_index][resonance_index]`.
    pub a3_table: Vec<[f64; RESONANCE_STEPS]>,
    /// MIDI note number (0-127) to pitch in Hz (A440 equal temperament).
    pub midi_to_hz: [f64; 128],
}

/// The global, compute-once lookup-table instance.
///
/// Use [`tables()`] to obtain a `&'static` reference.
pub static TABLES: LazyLock<LookupTables> = LazyLock::new(LookupTables::compute);

/// Returns a `&'static` reference to the global lookup tables, forcing
/// initialisation if they have not been computed yet.
pub fn tables() -> &'static LookupTables {
    LazyLock::force(&TABLES)
}

impl LookupTables {
    /// Computes every lookup table. Mirrors the Java static initialiser
    /// exactly, except that the table size is the *validated*
    /// [`table_size`] rather than the raw
    /// [`crate::utils::audio_constants::LOOKUP_TABLE_SIZE`] constant.
    fn compute() -> Self {
        let size = table_size();

        let mut sine = vec![0.0; size];
        let mut cosine = vec![0.0; size];
        let mut square = vec![0.0; size];
        let mut saw = vec![0.0; size];
        let mut triangle = vec![0.0; size];
        let mut tan_table = vec![0.0; size];

        // Sine Table
        for (i, entry) in sine.iter_mut().enumerate() {
            *entry = (2.0 * std::f64::consts::PI * i as f64 / size as f64).sin();
        }

        // Cosine Table
        for (i, entry) in cosine.iter_mut().enumerate() {
            *entry = (2.0 * std::f64::consts::PI * i as f64 / size as f64).cos();
        }

        // Square Table
        for (i, entry) in square.iter_mut().enumerate() {
            *entry = if i < size / 2 { 1.0 } else { -1.0 };
        }

        // Saw Table (from -1 to 1)
        for (i, entry) in saw.iter_mut().enumerate() {
            *entry = 2.0 * (i as f64 / size as f64) - 1.0;
        }

        // Triangle Table (from -1 to 1)
        for (i, entry) in triangle.iter_mut().enumerate() {
            let mut value = 2.0 * (i as f64 / size as f64);
            if value > 1.0 {
                value = 2.0 - value;
            }
            *entry = 2.0 * value - 1.0;
        }

        // Tan Table (from 0 to PI)
        for (i, entry) in tan_table.iter_mut().enumerate() {
            *entry = (std::f64::consts::PI * i as f64 / size as f64).tan();
        }

        // Filter Coefficients
        let mut a1_table = vec![[0.0; RESONANCE_STEPS]; size];
        let mut a2_table = vec![[0.0; RESONANCE_STEPS]; size];
        let mut a3_table = vec![[0.0; RESONANCE_STEPS]; size];

        for cutoff_index in 0..size {
            let g = tan_table[cutoff_index];

            for res_index in 0..RESONANCE_STEPS {
                // Map the index to a resonance value (from 1.0 to 20.0)
                let resonance_q = 1.0 + (res_index as f64 / (RESONANCE_STEPS - 1) as f64) * 19.0;
                let k = 1.0 / resonance_q;

                // Calculate Filter Coefficients
                let a1 = 1.0 / (1.0 + g * (g + k));
                let a2 = g * a1;
                let a3 = g * a2;

                // Store the results in 2D LUTs
                a1_table[cutoff_index][res_index] = a1;
                a2_table[cutoff_index][res_index] = a2;
                a3_table[cutoff_index][res_index] = a3;
            }
        }

        // Midi note to pitch
        let mut midi_to_hz = [0.0; 128];
        for (i, entry) in midi_to_hz.iter_mut().enumerate() {
            *entry = 440.0 * 2.0_f64.powf((i as f64 - 69.0) / 12.0);
        }

        LookupTables {
            table_size: size,
            sine,
            cosine,
            square,
            saw,
            triangle,
            tan_table,
            a1_table,
            a2_table,
            a3_table,
            midi_to_hz,
        }
    }
}
