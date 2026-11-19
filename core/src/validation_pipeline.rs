//! # Request validation pipeline
//!
//! One place that every JSON request body goes through before a handler sees it.
//! The stages run in a fixed order and stop at the first one that fails, so a
//! bad request is rejected early with a `400` and a precise error body:
//!
//! 1. **Schema** – required fields, types, lengths, ranges, allowed values,
//!    unknown fields.
//! 2. **Sanitization** – strips markup and control characters from every string
//!    (XSS) and rejects payloads that look like SQL injection. Parameterized
//!    queries remain the real defence against SQLi; this is defence in depth.
//! 3. **Business rules** – conditional and cross-field rules.
//! 4. **Async checks** – validators that need I/O, such as uniqueness lookups.
//! 5. **Transformation** – normalisation (defaults, case folding) applied to the
//!    value that finally reaches the handler.
//!
//! Failures are logged by field name and code only, never by value.
//!
//! ```text
//! {"error":{"code":"validation_failed","message":"Request validation failed",
//!   "stage":"schema","fields":[{"field":"name","code":"required","message":"..."}]}}
//! ```

use serde::Serialize;
use serde_json::{json, Map, Value};
use std::future::Future;
use std::pin::Pin;

/// One field-level problem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub field: String,
    pub code: String,
    pub message: String,
}

impl FieldError {
    pub fn new(field: &str, code: &str, message: &str) -> Self {
        FieldError {
            field: field.to_string(),
            code: code.to_string(),
            message: message.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Schema,
    Sanitization,
    BusinessRules,
    AsyncChecks,
}

/// Why a request was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationFailure {
    pub stage: Stage,
    pub errors: Vec<FieldError>,
}

impl ValidationFailure {
    /// Validation failures are always `400`.
    pub fn status(&self) -> u16 {
        400
    }

    pub fn to_json(&self) -> Value {
        json!({
            "error": {
                "code": "validation_failed",
                "message": "Request validation failed",
                "stage": self.stage,
                "fields": self.errors,
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Stage 1: schema
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    String,
    Integer,
    Number,
    Boolean,
    Array,
    Object,
}

impl FieldType {
    fn matches(self, value: &Value) -> bool {
        match self {
            FieldType::String => value.is_string(),
            FieldType::Integer => value.is_i64() || value.is_u64(),
            FieldType::Number => value.is_number(),
            FieldType::Boolean => value.is_boolean(),
            FieldType::Array => value.is_array(),
            FieldType::Object => value.is_object(),
        }
    }

    fn name(self) -> &'static str {
        match self {
            FieldType::String => "a string",
            FieldType::Integer => "an integer",
            FieldType::Number => "a number",
            FieldType::Boolean => "a boolean",
            FieldType::Array => "an array",
            FieldType::Object => "an object",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FieldRule {
    name: String,
    kind: FieldType,
    required: bool,
    min_len: Option<usize>,
    max_len: Option<usize>,
    min: Option<f64>,
    max: Option<f64>,
    one_of: Option<Vec<String>>,
}

impl FieldRule {
    fn new(name: &str, kind: FieldType) -> Self {
        FieldRule {
            name: name.to_string(),
            kind,
            required: false,
            min_len: None,
            max_len: None,
            min: None,
            max: None,
            one_of: None,
        }
    }

    pub fn string(name: &str) -> Self {
        Self::new(name, FieldType::String)
    }
    pub fn integer(name: &str) -> Self {
        Self::new(name, FieldType::Integer)
    }
    pub fn number(name: &str) -> Self {
        Self::new(name, FieldType::Number)
    }
    pub fn boolean(name: &str) -> Self {
        Self::new(name, FieldType::Boolean)
    }
    pub fn array(name: &str) -> Self {
        Self::new(name, FieldType::Array)
    }
    pub fn object(name: &str) -> Self {
        Self::new(name, FieldType::Object)
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
    /// Length bounds (characters for strings, items for arrays).
    pub fn len(mut self, min: usize, max: usize) -> Self {
        self.min_len = Some(min);
        self.max_len = Some(max);
        self
    }
    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self
    }
    pub fn one_of(mut self, allowed: &[&str]) -> Self {
        self.one_of = Some(allowed.iter().map(|s| s.to_string()).collect());
        self
    }

    fn check(&self, value: &Value, errors: &mut Vec<FieldError>) {
        if !self.kind.matches(value) {
            errors.push(FieldError::new(
                &self.name,
                "invalid_type",
                &format!("must be {}", self.kind.name()),
            ));
            return;
        }

        let length = match value {
            Value::String(s) => Some(s.chars().count()),
            Value::Array(a) => Some(a.len()),
            _ => None,
        };
        if let Some(length) = length {
            if self.min_len.is_some_and(|min| length < min) {
                errors.push(FieldError::new(
                    &self.name,
                    "too_short",
                    &format!("must have at least {} characters or items", self.min_len.unwrap_or(0)),
                ));
            }
            if self.max_len.is_some_and(|max| length > max) {
                errors.push(FieldError::new(
                    &self.name,
                    "too_long",
                    &format!("must have at most {} characters or items", self.max_len.unwrap_or(0)),
                ));
            }
        }

        if let Some(n) = value.as_f64() {
            if self.min.is_some_and(|min| n < min) || self.max.is_some_and(|max| n > max) {
                errors.push(FieldError::new(
                    &self.name,
                    "out_of_range",
                    &format!(
                        "must be between {} and {}",
                        self.min.unwrap_or(f64::MIN),
                        self.max.unwrap_or(f64::MAX)
                    ),
                ));
            }
        }

        if let (Some(allowed), Some(s)) = (&self.one_of, value.as_str()) {
            if !allowed.iter().any(|a| a == s) {
                errors.push(FieldError::new(
                    &self.name,
                    "invalid_choice",
                    &format!("must be one of: {}", allowed.join(", ")),
                ));
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Schema {
    fields: Vec<FieldRule>,
    allow_unknown: bool,
}

impl Schema {
    pub fn new(fields: Vec<FieldRule>) -> Self {
        Schema {
            fields,
            allow_unknown: false,
        }
    }

    pub fn allow_unknown(mut self) -> Self {
        self.allow_unknown = true;
        self
    }

    fn validate(&self, input: &Value) -> Vec<FieldError> {
        let Some(object) = input.as_object() else {
            return vec![FieldError::new("", "invalid_type", "request body must be a JSON object")];
        };

        let mut errors = Vec::new();
        for rule in &self.fields {
            match object.get(&rule.name) {
                None | Some(Value::Null) if rule.required => {
                    errors.push(FieldError::new(&rule.name, "required", "is required"))
                }
                None | Some(Value::Null) => {}
                Some(value) => rule.check(value, &mut errors),
            }
        }
        if !self.allow_unknown {
            for key in object.keys() {
                if !self.fields.iter().any(|rule| &rule.name == key) {
                    errors.push(FieldError::new(key, "unknown_field", "is not a recognised field"));
                }
            }
        }
        errors
    }
}

// ---------------------------------------------------------------------------
// Stage 2: sanitization
// ---------------------------------------------------------------------------

/// Fragments that have no place in ordinary input. Matched case-insensitively.
const SQL_INJECTION_PATTERNS: &[&str] = &[
    "union select",
    "drop table",
    "insert into",
    "delete from",
    "xp_cmdshell",
    "'; ",
    "' or '",
    "\" or \"",
    "or 1=1",
    "/*",
    "*/",
];

/// Remove `<script>` blocks and any remaining tags, then control characters
/// other than tab/newline/carriage return, then trim.
pub fn sanitize_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let lower = input.to_lowercase();
    let mut rest = input;
    let mut lower_rest = lower.as_str();

    // Drop whole <script>...</script> blocks first so their contents go too.
    while let Some(start) = lower_rest.find("<script") {
        out.push_str(&rest[..start]);
        match lower_rest[start..].find("</script>") {
            Some(end) => {
                let cut = start + end + "</script>".len();
                rest = &rest[cut..];
                lower_rest = &lower_rest[cut..];
            }
            None => {
                rest = "";
                lower_rest = "";
            }
        }
    }
    out.push_str(rest);

    // Strip remaining tags.
    let mut stripped = String::with_capacity(out.len());
    let mut in_tag = false;
    for c in out.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if in_tag => {}
            _ => stripped.push(c),
        }
    }

    stripped
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
        .collect::<String>()
        .trim()
        .to_string()
}

fn looks_like_sql_injection(value: &str) -> bool {
    let lower = value.to_lowercase();
    SQL_INJECTION_PATTERNS.iter().any(|p| lower.contains(p))
}

fn sanitize_value(path: &str, value: &Value, errors: &mut Vec<FieldError>) -> Value {
    match value {
        Value::String(s) => {
            let clean = sanitize_string(s);
            if looks_like_sql_injection(&clean) {
                errors.push(FieldError::new(
                    path,
                    "suspicious_input",
                    "contains characters or keywords that are not allowed",
                ));
            }
            Value::String(clean)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(i, v)| sanitize_value(&format!("{path}[{i}]"), v, errors))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, v) in map {
                let child = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
                out.insert(key.clone(), sanitize_value(&child, v, errors));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Stage 3: business rules
// ---------------------------------------------------------------------------

/// A rule over the whole (sanitized) body.
pub type Rule = Box<dyn Fn(&Value) -> Vec<FieldError> + Send + Sync>;

/// Ready-made business rules.
pub mod rules {
    use super::*;

    fn text<'a>(body: &'a Value, field: &str) -> Option<&'a str> {
        body.get(field).and_then(Value::as_str)
    }

    /// Run `rule` only when `condition` holds (conditional validation).
    pub fn when(
        condition: impl Fn(&Value) -> bool + Send + Sync + 'static,
        rule: Rule,
    ) -> Rule {
        Box::new(move |body| if condition(body) { rule(body) } else { Vec::new() })
    }

    /// `field` is required whenever `other` equals `value`.
    pub fn required_if(field: &str, other: &str, value: &str) -> Rule {
        let (field, other, value) = (field.to_string(), other.to_string(), value.to_string());
        Box::new(move |body| {
            let triggered = text(body, &other) == Some(value.as_str());
            let present = body.get(&field).is_some_and(|v| !v.is_null());
            if triggered && !present {
                vec![FieldError::new(
                    &field,
                    "required",
                    &format!("is required when {other} is {value}"),
                )]
            } else {
                Vec::new()
            }
        })
    }

    /// At most one of `a` and `b` may be present.
    pub fn mutually_exclusive(a: &str, b: &str) -> Rule {
        let (a, b) = (a.to_string(), b.to_string());
        Box::new(move |body| {
            let has = |f: &str| body.get(f).is_some_and(|v| !v.is_null());
            if has(&a) && has(&b) {
                vec![FieldError::new(&b, "conflict", &format!("cannot be combined with {a}"))]
            } else {
                Vec::new()
            }
        })
    }

    /// Cross-field comparison: numeric `low` must be strictly less than `high`.
    pub fn less_than(low: &str, high: &str) -> Rule {
        let (low, high) = (low.to_string(), high.to_string());
        Box::new(move |body| {
            match (
                body.get(&low).and_then(Value::as_f64),
                body.get(&high).and_then(Value::as_f64),
            ) {
                (Some(l), Some(h)) if l >= h => vec![FieldError::new(
                    &low,
                    "must_be_less_than",
                    &format!("must be less than {high}"),
                )],
                _ => Vec::new(),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Stage 4: async checks
// ---------------------------------------------------------------------------

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A validator that needs I/O, such as "is this name already taken?".
pub trait AsyncValidator: Send + Sync {
    fn validate<'a>(&'a self, body: &'a Value) -> BoxFuture<'a, Vec<FieldError>>;
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

/// Stage 5: rewrites the validated body.
pub type Transform = Box<dyn Fn(&mut Value) + Send + Sync>;

/// Ready-made transformations.
pub mod transforms {
    use super::*;

    pub fn lowercase(field: &str) -> Transform {
        let field = field.to_string();
        Box::new(move |body| {
            if let Some(Value::String(s)) = body.get_mut(&field) {
                *s = s.to_lowercase();
            }
        })
    }

    pub fn default_value(field: &str, default: Value) -> Transform {
        let field = field.to_string();
        Box::new(move |body| {
            if let Some(map) = body.as_object_mut() {
                map.entry(field.clone()).or_insert_with(|| default.clone());
            }
        })
    }
}

#[derive(Default)]
pub struct Pipeline {
    schema: Schema,
    rules: Vec<Rule>,
    async_validators: Vec<Box<dyn AsyncValidator>>,
    transforms: Vec<Transform>,
}

impl Pipeline {
    pub fn new(schema: Schema) -> Self {
        Pipeline {
            schema,
            rules: Vec::new(),
            async_validators: Vec::new(),
            transforms: Vec::new(),
        }
    }

    pub fn rule(mut self, rule: Rule) -> Self {
        self.rules.push(rule);
        self
    }

    pub fn async_validator(mut self, validator: impl AsyncValidator + 'static) -> Self {
        self.async_validators.push(Box::new(validator));
        self
    }

    pub fn transform(mut self, transform: Transform) -> Self {
        self.transforms.push(transform);
        self
    }

    fn fail<T>(stage: Stage, errors: Vec<FieldError>) -> Result<T, ValidationFailure> {
        let failure = ValidationFailure { stage, errors };
        // Field names and codes only: values may be personal data.
        tracing::warn!(
            stage = ?failure.stage,
            fields = ?failure.errors.iter().map(|e| format!("{}:{}", e.field, e.code)).collect::<Vec<_>>(),
            "request validation failed"
        );
        Err(failure)
    }

    /// Stages 1-3 and 5, for validation that needs no I/O.
    pub fn run(&self, input: Value) -> Result<Value, ValidationFailure> {
        let sanitized = self.run_sync_stages(&input)?;
        Ok(self.apply_transforms(sanitized))
    }

    /// All stages, including async checks.
    pub async fn run_async(&self, input: Value) -> Result<Value, ValidationFailure> {
        let sanitized = self.run_sync_stages(&input)?;

        let mut errors = Vec::new();
        for validator in &self.async_validators {
            errors.extend(validator.validate(&sanitized).await);
        }
        if !errors.is_empty() {
            return Self::fail(Stage::AsyncChecks, errors);
        }
        Ok(self.apply_transforms(sanitized))
    }

    fn run_sync_stages(&self, input: &Value) -> Result<Value, ValidationFailure> {
        let schema_errors = self.schema.validate(input);
        if !schema_errors.is_empty() {
            return Self::fail(Stage::Schema, schema_errors);
        }

        let mut sanitize_errors = Vec::new();
        let sanitized = sanitize_value("", input, &mut sanitize_errors);
        if !sanitize_errors.is_empty() {
            return Self::fail(Stage::Sanitization, sanitize_errors);
        }

        let rule_errors: Vec<FieldError> =
            self.rules.iter().flat_map(|rule| rule(&sanitized)).collect();
        if !rule_errors.is_empty() {
            return Self::fail(Stage::BusinessRules, rule_errors);
        }
        Ok(sanitized)
    }

    fn apply_transforms(&self, mut value: Value) -> Value {
        for transform in &self.transforms {
            transform(&mut value);
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    /// Minimal executor: every future in these tests is immediately ready.
    fn block_on<F: Future>(future: F) -> F::Output {
        fn raw() -> RawWaker {
            fn clone(_: *const ()) -> RawWaker {
                raw()
            }
            fn noop(_: *const ()) {}
            static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
            RawWaker::new(std::ptr::null(), &VTABLE)
        }
        let waker = unsafe { Waker::from_raw(raw()) };
        let mut cx = Context::from_waker(&waker);
        let mut future = Box::pin(future);
        loop {
            if let Poll::Ready(out) = future.as_mut().poll(&mut cx) {
                return out;
            }
        }
    }

    fn vault_schema() -> Schema {
        Schema::new(vec![
            FieldRule::string("name").required().len(1, 64),
            FieldRule::string("kind").one_of(&["basic", "managed"]),
            FieldRule::integer("fee_bps").range(0.0, 1000.0),
            FieldRule::number("floor"),
            FieldRule::number("ceiling"),
            FieldRule::string("manager"),
            FieldRule::string("manager_group"),
            FieldRule::string("email"),
            FieldRule::array("tags").len(0, 2),
        ])
    }

    fn fields(failure: &ValidationFailure) -> Vec<(String, String)> {
        failure.errors.iter().map(|e| (e.field.clone(), e.code.clone())).collect()
    }

    #[test]
    fn a_valid_body_passes_through() {
        let out = Pipeline::new(vault_schema()).run(json!({"name": "alpha", "fee_bps": 25})).unwrap();
        assert_eq!(out["name"], "alpha");
    }

    #[test]
    fn schema_errors_are_field_level_with_codes_and_all_reported_together() {
        let err = Pipeline::new(vault_schema())
            .run(json!({"kind": "weird", "fee_bps": 5000, "tags": ["a", "b", "c"], "extra": 1}))
            .unwrap_err();

        assert_eq!(err.stage, Stage::Schema);
        assert_eq!(err.status(), 400);
        let got = fields(&err);
        for expected in [
            ("name", "required"),
            ("kind", "invalid_choice"),
            ("fee_bps", "out_of_range"),
            ("tags", "too_long"),
            ("extra", "unknown_field"),
        ] {
            assert!(got.contains(&(expected.0.to_string(), expected.1.to_string())), "{expected:?} in {got:?}");
        }
    }

    #[test]
    fn wrong_types_and_non_objects_are_rejected() {
        let err = Pipeline::new(vault_schema()).run(json!({"name": 5})).unwrap_err();
        assert_eq!(fields(&err), vec![("name".to_string(), "invalid_type".to_string())]);

        let err = Pipeline::new(vault_schema()).run(json!(["not", "an", "object"])).unwrap_err();
        assert_eq!(err.errors[0].code, "invalid_type");
    }

    #[test]
    fn markup_is_stripped_from_every_string_including_nested_ones() {
        let schema = Schema::new(vec![FieldRule::string("name"), FieldRule::object("meta")]).allow_unknown();
        let out = Pipeline::new(schema)
            .run(json!({
                "name": "<b>Alpha</b><script>alert(1)</script>\u{0007}",
                "meta": {"note": ["<img src=x onerror=1>hello"]}
            }))
            .unwrap();
        assert_eq!(out["name"], "Alpha");
        assert_eq!(out["meta"]["note"][0], "hello");
    }

    #[test]
    fn newlines_and_tabs_survive_sanitization() {
        assert_eq!(sanitize_string("  a\nb\tc\r\n "), "a\nb\tc");
    }

    #[test]
    fn an_unterminated_script_block_is_dropped() {
        assert_eq!(sanitize_string("hi<script>alert(1)"), "hi");
    }

    #[test]
    fn sql_injection_payloads_are_rejected_not_silently_cleaned() {
        for payload in ["x'; DROP TABLE vaults;--", "a UNION SELECT password", "' OR '1'='1", "1 or 1=1"] {
            let err = Pipeline::new(vault_schema()).run(json!({"name": payload})).unwrap_err();
            assert_eq!(err.stage, Stage::Sanitization, "{payload}");
            assert_eq!(err.errors[0].code, "suspicious_input");
            assert_eq!(err.errors[0].field, "name");
        }
    }

    #[test]
    fn stages_stop_at_the_first_failure() {
        // Fails the schema, so the injection string is never even looked at.
        let err = Pipeline::new(vault_schema()).run(json!({"fee_bps": 99999, "manager": "' OR '1'='1"})).unwrap_err();
        assert_eq!(err.stage, Stage::Schema);
    }

    #[test]
    fn conditional_and_cross_field_rules() {
        let pipeline = Pipeline::new(vault_schema())
            .rule(rules::required_if("manager", "kind", "managed"))
            .rule(rules::mutually_exclusive("manager", "manager_group"))
            .rule(rules::less_than("floor", "ceiling"))
            .rule(rules::when(|b| b["kind"] == "basic", Box::new(|_| vec![FieldError::new("kind", "unsupported", "basic is disabled")])));

        let err = pipeline.run(json!({"name": "a", "kind": "managed"})).unwrap_err();
        assert_eq!(err.stage, Stage::BusinessRules);
        assert_eq!(fields(&err), vec![("manager".to_string(), "required".to_string())]);

        let err = pipeline
            .run(json!({"name": "a", "manager": "m", "manager_group": "g", "floor": 5, "ceiling": 5}))
            .unwrap_err();
        let got = fields(&err);
        assert!(got.contains(&("manager_group".to_string(), "conflict".to_string())));
        assert!(got.contains(&("floor".to_string(), "must_be_less_than".to_string())));

        let err = pipeline.run(json!({"name": "a", "kind": "basic"})).unwrap_err();
        assert_eq!(err.errors[0].code, "unsupported");

        assert!(pipeline.run(json!({"name": "a", "kind": "managed", "manager": "m", "floor": 1, "ceiling": 2})).is_ok());
    }

    struct UniqueName(Vec<&'static str>);

    impl AsyncValidator for UniqueName {
        fn validate<'a>(&'a self, body: &'a Value) -> BoxFuture<'a, Vec<FieldError>> {
            Box::pin(async move {
                let taken = body["name"].as_str().is_some_and(|n| self.0.contains(&n));
                if taken {
                    vec![FieldError::new("name", "not_unique", "is already taken")]
                } else {
                    Vec::new()
                }
            })
        }
    }

    #[test]
    fn async_validators_run_after_the_synchronous_stages() {
        let pipeline = Pipeline::new(vault_schema()).async_validator(UniqueName(vec!["taken"]));

        let err = block_on(pipeline.run_async(json!({"name": "taken"}))).unwrap_err();
        assert_eq!(err.stage, Stage::AsyncChecks);
        assert_eq!(err.errors[0].code, "not_unique");

        assert!(block_on(pipeline.run_async(json!({"name": "free"}))).is_ok());

        // A schema failure never reaches the (potentially expensive) async check.
        let err = block_on(pipeline.run_async(json!({}))).unwrap_err();
        assert_eq!(err.stage, Stage::Schema);
    }

    #[test]
    fn transformations_apply_to_the_validated_value() {
        let pipeline = Pipeline::new(vault_schema())
            .transform(transforms::lowercase("email"))
            .transform(transforms::default_value("kind", json!("basic")));

        let out = pipeline.run(json!({"name": "a", "email": "A@Example.COM"})).unwrap();

        assert_eq!(out["email"], "a@example.com");
        assert_eq!(out["kind"], "basic");
    }

    #[test]
    fn the_error_body_is_stable_and_never_echoes_input_values() {
        let err = Pipeline::new(vault_schema()).run(json!({"name": "x'; DROP TABLE y;--"})).unwrap_err();
        let body = err.to_json();

        assert_eq!(body["error"]["code"], "validation_failed");
        assert_eq!(body["error"]["stage"], "sanitization");
        assert_eq!(body["error"]["fields"][0]["field"], "name");
        assert!(!body.to_string().contains("DROP TABLE"));
    }
}
