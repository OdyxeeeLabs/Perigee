//! UTC RFC 3339 timestamps for public API payloads (API-36).
//!
//! Every timestamp the API emits is rendered in UTC as an RFC 3339 string with
//! an explicit `Z` designator, for example `2026-11-19T08:30:00Z` or
//! `2026-11-19T08:30:00.123456Z`. The rules are documented in
//! `docs/API_STYLE_GUIDE.md`:
//!
//! * **Always UTC.** Values that arrive with another offset are converted to
//!   UTC, never re-labelled. A value without any offset is rejected because its
//!   instant is ambiguous.
//! * **Stored precision is preserved.** Sub-second digits are emitted in
//!   groups of three (milliseconds, microseconds or nanoseconds) only when the
//!   stored value has them; whole seconds carry no fraction. Nothing is
//!   truncated or rounded.
//! * **Ledger sequences are not timestamps.** Numeric ledger sequence fields
//!   stay plain integers and must not be routed through this module.
//!
//! Use the serde helpers on DTO fields typed `DateTime<Utc>`:
//!
//! ```ignore
//! #[derive(Serialize, Deserialize)]
//! struct Example {
//!     #[serde(with = "crate::timestamps::rfc3339_utc")]
//!     created_at: DateTime<Utc>,
//!     #[serde(with = "crate::timestamps::rfc3339_utc_option", default)]
//!     revoked_at: Option<DateTime<Utc>>,
//!     ledger_sequence: u32, // stays numeric
//! }
//! ```

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use std::fmt;

/// A timestamp string that could not be accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimestampError {
    input: String,
    reason: String,
}

impl fmt::Display for TimestampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not an RFC 3339 timestamp with an explicit offset: {}",
            self.input, self.reason
        )
    }
}

impl std::error::Error for TimestampError {}

/// Render `value` as an RFC 3339 UTC string with a `Z` designator.
///
/// Fractional seconds are emitted with 0, 3, 6 or 9 digits, whichever is the
/// shortest that represents the stored value exactly.
pub fn format_rfc3339_utc(value: &DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

/// Parse an RFC 3339 string that carries an explicit offset (`Z` or `±hh:mm`)
/// and normalise it to UTC. Naive timestamps without an offset are rejected.
pub fn parse_rfc3339_utc(input: &str) -> Result<DateTime<Utc>, TimestampError> {
    DateTime::parse_from_rfc3339(input.trim())
        .map(|value| value.with_timezone(&Utc))
        .map_err(|err| TimestampError {
            input: input.to_string(),
            reason: err.to_string(),
        })
}

/// Convert a legacy Unix epoch (seconds) value to a UTC timestamp.
///
/// Intended for migrating `u64` epoch fields to `DateTime<Utc>`; returns `None`
/// for values chrono cannot represent.
pub fn from_unix_seconds(secs: u64) -> Option<DateTime<Utc>> {
    let secs = i64::try_from(secs).ok()?;
    Utc.timestamp_opt(secs, 0).single()
}

/// The current instant, for new timestamp fields.
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// `#[serde(with = "crate::timestamps::rfc3339_utc")]` for `DateTime<Utc>` fields.
pub mod rfc3339_utc {
    use super::{format_rfc3339_utc, parse_rfc3339_utc};
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        value: &DateTime<Utc>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format_rfc3339_utc(value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<DateTime<Utc>, D::Error> {
        let raw = String::deserialize(deserializer)?;
        parse_rfc3339_utc(&raw).map_err(serde::de::Error::custom)
    }
}

/// `#[serde(with = "crate::timestamps::rfc3339_utc_option", default)]` for
/// `Option<DateTime<Utc>>` fields. `None` serialises as `null`.
pub mod rfc3339_utc_option {
    use super::{format_rfc3339_utc, parse_rfc3339_utc};
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        value: &Option<DateTime<Utc>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(value) => serializer.serialize_str(&format_rfc3339_utc(value)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<DateTime<Utc>>, D::Error> {
        match Option::<String>::deserialize(deserializer)? {
            Some(raw) => parse_rfc3339_utc(&raw)
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, Timelike};
    use serde::{Deserialize, Serialize};

    fn at(nanos: u32) -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(2026, 11, 19)
            .unwrap()
            .and_hms_nano_opt(8, 30, 15, nanos)
            .unwrap()
            .and_utc()
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Payload {
        #[serde(with = "rfc3339_utc")]
        created_at: DateTime<Utc>,
        #[serde(with = "rfc3339_utc_option", default)]
        revoked_at: Option<DateTime<Utc>>,
        ledger_sequence: u32,
    }

    #[test]
    fn zero_offset_is_rendered_with_a_z_designator() {
        assert_eq!(format_rfc3339_utc(&at(0)), "2026-11-19T08:30:15Z");
        assert!(!format_rfc3339_utc(&at(0)).contains("+00:00"));
    }

    #[test]
    fn fractional_precision_is_preserved_without_rounding() {
        assert_eq!(
            format_rfc3339_utc(&at(123_000_000)),
            "2026-11-19T08:30:15.123Z"
        );
        assert_eq!(
            format_rfc3339_utc(&at(123_456_000)),
            "2026-11-19T08:30:15.123456Z"
        );
        assert_eq!(
            format_rfc3339_utc(&at(123_456_789)),
            "2026-11-19T08:30:15.123456789Z"
        );
        // A value just below the next second must not round up.
        assert_eq!(
            format_rfc3339_utc(&at(999_999_999)),
            "2026-11-19T08:30:15.999999999Z"
        );
    }

    #[test]
    fn values_with_other_offsets_are_converted_not_relabelled() {
        let parsed = parse_rfc3339_utc("2026-11-19T10:30:15.5+02:00").unwrap();
        assert_eq!(parsed, at(500_000_000));
        assert_eq!(format_rfc3339_utc(&parsed), "2026-11-19T08:30:15.500Z");

        let west = parse_rfc3339_utc("2026-11-19T03:30:15-05:00").unwrap();
        assert_eq!(format_rfc3339_utc(&west), "2026-11-19T08:30:15Z");
    }

    #[test]
    fn timestamps_without_an_offset_are_rejected() {
        assert!(parse_rfc3339_utc("2026-11-19T08:30:15").is_err());
        assert!(parse_rfc3339_utc("2026-11-19 08:30:15").is_err());
        assert!(parse_rfc3339_utc("1763541015").is_err());
        let err = parse_rfc3339_utc("yesterday").unwrap_err();
        assert!(err.to_string().contains("explicit offset"));
    }

    #[test]
    fn round_trip_parsing_is_lossless_at_every_precision() {
        for nanos in [0, 1_000_000, 120_000_000, 1_000, 999_999, 1, 123_456_789] {
            let original = at(nanos);
            let rendered = format_rfc3339_utc(&original);
            let parsed = parse_rfc3339_utc(&rendered).unwrap();
            assert_eq!(parsed, original, "round trip of {rendered}");
            assert_eq!(parsed.nanosecond(), nanos);
            assert_eq!(format_rfc3339_utc(&parsed), rendered);
        }
    }

    #[test]
    fn serde_helpers_round_trip_and_leave_ledger_sequences_numeric() {
        let payload = Payload {
            created_at: at(250_000_000),
            revoked_at: None,
            ledger_sequence: 51_234_567,
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["created_at"], "2026-11-19T08:30:15.250Z");
        assert!(json["revoked_at"].is_null());
        assert_eq!(json["ledger_sequence"], 51_234_567);
        assert!(json["ledger_sequence"].is_u64());

        let back: Payload = serde_json::from_value(json).unwrap();
        assert_eq!(back, payload);

        let offset_input = serde_json::json!({
            "created_at": "2026-11-19T09:30:15.250+01:00",
            "revoked_at": "2026-11-19T08:30:15Z",
            "ledger_sequence": 7
        });
        let parsed: Payload = serde_json::from_value(offset_input).unwrap();
        assert_eq!(parsed.created_at, at(250_000_000));
        assert_eq!(parsed.revoked_at, Some(at(0)));
    }

    #[test]
    fn serde_rejects_naive_timestamps() {
        let naive = serde_json::json!({
            "created_at": "2026-11-19T08:30:15",
            "ledger_sequence": 1
        });
        assert!(serde_json::from_value::<Payload>(naive).is_err());
    }

    #[test]
    fn legacy_epoch_seconds_convert_to_utc() {
        let converted = from_unix_seconds(at(0).timestamp() as u64).unwrap();
        assert_eq!(format_rfc3339_utc(&converted), "2026-11-19T08:30:15Z");
        assert!(from_unix_seconds(u64::MAX).is_none());
    }
}
