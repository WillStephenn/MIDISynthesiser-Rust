//! Tests for the startup configuration-validation layer
//! (`utils::engine_config`).
//!
//! Per `CLAUDE.md`'s testing philosophy: these tests exercise
//! [`EngineConfig::validate_values`] (the parameterised core of
//! [`EngineConfig::validated`]) with constructed inputs, never the compile-time
//! `audio_constants`. They assert *behaviour* -- "an out-of-range value falls
//! back to its documented default and produces exactly one warning naming the
//! field" -- and must keep passing regardless of how `audio_constants` is
//! retuned.

use midi_synthesiser::utils::engine_config::EngineConfig;

/// A configuration where every field is valid: `validate_values` should
/// return it unchanged with no warnings. Built from [`EngineConfig::default`]
/// (the documented safe defaults), which must themselves be valid.
fn valid_config() -> EngineConfig {
    EngineConfig::default()
}

/// Calls `validate_values` with every field taken from `base`, except for the
/// fields overridden via the closure.
fn validate(
    base: EngineConfig,
) -> (
    EngineConfig,
    Vec<midi_synthesiser::utils::engine_config::ConfigWarning>,
) {
    EngineConfig::validate_values(
        base.sample_rate,
        base.block_size,
        base.number_of_voices,
        base.lookup_table_size,
        base.buffer_size,
    )
}

#[test]
fn all_valid_values_produce_zero_warnings_and_are_returned_unchanged() {
    let base = valid_config();

    let (config, warnings) = validate(base);

    assert!(
        warnings.is_empty(),
        "expected no warnings for an all-valid config, got {warnings:?}"
    );
    assert_eq!(config, base, "a valid config should be returned unchanged");
}

// --- sample_rate -----------------------------------------------------------

/// Every (configured sample rate, expected reason substring) case that should
/// fall back to the default sample rate with exactly one warning naming
/// "sample_rate".
#[test]
fn invalid_sample_rate_falls_back_with_one_warning() {
    let base = valid_config();
    let invalid_rates = [
        0.0,           // not > 40
        40.0,          // boundary: not strictly greater than 40
        -44100.0,      // negative
        f64::NAN,      // not finite
        f64::INFINITY, // not finite
        500_000.0,     // above the sane upper bound
    ];

    for rate in invalid_rates {
        let (config, warnings) = EngineConfig::validate_values(
            rate,
            base.block_size,
            base.number_of_voices,
            base.lookup_table_size,
            base.buffer_size,
        );

        assert_eq!(
            warnings.len(),
            1,
            "sample_rate = {rate} should produce exactly one warning, got {warnings:?}"
        );
        assert_eq!(warnings[0].field, "sample_rate");
        assert_eq!(
            config.sample_rate,
            EngineConfig::default().sample_rate,
            "an invalid sample_rate ({rate}) should fall back to the default"
        );
        // Every other field stayed as configured.
        assert_eq!(config.block_size, base.block_size);
        assert_eq!(config.number_of_voices, base.number_of_voices);
        assert_eq!(config.lookup_table_size, base.lookup_table_size);
        assert_eq!(config.buffer_size, base.buffer_size);
    }
}

/// A sample rate just above the lower bound and at the upper bound is valid.
#[test]
fn sample_rate_boundary_values_are_valid() {
    let base = valid_config();

    for rate in [40.000001, 384_000.0] {
        let (config, warnings) = EngineConfig::validate_values(
            rate,
            base.block_size,
            base.number_of_voices,
            base.lookup_table_size,
            base.buffer_size,
        );
        assert!(
            warnings.is_empty(),
            "sample_rate = {rate} should be valid, got warnings {warnings:?}"
        );
        assert_eq!(config.sample_rate, rate);
    }
}

// --- block_size --------------------------------------------------------------

#[test]
fn invalid_block_size_falls_back_with_one_warning() {
    let base = valid_config();

    for &size in &[0usize, 8193, usize::MAX] {
        let (config, warnings) = EngineConfig::validate_values(
            base.sample_rate,
            size,
            base.number_of_voices,
            base.lookup_table_size,
            base.buffer_size,
        );

        assert_eq!(
            warnings.len(),
            1,
            "block_size = {size} should produce exactly one warning, got {warnings:?}"
        );
        assert_eq!(warnings[0].field, "block_size");
        assert_eq!(
            config.block_size,
            EngineConfig::default().block_size,
            "an invalid block_size ({size}) should fall back to the default"
        );
    }
}

#[test]
fn block_size_boundary_values_are_valid() {
    let base = valid_config();

    for &size in &[1usize, 8192] {
        let (config, warnings) = EngineConfig::validate_values(
            base.sample_rate,
            size,
            base.number_of_voices,
            base.lookup_table_size,
            // Keep buffer_size consistent so it doesn't also warn.
            size.max(base.buffer_size),
        );
        assert!(
            warnings.is_empty(),
            "block_size = {size} should be valid, got warnings {warnings:?}"
        );
        assert_eq!(config.block_size, size);
    }
}

// --- number_of_voices ---------------------------------------------------------

#[test]
fn invalid_number_of_voices_falls_back_with_one_warning() {
    let base = valid_config();

    for &voices in &[0usize, 129, usize::MAX] {
        let (config, warnings) = EngineConfig::validate_values(
            base.sample_rate,
            base.block_size,
            voices,
            base.lookup_table_size,
            base.buffer_size,
        );

        assert_eq!(
            warnings.len(),
            1,
            "number_of_voices = {voices} should produce exactly one warning, got {warnings:?}"
        );
        assert_eq!(warnings[0].field, "number_of_voices");
        assert_eq!(
            config.number_of_voices,
            EngineConfig::default().number_of_voices,
            "an invalid number_of_voices ({voices}) should fall back to the default"
        );
    }
}

#[test]
fn number_of_voices_boundary_values_are_valid() {
    let base = valid_config();

    for &voices in &[1usize, 128] {
        let (config, warnings) = EngineConfig::validate_values(
            base.sample_rate,
            base.block_size,
            voices,
            base.lookup_table_size,
            base.buffer_size,
        );
        assert!(
            warnings.is_empty(),
            "number_of_voices = {voices} should be valid, got warnings {warnings:?}"
        );
        assert_eq!(config.number_of_voices, voices);
    }
}

// --- lookup_table_size ----------------------------------------------------------

/// Power-of-two edge cases: 0 and 1 are too small (below the documented
/// minimum), 2 is a power of two but still below the minimum, `2^k` within
/// range is valid, and `2^k + 1` (not a power of two) falls back.
#[test]
fn lookup_table_size_power_of_two_edge_cases() {
    let base = valid_config();

    // Too small (whether or not they're powers of two) -> fallback.
    for &size in &[0usize, 1, 2, 255] {
        let (config, warnings) = EngineConfig::validate_values(
            base.sample_rate,
            base.block_size,
            base.number_of_voices,
            size,
            base.buffer_size,
        );
        assert_eq!(
            warnings.len(),
            1,
            "lookup_table_size = {size} should produce exactly one warning, got {warnings:?}"
        );
        assert_eq!(warnings[0].field, "lookup_table_size");
        assert_eq!(
            config.lookup_table_size,
            EngineConfig::default().lookup_table_size,
            "an invalid lookup_table_size ({size}) should fall back to the default"
        );
    }

    // A power of two within range -> valid, returned unchanged.
    for &size in &[256usize, 1024, 65536] {
        let (config, warnings) = EngineConfig::validate_values(
            base.sample_rate,
            base.block_size,
            base.number_of_voices,
            size,
            base.buffer_size,
        );
        assert!(
            warnings.is_empty(),
            "lookup_table_size = {size} (a power of two in range) should be valid, got {warnings:?}"
        );
        assert_eq!(config.lookup_table_size, size);
    }

    // `2^k + 1`: not a power of two -> fallback, even though it's in range.
    for &size in &[257usize, 1025, 65537] {
        let (config, warnings) = EngineConfig::validate_values(
            base.sample_rate,
            base.block_size,
            base.number_of_voices,
            size,
            base.buffer_size,
        );
        assert_eq!(
            warnings.len(),
            1,
            "lookup_table_size = {size} (not a power of two) should produce exactly one warning, got {warnings:?}"
        );
        assert_eq!(warnings[0].field, "lookup_table_size");
        assert_eq!(
            config.lookup_table_size,
            EngineConfig::default().lookup_table_size
        );
    }

    // A power of two above the documented maximum -> fallback.
    let (config, warnings) = EngineConfig::validate_values(
        base.sample_rate,
        base.block_size,
        base.number_of_voices,
        1 << 21,
        base.buffer_size,
    );
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].field, "lookup_table_size");
    assert_eq!(
        config.lookup_table_size,
        EngineConfig::default().lookup_table_size
    );
}

// --- buffer_size ----------------------------------------------------------------

#[test]
fn buffer_size_smaller_than_block_size_falls_back_with_one_warning() {
    let base = valid_config();
    let too_small = base.block_size - 1;

    let (config, warnings) = EngineConfig::validate_values(
        base.sample_rate,
        base.block_size,
        base.number_of_voices,
        base.lookup_table_size,
        too_small,
    );

    assert_eq!(
        warnings.len(),
        1,
        "buffer_size < block_size should produce exactly one warning, got {warnings:?}"
    );
    assert_eq!(warnings[0].field, "buffer_size");
    assert!(
        config.buffer_size >= config.block_size,
        "the returned config must always satisfy buffer_size >= block_size, got \
         buffer_size={}, block_size={}",
        config.buffer_size,
        config.block_size
    );
}

#[test]
fn buffer_size_equal_to_block_size_is_valid() {
    let base = valid_config();

    let (config, warnings) = EngineConfig::validate_values(
        base.sample_rate,
        base.block_size,
        base.number_of_voices,
        base.lookup_table_size,
        base.block_size,
    );

    assert!(
        warnings.is_empty(),
        "buffer_size == block_size should be valid, got {warnings:?}"
    );
    assert_eq!(config.buffer_size, base.block_size);
}

/// When `block_size` itself is invalid and falls back to its default, a
/// `buffer_size` that was valid relative to the *configured* (invalid)
/// block_size but not relative to the *validated* block_size must also fall
/// back, so the returned config stays internally consistent
/// (`buffer_size >= block_size`).
#[test]
fn buffer_size_is_rechecked_against_the_validated_block_size() {
    let base = valid_config();
    let invalid_block_size = 0usize; // falls back to EngineConfig::default().block_size
    let buffer_size = 1; // >= invalid_block_size (0), but < the fallback block_size

    let (config, warnings) = EngineConfig::validate_values(
        base.sample_rate,
        invalid_block_size,
        base.number_of_voices,
        base.lookup_table_size,
        buffer_size,
    );

    assert_eq!(config.block_size, EngineConfig::default().block_size);
    assert!(
        config.buffer_size >= config.block_size,
        "buffer_size ({}) must be >= the validated block_size ({})",
        config.buffer_size,
        config.block_size
    );

    let buffer_warning = warnings
        .iter()
        .find(|w| w.field == "buffer_size")
        .expect("an inconsistent buffer_size should still produce its own warning");
    assert!(!buffer_warning.reason.is_empty());
}

// --- Multiple violations ---------------------------------------------------

#[test]
fn multiple_violations_each_produce_their_own_warning() {
    let default_block_size = EngineConfig::default().block_size;

    let (config, warnings) = EngineConfig::validate_values(
        -1.0, // invalid sample_rate
        0,    // invalid block_size (falls back to the default block size)
        0,    // invalid number_of_voices
        100,  // invalid lookup_table_size (not a power of two, too small)
        // Valid relative to the *fallback* block size, so no extra warning.
        default_block_size,
    );

    let fields: Vec<&str> = warnings.iter().map(|w| w.field).collect();
    assert_eq!(
        fields,
        vec![
            "sample_rate",
            "block_size",
            "number_of_voices",
            "lookup_table_size",
        ],
        "each invalid field should produce its own warning, in field order"
    );

    let defaults = EngineConfig::default();
    assert_eq!(config.sample_rate, defaults.sample_rate);
    assert_eq!(config.block_size, defaults.block_size);
    assert_eq!(config.number_of_voices, defaults.number_of_voices);
    assert_eq!(config.lookup_table_size, defaults.lookup_table_size);
    assert!(config.buffer_size >= config.block_size);
}

// --- ConfigWarning::Display --------------------------------------------------

#[test]
fn config_warning_display_mentions_field_configured_and_fallback() {
    let (_config, warnings) = EngineConfig::validate_values(
        0.0, // invalid sample_rate
        EngineConfig::default().block_size,
        EngineConfig::default().number_of_voices,
        EngineConfig::default().lookup_table_size,
        EngineConfig::default().buffer_size,
    );

    let warning = &warnings[0];
    let text = warning.to_string();

    assert!(
        text.contains(&warning.configured_value),
        "warning text {text:?} should mention the configured value {:?}",
        warning.configured_value
    );
    assert!(
        text.contains(&warning.fallback_value),
        "warning text {text:?} should mention the fallback value {:?}",
        warning.fallback_value
    );
    assert!(
        text.contains("sample_rate"),
        "warning text {text:?} should name the field"
    );
}

// --- validated() wraps the compile-time constants ---------------------------

/// `EngineConfig::validated()` must return an internally-consistent config
/// (matching the invariants `validate_values` enforces) for whatever the
/// current `audio_constants` happen to be -- this is the load-bearing
/// behaviour for `lookup_tables` and the hosts, without pinning the actual
/// constant values.
#[test]
fn validated_config_is_internally_consistent() {
    let (config, _warnings) = EngineConfig::validated();

    assert!(config.sample_rate.is_finite());
    assert!(config.sample_rate > 40.0);
    assert!(config.block_size >= 1);
    assert!(config.number_of_voices >= 1);
    assert!(config.lookup_table_size.is_power_of_two());
    assert!(config.buffer_size >= config.block_size);
}
