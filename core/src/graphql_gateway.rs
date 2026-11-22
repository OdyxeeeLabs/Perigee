//! # GraphQL API gateway
//!
//! The protective layer in front of the federated subgraphs (vaults, users,
//! transactions, analytics). Each request goes through, in order:
//!
//! 1. **Authentication** – a [`Context`] built from verified JWT claims.
//! 2. **Persisted queries** – in allowlist mode only pre-registered query hashes
//!    run; clients may send just the hash ([`PersistedQueries`]).
//! 3. **Parsing** – a small GraphQL document parser ([`parse`]).
//! 4. **Complexity limits** – maximum depth and a cost estimate that multiplies
//!    list fields by their `first`/`limit` argument ([`analyze`]).
//! 5. **Per-operation rate limiting** ([`OperationRateLimiter`]).
//! 6. **Routing** – root fields are assigned to the subgraph that owns them
//!    ([`SubgraphRegistry`]) and handed to an [`Executor`].
//!
//! Batches (a JSON array of requests) are supported up to a maximum size, and
//! [`DataLoader`] gives resolvers per-request batching and caching to avoid N+1
//! lookups. Schema versions are tracked as tags ([`SchemaTags`]). See
//! `docs/GRAPHQL_GATEWAY.md`.

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::Hash;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use thiserror::Error;

pub const DEFAULT_MAX_DEPTH: usize = 10;
pub const DEFAULT_MAX_COST: u64 = 1_000;
pub const DEFAULT_MAX_BATCH: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GatewayError {
    #[error("syntax error: {0}")]
    Parse(String),
    #[error("a query or a persisted-query hash is required")]
    MissingQuery,
    #[error("persisted query not found")]
    PersistedQueryNotFound,
    #[error("this query is not on the allowlist")]
    PersistedQueryNotAllowed,
    #[error("the query does not match its persisted-query hash")]
    PersistedQueryHashMismatch,
    #[error("query depth {depth} exceeds the maximum of {max}")]
    DepthLimitExceeded { depth: usize, max: usize },
    #[error("query cost {cost} exceeds the maximum of {max}")]
    CostLimitExceeded { cost: u64, max: u64 },
    #[error("unknown fragment `{0}`")]
    UnknownFragment(String),
    #[error("fragment `{0}` spreads into itself")]
    FragmentCycle(String),
    #[error("operation `{0}` was not found in the document")]
    UnknownOperation(String),
    #[error("an operation name is required when the document has several")]
    AmbiguousOperation,
    #[error("no subgraph owns the field `{0}`")]
    UnknownField(String),
    #[error("too many operations in one batch (max {max})")]
    BatchTooLarge { max: usize },
    #[error("rate limit exceeded; retry in {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },
    #[error("authentication required")]
    Unauthenticated,
}

impl GatewayError {
    /// Stable code placed in `errors[].extensions.code`.
    pub fn code(&self) -> &'static str {
        match self {
            GatewayError::Parse(_) => "GRAPHQL_PARSE_FAILED",
            GatewayError::MissingQuery | GatewayError::AmbiguousOperation | GatewayError::UnknownOperation(_) => {
                "BAD_REQUEST"
            }
            GatewayError::PersistedQueryNotFound => "PERSISTED_QUERY_NOT_FOUND",
            GatewayError::PersistedQueryNotAllowed | GatewayError::PersistedQueryHashMismatch => {
                "PERSISTED_QUERY_NOT_ALLOWED"
            }
            GatewayError::DepthLimitExceeded { .. } => "QUERY_TOO_DEEP",
            GatewayError::CostLimitExceeded { .. } => "QUERY_TOO_COMPLEX",
            GatewayError::UnknownFragment(_) | GatewayError::FragmentCycle(_) => "GRAPHQL_VALIDATION_FAILED",
            GatewayError::UnknownField(_) => "GRAPHQL_VALIDATION_FAILED",
            GatewayError::BatchTooLarge { .. } => "BAD_USER_INPUT",
            GatewayError::RateLimited { .. } => "RATE_LIMITED",
            GatewayError::Unauthenticated => "UNAUTHENTICATED",
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "errors": [ { "message": self.to_string(), "extensions": { "code": self.code() } } ] })
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Punct(char),
    Spread,
    Name(String),
    Int(i64),
    Float,
    Str,
}

fn lex(source: &str) -> Result<Vec<Token>, GatewayError> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() || c == ',' || c == '\u{feff}' => i += 1,
            '#' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '.' => {
                if chars.get(i..i + 3) == Some(&['.', '.', '.']) {
                    tokens.push(Token::Spread);
                    i += 3;
                } else {
                    return Err(GatewayError::Parse(format!("unexpected `.` at {i}")));
                }
            }
            '"' => {
                if chars.get(i..i + 3) == Some(&['"', '"', '"']) {
                    i += 3;
                    loop {
                        if i + 2 >= chars.len() {
                            return Err(GatewayError::Parse("unterminated block string".into()));
                        }
                        if chars[i..i + 3] == ['"', '"', '"'] {
                            i += 3;
                            break;
                        }
                        i += if chars[i] == '\\' { 2 } else { 1 };
                    }
                } else {
                    i += 1;
                    loop {
                        match chars.get(i) {
                            None | Some('\n') => return Err(GatewayError::Parse("unterminated string".into())),
                            Some('\\') => i += 2,
                            Some('"') => {
                                i += 1;
                                break;
                            }
                            Some(_) => i += 1,
                        }
                    }
                }
                tokens.push(Token::Str);
            }
            c if c.is_ascii_digit() || c == '-' => {
                let start = i;
                i += 1;
                let mut float = false;
                while i < chars.len() && (chars[i].is_ascii_digit() || matches!(chars[i], '.' | 'e' | 'E' | '+' | '-')) {
                    float |= !chars[i].is_ascii_digit();
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                match (float, text.parse::<i64>()) {
                    (false, Ok(n)) => tokens.push(Token::Int(n)),
                    _ => tokens.push(Token::Float),
                }
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                tokens.push(Token::Name(chars[start..i].iter().collect()));
            }
            '{' | '}' | '(' | ')' | '[' | ']' | ':' | '!' | '=' | '@' | '$' | '|' | '&' => {
                tokens.push(Token::Punct(c));
                i += 1;
            }
            other => return Err(GatewayError::Parse(format!("unexpected character `{other}`"))),
        }
    }
    Ok(tokens)
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArgValue {
    Int(i64),
    Variable(String),
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub args: Vec<(String, ArgValue)>,
    pub selections: Vec<Selection>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    Field(Field),
    Inline(Vec<Selection>),
    Spread(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Query,
    Mutation,
    Subscription,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    pub kind: OperationKind,
    pub name: Option<String>,
    pub selections: Vec<Selection>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    pub operations: Vec<Operation>,
    pub fragments: HashMap<String, Vec<Selection>>,
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        token
    }

    fn at_punct(&self, c: char) -> bool {
        self.peek() == Some(&Token::Punct(c))
    }

    fn eat_punct(&mut self, c: char) -> bool {
        let found = self.at_punct(c);
        if found {
            self.pos += 1;
        }
        found
    }

    fn expect_punct(&mut self, c: char) -> Result<(), GatewayError> {
        if self.eat_punct(c) {
            Ok(())
        } else {
            Err(GatewayError::Parse(format!("expected `{c}`, found {:?}", self.peek())))
        }
    }

    fn name(&mut self) -> Result<String, GatewayError> {
        match self.bump() {
            Some(Token::Name(n)) => Ok(n),
            other => Err(GatewayError::Parse(format!("expected a name, found {other:?}"))),
        }
    }

    /// Skip one value: scalar, `$variable`, list, or object.
    fn value(&mut self) -> Result<ArgValue, GatewayError> {
        match self.bump() {
            Some(Token::Int(n)) => Ok(ArgValue::Int(n)),
            Some(Token::Float) | Some(Token::Str) | Some(Token::Name(_)) => Ok(ArgValue::Other),
            Some(Token::Punct('$')) => Ok(ArgValue::Variable(self.name()?)),
            Some(Token::Punct('[')) => {
                while !self.at_punct(']') {
                    if self.peek().is_none() {
                        return Err(GatewayError::Parse("unterminated list".into()));
                    }
                    self.value()?;
                }
                self.pos += 1;
                Ok(ArgValue::Other)
            }
            Some(Token::Punct('{')) => {
                while !self.at_punct('}') {
                    if self.peek().is_none() {
                        return Err(GatewayError::Parse("unterminated object".into()));
                    }
                    self.name()?;
                    self.expect_punct(':')?;
                    self.value()?;
                }
                self.pos += 1;
                Ok(ArgValue::Other)
            }
            other => Err(GatewayError::Parse(format!("unexpected value token {other:?}"))),
        }
    }

    fn arguments(&mut self) -> Result<Vec<(String, ArgValue)>, GatewayError> {
        let mut args = Vec::new();
        if self.eat_punct('(') {
            while !self.at_punct(')') {
                let name = self.name()?;
                self.expect_punct(':')?;
                args.push((name, self.value()?));
            }
            self.pos += 1;
        }
        Ok(args)
    }

    fn directives(&mut self) -> Result<(), GatewayError> {
        while self.eat_punct('@') {
            self.name()?;
            self.arguments()?;
        }
        Ok(())
    }

    /// Skip `($a: Int = 1, $b: [T!]!)`.
    fn variable_definitions(&mut self) -> Result<(), GatewayError> {
        if self.eat_punct('(') {
            let mut depth = 1;
            while depth > 0 {
                match self.bump() {
                    Some(Token::Punct('(')) => depth += 1,
                    Some(Token::Punct(')')) => depth -= 1,
                    Some(_) => {}
                    None => return Err(GatewayError::Parse("unterminated variable definitions".into())),
                }
            }
        }
        Ok(())
    }

    fn selection_set(&mut self) -> Result<Vec<Selection>, GatewayError> {
        self.expect_punct('{')?;
        let mut selections = Vec::new();
        while !self.at_punct('}') {
            if self.peek().is_none() {
                return Err(GatewayError::Parse("unterminated selection set".into()));
            }
            selections.push(self.selection()?);
        }
        self.pos += 1;
        if selections.is_empty() {
            return Err(GatewayError::Parse("empty selection set".into()));
        }
        Ok(selections)
    }

    fn selection(&mut self) -> Result<Selection, GatewayError> {
        if self.peek() == Some(&Token::Spread) {
            self.pos += 1;
            return match self.peek() {
                Some(Token::Name(n)) if n == "on" => {
                    self.pos += 1;
                    self.name()?;
                    self.directives()?;
                    Ok(Selection::Inline(self.selection_set()?))
                }
                Some(Token::Name(_)) => {
                    let name = self.name()?;
                    self.directives()?;
                    Ok(Selection::Spread(name))
                }
                _ => {
                    self.directives()?;
                    Ok(Selection::Inline(self.selection_set()?))
                }
            };
        }

        let mut name = self.name()?;
        if self.eat_punct(':') {
            name = self.name()?; // the alias is irrelevant to depth and cost
        }
        let args = self.arguments()?;
        self.directives()?;
        let selections = if self.at_punct('{') { self.selection_set()? } else { Vec::new() };
        Ok(Selection::Field(Field { name, args, selections }))
    }

    fn document(&mut self) -> Result<Document, GatewayError> {
        let mut doc = Document::default();
        while let Some(token) = self.peek().cloned() {
            match token {
                Token::Punct('{') => doc.operations.push(Operation {
                    kind: OperationKind::Query,
                    name: None,
                    selections: self.selection_set()?,
                }),
                Token::Name(keyword) if matches!(keyword.as_str(), "query" | "mutation" | "subscription") => {
                    self.pos += 1;
                    let kind = match keyword.as_str() {
                        "mutation" => OperationKind::Mutation,
                        "subscription" => OperationKind::Subscription,
                        _ => OperationKind::Query,
                    };
                    let name = match self.peek() {
                        Some(Token::Name(_)) => Some(self.name()?),
                        _ => None,
                    };
                    self.variable_definitions()?;
                    self.directives()?;
                    doc.operations.push(Operation { kind, name, selections: self.selection_set()? });
                }
                Token::Name(keyword) if keyword == "fragment" => {
                    self.pos += 1;
                    let name = self.name()?;
                    if self.name()? != "on" {
                        return Err(GatewayError::Parse("expected `on` in fragment definition".into()));
                    }
                    self.name()?;
                    self.directives()?;
                    doc.fragments.insert(name, self.selection_set()?);
                }
                other => return Err(GatewayError::Parse(format!("unexpected token {other:?}"))),
            }
        }
        if doc.operations.is_empty() {
            return Err(GatewayError::Parse("document contains no operation".into()));
        }
        Ok(doc)
    }
}

/// Parse a GraphQL executable document (queries, mutations, fragments).
pub fn parse(source: &str) -> Result<Document, GatewayError> {
    Parser { tokens: lex(source)?, pos: 0 }.document()
}

impl Document {
    fn operation(&self, name: Option<&str>) -> Result<&Operation, GatewayError> {
        match name {
            Some(wanted) => self
                .operations
                .iter()
                .find(|op| op.name.as_deref() == Some(wanted))
                .ok_or_else(|| GatewayError::UnknownOperation(wanted.to_string())),
            None if self.operations.len() == 1 => Ok(&self.operations[0]),
            None => Err(GatewayError::AmbiguousOperation),
        }
    }
}

// ---------------------------------------------------------------------------
// Complexity analysis
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_depth: usize,
    pub max_cost: u64,
    pub field_costs: HashMap<String, u64>,
    /// Applied when a list field has no `first`/`limit`/`last` argument.
    pub default_list_multiplier: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_depth: DEFAULT_MAX_DEPTH,
            max_cost: DEFAULT_MAX_COST,
            field_costs: HashMap::new(),
            default_list_multiplier: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    pub kind: OperationKind,
    pub name: Option<String>,
    pub depth: usize,
    pub cost: u64,
    /// Root fields after expanding fragments, in document order.
    pub root_fields: Vec<String>,
}

const PAGE_ARGS: [&str; 4] = ["first", "last", "limit", "take"];

struct Walker<'a> {
    doc: &'a Document,
    limits: &'a Limits,
    variables: &'a Value,
    stack: Vec<String>,
}

impl Walker<'_> {
    fn multiplier(&self, field: &Field) -> u64 {
        for (name, value) in &field.args {
            if !PAGE_ARGS.contains(&name.as_str()) {
                continue;
            }
            let n = match value {
                ArgValue::Int(n) => Some(*n),
                ArgValue::Variable(var) => self.variables.get(var).and_then(Value::as_i64),
                ArgValue::Other => None,
            };
            if let Some(n) = n {
                return n.clamp(1, 10_000) as u64;
            }
        }
        self.limits.default_list_multiplier.max(1)
    }

    /// `(depth, cost)` of a selection set whose fields sit at `level`.
    fn walk(&mut self, selections: &[Selection], level: usize, roots: &mut Option<&mut Vec<String>>) -> Result<(usize, u64), GatewayError> {
        let mut depth = level.saturating_sub(1);
        let mut cost = 0u64;
        for selection in selections {
            match selection {
                Selection::Field(field) => {
                    if let Some(roots) = roots.as_deref_mut() {
                        roots.push(field.name.clone());
                    }
                    let (child_depth, child_cost) = self.walk(&field.selections, level + 1, &mut None)?;
                    depth = depth.max(level).max(child_depth);
                    let own = if field.name == "__typename" {
                        0
                    } else {
                        self.limits.field_costs.get(&field.name).copied().unwrap_or(1)
                    };
                    cost = cost.saturating_add(own.saturating_add(self.multiplier(field).saturating_mul(child_cost)));
                }
                Selection::Inline(inner) => {
                    let (d, c) = self.walk(inner, level, roots)?;
                    depth = depth.max(d);
                    cost = cost.saturating_add(c);
                }
                Selection::Spread(name) => {
                    if self.stack.contains(name) {
                        return Err(GatewayError::FragmentCycle(name.clone()));
                    }
                    let fragment = self
                        .doc
                        .fragments
                        .get(name)
                        .ok_or_else(|| GatewayError::UnknownFragment(name.clone()))?;
                    self.stack.push(name.clone());
                    let (d, c) = self.walk(fragment, level, roots)?;
                    self.stack.pop();
                    depth = depth.max(d);
                    cost = cost.saturating_add(c);
                }
            }
        }
        Ok((depth, cost))
    }
}

/// Measure an operation and enforce `limits`.
pub fn analyze(doc: &Document, operation_name: Option<&str>, variables: &Value, limits: &Limits) -> Result<Analysis, GatewayError> {
    let operation = doc.operation(operation_name)?;
    let mut walker = Walker { doc, limits, variables, stack: Vec::new() };
    let mut root_fields = Vec::new();
    let (depth, cost) = walker.walk(&operation.selections, 1, &mut Some(&mut root_fields))?;

    if depth > limits.max_depth {
        return Err(GatewayError::DepthLimitExceeded { depth, max: limits.max_depth });
    }
    if cost > limits.max_cost {
        return Err(GatewayError::CostLimitExceeded { cost, max: limits.max_cost });
    }
    Ok(Analysis { kind: operation.kind, name: operation.name.clone(), depth, cost, root_fields })
}

// ---------------------------------------------------------------------------
// Persisted queries, requests, batching
// ---------------------------------------------------------------------------

pub fn hash_query(query: &str) -> String {
    hex::encode(Sha256::digest(query.as_bytes()))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraphqlRequest {
    pub query: Option<String>,
    pub operation_name: Option<String>,
    pub variables: Value,
    /// `extensions.persistedQuery.sha256Hash`
    pub persisted_hash: Option<String>,
}

impl GraphqlRequest {
    fn from_json(value: &Value) -> GraphqlRequest {
        GraphqlRequest {
            query: value.get("query").and_then(Value::as_str).map(str::to_string),
            operation_name: value.get("operationName").and_then(Value::as_str).map(str::to_string),
            variables: value.get("variables").cloned().unwrap_or_else(|| json!({})),
            persisted_hash: value
                .pointer("/extensions/persistedQuery/sha256Hash")
                .and_then(Value::as_str)
                .map(str::to_string),
        }
    }
}

/// Parse a request body: one object, or an array (a batch). The flag says
/// whether the response must also be an array.
pub fn parse_requests(body: &Value, max_batch: usize) -> Result<(Vec<GraphqlRequest>, bool), GatewayError> {
    match body {
        Value::Array(items) => {
            if items.is_empty() {
                return Err(GatewayError::MissingQuery);
            }
            if items.len() > max_batch {
                return Err(GatewayError::BatchTooLarge { max: max_batch });
            }
            Ok((items.iter().map(GraphqlRequest::from_json).collect(), true))
        }
        other => Ok((vec![GraphqlRequest::from_json(other)], false)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistedMode {
    /// Production: only registered queries run.
    AllowlistOnly,
    /// Development: any query runs; hashes still resolve.
    AllowAny,
}

pub struct PersistedQueries {
    queries: HashMap<String, String>,
    mode: PersistedMode,
}

impl PersistedQueries {
    pub fn new(mode: PersistedMode) -> Self {
        PersistedQueries { queries: HashMap::new(), mode }
    }

    /// Add a query to the allowlist (a build step); returns its hash.
    pub fn register(&mut self, query: &str) -> String {
        let hash = hash_query(query);
        self.queries.insert(hash.clone(), query.to_string());
        hash
    }

    /// The query text to run for `request`.
    pub fn resolve(&self, request: &GraphqlRequest) -> Result<String, GatewayError> {
        match (&request.persisted_hash, &request.query) {
            (Some(hash), Some(query)) => {
                if hash_query(query) != *hash {
                    Err(GatewayError::PersistedQueryHashMismatch)
                } else {
                    self.admit(hash, query)
                }
            }
            (Some(hash), None) => self.queries.get(hash).cloned().ok_or(GatewayError::PersistedQueryNotFound),
            (None, Some(query)) => self.admit(&hash_query(query), query),
            (None, None) => Err(GatewayError::MissingQuery),
        }
    }

    fn admit(&self, hash: &str, query: &str) -> Result<String, GatewayError> {
        if self.mode == PersistedMode::AllowAny || self.queries.contains_key(hash) {
            Ok(query.to_string())
        } else {
            Err(GatewayError::PersistedQueryNotAllowed)
        }
    }
}

// ---------------------------------------------------------------------------
// Context, rate limiting, routing
// ---------------------------------------------------------------------------

/// Who is calling. Build it from claims that the auth layer already verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub user_id: String,
    pub roles: Vec<String>,
}

impl Context {
    pub fn from_claims(claims: &Value) -> Result<Context, GatewayError> {
        let user_id = claims
            .get("sub")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or(GatewayError::Unauthenticated)?
            .to_string();
        let roles = match claims.get("roles") {
            Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            _ => claims.get("role").and_then(Value::as_str).map(|r| vec![r.to_string()]).unwrap_or_default(),
        };
        Ok(Context { user_id, roles })
    }
}

/// Fixed-window limits per `(user, operation)`.
pub struct OperationRateLimiter {
    default: (u32, u64),
    overrides: HashMap<String, (u32, u64)>,
    windows: Mutex<HashMap<(String, String), (u64, u32)>>,
}

impl OperationRateLimiter {
    /// `default` is `(requests, per_secs)`.
    pub fn new(default: (u32, u64)) -> Self {
        OperationRateLimiter { default, overrides: HashMap::new(), windows: Mutex::new(HashMap::new()) }
    }

    pub fn limit_operation(mut self, operation: &str, requests: u32, per_secs: u64) -> Self {
        self.overrides.insert(operation.to_string(), (requests, per_secs));
        self
    }

    pub fn check(&self, user_id: &str, operation: &str, now: u64) -> Result<(), GatewayError> {
        let (max, per_secs) = self.overrides.get(operation).copied().unwrap_or(self.default);
        let mut windows = self.windows.lock().map_err(|_| GatewayError::RateLimited { retry_after_secs: 1 })?;
        let slot = windows.entry((user_id.to_string(), operation.to_string())).or_insert((now, 0));
        if now.saturating_sub(slot.0) >= per_secs {
            *slot = (now, 0);
        }
        if slot.1 >= max {
            return Err(GatewayError::RateLimited { retry_after_secs: (slot.0 + per_secs).saturating_sub(now).max(1) });
        }
        slot.1 += 1;
        Ok(())
    }
}

/// Which subgraph owns which root field.
#[derive(Default)]
pub struct SubgraphRegistry {
    owners: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubgraphCall {
    pub subgraph: String,
    pub fields: Vec<String>,
}

impl SubgraphRegistry {
    pub fn own(mut self, subgraph: &str, fields: &[&str]) -> Self {
        for field in fields {
            self.owners.insert(field.to_string(), subgraph.to_string());
        }
        self
    }

    /// The four Perigee subgraphs.
    pub fn standard() -> Self {
        SubgraphRegistry::default()
            .own("vaults", &["vault", "vaults", "createVault", "updateVault", "deleteVault"])
            .own("users", &["me", "user", "users", "manager", "managers"])
            .own("transactions", &["transaction", "transactions", "submitTransaction"])
            .own("analytics", &["analytics", "feeHistory", "feeAnalytics", "marketConditions"])
    }

    /// Group root fields by owning subgraph. `__typename` and introspection stay at the gateway.
    pub fn plan(&self, root_fields: &[String]) -> Result<Vec<SubgraphCall>, GatewayError> {
        let mut calls: Vec<SubgraphCall> = Vec::new();
        for field in root_fields {
            if field.starts_with("__") {
                continue;
            }
            let subgraph = self.owners.get(field).ok_or_else(|| GatewayError::UnknownField(field.clone()))?;
            match calls.iter_mut().find(|c| &c.subgraph == subgraph) {
                Some(call) => call.fields.push(field.clone()),
                None => calls.push(SubgraphCall { subgraph: subgraph.clone(), fields: vec![field.clone()] }),
            }
        }
        Ok(calls)
    }
}

/// Schema versions by tag (`production`, `staging`, `v2-preview`).
#[derive(Default)]
pub struct SchemaTags {
    tags: BTreeMap<String, String>,
}

impl SchemaTags {
    /// Point `tag` at `sdl`; returns the schema hash.
    pub fn publish(&mut self, tag: &str, sdl: &str) -> String {
        let hash = hash_query(sdl);
        self.tags.insert(tag.to_string(), hash.clone());
        hash
    }

    pub fn hash_for(&self, tag: &str) -> Option<&str> {
        self.tags.get(tag).map(String::as_str)
    }
}

// ---------------------------------------------------------------------------
// DataLoader
// ---------------------------------------------------------------------------

/// Per-request batching and caching for resolvers. All keys missing from the
/// cache are fetched with a single call to `batch_fn`.
pub struct DataLoader<K, V, F>
where
    K: Eq + Hash + Clone,
    V: Clone,
    F: Fn(&[K]) -> HashMap<K, V>,
{
    batch_fn: F,
    cache: Mutex<HashMap<K, Option<V>>>,
    batches: AtomicUsize,
}

impl<K, V, F> DataLoader<K, V, F>
where
    K: Eq + Hash + Clone,
    V: Clone,
    F: Fn(&[K]) -> HashMap<K, V>,
{
    pub fn new(batch_fn: F) -> Self {
        DataLoader { batch_fn, cache: Mutex::new(HashMap::new()), batches: AtomicUsize::new(0) }
    }

    /// Values for `keys` (missing ones are omitted). Duplicates and cached keys
    /// are not fetched again.
    pub fn load_many(&self, keys: &[K]) -> HashMap<K, V> {
        let mut cache = match self.cache.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut seen = HashSet::new();
        let missing: Vec<K> = keys
            .iter()
            .filter(|k| !cache.contains_key(*k) && seen.insert((*k).clone()))
            .cloned()
            .collect();

        if !missing.is_empty() {
            self.batches.fetch_add(1, Ordering::SeqCst);
            let mut fetched = (self.batch_fn)(&missing);
            for key in missing {
                let value = fetched.remove(&key);
                cache.insert(key, value);
            }
        }

        keys.iter()
            .filter_map(|k| cache.get(k).and_then(|v| v.clone()).map(|v| (k.clone(), v)))
            .collect()
    }

    pub fn load(&self, key: &K) -> Option<V> {
        self.load_many(std::slice::from_ref(key)).remove(key)
    }

    /// Seed the cache (for example with rows already in hand).
    pub fn prime(&self, key: K, value: V) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key, Some(value));
        }
    }

    /// How many times `batch_fn` was called.
    pub fn batch_count(&self) -> usize {
        self.batches.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// Gateway
// ---------------------------------------------------------------------------

/// Runs an admitted operation against the subgraphs.
pub trait Executor: Send + Sync {
    fn execute(&self, calls: &[SubgraphCall], query: &str, request: &GraphqlRequest, context: &Context) -> Value;
}

pub struct Gateway<E: Executor> {
    pub limits: Limits,
    pub persisted: PersistedQueries,
    pub rate_limiter: OperationRateLimiter,
    pub subgraphs: SubgraphRegistry,
    pub max_batch: usize,
    pub executor: E,
}

impl<E: Executor> Gateway<E> {
    /// Handle one HTTP body (single request or batch) for an authenticated caller.
    pub fn handle(&self, body: &Value, context: &Context, now: u64) -> Value {
        match parse_requests(body, self.max_batch) {
            Err(error) => error.to_json(),
            Ok((requests, is_batch)) => {
                let results: Vec<Value> = requests
                    .iter()
                    .map(|request| self.handle_one(request, context, now).unwrap_or_else(|e| e.to_json()))
                    .collect();
                if is_batch {
                    Value::Array(results)
                } else {
                    results.into_iter().next().unwrap_or(Value::Null)
                }
            }
        }
    }

    fn handle_one(&self, request: &GraphqlRequest, context: &Context, now: u64) -> Result<Value, GatewayError> {
        let query = self.persisted.resolve(request)?;
        let document = parse(&query)?;
        let analysis = analyze(&document, request.operation_name.as_deref(), &request.variables, &self.limits)?;

        let operation = analysis.name.clone().unwrap_or_else(|| "anonymous".to_string());
        self.rate_limiter.check(&context.user_id, &operation, now)?;

        let calls = self.subgraphs.plan(&analysis.root_fields)?;
        Ok(self.executor.execute(&calls, &query, request, context))
    }
}

// ---------------------------------------------------------------------------
// Vault event subscriptions (bounded)
// ---------------------------------------------------------------------------

pub const DEFAULT_MAX_SUBSCRIPTIONS: usize = 100;
pub const DEFAULT_MAX_SUBSCRIPTIONS_PER_USER: usize = 5;
/// Events buffered per subscription before it is dropped as a slow consumer.
pub const DEFAULT_SUBSCRIPTION_BUFFER: usize = 64;
/// Recent events kept so a reconnecting client can resume from its cursor.
pub const DEFAULT_REPLAY_EVENTS: usize = 256;

/// A committed vault event. `seq` is the cursor: strictly increasing, and a
/// client resumes with the last `seq` it applied (exclusive), so replays never
/// deliver an event twice.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VaultEvent {
    pub seq: u64,
    pub tenant_id: String,
    pub vault_id: String,
    pub kind: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SubscriptionError {
    #[error("too many active subscriptions")]
    TooManySubscriptions,
    #[error("the cursor is ahead of the event stream")]
    InvalidCursor,
    #[error("the cursor is older than the retained events; resync and subscribe again")]
    CursorExpired,
    #[error("subscriber fell behind its buffer of {0} events; reconnect with the last cursor")]
    Overflowed(usize),
    #[error("unknown subscription")]
    UnknownSubscription,
}

impl SubscriptionError {
    pub fn code(&self) -> &'static str {
        match self {
            SubscriptionError::TooManySubscriptions | SubscriptionError::Overflowed(_) => "RATE_LIMITED",
            SubscriptionError::InvalidCursor | SubscriptionError::CursorExpired => "BAD_USER_INPUT",
            SubscriptionError::UnknownSubscription => "BAD_REQUEST",
        }
    }
}

struct Subscriber {
    user_id: String,
    tenant_id: String,
    queue: std::collections::VecDeque<VaultEvent>,
    overflowed: bool,
}

#[derive(Default)]
struct HubState {
    last_seq: u64,
    next_id: u64,
    log: std::collections::VecDeque<VaultEvent>,
    subscribers: HashMap<u64, Subscriber>,
}

/// Fan-out of vault events with hard limits on subscriptions and buffering.
/// A subscriber only ever receives events of the tenant it subscribed for.
pub struct SubscriptionHub {
    max_total: usize,
    max_per_user: usize,
    buffer: usize,
    replay: usize,
    state: Mutex<HubState>,
}

impl Default for SubscriptionHub {
    fn default() -> Self {
        SubscriptionHub::new(
            DEFAULT_MAX_SUBSCRIPTIONS,
            DEFAULT_MAX_SUBSCRIPTIONS_PER_USER,
            DEFAULT_SUBSCRIPTION_BUFFER,
            DEFAULT_REPLAY_EVENTS,
        )
    }
}

impl SubscriptionHub {
    pub fn new(max_total: usize, max_per_user: usize, buffer: usize, replay: usize) -> Self {
        SubscriptionHub { max_total, max_per_user, buffer, replay, state: Mutex::new(HubState::default()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HubState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Open a subscription for `tenant_id`, which the caller must take from the
    /// verified claims. `after` is the last `seq` the client applied.
    pub fn subscribe(&self, context: &Context, tenant_id: &str, after: Option<u64>) -> Result<u64, SubscriptionError> {
        let mut state = self.lock();
        let per_user = state.subscribers.values().filter(|s| s.user_id == context.user_id).count();
        if state.subscribers.len() >= self.max_total || per_user >= self.max_per_user {
            return Err(SubscriptionError::TooManySubscriptions);
        }
        let mut queue = std::collections::VecDeque::new();
        if let Some(cursor) = after {
            if cursor > state.last_seq {
                return Err(SubscriptionError::InvalidCursor);
            }
            let oldest = state.log.front().map(|e| e.seq).unwrap_or(state.last_seq + 1);
            if cursor + 1 < oldest {
                return Err(SubscriptionError::CursorExpired);
            }
            queue.extend(state.log.iter().filter(|e| e.seq > cursor && e.tenant_id == tenant_id).cloned());
            if queue.len() > self.buffer {
                return Err(SubscriptionError::CursorExpired);
            }
        }
        state.next_id += 1;
        let id = state.next_id;
        state.subscribers.insert(
            id,
            Subscriber { user_id: context.user_id.clone(), tenant_id: tenant_id.to_string(), queue, overflowed: false },
        );
        Ok(id)
    }

    /// Publish an event. Call this only after the change it describes has been
    /// committed, so no subscriber sees state that could still roll back.
    pub fn publish(&self, tenant_id: &str, vault_id: &str, kind: &str, payload: Value) -> u64 {
        let mut state = self.lock();
        state.last_seq += 1;
        let event = VaultEvent {
            seq: state.last_seq,
            tenant_id: tenant_id.to_string(),
            vault_id: vault_id.to_string(),
            kind: kind.to_string(),
            payload,
        };
        state.log.push_back(event.clone());
        while state.log.len() > self.replay {
            state.log.pop_front();
        }
        for subscriber in state.subscribers.values_mut().filter(|s| s.tenant_id == tenant_id) {
            if subscriber.queue.len() >= self.buffer {
                subscriber.overflowed = true;
            } else {
                subscriber.queue.push_back(event.clone());
            }
        }
        event.seq
    }

    /// Take the buffered events. A subscriber that overflowed is closed; it must
    /// reconnect with the last `seq` it applied.
    pub fn poll(&self, id: u64) -> Result<Vec<VaultEvent>, SubscriptionError> {
        let mut state = self.lock();
        let subscriber = state.subscribers.get_mut(&id).ok_or(SubscriptionError::UnknownSubscription)?;
        if subscriber.overflowed {
            state.subscribers.remove(&id);
            return Err(SubscriptionError::Overflowed(self.buffer));
        }
        Ok(subscriber.queue.drain(..).collect())
    }

    pub fn unsubscribe(&self, id: u64) {
        self.lock().subscribers.remove(&id);
    }
}

#[cfg(test)]
mod subscription_tests {
    use super::*;

    fn user(id: &str) -> Context {
        Context { user_id: id.to_string(), roles: vec![] }
    }

    #[test]
    fn delivers_only_to_the_subscribed_tenant() {
        let hub = SubscriptionHub::default();
        let mine = hub.subscribe(&user("a"), "t1", None).unwrap();
        let theirs = hub.subscribe(&user("b"), "t2", None).unwrap();
        hub.publish("t1", "v1", "updated", json!({}));
        assert_eq!(hub.poll(mine).unwrap().len(), 1);
        assert!(hub.poll(theirs).unwrap().is_empty());
    }

    #[test]
    fn limits_subscriptions_and_drops_slow_consumers() {
        let hub = SubscriptionHub::new(10, 1, 2, 8);
        let id = hub.subscribe(&user("a"), "t1", None).unwrap();
        assert_eq!(hub.subscribe(&user("a"), "t1", None), Err(SubscriptionError::TooManySubscriptions));
        for _ in 0..3 {
            hub.publish("t1", "v1", "updated", json!({}));
        }
        assert_eq!(hub.poll(id), Err(SubscriptionError::Overflowed(2)));
        assert_eq!(hub.poll(id), Err(SubscriptionError::UnknownSubscription));
    }

    #[test]
    fn resuming_from_a_cursor_does_not_repeat_events() {
        let hub = SubscriptionHub::default();
        let first = hub.publish("t1", "v1", "created", json!({}));
        hub.publish("t1", "v1", "updated", json!({}));
        let id = hub.subscribe(&user("a"), "t1", Some(first)).unwrap();
        let events = hub.poll(id).unwrap();
        assert_eq!(events.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![2]);
        assert_eq!(hub.subscribe(&user("b"), "t1", Some(99)), Err(SubscriptionError::InvalidCursor));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Value {
        json!({})
    }

    fn measure(query: &str) -> Result<Analysis, GatewayError> {
        analyze(&parse(query)?, None, &vars(), &Limits::default())
    }

    #[test]
    fn parses_queries_with_aliases_arguments_variables_directives_and_comments() {
        let doc = parse(
            r#"
            # fetch vaults
            query Vaults($first: Int = 5, $tags: [String!]) @cached(ttl: 60) {
              mine: vaults(first: $first, filter: {status: "active", ids: [1, 2]}) @include(if: true) {
                id
                owner { name, email }
                ... on Vault { name }
                ...Extra
              }
            }
            fragment Extra on Vault { createdAt note(text: "multi \" quote") }
            "#,
        )
        .unwrap();
        assert_eq!(doc.operations.len(), 1);
        assert_eq!(doc.operations[0].name.as_deref(), Some("Vaults"));
        assert!(doc.fragments.contains_key("Extra"));
    }

    #[test]
    fn rejects_malformed_documents() {
        for bad in ["", "{", "query {", "{ a { } }", "{ a(b: ) }", "fragment F { x }", "{ a } }", "query Q(", "{ ..", "{ a(b: \"x) }"] {
            assert!(matches!(parse(bad), Err(GatewayError::Parse(_))), "{bad:?}");
        }
    }

    #[test]
    fn depth_counts_nested_selection_levels() {
        assert_eq!(measure("{ a }").unwrap().depth, 1);
        assert_eq!(measure("{ a { b { c } } }").unwrap().depth, 3);
        assert_eq!(measure("{ a { ... on T { b { c } } } }").unwrap().depth, 3);
    }

    #[test]
    fn deep_queries_are_rejected() {
        let mut query = "x".to_string();
        for _ in 0..12 {
            query = format!("a {{ {query} }}");
        }
        let err = measure(&format!("{{ {query} }}")).unwrap_err();
        assert!(matches!(err, GatewayError::DepthLimitExceeded { max: 10, .. }));
        assert_eq!(err.code(), "QUERY_TOO_DEEP");
    }

    #[test]
    fn list_arguments_multiply_the_cost_of_their_children() {
        assert_eq!(measure("{ vault { id name } }").unwrap().cost, 3);
        // 1 (vaults) + 50 * (id + name) = 101
        assert_eq!(measure("{ vaults(first: 50) { id name } }").unwrap().cost, 101);
        // nested lists compound: 1 + 10 * (1 + 20 * 1)
        assert_eq!(measure("{ a(limit: 10) { b(first: 20) { c } } }").unwrap().cost, 211);
        assert_eq!(measure("{ __typename }").unwrap().cost, 0);
    }

    #[test]
    fn variables_supply_page_sizes_and_unbounded_queries_are_rejected() {
        let doc = parse("query Q($n: Int) { vaults(first: $n) { id owner { id } } }").unwrap();
        let cheap = analyze(&doc, None, &json!({"n": 5}), &Limits::default()).unwrap();
        assert_eq!(cheap.cost, 1 + 5 * (1 + 1 + 1));

        let err = analyze(&doc, None, &json!({"n": 5000}), &Limits::default()).unwrap_err();
        assert!(matches!(err, GatewayError::CostLimitExceeded { max: 1000, .. }));
        assert_eq!(err.code(), "QUERY_TOO_COMPLEX");
    }

    #[test]
    fn aliases_cannot_hide_repeated_work() {
        let cost = measure("{ a: vaults(first: 100) { id } b: vaults(first: 100) { id } c: vaults(first: 100) { id } }").unwrap().cost;
        assert_eq!(cost, 3 * (1 + 100));
    }

    #[test]
    fn fragments_are_expanded_and_cycles_detected() {
        let ok = measure("{ a { ...F } } fragment F on T { b { c } }").unwrap();
        assert_eq!(ok.depth, 3);

        assert_eq!(measure("{ a { ...Missing } }"), Err(GatewayError::UnknownFragment("Missing".into())));
        assert_eq!(
            measure("{ a { ...A } } fragment A on T { b { ...B } } fragment B on T { c { ...A } }"),
            Err(GatewayError::FragmentCycle("A".into()))
        );
    }

    #[test]
    fn custom_field_costs_apply() {
        let mut limits = Limits::default();
        limits.field_costs.insert("analytics".into(), 50);
        let doc = parse("{ analytics { total } }").unwrap();
        assert_eq!(analyze(&doc, None, &vars(), &limits).unwrap().cost, 51);
    }

    #[test]
    fn operation_selection() {
        let doc = parse("query A { a } query B { b }").unwrap();
        assert_eq!(analyze(&doc, None, &vars(), &Limits::default()), Err(GatewayError::AmbiguousOperation));
        assert_eq!(analyze(&doc, Some("B"), &vars(), &Limits::default()).unwrap().root_fields, vec!["b"]);
        assert_eq!(analyze(&doc, Some("C"), &vars(), &Limits::default()), Err(GatewayError::UnknownOperation("C".into())));
        let mutation = parse("mutation Make { createVault { id } }").unwrap();
        assert_eq!(analyze(&mutation, None, &vars(), &Limits::default()).unwrap().kind, OperationKind::Mutation);
    }

    #[test]
    fn persisted_queries_enforce_the_allowlist() {
        let mut store = PersistedQueries::new(PersistedMode::AllowlistOnly);
        let query = "{ vaults(first: 1) { id } }";
        let hash = store.register(query);

        let by_hash = GraphqlRequest { persisted_hash: Some(hash.clone()), ..Default::default() };
        assert_eq!(store.resolve(&by_hash).unwrap(), query);

        let by_text = GraphqlRequest { query: Some(query.into()), ..Default::default() };
        assert_eq!(store.resolve(&by_text).unwrap(), query);

        let arbitrary = GraphqlRequest { query: Some("{ users { email } }".into()), ..Default::default() };
        assert_eq!(store.resolve(&arbitrary), Err(GatewayError::PersistedQueryNotAllowed));

        let unknown = GraphqlRequest { persisted_hash: Some("00".repeat(32)), ..Default::default() };
        assert_eq!(store.resolve(&unknown), Err(GatewayError::PersistedQueryNotFound));

        let smuggled = GraphqlRequest { persisted_hash: Some(hash), query: Some("{ users { email } }".into()), ..Default::default() };
        assert_eq!(store.resolve(&smuggled), Err(GatewayError::PersistedQueryHashMismatch));

        assert_eq!(store.resolve(&GraphqlRequest::default()), Err(GatewayError::MissingQuery));
        assert!(PersistedQueries::new(PersistedMode::AllowAny).resolve(&arbitrary).is_ok());
    }

    #[test]
    fn batches_are_parsed_and_capped() {
        let single = json!({"query": "{ a }", "operationName": "A", "variables": {"x": 1}});
        let (requests, is_batch) = parse_requests(&single, 10).unwrap();
        assert!(!is_batch);
        assert_eq!(requests[0].operation_name.as_deref(), Some("A"));
        assert_eq!(requests[0].variables, json!({"x": 1}));

        let batch = json!([{"query": "{ a }"}, {"extensions": {"persistedQuery": {"sha256Hash": "abc"}}}]);
        let (requests, is_batch) = parse_requests(&batch, 10).unwrap();
        assert!(is_batch);
        assert_eq!(requests[1].persisted_hash.as_deref(), Some("abc"));

        let many = Value::Array(vec![json!({"query": "{ a }"}); 11]);
        assert_eq!(parse_requests(&many, 10), Err(GatewayError::BatchTooLarge { max: 10 }));
        assert_eq!(parse_requests(&json!([]), 10), Err(GatewayError::MissingQuery));
    }

    #[test]
    fn contexts_come_from_verified_claims() {
        let ctx = Context::from_claims(&json!({"sub": "G123", "roles": ["manager", "admin"]})).unwrap();
        assert_eq!((ctx.user_id.as_str(), ctx.roles.len()), ("G123", 2));
        assert_eq!(Context::from_claims(&json!({"sub": "G1", "role": "manager"})).unwrap().roles, vec!["manager"]);
        assert_eq!(Context::from_claims(&json!({})), Err(GatewayError::Unauthenticated));
        assert_eq!(Context::from_claims(&json!({"sub": ""})), Err(GatewayError::Unauthenticated));
    }

    #[test]
    fn rate_limits_apply_per_user_and_operation() {
        let limiter = OperationRateLimiter::new((3, 60)).limit_operation("CreateVault", 1, 60);

        assert!(limiter.check("u1", "CreateVault", 0).is_ok());
        let err = limiter.check("u1", "CreateVault", 10).unwrap_err();
        assert_eq!(err, GatewayError::RateLimited { retry_after_secs: 50 });
        assert_eq!(err.code(), "RATE_LIMITED");

        assert!(limiter.check("u2", "CreateVault", 10).is_ok(), "other users are unaffected");
        for _ in 0..3 {
            assert!(limiter.check("u1", "ListVaults", 10).is_ok());
        }
        assert!(limiter.check("u1", "ListVaults", 11).is_err());
        assert!(limiter.check("u1", "ListVaults", 70).is_ok(), "window reset");
    }

    #[test]
    fn root_fields_are_routed_to_their_subgraphs() {
        let registry = SubgraphRegistry::standard();
        let fields: Vec<String> = ["vaults", "me", "__typename", "feeHistory", "vault"].iter().map(|s| s.to_string()).collect();
        let plan = registry.plan(&fields).unwrap();

        assert_eq!(
            plan,
            vec![
                SubgraphCall { subgraph: "vaults".into(), fields: vec!["vaults".into(), "vault".into()] },
                SubgraphCall { subgraph: "users".into(), fields: vec!["me".into()] },
                SubgraphCall { subgraph: "analytics".into(), fields: vec!["feeHistory".into()] },
            ]
        );
        assert_eq!(registry.plan(&["secrets".to_string()]), Err(GatewayError::UnknownField("secrets".into())));
    }

    #[test]
    fn the_dataloader_batches_and_caches() {
        let calls = std::sync::Arc::new(Mutex::new(Vec::<Vec<u32>>::new()));
        let seen = calls.clone();
        let loader = DataLoader::new(move |keys: &[u32]| {
            seen.lock().unwrap().push(keys.to_vec());
            keys.iter().filter(|k| **k != 99).map(|k| (*k, format!("user-{k}"))).collect()
        });

        // N+1 shape: the same owner appears many times, plus one missing id.
        let owners = [1, 2, 1, 3, 2, 99, 1];
        let loaded = loader.load_many(&owners);

        assert_eq!(loader.batch_count(), 1, "one round trip for every key");
        assert_eq!(calls.lock().unwrap()[0], vec![1, 2, 3, 99], "deduplicated, order kept");
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[&1], "user-1");
        assert!(!loaded.contains_key(&99));

        assert_eq!(loader.load(&2), Some("user-2".to_string()), "cache hit");
        assert_eq!(loader.load(&99), None, "misses are cached too");
        assert_eq!(loader.batch_count(), 1);

        loader.prime(7, "primed".into());
        assert_eq!(loader.load(&7), Some("primed".to_string()));
        assert_eq!(loader.load(&8), Some("user-8".to_string()));
        assert_eq!(loader.batch_count(), 2);
    }

    #[test]
    fn schema_tags_track_versions() {
        let mut tags = SchemaTags::default();
        let a = tags.publish("production", "type Query { a: Int }");
        let b = tags.publish("staging", "type Query { a: Int b: Int }");
        assert_ne!(a, b);
        assert_eq!(tags.hash_for("production"), Some(a.as_str()));
        assert_eq!(tags.hash_for("nope"), None);
        tags.publish("production", "type Query { a: Int b: Int }");
        assert_eq!(tags.hash_for("production"), Some(b.as_str()));
    }

    struct EchoExecutor;

    impl Executor for EchoExecutor {
        fn execute(&self, calls: &[SubgraphCall], _query: &str, _request: &GraphqlRequest, context: &Context) -> Value {
            json!({"data": {"subgraphs": calls.iter().map(|c| c.subgraph.clone()).collect::<Vec<_>>(), "user": context.user_id}})
        }
    }

    fn gateway(mode: PersistedMode) -> Gateway<EchoExecutor> {
        Gateway {
            limits: Limits::default(),
            persisted: PersistedQueries::new(mode),
            rate_limiter: OperationRateLimiter::new((2, 60)),
            subgraphs: SubgraphRegistry::standard(),
            max_batch: DEFAULT_MAX_BATCH,
            executor: EchoExecutor,
        }
    }

    fn ctx() -> Context {
        Context { user_id: "u1".into(), roles: vec![] }
    }

    #[test]
    fn the_gateway_runs_the_whole_pipeline() {
        let gw = gateway(PersistedMode::AllowAny);
        let out = gw.handle(&json!({"query": "query List { vaults(first: 5) { id } me { id } }"}), &ctx(), 0);
        assert_eq!(out["data"]["subgraphs"], json!(["vaults", "users"]));
        assert_eq!(out["data"]["user"], "u1");

        let too_complex = gw.handle(&json!({"query": "{ vaults(first: 9999) { id } }"}), &ctx(), 0);
        assert_eq!(too_complex["errors"][0]["extensions"]["code"], "QUERY_TOO_COMPLEX");

        let unknown = gw.handle(&json!({"query": "{ nothing }"}), &ctx(), 0);
        assert_eq!(unknown["errors"][0]["extensions"]["code"], "GRAPHQL_VALIDATION_FAILED");

        let syntax = gw.handle(&json!({"query": "{ oops"}), &ctx(), 0);
        assert_eq!(syntax["errors"][0]["extensions"]["code"], "GRAPHQL_PARSE_FAILED");
    }

    #[test]
    fn batches_return_one_result_per_operation_and_failures_do_not_poison_the_rest() {
        let gw = gateway(PersistedMode::AllowAny);
        let out = gw.handle(&json!([{"query": "{ me { id } }"}, {"query": "{ nope"}, {"query": "{ vaults { id } }"}]), &ctx(), 0);

        let results = out.as_array().unwrap();
        assert_eq!(results.len(), 3);
        assert!(results[0]["data"].is_object());
        assert!(results[1]["errors"].is_array());
        assert!(results[2]["data"].is_object());

        let oversized = gw.handle(&Value::Array(vec![json!({"query": "{ me { id } }"}); 11]), &ctx(), 0);
        assert_eq!(oversized["errors"][0]["extensions"]["code"], "BAD_USER_INPUT");
    }

    #[test]
    fn allowlist_mode_blocks_arbitrary_queries_but_serves_persisted_ones() {
        let mut gw = gateway(PersistedMode::AllowlistOnly);
        let hash = gw.persisted.register("query Mine { me { id } }");

        let ok = gw.handle(&json!({"extensions": {"persistedQuery": {"sha256Hash": hash}}}), &ctx(), 0);
        assert_eq!(ok["data"]["subgraphs"], json!(["users"]));

        let blocked = gw.handle(&json!({"query": "{ me { id } }"}), &ctx(), 0);
        assert_eq!(blocked["errors"][0]["extensions"]["code"], "PERSISTED_QUERY_NOT_ALLOWED");
    }

    #[test]
    fn the_gateway_rate_limits_per_operation_name() {
        let gw = gateway(PersistedMode::AllowAny);
        let query = json!({"query": "query List { me { id } }"});
        assert!(gw.handle(&query, &ctx(), 0)["data"].is_object());
        assert!(gw.handle(&query, &ctx(), 1)["data"].is_object());
        assert_eq!(gw.handle(&query, &ctx(), 2)["errors"][0]["extensions"]["code"], "RATE_LIMITED");
        // A differently named operation has its own budget.
        assert!(gw.handle(&json!({"query": "query Other { me { id } }"}), &ctx(), 2)["data"].is_object());
    }
}
