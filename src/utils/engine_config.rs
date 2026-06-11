//! Startup configuration validation (architecture constraint: "Engine
//! configuration is validated, not trusted").
//!
//! The constants in [`crate::utils::audio_constants`] are user-tunable.
//! Editing them must never panic or silently corrupt the engine: every
//! structural invariant the engine actually relies on is checked here, once,
//! at startup. Any constant that violates its invariant is replaced with a
//! documented safe default and a [`ConfigWarning`] is returned describing
//! what happened, so hosts (CLI/GUI) can surface it to the user.
//!
//! This module is `std`-only and performs no I/O: it just turns raw numbers
//! into a [`EngineConfig`] plus a list of warnings. [`EngineConfig::validated`]
//! is a thin wrapper over [`EngineConfig::validate_values`] that reads the
//! compile-time constants; the latter is the testable core.

use std::fmt;
use std::sync::OnceLock;

use crate::utils::audio_constants;

/// Safe default sample rate, in Hz. Used when the configured
/// [`audio_constants::SAMPLE_RATE`] fails validation.
pub const DEFAULT_SAMPLE_RATE: f64 = 44100.0;

/// Safe default block size, in frames. Used when the configured
/// [`audio_constants::BLOCK_SIZE`] fails validation.
pub const DEFAULT_BLOCK_SIZE: usize = 256;

/// Safe default voice count. Used when the configured
/// [`audio_constants::NUMBER_OF_VOICES`] fails validation.
pub const DEFAULT_NUMBER_OF_VOICES: usize = 8;

/// Safe default lookup-table size. Used when the configured
/// [`audio_constants::LOOKUP_TABLE_SIZE`] fails validation.
pub const DEFAULT_LOOKUP_TABLE_SIZE: usize = 32768;

/// Largest sample rate accepted, in Hz. Generously above the highest sample
/// rate in common professional audio use (768 kHz exists, but 384 kHz covers
/// every realistic interface while keeping `sample_rate / 2` well clear of
/// any `f64` precision concerns used by the filter/Nyquist maths).
const MAX_SAMPLE_RATE: f64 = 384_000.0;

/// Largest block size accepted, in frames. Generous upper bound for engine
/// processing blocks; far larger than typical low-latency (32-512) or
/// high-latency (2048-4096) audio buffer sizes.
const MAX_BLOCK_SIZE: usize = 8192;

/// Largest voice count accepted. Generous upper bound for a polyphonic
/// synthesiser; well beyond what is realistically playable or renderable.
const MAX_NUMBER_OF_VOICES: usize = 128;

/// Smallest lookup-table size accepted. Below this the waveform tables become
/// audibly coarse (severe quantisation of the oscillator phase).
const MIN_LOOKUP_TABLE_SIZE: usize = 256;

/// Largest lookup-table size accepted.
///
/// [`crate::utils::lookup_tables::LookupTables::compute`] allocates three
/// `Vec<[f64; RESONANCE_STEPS]>` filter-coefficient tables (`RESONANCE_STEPS`
/// = 128) of length `LOOKUP_TABLE_SIZE`, i.e. `3 * N * 128 * 8` bytes. At
/// `2^20` that is already ~3.2 GB; doubling to `2^21` would be ~6.4 GB. `2^20`
/// is therefore the largest size that does not risk an allocation
/// failure/abort (itself a form of "panic on startup") on ordinary machines,
/// while still being far larger than any size that improves audio quality.
const MAX_LOOKUP_TABLE_SIZE: usize = 1 << 20;

/// Validated, internally-consistent engine configuration.
///
/// Every field has already been checked against the invariants the engine
/// relies on (see [`EngineConfig::validate_values`]); constructing the engine
/// from this struct can never trigger the `assert!`s in
/// [`crate::core::synthesiser::Synthesiser::new`] or the lookup-table phase
/// wrapping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EngineConfig {
    /// The audio sample rate in Hz. Finite, `> 40.0`, `<= 384 000.0`.
    pub sample_rate: f64,
    /// The number of frames processed per engine block. `1..=8192`.
    pub block_size: usize,
    /// The number of polyphonic voices. `1..=128`.
    pub number_of_voices: usize,
    /// The number of entries in each waveform/filter lookup table. A power of
    /// two in `256..=2^20`.
    pub lookup_table_size: usize,
    /// The size of the output device buffer, in frames. `>= block_size`.
    pub buffer_size: usize,
}

impl Default for EngineConfig {
    /// The documented set of safe defaults, used whenever a configured value
    /// fails validation.
    fn default() -> Self {
        EngineConfig {
            sample_rate: DEFAULT_SAMPLE_RATE,
            block_size: DEFAULT_BLOCK_SIZE,
            number_of_voices: DEFAULT_NUMBER_OF_VOICES,
            lookup_table_size: DEFAULT_LOOKUP_TABLE_SIZE,
            buffer_size: 8 * DEFAULT_BLOCK_SIZE,
        }
    }
}

/// Describes a single configuration value that failed validation and the
/// fallback that was substituted for it.
///
/// Hosts must surface every [`ConfigWarning`] returned by
/// [`EngineConfig::validated`] to the user (a GUI banner, CLI stderr lines,
/// etc.) per the "Engine configuration is validated, not trusted"
/// architecture constraint.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigWarning {
    /// The name of the configuration field that failed validation, e.g.
    /// `"sample_rate"`.
    pub field: &'static str,
    /// A human-readable rendering of the configured (invalid) value.
    pub configured_value: String,
    /// A human-readable rendering of the safe default that was substituted.
    pub fallback_value: String,
    /// A human-readable explanation of why the configured value was rejected.
    pub reason: String,
}

impl fmt::Display for ConfigWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Invalid {field}: {configured} ({reason}); using default {fallback} instead.",
            field = self.field,
            configured = self.configured_value,
            reason = self.reason,
            fallback = self.fallback_value,
        )
    }
}

impl EngineConfig {
    /// Validates the compile-time constants in [`audio_constants`], falling
    /// back to documented safe defaults for any invariant violation.
    ///
    /// Thin wrapper over [`Self::validate_values`]; see that function for the
    /// invariants enforced.
    pub fn validated() -> (EngineConfig, Vec<ConfigWarning>) {
        Self::validate_values(
            audio_constants::SAMPLE_RATE,
            audio_constants::BLOCK_SIZE,
            audio_constants::NUMBER_OF_VOICES,
            audio_constants::LOOKUP_TABLE_SIZE,
            audio_constants::BUFFER_SIZE,
        )
    }

    /// Validates a set of raw configuration values independently, returning a
    /// fully-consistent [`EngineConfig`] plus one [`ConfigWarning`] per
    /// invariant violated.
    ///
    /// Each field is checked independently against the bounds the engine
    /// actually relies on:
    ///
    /// - `sample_rate`: must be finite, `> 40.0` (matches the assertion in
    ///   [`crate::core::synthesiser::Synthesiser::new`] and
    ///   [`crate::components::oscillators::oscillator::OscillatorCore::new`]
    ///   /[`crate::components::envelope::Envelope::new`]
    ///   /[`crate::components::filters::resonant_low_pass_filter::ResonantLowPassFilter::new`],
    ///   which all assert `sample_rate > 0.0`) and `<= 384 000.0`.
    /// - `block_size`: must be `>= 1` (a zero block size makes
    ///   `lfo_output_buffer[block_size - 1]` in
    ///   [`crate::core::synthesiser::Synthesiser::process_block`] panic with
    ///   an out-of-bounds/underflow index) and `<= 8192`.
    /// - `number_of_voices`: must be `>= 1` (matches the assertion in
    ///   [`crate::core::synthesiser::Synthesiser::new`]) and `<= 128`.
    /// - `lookup_table_size`: must be a power of two (the oscillators wrap
    ///   the phase accumulator with a bitmask,
    ///   `(phase as i32 as usize) & (TABLE_SIZE - 1)`, in
    ///   [`crate::components::oscillators::oscillator::OscillatorCore::process_table_block`];
    ///   this only wraps correctly when `TABLE_SIZE` is a power of two) and
    ///   in `256..=2^20` (memory bound on the filter-coefficient tables; see
    ///   [`MAX_LOOKUP_TABLE_SIZE`]).
    /// - `buffer_size`: must be `>= block_size` (the device buffer must be
    ///   able to hold at least one engine block).
    ///
    /// Each invariant is checked independently of the others. The exception
    /// is `buffer_size`, which is checked against the *validated* block size
    /// (after any block-size fallback): the returned [`EngineConfig`] must
    /// always satisfy `buffer_size >= block_size`, so if `block_size` itself
    /// fell back to its default, `buffer_size` is re-checked (and, if
    /// necessary, also falls back) against that default rather than the
    /// original configured `block_size`.
    pub fn validate_values(
        sample_rate: f64,
        block_size: usize,
        number_of_voices: usize,
        lookup_table_size: usize,
        buffer_size: usize,
    ) -> (EngineConfig, Vec<ConfigWarning>) {
        let defaults = EngineConfig::default();
        let mut warnings = Vec::new();

        let valid_sample_rate =
            if sample_rate.is_finite() && sample_rate > 40.0 && sample_rate <= MAX_SAMPLE_RATE {
                sample_rate
            } else {
                warnings.push(ConfigWarning {
                    field: "sample_rate",
                    configured_value: format!("{sample_rate}"),
                    fallback_value: format!("{}", defaults.sample_rate),
                    reason: format!("must be finite and in the range (40.0, {MAX_SAMPLE_RATE}] Hz"),
                });
                defaults.sample_rate
            };

        let valid_block_size = if (1..=MAX_BLOCK_SIZE).contains(&block_size) {
            block_size
        } else {
            warnings.push(ConfigWarning {
                field: "block_size",
                configured_value: format!("{block_size}"),
                fallback_value: format!("{}", defaults.block_size),
                reason: format!("must be in the range 1..={MAX_BLOCK_SIZE} frames"),
            });
            defaults.block_size
        };

        let valid_number_of_voices = if (1..=MAX_NUMBER_OF_VOICES).contains(&number_of_voices) {
            number_of_voices
        } else {
            warnings.push(ConfigWarning {
                field: "number_of_voices",
                configured_value: format!("{number_of_voices}"),
                fallback_value: format!("{}", defaults.number_of_voices),
                reason: format!("must be in the range 1..={MAX_NUMBER_OF_VOICES}"),
            });
            defaults.number_of_voices
        };

        let valid_lookup_table_size = if lookup_table_size.is_power_of_two()
            && (MIN_LOOKUP_TABLE_SIZE..=MAX_LOOKUP_TABLE_SIZE).contains(&lookup_table_size)
        {
            lookup_table_size
        } else {
            warnings.push(ConfigWarning {
                field: "lookup_table_size",
                configured_value: format!("{lookup_table_size}"),
                fallback_value: format!("{}", defaults.lookup_table_size),
                reason: format!(
                    "must be a power of two in the range {MIN_LOOKUP_TABLE_SIZE}..={MAX_LOOKUP_TABLE_SIZE}"
                ),
            });
            defaults.lookup_table_size
        };

        // Checked against the *validated* block size (see doc comment above)
        // so the returned config always satisfies `buffer_size >= block_size`.
        let valid_buffer_size = if buffer_size >= valid_block_size {
            buffer_size
        } else {
            let fallback = 8 * valid_block_size;
            warnings.push(ConfigWarning {
                field: "buffer_size",
                configured_value: format!("{buffer_size}"),
                fallback_value: format!("{fallback}"),
                reason: format!(
                    "must be >= block_size ({valid_block_size}); the device buffer must hold at least one engine block"
                ),
            });
            fallback
        };

        let config = EngineConfig {
            sample_rate: valid_sample_rate,
            block_size: valid_block_size,
            number_of_voices: valid_number_of_voices,
            lookup_table_size: valid_lookup_table_size,
            buffer_size: valid_buffer_size,
        };

        (config, warnings)
    }
}

/// Process-wide, compute-once validated configuration and the warnings
/// produced while validating it.
///
/// This is the single source of truth: [`validated_config`] and
/// [`config_warnings`] both read from here, and
/// [`crate::utils::lookup_tables::TABLES`] is sized from
/// `validated_config().lookup_table_size` rather than the raw
/// [`audio_constants::LOOKUP_TABLE_SIZE`] constant. Hosts (CLI/GUI) read
/// [`config_warnings`] at startup to surface them to the user.
static VALIDATED: OnceLock<(EngineConfig, Vec<ConfigWarning>)> = OnceLock::new();

/// Returns the process-wide validated [`EngineConfig`], computing it on first
/// use.
///
/// Every consumer that needs the engine configuration (lookup-table sizing,
/// `Synthesiser::new` call sites) should read this rather than the raw
/// [`audio_constants`] so the whole process agrees on one validated config.
pub fn validated_config() -> &'static EngineConfig {
    &VALIDATED.get_or_init(EngineConfig::validated).0
}

/// Returns the warnings produced while computing [`validated_config`].
///
/// Empty when every constant in [`audio_constants`] is valid. Hosts must
/// surface each of these to the user (GUI banner / CLI stderr).
pub fn config_warnings() -> &'static [ConfigWarning] {
    &VALIDATED.get_or_init(EngineConfig::validated).1
}
