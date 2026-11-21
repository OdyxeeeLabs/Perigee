use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
enum TypeRef {
    Named(String),
    List(Box<TypeRef>),
    NonNull(Box<TypeRef>),
}

impl fmt::Display for TypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(name) => write!(f, "{name}"),
            Self::List(inner) => write!(f, "[{inner}]"),
            Self::NonNull(inner) => write!(f, "{inner}!"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefinitionKind {
    Scalar,
    Object,
    Interface,
    InputObject,
    Enum,
    Union,
}

impl DefinitionKind {
    fn label(self) -> &'static str {
        match self {
            Self::Scalar => "scalar",
            Self::Object => "type",
            Self::Interface => "interface",
            Self::InputObject => "input",
            Self::Enum => "enum",
            Self::Union => "union",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InputValue {
    ty: TypeRef,
    default_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct FieldDefinition {
    ty: Option<TypeRef>,
    args: BTreeMap<String, InputValue>,
    default_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct TypeDefinition {
    kind: Option<DefinitionKind>,
    fields: BTreeMap<String, FieldDefinition>,
    enum_values: BTreeSet<String>,
    union_members: BTreeSet<String>,
    implements: BTreeSet<String>,
}

impl TypeDefinition {
    fn merge(&mut self, other: Self) -> Result<(), String> {
        if let Some(kind) = other.kind {
            match self.kind {
                Some(existing) if existing != kind => {
                    return Err(format!(
                        "conflicting kinds {} and {} for the same type",
                        existing.label(),
                        kind.label()
                    ));
                }
                None => self.kind = Some(kind),
                _ => {}
            }
        }
        for (name, field) in other.fields {
            if let Some(existing) = self.fields.insert(name.clone(), field.clone()) {
                if existing != field {
                    return Err(format!("conflicting definitions for field `{name}`"));
                }
            }
        }
        self.enum_values.extend(other.enum_values);
        self.union_members.extend(other.union_members);
        self.implements.extend(other.implements);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectiveDefinition {
    args: BTreeMap<String, InputValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct SchemaDefinition {
    operations: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Schema {
    definitions: BTreeMap<String, TypeDefinition>,
    directives: BTreeMap<String, DirectiveDefinition>,
    schema: Option<SchemaDefinition>,
}

/// A single schema change that would break an existing client operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakingChange {
    pub kind: &'static str,
    pub path: String,
    pub baseline: String,
    pub proposed: String,
    pub impact: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Name(String),
    String(String),
    Number(String),
    Punct(char),
    Eof,
}

fn starts_with(chars: &[char], at: usize, needle: &[char]) -> bool {
    at + needle.len() <= chars.len() && chars[at..at + needle.len()] == *needle
}

fn lex(input: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if ch.is_whitespace() || ch == ',' || ch == '\u{feff}' {
            i += 1;
            continue;
        }
        if ch == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if ch == '"' {
            if starts_with(&chars, i, &['"', '"', '"']) {
                i += 3;
                let mut value = String::new();
                let mut closed = false;
                while i < chars.len() {
                    if starts_with(&chars, i, &['\\', '"', '"', '"']) {
                        value.push_str("\"\"\"");
                        i += 4;
                    } else if starts_with(&chars, i, &['"', '"', '"']) {
                        i += 3;
                        closed = true;
                        break;
                    } else {
                        value.push(chars[i]);
                        i += 1;
                    }
                }
                if !closed {
                    return Err("unterminated block string".to_string());
                }
                tokens.push(Token::String(value));
                continue;
            }
            i += 1;
            let mut value = String::new();
            let mut closed = false;
            while i < chars.len() {
                let current = chars[i];
                if current == '"' {
                    i += 1;
                    closed = true;
                    break;
                }
                if current == '\\' {
                    i += 1;
                    let escaped = *chars.get(i).ok_or("unterminated string escape")?;
                    match escaped {
                        'n' => value.push('\n'),
                        'r' => value.push('\r'),
                        't' => value.push('\t'),
                        other => value.push(other),
                    }
                    i += 1;
                } else {
                    value.push(current);
                    i += 1;
                }
            }
            if !closed {
                return Err("unterminated string".to_string());
            }
            tokens.push(Token::String(value));
            continue;
        }
        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Name(chars[start..i].iter().collect()));
            continue;
        }
        if ch.is_ascii_digit()
            || (ch == '-' && chars.get(i + 1).is_some_and(|next| next.is_ascii_digit()))
        {
            let start = i;
            i += 1;
            while i < chars.len() {
                let current = chars[i];
                let exponent_jump = matches!(current, 'e' | 'E')
                    && matches!(chars.get(i + 1), Some('+') | Some('-'));
                if current.is_ascii_digit()
                    || current == '.'
                    || matches!(current, 'e' | 'E')
                    || exponent_jump
                {
                    i += 1 + usize::from(exponent_jump);
                } else {
                    break;
                }
            }
            tokens.push(Token::Number(chars[start..i].iter().collect()));
            continue;
        }
        tokens.push(Token::Punct(ch));
        i += 1;
    }

    tokens.push(Token::Eof);
    Ok(tokens)
}

fn describe(token: &Token) -> String {
    match token {
        Token::Name(name) => format!("name `{name}`"),
        Token::String(value) => format!("string \"{value}\""),
        Token::Number(value) => format!("number `{value}`"),
        Token::Punct(ch) => format!("`{ch}`"),
        Token::Eof => "end of file".to_string(),
    }
}

#[derive(Debug)]
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn token(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn at_eof(&self) -> bool {
        matches!(self.token(), Token::Eof)
    }

    fn at_punct(&self, ch: char) -> bool {
        matches!(self.token(), Token::Punct(actual) if *actual == ch)
    }

    fn advance(&mut self) -> Token {
        let token = self.token().clone();
        if !self.at_eof() {
            self.pos += 1;
        }
        token
    }

    fn consume_punct(&mut self, ch: char) -> bool {
        if self.at_punct(ch) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, ch: char) -> Result<(), String> {
        if self.consume_punct(ch) {
            Ok(())
        } else {
            Err(format!("expected `{ch}`, found {}", describe(self.token())))
        }
    }

    fn consume_name(&mut self, name: &str) -> bool {
        if matches!(self.token(), Token::Name(actual) if actual == name) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn peek_name(&self) -> Option<&str> {
        match self.token() {
            Token::Name(name) => Some(name),
            _ => None,
        }
    }

    fn expect_name(&mut self, context: &str) -> Result<String, String> {
        match self.advance() {
            Token::Name(name) => Ok(name),
            other => Err(format!("expected {context}, found {}", describe(&other))),
        }
    }

    fn skip_descriptions(&mut self) {
        while matches!(self.token(), Token::String(_)) {
            self.advance();
        }
    }

    fn skip_directives(&mut self) -> Result<(), String> {
        while self.consume_punct('@') {
            self.expect_name("directive name")?;
            if self.consume_punct('(') {
                let mut depth = 1usize;
                while depth > 0 {
                    match self.advance() {
                        Token::Eof => return Err("unterminated directive arguments".to_string()),
                        Token::Punct('(') => depth += 1,
                        Token::Punct(')') => depth -= 1,
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }

    fn parse_type_ref(&mut self) -> Result<TypeRef, String> {
        let base = if self.consume_punct('[') {
            let inner = self.parse_type_ref()?;
            self.expect_punct(']')?;
            TypeRef::List(Box::new(inner))
        } else {
            TypeRef::Named(self.expect_name("type name")?)
        };
        if self.consume_punct('!') {
            Ok(TypeRef::NonNull(Box::new(base)))
        } else {
            Ok(base)
        }
    }

    fn parse_value(&mut self) -> Result<String, String> {
        match self.advance() {
            Token::Name(name) => Ok(name),
            Token::Number(number) => Ok(number),
            Token::String(value) => Ok(format!(
                "\"{}\"",
                value.replace('\\', "\\\\").replace('"', "\\\"")
            )),
            Token::Punct('$') => Ok(format!("${}", self.expect_name("variable name")?)),
            Token::Punct('[') => {
                let mut values = Vec::new();
                while !self.consume_punct(']') {
                    if self.at_eof() {
                        return Err("unterminated list value".to_string());
                    }
                    values.push(self.parse_value()?);
                }
                Ok(format!("[{}]", values.join(", ")))
            }
            Token::Punct('{') => {
                let mut fields = Vec::new();
                while !self.consume_punct('}') {
                    if self.at_eof() {
                        return Err("unterminated object value".to_string());
                    }
                    let name = self.expect_name("object field name")?;
                    self.expect_punct(':')?;
                    fields.push(format!("{}: {}", name, self.parse_value()?));
                }
                Ok(format!("{{ {} }}", fields.join(", ")))
            }
            other => Err(format!("expected a value, found {}", describe(&other))),
        }
    }

    fn parse_input_values(&mut self) -> Result<BTreeMap<String, InputValue>, String> {
        self.expect_punct('(')?;
        let mut values = BTreeMap::new();
        while !self.consume_punct(')') {
            if self.at_eof() {
                return Err("unterminated argument list".to_string());
            }
            self.skip_descriptions();
            let name = self.expect_name("input value name")?;
            self.expect_punct(':')?;
            let ty = self.parse_type_ref()?;
            let default_value = if self.consume_punct('=') {
                Some(self.parse_value()?)
            } else {
                None
            };
            self.skip_directives()?;
            if values
                .insert(name.clone(), InputValue { ty, default_value })
                .is_some()
            {
                return Err(format!("duplicate input value `{name}`"));
            }
        }
        Ok(values)
    }

    fn parse_type_definition(
        &mut self,
        kind: DefinitionKind,
        extend: bool,
    ) -> Result<(String, TypeDefinition), String> {
        let name = self.expect_name("type name")?;
        let mut definition = TypeDefinition::default();
        if !extend {
            definition.kind = Some(kind);
        }

        if matches!(kind, DefinitionKind::Object | DefinitionKind::Interface)
            && self.consume_name("implements")
        {
            loop {
                let interface = self.expect_name("interface name")?;
                definition.implements.insert(interface);
                if !self.consume_punct('&') {
                    break;
                }
            }
        }
        self.skip_directives()?;

        match kind {
            DefinitionKind::Scalar => {}
            DefinitionKind::Object | DefinitionKind::Interface | DefinitionKind::InputObject => {
                self.expect_punct('{')?;
                while !self.consume_punct('}') {
                    if self.at_eof() {
                        return Err(format!("unterminated {} `{name}`", kind.label()));
                    }
                    self.skip_descriptions();
                    let field_name = self.expect_name("field name")?;
                    let args = if kind == DefinitionKind::InputObject || !self.at_punct('(') {
                        BTreeMap::new()
                    } else {
                        self.parse_input_values()?
                    };
                    self.expect_punct(':')?;
                    let ty = self.parse_type_ref()?;
                    let default_value =
                        if kind == DefinitionKind::InputObject && self.consume_punct('=') {
                            Some(self.parse_value()?)
                        } else {
                            None
                        };
                    self.skip_directives()?;
                    let field = FieldDefinition {
                        ty: Some(ty),
                        args,
                        default_value,
                    };
                    if definition
                        .fields
                        .insert(field_name.clone(), field)
                        .is_some()
                    {
                        return Err(format!("duplicate field `{field_name}` on `{name}`"));
                    }
                }
            }
            DefinitionKind::Enum => {
                self.expect_punct('{')?;
                while !self.consume_punct('}') {
                    if self.at_eof() {
                        return Err(format!("unterminated enum `{name}`"));
                    }
                    self.skip_descriptions();
                    let value = self.expect_name("enum value")?;
                    self.skip_directives()?;
                    if !definition.enum_values.insert(value) {
                        return Err(format!("duplicate enum value on `{name}`"));
                    }
                }
            }
            DefinitionKind::Union => {
                self.expect_punct('=')?;
                loop {
                    let member = self.expect_name("union member")?;
                    definition.union_members.insert(member);
                    if !self.consume_punct('|') {
                        break;
                    }
                }
                self.skip_directives()?;
            }
        }
        Ok((name, definition))
    }

    fn parse_schema_definition(&mut self) -> Result<SchemaDefinition, String> {
        self.skip_directives()?;
        self.expect_punct('{')?;
        let mut operations = BTreeMap::new();
        while !self.consume_punct('}') {
            if self.at_eof() {
                return Err("unterminated schema definition".to_string());
            }
            let operation = self.expect_name("operation type")?;
            self.expect_punct(':')?;
            let root = self.expect_name("operation root type")?;
            if operations.insert(operation.clone(), root).is_some() {
                return Err(format!("duplicate operation type `{operation}`"));
            }
        }
        Ok(SchemaDefinition { operations })
    }

    fn parse_directive_definition(&mut self) -> Result<(String, DirectiveDefinition), String> {
        self.expect_punct('@')?;
        let name = self.expect_name("directive name")?;
        let args = if self.at_punct('(') {
            self.parse_input_values()?
        } else {
            BTreeMap::new()
        };
        self.consume_name("repeatable");
        self.expect_name("on")?;
        loop {
            self.expect_name("directive location")?;
            if !self.consume_punct('|') {
                break;
            }
        }
        self.skip_directives()?;
        Ok((name, DirectiveDefinition { args }))
    }
}

fn parse_schema(input: &str) -> Result<Schema, String> {
    let mut parser = Parser::new(lex(input)?);
    let mut schema = Schema::default();

    while !parser.at_eof() {
        parser.skip_descriptions();
        if parser.at_eof() {
            break;
        }
        let extend = parser.consume_name("extend");
        let keyword = parser
            .peek_name()
            .ok_or_else(|| format!("expected a definition, found {}", describe(parser.token())))?
            .to_string();
        parser.advance();

        match keyword.as_str() {
            "schema" => {
                let definition = parser.parse_schema_definition()?;
                match (extend, schema.schema.as_mut()) {
                    (false, None) => schema.schema = Some(definition),
                    (true, Some(existing)) => existing.operations.extend(definition.operations),
                    (true, None) => {
                        return Err(
                            "cannot extend a schema definition that has not been declared"
                                .to_string(),
                        )
                    }
                    (false, Some(_)) => return Err("duplicate schema definition".to_string()),
                }
            }
            "directive" => {
                let (name, definition) = parser.parse_directive_definition()?;
                if schema.directives.insert(name.clone(), definition).is_some() {
                    return Err(format!("duplicate directive `@{name}`"));
                }
            }
            keyword => {
                let kind = match keyword {
                    "scalar" => DefinitionKind::Scalar,
                    "type" => DefinitionKind::Object,
                    "interface" => DefinitionKind::Interface,
                    "input" => DefinitionKind::InputObject,
                    "enum" => DefinitionKind::Enum,
                    "union" => DefinitionKind::Union,
                    other => return Err(format!("unsupported definition `{other}`")),
                };
                let (name, definition) = parser.parse_type_definition(kind, extend)?;
                if extend {
                    let existing = schema
                        .definitions
                        .get_mut(&name)
                        .ok_or_else(|| format!("cannot extend unknown type `{name}`"))?;
                    existing.merge(definition)?;
                } else if schema
                    .definitions
                    .insert(name.clone(), definition)
                    .is_some()
                {
                    return Err(format!("duplicate type `{name}`"));
                }
            }
        }
    }

    if schema.definitions.is_empty() {
        return Err("schema does not define any types".to_string());
    }
    Ok(schema)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variance {
    Input,
    Output,
}

fn strip_non_null(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::NonNull(inner) => inner,
        other => other,
    }
}

fn push_change(
    changes: &mut Vec<BreakingChange>,
    kind: &'static str,
    path: impl Into<String>,
    baseline: impl Into<String>,
    proposed: impl Into<String>,
    impact: &'static str,
) {
    changes.push(BreakingChange {
        kind,
        path: path.into(),
        baseline: baseline.into(),
        proposed: proposed.into(),
        impact,
    });
}

fn compare_type(
    base: &TypeRef,
    proposed: &TypeRef,
    variance: Variance,
    path: &str,
    changes: &mut Vec<BreakingChange>,
) {
    let base_required = matches!(base, TypeRef::NonNull(_));
    let proposed_required = matches!(proposed, TypeRef::NonNull(_));

    if strip_non_null(base) != strip_non_null(proposed) {
        push_change(
            changes,
            "type-changed",
            path,
            base.to_string(),
            proposed.to_string(),
            "Existing operations validate against the baseline type.",
        );
        return;
    }

    if !base_required && proposed_required {
        push_change(
            changes,
            "nullable-to-required",
            path,
            base.to_string(),
            proposed.to_string(),
            "Clients that omit this value or send null will break.",
        );
    } else if base_required && !proposed_required && variance == Variance::Output {
        push_change(
            changes,
            "required-to-nullable",
            path,
            base.to_string(),
            proposed.to_string(),
            "Clients may rely on this field always returning a value.",
        );
    }
}

fn compare_defaults(
    base: Option<&String>,
    proposed: Option<&String>,
    path: &str,
    changes: &mut Vec<BreakingChange>,
) {
    if base == proposed {
        return;
    }
    match proposed {
        None => push_change(
            changes,
            "default-removed",
            path,
            base.cloned().unwrap_or_else(|| "none".to_string()),
            "none",
            "Omitting this input now changes server behaviour.",
        ),
        Some(proposed_default) => {
            if base.is_some() {
                push_change(
                    changes,
                    "default-changed",
                    path,
                    base.cloned().unwrap_or_default(),
                    proposed_default.clone(),
                    "Operations that omit this input will observe a different value.",
                );
            }
        }
    }
}

fn compare_input_values(
    base: &BTreeMap<String, InputValue>,
    proposed: &BTreeMap<String, InputValue>,
    parent: &str,
    changes: &mut Vec<BreakingChange>,
) {
    for (name, base_value) in base {
        let path = format!("{parent}({name}:)");
        match proposed.get(name) {
            None => push_change(
                changes,
                "input-removed",
                path,
                base_value.ty.to_string(),
                "absent",
                "Existing operations that supply this input will fail validation.",
            ),
            Some(proposed_value) => {
                compare_type(
                    &base_value.ty,
                    &proposed_value.ty,
                    Variance::Input,
                    &path,
                    changes,
                );
                compare_defaults(
                    base_value.default_value.as_ref(),
                    proposed_value.default_value.as_ref(),
                    &path,
                    changes,
                );
            }
        }
    }
}

fn compare_definition(
    name: &str,
    base: &TypeDefinition,
    proposed: &TypeDefinition,
    changes: &mut Vec<BreakingChange>,
) {
    match (base.kind, proposed.kind) {
        (Some(base_kind), Some(proposed_kind)) if base_kind != proposed_kind => {
            push_change(
                changes,
                "type-kind-changed",
                name,
                base_kind.label(),
                proposed_kind.label(),
                "Queries valid for the baseline kind may no longer validate.",
            );
            return;
        }
        (Some(_), None) => {
            push_change(
                changes,
                "type-kind-changed",
                name,
                base.kind.map(DefinitionKind::label).unwrap_or("unknown"),
                "unknown",
                "The proposed type has no definition kind.",
            );
            return;
        }
        _ => {}
    }

    for interface in &base.implements {
        if !proposed.implements.contains(interface) {
            push_change(
                changes,
                "interface-removed",
                format!("{name} implements {interface}"),
                interface,
                "absent",
                "Fragments and variables depending on this interface will break.",
            );
        }
    }

    let field_variance = if base.kind == Some(DefinitionKind::InputObject) {
        Variance::Input
    } else {
        Variance::Output
    };
    for (field_name, base_field) in &base.fields {
        let path = format!("{name}.{field_name}");
        match proposed.fields.get(field_name) {
            None => {
                let baseline = base_field
                    .ty
                    .clone()
                    .map(|ty| ty.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                push_change(
                    changes,
                    "field-removed",
                    path,
                    baseline,
                    "absent",
                    "Existing queries selecting this field will fail validation.",
                );
            }
            Some(proposed_field) => {
                if let (Some(base_ty), Some(proposed_ty)) = (&base_field.ty, &proposed_field.ty) {
                    compare_type(base_ty, proposed_ty, field_variance, &path, changes);
                }
                if field_variance == Variance::Input {
                    compare_defaults(
                        base_field.default_value.as_ref(),
                        proposed_field.default_value.as_ref(),
                        &path,
                        changes,
                    );
                }
                compare_input_values(&base_field.args, &proposed_field.args, &path, changes);
            }
        }
    }

    for value in &base.enum_values {
        if !proposed.enum_values.contains(value) {
            push_change(
                changes,
                "enum-value-removed",
                format!("{name}.{value}"),
                value,
                "absent",
                "Clients that send or compare this literal will break.",
            );
        }
    }

    for member in &base.union_members {
        if !proposed.union_members.contains(member) {
            push_change(
                changes,
                "union-member-removed",
                format!("{name} = {member}"),
                member,
                "absent",
                "Fragments on this union member will fail validation.",
            );
        }
    }
}

impl Schema {
    fn breaking_changes(&self, proposed: &Self) -> Vec<BreakingChange> {
        let mut changes = Vec::new();

        if let Some(base_root) = &self.schema {
            match &proposed.schema {
                None => push_change(
                    &mut changes,
                    "root-operation-removed",
                    "schema",
                    base_root.operations.len().to_string(),
                    "absent",
                    "Client operations against these root types will fail validation.",
                ),
                Some(proposed_root) => {
                    for (operation, base_type) in &base_root.operations {
                        let path = format!("schema.{operation}");
                        match proposed_root.operations.get(operation) {
                            None => push_change(
                                &mut changes,
                                "root-operation-removed",
                                path,
                                base_type,
                                "absent",
                                "Client operations against this root type will fail validation.",
                            ),
                            Some(proposed_type) if proposed_type != base_type => push_change(
                                &mut changes,
                                "root-operation-changed",
                                path,
                                base_type,
                                proposed_type,
                                "Existing operations target the baseline root type.",
                            ),
                            Some(_) => {}
                        }
                    }
                }
            }
        }

        for (name, base_definition) in &self.definitions {
            match proposed.definitions.get(name) {
                None => push_change(
                    &mut changes,
                    "type-removed",
                    name,
                    base_definition
                        .kind
                        .map(DefinitionKind::label)
                        .unwrap_or("type"),
                    "absent",
                    "Clients that reference this type will fail validation.",
                ),
                Some(proposed_definition) => {
                    compare_definition(name, base_definition, proposed_definition, &mut changes)
                }
            }
        }

        for (name, base_directive) in &self.directives {
            if let Some(proposed_directive) = proposed.directives.get(name) {
                compare_input_values(
                    &base_directive.args,
                    &proposed_directive.args,
                    &format!("directive @{name}"),
                    &mut changes,
                );
            } else {
                push_change(
                    &mut changes,
                    "directive-removed",
                    format!("directive @{name}"),
                    "defined",
                    "absent",
                    "Existing documents using this directive will fail validation.",
                );
            }
        }

        changes
    }
}

/// Compare a proposed SDL document with the checked-in baseline.
///
/// `Ok` with an empty list means the proposed schema is additive or otherwise
/// compatible. `Ok` with entries means each entry is a client-breaking change.
pub fn compare_schemas(baseline: &str, proposed: &str) -> Result<Vec<BreakingChange>, String> {
    let baseline = parse_schema(baseline).map_err(|error| format!("baseline schema: {error}"))?;
    let proposed = parse_schema(proposed).map_err(|error| format!("proposed schema: {error}"))?;
    Ok(baseline.breaking_changes(&proposed))
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

/// Render diagnostics suitable for a CI log or GitHub step summary.
pub fn format_report(changes: &[BreakingChange]) -> String {
    if changes.is_empty() {
        return "GraphQL schema compatibility: no breaking changes detected.\n".to_string();
    }

    let mut report = format!(
        "GraphQL schema compatibility: {} breaking change(s) detected.\n\n\
         | Classification | Path | Baseline | Proposed | Why it breaks clients |\n\
         | --- | --- | --- | --- | --- |\n",
        changes.len()
    );
    for change in changes {
        report.push_str(&format!(
            "| {} | `{}` | `{}` | `{}` | {} |\n",
            change.kind,
            markdown_cell(&change.path),
            markdown_cell(&change.baseline),
            markdown_cell(&change.proposed),
            change.impact
        ));
    }
    report.push_str("\nAdditive changes pass automatically. Intentional breaking changes require an explicit, reviewed baseline update.\n");
    report
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{compare_schemas, format_report, parse_schema, DefinitionKind, TypeRef};

    const BASELINE: &str = r#"
        schema { query: Query mutation: Mutation }

        scalar DateTime

        "A persisted vault."
        type Vault implements Node {
          id: ID!
          name: String!
          owner: Manager!
          createdAt: DateTime
        }

        interface Node { id: ID! }

        type Manager implements Node {
          id: ID!
          email: String
          vaults(first: Int = 20, after: String): [Vault!]!
        }

        enum VaultStatus { ACTIVE PAUSED ARCHIVED }

        union SearchResult = Vault | Manager

        input VaultFilter {
          status: VaultStatus
          createdAfter: DateTime
        }

        type Query {
          vault(id: ID!): Vault
          vaults(filter: VaultFilter, first: Int = 20): [Vault!]!
          search(term: String!): [SearchResult!]!
        }

        type Mutation {
          createVault(name: String!, ownerId: ID!): Vault!
        }

        directive @auth(role: String = "user") on FIELD_DEFINITION
    "#;

    fn change_kinds(baseline: &str, proposed: &str) -> Vec<&'static str> {
        compare_schemas(baseline, proposed)
            .unwrap()
            .into_iter()
            .map(|change| change.kind)
            .collect()
    }

    #[test]
    fn parser_handles_descriptions_directives_and_complex_values() {
        let schema = parse_schema(
            r#"
            "Root description"
            type Demo @tag(name: "quoted, value") {
              items(filter: Filter = { status: ACTIVE, limit: 10 }, tags: [String!]! = ["a", "b"]): [String]!
              nested: [[Int!]!]
            }
            input Filter { status: Status = ACTIVE limit: Int = 10 }
            enum Status { ACTIVE }
            "#,
        )
        .unwrap();

        assert_eq!(
            schema.definitions.get("Demo").unwrap().kind,
            Some(DefinitionKind::Object)
        );
        let items = &schema.definitions.get("Demo").unwrap().fields["items"];
        let filter = &items.args["filter"];
        assert_eq!(
            filter.default_value.as_deref(),
            Some("{ status: ACTIVE, limit: 10 }")
        );
        assert_eq!(filter.ty.to_string(), "Filter");
        assert_eq!(items.ty.as_ref().unwrap().to_string(), "[String]!");
        assert_eq!(
            schema.definitions.get("Demo").unwrap().fields["nested"]
                .ty
                .as_ref()
                .unwrap()
                .to_string(),
            "[[Int!]!]"
        );
    }

    #[test]
    fn parser_merges_extensions() {
        let schema = parse_schema(&format!(
            "{BASELINE}\nextend type Manager {{ displayName: String }}\n"
        ))
        .unwrap();
        let manager = &schema.definitions["Manager"];
        assert!(manager.fields.contains_key("displayName"));
        assert_eq!(manager.fields.len(), 4);
    }

    #[test]
    fn rejects_malformed_schemas_with_context() {
        let error = parse_schema("type Broken { id: }").unwrap_err();
        assert!(error.contains("expected type name"), "{error}");
        assert!(compare_schemas("type A { id: ID }", "type B { id: }")
            .unwrap_err()
            .contains("proposed schema"));
    }

    #[test]
    fn additive_changes_pass() {
        let proposed = BASELINE.replace(
            "type Query {\n",
            "type Query {\n          newField: String\n",
        );
        assert!(compare_schemas(BASELINE, &proposed).unwrap().is_empty());
    }

    #[test]
    fn input_relaxation_passes() {
        let required_input = BASELINE.replace("after: String)", "after: String!)");
        let relaxed_input = required_input.replace("after: String!)", "after: String)");
        assert!(compare_schemas(&required_input, &relaxed_input)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn removed_fields_types_and_arguments_are_breaking() {
        let proposed = BASELINE
            .replace("          createdAt: DateTime\n", "")
            .replace(
                "enum VaultStatus { ACTIVE PAUSED ARCHIVED }",
                "enum VaultStatus { ACTIVE }",
            )
            .replace(
                "vaults(filter: VaultFilter, first: Int = 20)",
                "vaults(first: Int = 20)",
            );
        assert_eq!(
            change_kinds(BASELINE, &proposed),
            vec![
                "input-removed",
                "field-removed",
                "enum-value-removed",
                "enum-value-removed"
            ]
        );
    }

    #[test]
    fn nullable_to_required_is_breaking_in_both_directions() {
        let output = BASELINE.replace("createdAt: DateTime\n", "createdAt: DateTime!\n");
        let input = BASELINE.replace("after: String): [Vault!]!", "after: String!): [Vault!]!");
        assert!(change_kinds(BASELINE, &output).contains(&"nullable-to-required"));
        assert!(change_kinds(BASELINE, &input).contains(&"nullable-to-required"));
    }

    #[test]
    fn type_changes_are_classified() {
        let named = BASELINE.replace("createdAt: DateTime\n", "createdAt: String\n");
        let list = BASELINE.replace("createdAfter: DateTime\n", "createdAfter: [DateTime]\n");
        assert_eq!(change_kinds(BASELINE, &named), vec!["type-changed"]);
        assert_eq!(change_kinds(BASELINE, &list), vec!["type-changed"]);
    }

    #[test]
    fn output_relaxation_is_breaking_but_input_relaxation_is_safe() {
        let output = BASELINE.replace("id: ID!\n", "id: ID\n");
        let input = BASELINE.replace("vault(id: ID!)", "vault(id: ID)");
        assert!(change_kinds(BASELINE, &output).contains(&"required-to-nullable"));
        assert!(!change_kinds(BASELINE, &input).contains(&"required-to-nullable"));
    }

    #[test]
    fn root_kind_interface_union_and_default_changes_are_breaking() {
        let proposed = BASELINE
            .replace("mutation: Mutation", "mutation: Changes")
            .replace("implements Node", "")
            .replace("type Vault  {", "input Vault {")
            .replace(
                "union SearchResult = Vault | Manager",
                "union SearchResult = Vault",
            )
            .replace(
                "first: Int = 20, after: String",
                "first: Int, after: String",
            );
        let kinds = change_kinds(BASELINE, &proposed);
        assert!(kinds.contains(&"root-operation-changed"), "{kinds:?}");
        assert!(kinds.contains(&"type-kind-changed"), "{kinds:?}");
        assert!(kinds.contains(&"interface-removed"), "{kinds:?}");
        assert!(kinds.contains(&"union-member-removed"), "{kinds:?}");
        assert!(kinds.contains(&"default-removed"), "{kinds:?}");
    }

    #[test]
    fn report_contains_actionable_diagnostics() {
        let proposed = BASELINE.replace("          name: String!\n", "");
        let changes = compare_schemas(BASELINE, &proposed).unwrap();
        let report = format_report(&changes);
        assert!(report.contains("1 breaking change"));
        assert!(report.contains("field-removed"));
        assert!(report.contains("`Vault.name`"));
        assert!(report.contains("String!"));
        assert!(report.contains("reviewed baseline update"));
    }

    #[test]
    fn type_display_is_stable() {
        let nested = TypeRef::NonNull(Box::new(TypeRef::List(Box::new(TypeRef::NonNull(
            Box::new(TypeRef::Named("Vault".to_string())),
        )))));
        assert_eq!(nested.to_string(), "[Vault!]!");
    }
}
