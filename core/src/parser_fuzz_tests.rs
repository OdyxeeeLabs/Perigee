//! Property-based fuzz tests for the `ArgParser`.
//!
//! Exercises the parser with randomly generated JSON, primitive values,
//! address-like strings, and completely arbitrary input to ensure it
//! never panics on any input.

use crate::parser::ArgParser;
use proptest::prelude::*;
use soroban_sdk::xdr::ScVal;

// ── Strategies ───────────────────────────────────────────────────────────────

/// Generate random valid JSON values.
fn arb_json_string() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        Just("null".to_string()),
        any::<bool>().prop_map(|b| b.to_string()),
        any::<i64>().prop_map(|n| n.to_string()),
        "[a-zA-Z0-9_ ]{0,32}".prop_map(|s| format!("\"{}\"", s)),
        // Symbol strings
        "[a-z_]{1,16}".prop_map(|s| format!("\":{}\"", s)),
        // Hex bytes
        "[0-9a-f]{2,16}".prop_map(|h| format!("\"0x{}\"", h)),
    ];

    leaf.prop_recursive(
        2,  // max depth
        16, // max nodes
        4,  // items per collection
        |inner| {
            prop_oneof![
                // JSON arrays
                prop::collection::vec(inner.clone(), 0..4)
                    .prop_map(|items| format!("[{}]", items.join(","))),
                // JSON objects with symbol keys
                prop::collection::vec(
                    ("[a-z]{1,8}", inner),
                    0..4
                )
                .prop_map(|entries| {
                    let pairs: Vec<String> = entries
                        .into_iter()
                        .map(|(k, v)| format!("\"{}\":{}", k, v))
                        .collect();
                    format!("{{{}}}", pairs.join(","))
                }),
            ]
        },
    )
}

// ── Property Tests ───────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Randomly generated valid JSON should either parse successfully or
    /// return a structured error — never panic.
    #[test]
    fn fuzz_parse_arbitrary_json_no_panic(json in arb_json_string()) {
        let _result = ArgParser::parse(&json);
        // Ok or Err are both valid; no panic is the invariant.
    }

    /// Primitive JSON values (integers, booleans, null) should always parse
    /// into the corresponding ScVal variant.
    #[test]
    fn fuzz_parse_round_trip_primitives(
        choice in prop_oneof![
            // Null → Void
            Just(("null".to_string(), "void")),
            // Booleans
            Just(("true".to_string(), "bool")),
            Just(("false".to_string(), "bool")),
            // Integers
            (-1_000_000i64..1_000_000i64).prop_map(|n| (n.to_string(), "int")),
        ]
    ) {
        let (input, expected_kind) = choice;
        let result = ArgParser::parse(&input);
        prop_assert!(result.is_ok(), "Failed to parse '{}': {:?}", input, result);

        let val = result.unwrap();
        match expected_kind {
            "void" => prop_assert!(matches!(val, ScVal::Void)),
            "bool" => prop_assert!(matches!(val, ScVal::Bool(_))),
            "int" => prop_assert!(
                matches!(val, ScVal::I64(_) | ScVal::U64(_)),
                "Expected integer ScVal, got {:?}",
                val
            ),
            _ => unreachable!(),
        }
    }

    /// Completely arbitrary UTF-8 strings must never cause a panic.
    /// The parser should gracefully return an error for unparseable input.
    #[test]
    fn fuzz_parse_random_bytes_no_panic(input in "\\PC{0,128}") {
        let _result = ArgParser::parse(&input);
    }

    /// Strings that look like Stellar addresses (56 chars starting with G or C)
    /// should either parse as an Address or return an error — never panic.
    #[test]
    fn fuzz_parse_address_length_strings(
        prefix in prop_oneof![Just('G'), Just('C')],
        body in "[A-Z2-7]{55}",
    ) {
        let addr = format!("{}{}", prefix, body);
        let quoted = format!("\"{}\"", addr);
        let _result = ArgParser::parse(&quoted);
        // Valid addresses → ScVal::Address, invalid → Err, but never panic.
    }
}
