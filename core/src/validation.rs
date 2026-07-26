use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct ValidationRule {
    pub field: String,
    pub rule_type: ValidationRuleType,
}

#[derive(Debug, Clone)]
pub enum ValidationRuleType {
    Required,
    MaxLength(usize),
    MinValue(f64),
    MaxValue(f64),
    Pattern(String),
    Custom(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
    pub code: String,
}

pub struct ValidationPipe {
    rules: Vec<ValidationRule>,
    whitelist: bool,
    transform: bool,
}

impl ValidationPipe {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            whitelist: false,
            transform: false,
        }
    }

    pub fn with_whitelist(mut self) -> Self {
        self.whitelist = true;
        self
    }

    pub fn with_transform(mut self) -> Self {
        self.transform = true;
        self
    }

    pub fn add_rule(&mut self, rule: ValidationRule) {
        self.rules.push(rule);
    }

    pub fn validate(&self, data: &serde_json::Value) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();

        for rule in &self.rules {
            let value = data.get(&rule.field);
            match &rule.rule_type {
                ValidationRuleType::Required => {
                    if let Err(e) = validate_required(&rule.field, value) {
                        errors.push(e);
                    }
                }
                ValidationRuleType::MaxLength(max) => {
                    if let Some(val) = value.and_then(|v| v.as_str()) {
                        if let Err(e) = validate_max_length(&rule.field, val, *max) {
                            errors.push(e);
                        }
                    }
                }
                ValidationRuleType::MinValue(min) => {
                    if let Some(val) = value.and_then(|v| v.as_f64()) {
                        if val < *min {
                            errors.push(ValidationError {
                                field: rule.field.clone(),
                                message: format!("Value must be at least {}", min),
                                code: "MIN_VALUE".to_string(),
                            });
                        }
                    }
                }
                ValidationRuleType::MaxValue(max) => {
                    if let Some(val) = value.and_then(|v| v.as_f64()) {
                        if val > *max {
                            errors.push(ValidationError {
                                field: rule.field.clone(),
                                message: format!("Value must be at most {}", max),
                                code: "MAX_VALUE".to_string(),
                            });
                        }
                    }
                }
                ValidationRuleType::Pattern(pattern) => {
                    if let Some(val) = value.and_then(|v| v.as_str()) {
                        if !regex_match(val, pattern) {
                            errors.push(ValidationError {
                                field: rule.field.clone(),
                                message: format!("Value does not match pattern: {}", pattern),
                                code: "PATTERN_MISMATCH".to_string(),
                            });
                        }
                    }
                }
                ValidationRuleType::Custom(name) => {
                    errors.push(ValidationError {
                        field: rule.field.clone(),
                        message: format!("Custom rule '{}' not implemented", name),
                        code: "CUSTOM_RULE_UNIMPLEMENTED".to_string(),
                    });
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl Default for ValidationPipe {
    fn default() -> Self {
        Self::new()
    }
}

pub fn validate_required(
    field: &str,
    value: Option<&serde_json::Value>,
) -> Result<(), ValidationError> {
    match value {
        Some(serde_json::Value::Null) | None => Err(ValidationError {
            field: field.to_string(),
            message: format!("Field '{}' is required", field),
            code: "REQUIRED".to_string(),
        }),
        Some(_) => Ok(()),
    }
}

pub fn validate_max_length(
    field: &str,
    value: &str,
    max: usize,
) -> Result<(), ValidationError> {
    if value.len() > max {
        Err(ValidationError {
            field: field.to_string(),
            message: format!(
                "Field '{}' exceeds maximum length of {}",
                field, max
            ),
            code: "MAX_LENGTH".to_string(),
        })
    } else {
        Ok(())
    }
}

pub fn validate_range(
    field: &str,
    value: f64,
    min: f64,
    max: f64,
) -> Result<(), ValidationError> {
    if value < min || value > max {
        Err(ValidationError {
            field: field.to_string(),
            message: format!(
                "Field '{}' must be between {} and {}",
                field, min, max
            ),
            code: "OUT_OF_RANGE".to_string(),
        })
    } else {
        Ok(())
    }
}

fn regex_match(value: &str, pattern: &str) -> bool {
    match pattern {
        "email" => value.contains('@') && value.contains('.'),
        "numeric" => value.chars().all(|c| c.is_ascii_digit()),
        "alphanumeric" => value.chars().all(|c| c.is_ascii_alphanumeric()),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_validate_required_present() {
        assert!(validate_required("name", Some(&json!("test"))).is_ok());
    }

    #[test]
    fn test_validate_required_missing() {
        assert!(validate_required("name", None).is_err());
    }

    #[test]
    fn test_validate_required_null() {
        assert!(validate_required("name", Some(&json!(null))).is_err());
    }

    #[test]
    fn test_validate_max_length_ok() {
        assert!(validate_max_length("name", "ab", 5).is_ok());
    }

    #[test]
    fn test_validate_max_length_fail() {
        assert!(validate_max_length("name", "abcdef", 3).is_err());
    }

    #[test]
    fn test_validate_range_ok() {
        assert!(validate_range("age", 25.0, 0.0, 150.0).is_ok());
    }

    #[test]
    fn test_validate_range_fail() {
        assert!(validate_range("age", 200.0, 0.0, 150.0).is_err());
    }

    #[test]
    fn test_validation_pipe() {
        let mut pipe = ValidationPipe::new();
        pipe.add_rule(ValidationRule {
            field: "name".to_string(),
            rule_type: ValidationRuleType::Required,
        });
        pipe.add_rule(ValidationRule {
            field: "name".to_string(),
            rule_type: ValidationRuleType::MaxLength(5),
        });

        assert!(pipe.validate(&json!({"name": "abc"})).is_ok());
        assert!(pipe.validate(&json!({})).is_err());
        assert!(pipe.validate(&json!({"name": "toolongname"})).is_err());
    }
}
