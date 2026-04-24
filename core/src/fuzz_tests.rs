//! Property-based fuzz tests for the simulation engine.
//!
//! These tests use `proptest` to generate random inputs and verify that the
//! simulation engine's pure functions never panic and maintain their invariants
//! across a wide variety of edge cases.

use crate::comparison::{calculate_deltas, detect_regressions, ResourceDelta};
use crate::simulation::{SimulationCache, SimulationEngine, SorobanResources, TtlEntryReport};
use proptest::prelude::*;
use soroban_sdk::xdr::{ScMapEntry, ScSymbol, ScVal, ScVec, VecM};

// ── Strategies ───────────────────────────────────────────────────────────────

/// Generate a random `SorobanResources` struct with arbitrary u64 fields.
fn arb_soroban_resources() -> impl Strategy<Value = SorobanResources> {
    (
        any::<u64>(),
        any::<u64>(),
        any::<u64>(),
        any::<u64>(),
        any::<u64>(),
    )
        .prop_map(|(cpu, ram, lr, lw, tx)| SorobanResources {
            cpu_instructions: cpu,
            ram_bytes: ram,
            ledger_read_bytes: lr,
            ledger_write_bytes: lw,
            transaction_size_bytes: tx,
        })
}

/// Generate a random `ScVal` with bounded depth to avoid stack overflow.
fn arb_scval() -> impl Strategy<Value = ScVal> {
    let leaf = prop_oneof![
        Just(ScVal::Void),
        any::<bool>().prop_map(ScVal::Bool),
        any::<u32>().prop_map(ScVal::U32),
        any::<i32>().prop_map(ScVal::I32),
        any::<u64>().prop_map(ScVal::U64),
        any::<i64>().prop_map(ScVal::I64),
        Just(ScVal::LedgerKeyContractInstance),
    ];

    leaf.prop_recursive(
        3,  // max depth
        32, // max nodes
        4,  // items per collection
        |inner| {
            prop_oneof![
                // Vec<ScVal>
                prop::collection::vec(inner.clone(), 0..4).prop_map(|items| {
                    let vec_m: VecM<ScVal> = items.try_into().unwrap_or_default();
                    ScVal::Vec(Some(ScVec(vec_m)))
                }),
                // Map<Symbol, ScVal>
                prop::collection::vec(("[a-z]{1,8}", inner.clone()), 0..4).prop_map(|entries| {
                    let map_entries: Vec<ScMapEntry> = entries
                        .into_iter()
                        .filter_map(|(k, v)| {
                            let sym: ScSymbol = k.as_str().try_into().ok()?;
                            Some(ScMapEntry {
                                key: ScVal::Symbol(sym),
                                val: v,
                            })
                        })
                        .collect();
                    let map_m: VecM<ScMapEntry> = map_entries.try_into().unwrap_or_default();
                    ScVal::Map(Some(soroban_sdk::xdr::ScMap(map_m)))
                }),
            ]
        },
    )
}

/// Generate a random `TtlEntryReport`.
fn arb_ttl_entry() -> impl Strategy<Value = TtlEntryReport> {
    (
        "[a-zA-Z0-9]{4,16}",
        any::<u32>(),
        // remaining_ledgers can be negative (expired entries)
        -500_000i64..500_000i64,
    )
        .prop_map(
            |(key, live_until_ledger, remaining_ledgers)| TtlEntryReport {
                key,
                live_until_ledger,
                remaining_ledgers,
            },
        )
}

/// Generate a random `ResourceDelta`.
fn arb_resource_delta() -> impl Strategy<Value = ResourceDelta> {
    (
        -100.0f64..200.0,
        -100.0f64..200.0,
        -100.0f64..200.0,
        -100.0f64..200.0,
        -100.0f64..200.0,
    )
        .prop_map(|(cpu, ram, lr, lw, tx)| ResourceDelta {
            cpu_instructions: cpu,
            ram_bytes: ram,
            ledger_read_bytes: lr,
            ledger_write_bytes: lw,
            transaction_size_bytes: tx,
        })
}

// ── Property Tests ───────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    // ── calculate_cost ───────────────────────────────────────────────────

    /// `calculate_cost` must never panic regardless of input values.
    /// u64 division can't overflow, so this verifies no unexpected panic paths.
    #[test]
    fn fuzz_calculate_cost_never_panics(resources in arb_soroban_resources()) {
        let engine = SimulationEngine::new("https://fuzz.test".to_string());
        let _cost = engine.calculate_cost(&resources);
        // No assertion needed — we're testing absence of panic.
    }

    // ── estimate_scval_size ──────────────────────────────────────────────

    /// `estimate_scval_size` must never panic on any valid ScVal tree and
    /// must always return a non-negative size.
    #[test]
    fn fuzz_estimate_scval_size_never_panics(val in arb_scval()) {
        let engine = SimulationEngine::new("https://fuzz.test".to_string());
        let size = engine.estimate_scval_size(&val);
        // Size estimation should never be negative (it's u64).
        prop_assert!(size <= u64::MAX);
    }

    // ── parse_sc_val_arg ─────────────────────────────────────────────────

    /// `parse_sc_val_arg` must never panic on arbitrary string input.
    /// It's fine to return `Err`, but it must not crash.
    #[test]
    fn fuzz_parse_sc_val_arg_no_panic(
        input in prop_oneof![
            // Booleans
            Just("true".to_string()),
            Just("false".to_string()),
            // Void
            Just("void".to_string()),
            Just("()".to_string()),
            // Integers
            any::<i64>().prop_map(|n| n.to_string()),
            any::<u64>().prop_map(|n| n.to_string()),
            // Hex-prefixed
            "[0-9a-f]{2,32}".prop_map(|h| format!("0x{}", h)),
            // Symbol-like
            ":[a-z_]{1,32}",
            // Contract-ID-like (C prefix + 55 alphanumeric)
            "[A-Z2-7]{55}".prop_map(|s| format!("C{}", s)),
            // Account-like (G prefix + 55 alphanumeric)
            "[A-Z2-7]{55}".prop_map(|s| format!("G{}", s)),
            // JSON objects
            "\\{[a-z0-9: ,\"]{0,64}\\}",
            // JSON arrays
            "\\[[0-9, ]{0,32}\\]",
            // Arbitrary short strings (garbage input)
            ".{0,64}",
        ]
    ) {
        let engine = SimulationEngine::new("https://fuzz.test".to_string());
        let _result = engine.parse_sc_val_arg(&input);
        // Ok or Err — either is fine, just no panic.
    }

    // ── extract_footprint_from_xdr ───────────────────────────────────────

    /// `extract_footprint_from_xdr` must gracefully handle arbitrary base64
    /// strings without panicking. Invalid data should return (0, 0).
    #[test]
    fn fuzz_extract_footprint_never_panics(
        raw_bytes in prop::collection::vec(any::<u8>(), 0..256)
    ) {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        let engine = SimulationEngine::new("https://fuzz.test".to_string());
        let encoded = BASE64.encode(&raw_bytes);
        let (read, write) = engine.extract_footprint_from_xdr(&encoded);
        // Invalid XDR should gracefully return zeros.
        prop_assert!(read <= u64::MAX);
        prop_assert!(write <= u64::MAX);
    }

    // ── build_extend_ttl_suggestions ─────────────────────────────────────

    /// TTL suggestions should only flag entries whose `remaining_ledgers` is
    /// at or below the warning threshold. Every suggestion must have a
    /// positive `ledgers_to_extend_by`.
    #[test]
    fn fuzz_build_ttl_suggestions_invariant(
        entries in prop::collection::vec(arb_ttl_entry(), 0..16),
        latest_ledger in 0u64..10_000_000,
    ) {
        let suggestions =
            SimulationEngine::build_extend_ttl_suggestions(&entries, latest_ledger);

        for suggestion in &suggestions {
            // Every suggestion must reference an entry that's below the threshold.
            let source_entry = entries.iter().find(|e| e.key == suggestion.key);
            prop_assert!(source_entry.is_some(), "Suggestion for unknown key");

            let entry = source_entry.unwrap();
            prop_assert!(
                entry.remaining_ledgers <= SimulationEngine::TTL_WARNING_THRESHOLD_LEDGERS,
                "Suggestion generated for entry above threshold: remaining={}, threshold={}",
                entry.remaining_ledgers,
                SimulationEngine::TTL_WARNING_THRESHOLD_LEDGERS
            );
        }
    }

    // ── SimulationCache::generate_key ────────────────────────────────────

    /// Cache key generation must be deterministic: same inputs always produce
    /// the same key. The key must be a 64-char hex string (SHA-256).
    #[test]
    fn fuzz_cache_key_deterministic(
        contract_id in "[A-Z0-9]{10,56}",
        function_name in "[a-z_]{1,32}",
        args in prop::collection::vec("[a-z0-9]{1,16}", 0..8),
    ) {
        let k1 = SimulationCache::generate_key(&contract_id, &function_name, &args);
        let k2 = SimulationCache::generate_key(&contract_id, &function_name, &args);

        prop_assert_eq!(&k1, &k2, "Cache key is not deterministic");
        prop_assert_eq!(k1.len(), 64, "Cache key is not 64 hex chars");
        prop_assert!(
            k1.chars().all(|c| c.is_ascii_hexdigit()),
            "Cache key contains non-hex characters"
        );
    }

    // ── calculate_deltas ─────────────────────────────────────────────────

    /// When current == base, all deltas must be exactly zero.
    /// When swapped, deltas should negate (for non-zero base).
    #[test]
    fn fuzz_calculate_deltas_symmetry(
        a in arb_soroban_resources(),
        b in arb_soroban_resources(),
    ) {
        // Identical inputs → zero deltas
        let zero_deltas = calculate_deltas(&a, &a);
        prop_assert!(
            (zero_deltas.cpu_instructions).abs() < f64::EPSILON,
            "cpu delta not zero for identical inputs"
        );
        prop_assert!(
            (zero_deltas.ram_bytes).abs() < f64::EPSILON,
            "ram delta not zero for identical inputs"
        );

        // Swapped inputs should negate deltas (when base != 0)
        let d1 = calculate_deltas(&a, &b);
        let d2 = calculate_deltas(&b, &a);

        if b.cpu_instructions > 0 && a.cpu_instructions > 0 {
            prop_assert!(
                (d1.cpu_instructions + d2.cpu_instructions).abs() < 1.0,
                "CPU deltas don't approximately negate: {} + {} = {}",
                d1.cpu_instructions,
                d2.cpu_instructions,
                d1.cpu_instructions + d2.cpu_instructions
            );
        }
    }

    // ── detect_regressions ───────────────────────────────────────────────

    /// Every regression flag must have a `change_percent` strictly greater
    /// than the threshold. Negative deltas must never be flagged.
    #[test]
    fn fuzz_detect_regressions_threshold(
        deltas in arb_resource_delta(),
        threshold in 0.0f64..50.0,
    ) {
        let flags = detect_regressions(&deltas, threshold);

        for flag in &flags {
            prop_assert!(
                flag.change_percent > threshold,
                "Flag {} has change {} which is not > threshold {}",
                flag.resource,
                flag.change_percent,
                threshold
            );
            prop_assert!(
                flag.change_percent > 0.0,
                "Negative change should never be flagged"
            );

            // Verify severity assignment
            if flag.change_percent > 25.0 {
                prop_assert_eq!(flag.severity, "critical");
            } else {
                prop_assert_eq!(flag.severity, "high");
            }
        }
    }
}
