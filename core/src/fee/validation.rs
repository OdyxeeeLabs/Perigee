use crate::fee::persistence::LedgerFeeSample;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeeValidationError {
    InvalidSafetyMargin(String),
    InvalidCharge(String),
    NegativeFee(String),
    OutsideTolerance { predicted_fee: i64, actual_fee: i64 },
}

impl fmt::Display for FeeValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSafetyMargin(message)
            | Self::InvalidCharge(message)
            | Self::NegativeFee(message) => formatter.write_str(message),
            Self::OutsideTolerance {
                predicted_fee,
                actual_fee,
            } => write!(
                formatter,
                "actual fee {actual_fee} is outside the configured tolerance for predicted fee {predicted_fee}"
            ),
        }
    }
}

impl std::error::Error for FeeValidationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeValidator {
    max_fee_bps: u32,
}

impl FeeValidator {
    pub fn new(max_fee_pct: f64) -> Self {
        let max_fee_bps = if max_fee_pct.is_finite() && max_fee_pct > 0.0 {
            crate::rounding::decimal_to_bps(max_fee_pct)
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(0)
        } else {
            0
        };
        Self { max_fee_bps }
    }

    pub fn from_bps(max_fee_bps: u32) -> Self {
        Self { max_fee_bps }
    }

    pub fn max_fee_bps(&self) -> u32 {
        self.max_fee_bps
    }

    pub fn is_acceptable(&self, predicted_fee: i64, actual_fee: i64) -> bool {
        if predicted_fee == 0 {
            return actual_fee >= 0;
        }
        if predicted_fee < 0 || actual_fee < 0 {
            return false;
        }

        let difference = (actual_fee as i128 - predicted_fee as i128).unsigned_abs();
        difference * 10_000 <= predicted_fee as u128 * self.max_fee_bps as u128
    }

    pub fn validate(&self, predicted_fee: i64, actual_fee: i64) -> Result<(), FeeValidationError> {
        if predicted_fee < 0 || actual_fee < 0 {
            return Err(FeeValidationError::NegativeFee(
                "predicted_fee and actual_fee must not be negative".to_string(),
            ));
        }
        if !self.is_acceptable(predicted_fee, actual_fee) {
            return Err(FeeValidationError::OutsideTolerance {
                predicted_fee,
                actual_fee,
            });
        }
        Ok(())
    }
}

pub fn validate_safety_margin_bps(value: u32) -> Result<u32, FeeValidationError> {
    if !(crate::fee::calculation::SAFETY_MARGIN_MIN_BPS
        ..=crate::fee::calculation::SAFETY_MARGIN_MAX_BPS)
        .contains(&value)
    {
        return Err(FeeValidationError::InvalidSafetyMargin(format!(
            "safety_margin_bps must be in [{}, {}] (got {})",
            crate::fee::calculation::SAFETY_MARGIN_MIN_BPS,
            crate::fee::calculation::SAFETY_MARGIN_MAX_BPS,
            value
        )));
    }
    Ok(value)
}

pub fn safety_margin_to_bps(value: f64) -> Result<u32, FeeValidationError> {
    if !value.is_finite() || value <= 0.0 {
        return Err(FeeValidationError::InvalidSafetyMargin(format!(
            "safety_margin must be a finite positive multiplier (got {})",
            value
        )));
    }
    let bps = crate::rounding::decimal_to_bps(value)
        .and_then(|converted| u32::try_from(converted).ok())
        .ok_or_else(|| {
            FeeValidationError::InvalidSafetyMargin(format!(
                "safety_margin {value} is out of representable range"
            ))
        })?;
    validate_safety_margin_bps(bps)
}

pub fn validate_charge_request(
    idempotency_key: &str,
    payer: &str,
    amount_stroops: i64,
) -> Result<(), FeeValidationError> {
    if idempotency_key.trim().is_empty() {
        return Err(FeeValidationError::InvalidCharge(
            "idempotency_key must not be empty".to_string(),
        ));
    }
    if payer.trim().is_empty() {
        return Err(FeeValidationError::InvalidCharge(
            "payer must not be empty".to_string(),
        ));
    }
    if amount_stroops <= 0 {
        return Err(FeeValidationError::InvalidCharge(format!(
            "amount_stroops must be positive (got {})",
            amount_stroops
        )));
    }
    Ok(())
}

pub fn validate_ledger_fee_sample(
    sample: &LedgerFeeSample,
) -> Result<(), FeeValidationError> {
    if sample.ledger_sequence < 0 {
        return Err(FeeValidationError::NegativeFee(
            "ledger_sequence must not be negative".to_string(),
        ));
    }
    if sample.base_reserve < 0
        || sample.base_fee < 0
        || sample.max_fee < 0
        || sample.fee_charged < 0
        || sample.transaction_count < 0
    {
        return Err(FeeValidationError::NegativeFee(
            "fee sample amounts and transaction_count must not be negative".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_tolerance_uses_integer_comparison() {
        let validator = FeeValidator::new(10.0);
        assert!(validator.is_acceptable(100, 110));
        assert!(!validator.is_acceptable(100, 111));
        assert!(validator.is_acceptable(0, 100));
    }

    #[test]
    fn invalid_charges_are_rejected() {
        assert!(validate_charge_request("", "payer", 1).is_err());
        assert!(validate_charge_request("key", "", 1).is_err());
        assert!(validate_charge_request("key", "payer", 0).is_err());
    }
}
