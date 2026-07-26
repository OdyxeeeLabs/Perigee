use thiserror::Error;

#[derive(Error, Debug)]
pub enum FeeValidationError {
    #[error("Markup {0}% exceeds maximum allowed {1}%")]
    MarkupExceedsMaximum(f64, f64),

    #[error("Invalid fee split: parts do not sum to 100%")]
    InvalidFeeSplit,
}

pub struct FeeSplitGuard {
    max_markup_pct: f64,
}

impl FeeSplitGuard {
    pub fn new(max_markup_pct: f64) -> Self {
        Self { max_markup_pct }
    }

    pub fn validate_markup(&self, markup_pct: f64) -> Result<(), FeeValidationError> {
        if markup_pct > self.max_markup_pct {
            Err(FeeValidationError::MarkupExceedsMaximum(
                markup_pct,
                self.max_markup_pct,
            ))
        } else {
            Ok(())
        }
    }

    pub fn validate_split(parts: &[f64]) -> Result<(), FeeValidationError> {
        let sum: f64 = parts.iter().sum();
        if (sum - 100.0).abs() > f64::EPSILON {
            Err(FeeValidationError::InvalidFeeSplit)
        } else {
            Ok(())
        }
    }

    pub fn clamp_markup(&self, markup_pct: f64) -> f64 {
        markup_pct.min(self.max_markup_pct).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_markup_ok() {
        let guard = FeeSplitGuard::new(10.0);
        assert!(guard.validate_markup(5.0).is_ok());
    }

    #[test]
    fn test_validate_markup_exceeds() {
        let guard = FeeSplitGuard::new(10.0);
        assert!(guard.validate_markup(15.0).is_err());
    }

    #[test]
    fn test_validate_split_ok() {
        assert!(FeeSplitGuard::validate_split(&[50.0, 30.0, 20.0]).is_ok());
    }

    #[test]
    fn test_validate_split_invalid() {
        assert!(FeeSplitGuard::validate_split(&[50.0, 30.0]).is_err());
    }

    #[test]
    fn test_clamp_markup() {
        let guard = FeeSplitGuard::new(10.0);
        assert_eq!(guard.clamp_markup(5.0), 5.0);
        assert_eq!(guard.clamp_markup(15.0), 10.0);
        assert_eq!(guard.clamp_markup(-5.0), 0.0);
    }
}
