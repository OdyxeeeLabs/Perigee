//! Vault policy configuration validation (BE-033 / issue #270).
//!
//! Vault creation stored `config_json` verbatim: a policy could declare an
//! empty asset allow-list, a fee of 400%, or a high-water mark that never
//! matched the deposit it is supposed to track, and nothing complained before
//! the row was written. [`crate::vault_store::VaultStore`] now calls
//! [`validate_policy_config`] before storing (and before applying a config
//! update), so an internally inconsistent policy is refused at the door.
//!
//! # What is checked
//!
//! | Acceptance criterion | Rule |
//! |----------------------|------|
//! | allowed assets non-empty | when any policy constraint is declared, `allowed_assets` must exist and be a non-empty list of non-empty strings |
//! | fee percentage in 0–100% | `fee_percent` (or `fee_bps`, 0–10000) must be a number inside its range |
//! | high-water mark = deposit | `high_water_mark` and `deposit_amount` must both be present and numerically equal |
//! | structured errors | every failure is a typed [`VaultValidationError`] returned through `Result` — never a panic |
//!
//! `allowed_operations`, when present, must also be a non-empty list of
//! non-empty strings.
//!
//! # Where the fields live
//!
//! Both the root object and a nested `policy` object are searched, nested
//! first — the same layout [`crate::policy_expiry`] accepts. Each field is
//! accepted in `snake_case` and `camelCase` spellings, because the config is
//! authored by both Rust handlers and the TypeScript front end.
//!
//! # Configurations that declare nothing are left alone
//!
//! Existing vaults default to `config_json = "{}"`, and a policy may
//! legitimately carry nothing but an `expires_at` (that is BE-023's business).
//! Validation therefore only runs once the configuration declares one of the
//! fields above, so no stored vault is retroactively invalidated and the
//! expiry-only fixtures keep working.
//!
//! # Malformed JSON is deliberately out of scope here
//!
//! Unparseable `config_json` is skipped by this module. [`crate::policy_expiry`]
//! already fails *closed* on it at operation time, so a policy nobody can read
//! authorises nothing; rejecting it at creation as well would change storage
//! behaviour that BE-023 deliberately left open (its regression test seeds a
//! malformed config through `VaultStore::create` on purpose).
//!
//! A JSON value that parses but is not an object (`"[]"`, `"5"`, `"\"x\""`)
//! *is* rejected: there is nowhere for a policy to live in those.

use serde_json::Value;
use thiserror::Error;

/// Why a vault's policy configuration was rejected.
///
/// Carried in a `Result` all the way to the HTTP layer — see
/// `VaultStoreError::InvalidPolicy`, which maps it onto
/// `ErrorCode::ValidationFailed` (HTTP 400).
#[derive(Debug, Clone, PartialEq, Error)]
pub enum VaultValidationError {
    /// `config_json` parsed as JSON but is not an object.
    #[error("vault policy config must be a JSON object, found {found}")]
    NotAnObject {
        /// JSON type that was found instead (`"array"`, `"string"`, …).
        found: String,
    },

    /// A list field is absent (while other constraints are declared) or empty.
    #[error("`{field}` must be a non-empty list of non-empty strings")]
    EmptyList {
        /// Canonical field name, e.g. `allowed_assets`.
        field: &'static str,
    },

    /// A list field has the wrong shape.
    #[error("`{field}` is invalid: {reason}")]
    InvalidList {
        /// Canonical field name.
        field: &'static str,
        /// Human-readable reason, e.g. `entry 2 is not a string`.
        reason: String,
    },

    /// A field that must hold a number does not.
    #[error("`{field}` must be a number, found {got}")]
    InvalidNumber {
        /// Canonical field name.
        field: &'static str,
        /// JSON type of the offending value.
        got: String,
    },

    /// A numeric field is outside its permitted range.
    #[error("`{field}` must be between {min} and {max}, found {value}")]
    OutOfRange {
        /// Canonical field name.
        field: &'static str,
        /// The rejected value.
        value: f64,
        /// Inclusive lower bound.
        min: f64,
        /// Inclusive upper bound.
        max: f64,
    },

    /// The high-water mark does not match the deposit it was seeded from.
    #[error(
        "`high_water_mark` ({high_water_mark}) must equal `deposit_amount` ({deposit_amount})"
    )]
    HighWaterMarkMismatch {
        /// The configured high-water mark.
        high_water_mark: f64,
        /// The configured deposit amount.
        deposit_amount: f64,
    },

    /// Only one half of the high-water-mark/deposit pair was declared, so the
    /// "initialised to the deposit amount" invariant cannot hold.
    #[error("`{field}` is required whenever `{sibling}` is set")]
    MissingPairField {
        /// The field that is missing.
        field: &'static str,
        /// The field that was present and demands it.
        sibling: &'static str,
    },
}

impl VaultValidationError {
    /// Stable, machine-readable identifier for this failure.
    ///
    /// It is appended to the API error message so clients can branch on a
    /// constant instead of parsing prose.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotAnObject { .. } => "POLICY_NOT_AN_OBJECT",
            Self::EmptyList { .. } => "POLICY_EMPTY_LIST",
            Self::InvalidList { .. } => "POLICY_INVALID_LIST",
            Self::InvalidNumber { .. } => "POLICY_INVALID_NUMBER",
            Self::OutOfRange { .. } => "POLICY_VALUE_OUT_OF_RANGE",
            Self::HighWaterMarkMismatch { .. } => "POLICY_HIGH_WATER_MARK_MISMATCH",
            Self::MissingPairField { .. } => "POLICY_MISSING_PAIR_FIELD",
        }
    }
}

// ── Field names ──────────────────────────────────────────────────────────────
//
// Each entry is `[canonical_snake_case, …camelCase aliases]`; the canonical
// spelling is what error messages and `field` values use.

const ALLOWED_ASSETS: &[&str] = &["allowed_assets", "allowedAssets"];
const ALLOWED_OPERATIONS: &[&str] = &["allowed_operations", "allowedOperations"];
const FEE_PERCENT: &[&str] = &[
    "fee_percent",
    "feePercentage",
    "fee_percentage",
    "management_fee_percent",
    "managementFeePercent",
];
const FEE_BPS: &[&str] = &["fee_bps", "feeBps"];
const HIGH_WATER_MARK: &[&str] = &["high_water_mark", "highWaterMark"];
const DEPOSIT_AMOUNT: &[&str] = &[
    "deposit_amount",
    "depositAmount",
    "initial_deposit",
    "initialDeposit",
];

const FEE_PERCENT_MAX: f64 = 100.0;
const FEE_BPS_MAX: f64 = 10_000.0;

/// Validate the policy configuration carried in a vault's `config_json`.
///
/// Accepts the raw string exactly as the store would persist it; nothing is
/// written unless this returns `Ok(())`.
///
/// ```
/// use Perigee_core::vault_validation::validate_policy_config;
///
/// assert!(validate_policy_config("{}").is_ok());
/// assert!(validate_policy_config(r#"{"policy":{"allowed_assets":[]}}"#).is_err());
/// ```
pub fn validate_policy_config(config_json: &str) -> Result<(), VaultValidationError> {
    let trimmed = config_json.trim();

    // No config at all: nothing to validate (same opt-in rule as policy
    // expiry, so existing vaults are unaffected).
    if trimmed.is_empty() {
        return Ok(());
    }

    let root: Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        // Malformed JSON is refused at operation time by `policy_expiry`
        // (BE-023), which fails closed. See the module docs.
        Err(_) => return Ok(()),
    };

    if !root.is_object() {
        return Err(VaultValidationError::NotAnObject {
            found: json_type(&root).to_string(),
        });
    }

    validate_policy_object(&root)
}

/// The rules, applied to a parsed configuration object.
fn validate_policy_object(root: &Value) -> Result<(), VaultValidationError> {
    let allowed_assets = find_field(root, ALLOWED_ASSETS);
    let allowed_operations = find_field(root, ALLOWED_OPERATIONS);
    let fee_percent = find_field(root, FEE_PERCENT);
    let fee_bps = find_field(root, FEE_BPS);
    let high_water_mark = find_field(root, HIGH_WATER_MARK);
    let deposit_amount = find_field(root, DEPOSIT_AMOUNT);

    let declares_a_constraint = [
        allowed_assets,
        allowed_operations,
        fee_percent,
        fee_bps,
        high_water_mark,
        deposit_amount,
    ]
    .iter()
    .any(Option::is_some);

    // A config that declares none of the fields above (e.g. "{}" or an
    // expiry-only policy) carries no policy to be inconsistent about.
    if !declares_a_constraint {
        return Ok(());
    }

    // ── Allowed assets ───────────────────────────────────────────────────────
    // Declaring any constraint means declaring what it applies to, so the
    // allow-list is mandatory from this point on — absent counts as empty.
    match allowed_assets {
        Some(value) => validate_string_list("allowed_assets", value)?,
        None => {
            return Err(VaultValidationError::EmptyList {
                field: "allowed_assets",
            })
        }
    }

    // ── Allowed operations ───────────────────────────────────────────────────
    if let Some(value) = allowed_operations {
        validate_string_list("allowed_operations", value)?;
    }

    // ── Fee parameters ───────────────────────────────────────────────────────
    if let Some(value) = fee_percent {
        let fee = as_number("fee_percent", value)?;
        ensure_range("fee_percent", fee, 0.0, FEE_PERCENT_MAX)?;
    }
    if let Some(value) = fee_bps {
        let fee = as_number("fee_bps", value)?;
        ensure_range("fee_bps", fee, 0.0, FEE_BPS_MAX)?;
    }

    // ── High-water mark ──────────────────────────────────────────────────────
    match (high_water_mark, deposit_amount) {
        (Some(high_water_mark), Some(deposit_amount)) => {
            let high_water_mark = as_number("high_water_mark", high_water_mark)?;
            let deposit_amount = as_number("deposit_amount", deposit_amount)?;
            if !nearly_equal(high_water_mark, deposit_amount) {
                return Err(VaultValidationError::HighWaterMarkMismatch {
                    high_water_mark,
                    deposit_amount,
                });
            }
        }
        (Some(_), None) => {
            return Err(VaultValidationError::MissingPairField {
                field: "deposit_amount",
                sibling: "high_water_mark",
            })
        }
        (None, Some(_)) => {
            return Err(VaultValidationError::MissingPairField {
                field: "high_water_mark",
                sibling: "deposit_amount",
            })
        }
        (None, None) => {}
    }

    Ok(())
}

/// Locate a policy field, preferring the nested `policy` object over the root
/// and skipping explicit `null`s (which read as "not set").
fn find_field<'a>(root: &'a Value, names: &[&str]) -> Option<&'a Value> {
    let policy = root.get("policy").filter(|value| value.is_object());

    let nested = names
        .iter()
        .find_map(|name| policy.and_then(|policy| policy.get(*name)))
        .filter(|value| !value.is_null());

    let at_root = names
        .iter()
        .find_map(|name| root.get(*name))
        .filter(|value| !value.is_null());

    nested.or(at_root)
}

/// A list field must hold at least one non-empty string.
fn validate_string_list(
    field: &'static str,
    value: &Value,
) -> Result<(), VaultValidationError> {
    let items = value.as_array().ok_or_else(|| VaultValidationError::InvalidList {
        field,
        reason: format!("expected an array, found {}", json_type(value)),
    })?;

    if items.is_empty() {
        return Err(VaultValidationError::EmptyList { field });
    }

    for (index, item) in items.iter().enumerate() {
        match item.as_str() {
            Some(text) if !text.trim().is_empty() => {}
            Some(_) => {
                return Err(VaultValidationError::InvalidList {
                    field,
                    reason: format!("entry {index} is empty"),
                })
            }
            None => {
                return Err(VaultValidationError::InvalidList {
                    field,
                    reason: format!("entry {index} is not a string"),
                })
            }
        }
    }

    Ok(())
}

/// Numbers may arrive as JSON numbers or as numeric strings (the config is
/// authored by hand as well as by code); anything else is a type error.
fn as_number(field: &'static str, value: &Value) -> Result<f64, VaultValidationError> {
    let parsed = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    };

    parsed
        .filter(|number| number.is_finite())
        .ok_or_else(|| VaultValidationError::InvalidNumber {
            field,
            got: json_type(value).to_string(),
        })
}

fn ensure_range(
    field: &'static str,
    value: f64,
    min: f64,
    max: f64,
) -> Result<(), VaultValidationError> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(VaultValidationError::OutOfRange {
            field,
            value,
            min,
            max,
        })
    }
}

/// Compare high-water mark and deposit amount without tripping over the last
/// bits of an f64 that parsed from decimal input.
fn nearly_equal(a: f64, b: f64) -> bool {
    (a - b).abs() <= f64::EPSILON * a.abs().max(b.abs()).max(1.0)
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run validation and unwrap the failure, so a test can match on it.
    fn failure(config: &str) -> VaultValidationError {
        match validate_policy_config(config) {
            Ok(()) => panic!("expected {config} to be rejected"),
            Err(error) => error,
        }
    }

    // ── Accepted configurations ──────────────────────────────────────────────

    #[test]
    fn an_empty_config_is_accepted() {
        assert!(validate_policy_config("").is_ok());
        assert!(validate_policy_config("   ").is_ok());
        assert!(validate_policy_config("{}").is_ok());
    }

    /// BE-023 fixtures: a policy carrying only an expiry declares none of the
    /// fields this module owns, so it must keep storing cleanly.
    #[test]
    fn an_expiry_only_policy_is_accepted() {
        let config = r#"{"policy":{"expires_at":"2099-01-01T00:00:00Z"}}"#;
        assert!(validate_policy_config(config).is_ok());
    }

    #[test]
    fn a_consistent_policy_is_accepted() {
        let config = r#"{
            "policy": {
                "allowed_assets": ["XLM", "USDC"],
                "allowed_operations": ["rebalance", "withdraw"],
                "fee_percent": 1.5,
                "deposit_amount": 1000,
                "high_water_mark": 1000
            }
        }"#;
        assert!(validate_policy_config(config).is_ok());
    }

    #[test]
    fn camel_case_field_names_are_accepted() {
        let config = r#"{
            "policy": {
                "allowedAssets": ["XLM"],
                "feePercentage": 10,
                "depositAmount": 500,
                "highWaterMark": 500
            }
        }"#;
        assert!(validate_policy_config(config).is_ok());
    }

    #[test]
    fn numeric_strings_are_accepted_for_numbers() {
        let config = r#"{"policy":{"allowed_assets":["XLM"],"fee_percent":"2.5"}}"#;
        assert!(validate_policy_config(config).is_ok());
    }

    #[test]
    fn root_level_fields_are_read_when_there_is_no_policy_block() {
        let config = r#"{"allowed_assets":["XLM"],"fee_percent":0}"#;
        assert!(validate_policy_config(config).is_ok());
    }

    /// Malformed JSON stays `policy_expiry`'s job (BE-023 fails closed at
    /// operation time); this module must not reject it at creation.
    #[test]
    fn malformed_json_is_left_to_policy_expiry() {
        assert!(validate_policy_config("{not json").is_ok());
        assert!(
            crate::policy_expiry::ensure_policy_active("{not json", chrono::Utc::now()).is_err()
        );
    }

    // ── Allowed assets ───────────────────────────────────────────────────────

    #[test]
    fn an_empty_allowed_assets_list_is_rejected() {
        let error = failure(r#"{"policy":{"allowed_assets":[]}}"#);
        assert!(matches!(
            error,
            VaultValidationError::EmptyList {
                field: "allowed_assets"
            }
        ));
        assert_eq!(error.code(), "POLICY_EMPTY_LIST");
        assert!(error.to_string().contains("allowed_assets"));
    }

    #[test]
    fn allowed_assets_is_required_as_soon_as_any_constraint_is_declared() {
        let error = failure(r#"{"policy":{"allowed_operations":["rebalance"]}}"#);
        assert!(matches!(
            error,
            VaultValidationError::EmptyList {
                field: "allowed_assets"
            }
        ));
    }

    #[test]
    fn allowed_assets_must_be_a_list_of_strings() {
        let error = failure(r#"{"policy":{"allowed_assets":"XLM"}}"#);
        assert!(matches!(
            error,
            VaultValidationError::InvalidList {
                field: "allowed_assets",
                ..
            }
        ));

        let error = failure(r#"{"policy":{"allowed_assets":["XLM", 42]}}"#);
        assert!(matches!(
            error,
            VaultValidationError::InvalidList {
                field: "allowed_assets",
                ..
            }
        ));

        let error = failure(r#"{"policy":{"allowed_assets":[""]}}"#);
        assert!(matches!(
            error,
            VaultValidationError::InvalidList {
                field: "allowed_assets",
                ..
            }
        ));
    }

    #[test]
    fn an_empty_allowed_operations_list_is_rejected() {
        let error = failure(
            r#"{"policy":{"allowed_assets":["XLM"],"allowed_operations":[]}}"#,
        );
        assert!(matches!(
            error,
            VaultValidationError::EmptyList {
                field: "allowed_operations"
            }
        ));
    }

    /// The nested `policy` block wins over a root-level copy, mirroring
    /// `policy_expiry`: the more specific statement is the one that counts.
    #[test]
    fn the_nested_policy_block_wins_over_the_root() {
        let error = failure(r#"{"policy":{"allowed_assets":[]},"allowed_assets":["XLM"]}"#);
        assert!(matches!(
            error,
            VaultValidationError::EmptyList {
                field: "allowed_assets"
            }
        ));
    }

    // ── Fee parameters ───────────────────────────────────────────────────────

    #[test]
    fn a_fee_above_one_hundred_percent_is_rejected() {
        let error = failure(r#"{"policy":{"allowed_assets":["XLM"],"fee_percent":250}}"#);

        if let VaultValidationError::OutOfRange {
            field,
            value,
            min,
            max,
        } = error
        {
            assert_eq!(field, "fee_percent");
            assert_eq!(value, 250.0);
            assert_eq!((min, max), (0.0, 100.0));
        } else {
            panic!("expected OutOfRange, got {error:?}");
        }
        assert_eq!(error.code(), "POLICY_VALUE_OUT_OF_RANGE");
    }

    #[test]
    fn a_negative_fee_is_rejected() {
        let error = failure(r#"{"policy":{"allowed_assets":["XLM"],"fee_percent":-1}}"#);
        assert!(matches!(
            error,
            VaultValidationError::OutOfRange {
                field: "fee_percent",
                ..
            }
        ));
    }

    #[test]
    fn fee_bps_is_capped_at_ten_thousand() {
        let config = r#"{"policy":{"allowed_assets":["XLM"],"fee_bps":10000}}"#;
        assert!(validate_policy_config(config).is_ok());

        let error = failure(r#"{"policy":{"allowed_assets":["XLM"],"fee_bps":20000}}"#);
        if let VaultValidationError::OutOfRange { field, max, .. } = error {
            assert_eq!(field, "fee_bps");
            assert_eq!(max, 10_000.0);
        } else {
            panic!("expected OutOfRange, got {error:?}");
        }
    }

    #[test]
    fn a_non_numeric_fee_is_rejected() {
        let error = failure(r#"{"policy":{"allowed_assets":["XLM"],"fee_percent":"lots"}}"#);
        assert!(matches!(
            error,
            VaultValidationError::InvalidNumber {
                field: "fee_percent",
                ..
            }
        ));
        assert_eq!(error.code(), "POLICY_INVALID_NUMBER");
    }

    // ── High-water mark ──────────────────────────────────────────────────────

    #[test]
    fn a_high_water_mark_that_differs_from_the_deposit_is_rejected() {
        let error = failure(
            r#"{"policy":{"allowed_assets":["XLM"],"deposit_amount":1000,"high_water_mark":900}}"#,
        );

        if let VaultValidationError::HighWaterMarkMismatch {
            high_water_mark,
            deposit_amount,
        } = error
        {
            assert_eq!(high_water_mark, 900.0);
            assert_eq!(deposit_amount, 1000.0);
        } else {
            panic!("expected HighWaterMarkMismatch, got {error:?}");
        }
        assert_eq!(error.code(), "POLICY_HIGH_WATER_MARK_MISMATCH");
    }

    #[test]
    fn a_deposit_without_a_high_water_mark_is_rejected() {
        let error = failure(r#"{"policy":{"allowed_assets":["XLM"],"deposit_amount":1000}}"#);
        assert!(matches!(
            error,
            VaultValidationError::MissingPairField {
                field: "high_water_mark",
                sibling: "deposit_amount",
            }
        ));
    }

    #[test]
    fn a_high_water_mark_without_a_deposit_is_rejected() {
        let error = failure(r#"{"policy":{"allowed_assets":["XLM"],"high_water_mark":1000}}"#);
        assert!(matches!(
            error,
            VaultValidationError::MissingPairField {
                field: "deposit_amount",
                sibling: "high_water_mark",
            }
        ));
    }

    #[test]
    fn a_non_numeric_high_water_mark_is_rejected() {
        let error = failure(
            r#"{"policy":{"allowed_assets":["XLM"],"deposit_amount":1000,"high_water_mark":"n/a"}}"#,
        );
        assert!(matches!(
            error,
            VaultValidationError::InvalidNumber {
                field: "high_water_mark",
                ..
            }
        ));
    }

    // ── Configurations that are not policy objects at all ────────────────────

    #[test]
    fn a_non_object_config_is_rejected() {
        let error = failure("[1,2,3]");
        assert!(matches!(
            error,
            VaultValidationError::NotAnObject { .. }
        ));
        assert!(error.to_string().contains("array"));
        assert_eq!(error.code(), "POLICY_NOT_AN_OBJECT");
    }
}
