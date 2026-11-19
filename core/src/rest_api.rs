//! # REST API standards and OpenAPI tooling
//!
//! Everything needed to publish the REST API as an **OpenAPI 3.1** contract:
//!
//! * [`ProblemDetails`] – the RFC 7807 (`application/problem+json`) error body
//!   every endpoint uses.
//! * [`to_openapi_3_1`] – upgrades the 3.0 document that `utoipa` produces
//!   (`nullable`, `example`, boolean `exclusiveMinimum`) to valid 3.1.
//! * [`standardize_errors`] – points every `4xx`/`5xx` response at the shared
//!   `Problem` schema, and [`set_version_server`] publishes the `/v1` prefix.
//! * [`postman_collection`] – builds a Postman v2.1 collection from the spec.
//! * [`redoc_html`] – a Redoc page for the spec.
//! * [`contract_violations`] – the checks CI runs against the generated spec.
//!
//! The `openapi-tools` binary wraps these for use in scripts and CI; see
//! `docs/OPENAPI.md`.

use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const PROBLEM_CONTENT_TYPE: &str = "application/problem+json";
pub const PROBLEM_BASE_URI: &str = "https://docs.perigee.dev/problems/";

/// RFC 7807 problem details. Extra members (for example `errors`) go in `extensions`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProblemDetails {
    #[serde(rename = "type")]
    pub problem_type: String,
    pub title: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

fn reason(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        410 => "Gone",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

fn slug(text: &str) -> String {
    text.to_lowercase().replace(' ', "-")
}

impl ProblemDetails {
    /// A problem for `status` whose `type` URI is derived from its reason phrase.
    pub fn from_status(status: u16, detail: &str) -> Self {
        ProblemDetails {
            problem_type: format!("{PROBLEM_BASE_URI}{}", slug(reason(status))),
            title: reason(status).to_string(),
            status,
            detail: (!detail.is_empty()).then(|| detail.to_string()),
            instance: None,
            extensions: BTreeMap::new(),
        }
    }

    /// Use a more specific, documented problem type, for example `validation-failed`.
    pub fn with_type(mut self, slug: &str, title: &str) -> Self {
        self.problem_type = format!("{PROBLEM_BASE_URI}{slug}");
        self.title = title.to_string();
        self
    }

    pub fn with_instance(mut self, instance: &str) -> Self {
        self.instance = Some(instance.to_string());
        self
    }

    pub fn with_extension(mut self, name: &str, value: Value) -> Self {
        self.extensions.insert(name.to_string(), value);
        self
    }

    /// Field-level validation errors as an `errors` extension.
    pub fn with_field_errors(self, errors: &[(&str, &str, &str)]) -> Self {
        let list: Vec<Value> = errors
            .iter()
            .map(|(field, code, message)| json!({"field": field, "code": code, "message": message}))
            .collect();
        self.with_extension("errors", Value::Array(list))
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

// ---------------------------------------------------------------------------
// OpenAPI 3.0 -> 3.1
// ---------------------------------------------------------------------------

fn upgrade_schema(node: &mut Value) {
    match node {
        Value::Object(map) => {
            // `nullable: true` becomes a type union with "null".
            if map.remove("nullable") == Some(Value::Bool(true)) {
                match map.get("type").cloned() {
                    Some(Value::String(t)) => {
                        map.insert("type".into(), json!([t, "null"]));
                    }
                    Some(Value::Array(mut types)) => {
                        if !types.iter().any(|t| t == "null") {
                            types.push(json!("null"));
                        }
                        map.insert("type".into(), Value::Array(types));
                    }
                    _ => {
                        if let Some(inner) = map.remove("allOf") {
                            map.insert("anyOf".into(), json!([{"allOf": inner}, {"type": "null"}]));
                        } else if let Some(reference) = map.remove("$ref") {
                            map.insert("anyOf".into(), json!([{"$ref": reference}, {"type": "null"}]));
                        }
                    }
                }
            }
            // `example` is deprecated in favour of `examples` (an array).
            if let Some(example) = map.remove("example") {
                map.entry("examples").or_insert_with(|| json!([example]));
            }
            // Boolean exclusive bounds become numeric ones.
            for (flag, bound) in [("exclusiveMinimum", "minimum"), ("exclusiveMaximum", "maximum")] {
                if let Some(Value::Bool(exclusive)) = map.get(flag).cloned() {
                    map.remove(flag);
                    if exclusive {
                        if let Some(value) = map.remove(bound) {
                            map.insert(flag.into(), value);
                        }
                    }
                }
            }
            for value in map.values_mut() {
                upgrade_schema(value);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(upgrade_schema),
        _ => {}
    }
}

/// Convert an OpenAPI 3.0.x document to 3.1.0.
pub fn to_openapi_3_1(spec: &Value) -> Value {
    let mut out = spec.clone();
    upgrade_schema(&mut out);
    if let Some(map) = out.as_object_mut() {
        map.insert("openapi".into(), json!("3.1.0"));
        map.entry("jsonSchemaDialect")
            .or_insert_with(|| json!("https://spec.openapis.org/oas/3.1/dialect/base"));
    }
    out
}

// ---------------------------------------------------------------------------
// Errors, versioning
// ---------------------------------------------------------------------------

fn problem_schema() -> Value {
    json!({
        "type": "object",
        "required": ["type", "title", "status"],
        "properties": {
            "type": {"type": "string", "format": "uri", "description": "Identifies the problem type"},
            "title": {"type": "string"},
            "status": {"type": "integer", "minimum": 100, "maximum": 599},
            "detail": {"type": "string"},
            "instance": {"type": "string"},
            "errors": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["field", "code", "message"],
                    "properties": {
                        "field": {"type": "string"},
                        "code": {"type": "string"},
                        "message": {"type": "string"}
                    }
                }
            }
        },
        "additionalProperties": true
    })
}

const HTTP_METHODS: [&str; 7] = ["get", "put", "post", "delete", "options", "head", "patch"];

fn operations(spec: &Value) -> Vec<(String, String, &Value)> {
    let mut found = Vec::new();
    if let Some(paths) = spec.get("paths").and_then(Value::as_object) {
        for (path, item) in paths {
            for method in HTTP_METHODS {
                if let Some(operation) = item.get(method) {
                    found.push((path.clone(), method.to_string(), operation));
                }
            }
        }
    }
    found
}

/// Adds the shared `Problem` schema and rewrites every error response to use
/// it as `application/problem+json`.
pub fn standardize_errors(spec: &Value) -> Value {
    let mut out = spec.clone();

    let components = out
        .as_object_mut()
        .map(|o| o.entry("components").or_insert_with(|| json!({})));
    if let Some(Value::Object(components)) = components {
        let schemas = components.entry("schemas").or_insert_with(|| json!({}));
        if let Some(schemas) = schemas.as_object_mut() {
            schemas.insert("Problem".into(), problem_schema());
        }
    }

    if let Some(paths) = out.get_mut("paths").and_then(Value::as_object_mut) {
        for item in paths.values_mut() {
            for method in HTTP_METHODS {
                let Some(responses) = item.get_mut(method).and_then(|op| op.get_mut("responses")).and_then(Value::as_object_mut) else {
                    continue;
                };
                for (status, response) in responses.iter_mut() {
                    let is_error = status.parse::<u16>().is_ok_and(|s| s >= 400) || status == "default";
                    if is_error {
                        if let Some(map) = response.as_object_mut() {
                            map.insert(
                                "content".into(),
                                json!({PROBLEM_CONTENT_TYPE: {"schema": {"$ref": "#/components/schemas/Problem"}}}),
                            );
                        }
                    }
                }
            }
        }
    }
    out
}

/// Publish the URL version prefix (`/v1`) as the server base.
pub fn set_version_server(spec: &Value, version: &str) -> Value {
    let mut out = spec.clone();
    if let Some(map) = out.as_object_mut() {
        map.insert("servers".into(), json!([{"url": format!("/{version}"), "description": format!("API {version}")}]));
    }
    out
}

// ---------------------------------------------------------------------------
// Contract checks
// ---------------------------------------------------------------------------

fn is_collection_get(path: &str, method: &str) -> bool {
    method == "get" && !path.contains('{') && !path.ends_with("/health") && !path.ends_with("/ready")
        && !path.ends_with("/metrics") && !path.contains("jwks")
}

/// Problems with the spec; empty means it passes. These are the CI contract tests.
pub fn contract_violations(spec: &Value) -> Vec<String> {
    let mut violations = Vec::new();

    if !spec.get("openapi").and_then(Value::as_str).is_some_and(|v| v.starts_with("3.1")) {
        violations.push("openapi version must be 3.1.x".to_string());
    }
    if spec.pointer("/info/version").and_then(Value::as_str).is_none_or(str::is_empty) {
        violations.push("info.version is required".to_string());
    }
    if spec.pointer("/components/schemas/Problem").is_none() {
        violations.push("components.schemas.Problem (RFC 7807) is missing".to_string());
    }

    let mut ids = BTreeMap::<String, String>::new();
    for (path, method, operation) in operations(spec) {
        let label = format!("{} {path}", method.to_uppercase());

        match operation.get("operationId").and_then(Value::as_str) {
            None => violations.push(format!("{label}: operationId is required (client generation needs it)")),
            Some(id) => {
                if let Some(previous) = ids.insert(id.to_string(), label.clone()) {
                    violations.push(format!("{label}: operationId `{id}` is already used by {previous}"));
                }
            }
        }
        if operation.get("tags").and_then(Value::as_array).is_none_or(Vec::is_empty) {
            violations.push(format!("{label}: at least one tag is required"));
        }

        if let Some(responses) = operation.get("responses").and_then(Value::as_object) {
            for (status, response) in responses {
                let is_error = status.parse::<u16>().is_ok_and(|s| s >= 400);
                if is_error && response.pointer(&format!("/content/{}", PROBLEM_CONTENT_TYPE.replace('/', "~1"))).is_none() {
                    violations.push(format!("{label}: {status} response must be {PROBLEM_CONTENT_TYPE}"));
                }
            }
        } else {
            violations.push(format!("{label}: responses are required"));
        }

        if is_collection_get(&path, &method) {
            let params: Vec<&str> = operation
                .get("parameters")
                .and_then(Value::as_array)
                .map(|ps| ps.iter().filter_map(|p| p.get("name").and_then(Value::as_str)).collect())
                .unwrap_or_default();
            let returns_list = operation
                .pointer("/responses/200/content/application~1json/schema/type")
                .and_then(Value::as_str)
                == Some("array");
            if returns_list && !(params.contains(&"limit") && params.contains(&"cursor")) {
                violations.push(format!("{label}: list endpoints must accept `limit` and `cursor`"));
            }
        }
    }
    violations
}

// ---------------------------------------------------------------------------
// Postman and Redoc
// ---------------------------------------------------------------------------

fn postman_path(path: &str) -> Vec<String> {
    path.trim_start_matches('/')
        .split('/')
        .map(|segment| match segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            Some(name) => format!(":{name}"),
            None => segment.to_string(),
        })
        .collect()
}

fn example_body(operation: &Value) -> String {
    let schema = operation.pointer("/requestBody/content/application~1json/schema");
    let example = schema
        .and_then(|s| s.get("example").or_else(|| s.get("examples").and_then(|e| e.get(0))))
        .cloned();
    serde_json::to_string_pretty(&example.unwrap_or_else(|| json!({}))).unwrap_or_else(|_| "{}".to_string())
}

/// Postman collection (v2.1) with one folder per tag. Requests use the
/// `{{baseUrl}}` and `{{token}}` collection variables.
pub fn postman_collection(spec: &Value) -> Value {
    let mut folders: BTreeMap<String, Vec<Value>> = BTreeMap::new();

    for (path, method, operation) in operations(spec) {
        let tag = operation
            .pointer("/tags/0")
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_string();
        let name = operation
            .get("summary")
            .or_else(|| operation.get("operationId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("{} {path}", method.to_uppercase()));

        let params = operation.get("parameters").and_then(Value::as_array).cloned().unwrap_or_default();
        let query: Vec<Value> = params
            .iter()
            .filter(|p| p.get("in") == Some(&json!("query")))
            .map(|p| json!({"key": p.get("name"), "value": "", "disabled": p.get("required") != Some(&json!(true))}))
            .collect();

        let mut headers = vec![json!({"key": "Accept", "value": "application/json"})];
        let mut request = Map::new();
        request.insert("method".into(), json!(method.to_uppercase()));
        if operation.get("requestBody").is_some() {
            headers.push(json!({"key": "Content-Type", "value": "application/json"}));
            request.insert("body".into(), json!({"mode": "raw", "raw": example_body(operation), "options": {"raw": {"language": "json"}}}));
        }
        if operation.get("security").and_then(Value::as_array).is_some_and(|s| !s.is_empty()) {
            request.insert(
                "auth".into(),
                json!({"type": "bearer", "bearer": [{"key": "token", "value": "{{token}}", "type": "string"}]}),
            );
        }
        request.insert("header".into(), Value::Array(headers));
        request.insert(
            "url".into(),
            json!({
                "raw": format!("{{{{baseUrl}}}}{path}"),
                "host": ["{{baseUrl}}"],
                "path": postman_path(&path),
                "query": query,
            }),
        );

        folders.entry(tag).or_default().push(json!({"name": name, "request": Value::Object(request)}));
    }

    let items: Vec<Value> = folders.into_iter().map(|(tag, item)| json!({"name": tag, "item": item})).collect();
    json!({
        "info": {
            "name": spec.pointer("/info/title").and_then(Value::as_str).unwrap_or("API"),
            "description": spec.pointer("/info/description").and_then(Value::as_str).unwrap_or(""),
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json",
        },
        "variable": [
            {"key": "baseUrl", "value": "http://localhost:3000/v1"},
            {"key": "token", "value": ""},
        ],
        "item": items,
    })
}

fn escape_attribute(value: &str) -> String {
    value.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

/// A standalone Redoc page for `spec_url`.
pub fn redoc_html(title: &str, spec_url: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\">\n  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n  <title>{}</title>\n</head>\n<body>\n  <redoc spec-url=\"{}\"></redoc>\n  <script src=\"https://cdn.redoc.ly/redoc/v2.1.5/bundles/redoc.standalone.js\"></script>\n</body>\n</html>\n",
        escape_attribute(title),
        escape_attribute(spec_url),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({
            "openapi": "3.0.3",
            "info": {"title": "Perigee API", "version": "0.1.0", "description": "d"},
            "paths": {
                "/vaults": {
                    "get": {
                        "operationId": "list_vaults",
                        "summary": "List vaults",
                        "tags": ["Vaults"],
                        "parameters": [
                            {"name": "limit", "in": "query", "schema": {"type": "integer"}},
                            {"name": "cursor", "in": "query", "schema": {"type": "string"}},
                            {"name": "manager_id", "in": "query", "required": true, "schema": {"type": "string"}}
                        ],
                        "responses": {
                            "200": {"description": "ok", "content": {"application/json": {"schema": {"type": "array"}}}},
                            "401": {"description": "Unauthorized"}
                        },
                        "security": [{"bearerAuth": []}]
                    },
                    "post": {
                        "operationId": "create_vault",
                        "tags": ["Vaults"],
                        "requestBody": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/CreateVault", "example": {"name": "demo"}}}}},
                        "responses": {"200": {"description": "ok"}, "400": {"description": "Invalid request"}}
                    }
                },
                "/vaults/{id}": {
                    "get": {
                        "operationId": "get_vault",
                        "summary": "Get a vault",
                        "tags": ["Vaults"],
                        "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}],
                        "responses": {"200": {"description": "ok"}, "404": {"description": "Not found"}}
                    }
                }
            },
            "components": {"schemas": {
                "CreateVault": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "example": "demo"},
                        "note": {"type": "string", "nullable": true},
                        "owner": {"nullable": true, "allOf": [{"$ref": "#/components/schemas/Owner"}]},
                        "fee": {"type": "number", "minimum": 0, "exclusiveMinimum": true}
                    }
                },
                "Owner": {"type": "object"}
            }}
        })
    }

    #[test]
    fn problem_details_follow_rfc_7807() {
        let problem = ProblemDetails::from_status(404, "vault v1 does not exist")
            .with_instance("/v1/vaults/v1")
            .to_json();
        assert_eq!(problem["type"], "https://docs.perigee.dev/problems/not-found");
        assert_eq!(problem["title"], "Not Found");
        assert_eq!(problem["status"], 404);
        assert_eq!(problem["detail"], "vault v1 does not exist");
        assert_eq!(problem["instance"], "/v1/vaults/v1");
    }

    #[test]
    fn problems_can_carry_a_specific_type_and_field_errors() {
        let problem = ProblemDetails::from_status(422, "")
            .with_type("validation-failed", "Validation failed")
            .with_field_errors(&[("name", "required", "is required")])
            .to_json();
        assert_eq!(problem["type"], "https://docs.perigee.dev/problems/validation-failed");
        assert!(problem.get("detail").is_none(), "empty detail is omitted");
        assert_eq!(problem["errors"][0]["field"], "name");
        assert_eq!(ProblemDetails::from_status(429, "x").problem_type, format!("{PROBLEM_BASE_URI}too-many-requests"));
    }

    #[test]
    fn the_upgrade_produces_3_1_schemas() {
        let upgraded = to_openapi_3_1(&sample());
        assert_eq!(upgraded["openapi"], "3.1.0");

        let props = &upgraded["components"]["schemas"]["CreateVault"]["properties"];
        assert_eq!(props["note"]["type"], json!(["string", "null"]));
        assert!(props["note"].get("nullable").is_none());
        assert_eq!(props["name"]["examples"], json!(["demo"]));
        assert!(props["name"].get("example").is_none());
        assert_eq!(props["fee"]["exclusiveMinimum"], json!(0));
        assert!(props["fee"].get("minimum").is_none());
        assert_eq!(props["owner"]["anyOf"][1], json!({"type": "null"}));
        assert!(props["owner"].get("nullable").is_none());
    }

    #[test]
    fn the_upgrade_is_idempotent_and_does_not_touch_the_input() {
        let original = sample();
        let once = to_openapi_3_1(&original);
        assert_eq!(to_openapi_3_1(&once), once);
        assert_eq!(original["openapi"], "3.0.3");
    }

    #[test]
    fn errors_are_standardized_to_problem_json() {
        let out = standardize_errors(&sample());
        assert!(out["components"]["schemas"]["Problem"]["properties"]["status"].is_object());
        let unauthorized = &out["paths"]["/vaults"]["get"]["responses"]["401"];
        assert_eq!(unauthorized["description"], "Unauthorized");
        assert_eq!(unauthorized["content"][PROBLEM_CONTENT_TYPE]["schema"]["$ref"], "#/components/schemas/Problem");
        assert!(out["paths"]["/vaults"]["get"]["responses"]["200"].get("content").is_some());
        assert!(out["paths"]["/vaults/{id}"]["get"]["responses"]["200"].get("content").is_none(), "successes are left alone");
    }

    #[test]
    fn the_version_prefix_is_published_as_the_server() {
        assert_eq!(set_version_server(&sample(), "v1")["servers"][0]["url"], "/v1");
    }

    #[test]
    fn the_raw_utoipa_output_fails_the_contract_and_the_prepared_one_passes() {
        let raw = contract_violations(&sample());
        assert!(raw.iter().any(|v| v.contains("3.1")));
        assert!(raw.iter().any(|v| v.contains("Problem")));
        assert!(raw.iter().any(|v| v.contains("401 response must be application/problem+json")));
        assert!(raw.iter().any(|v| v.contains("create_vault") || v.contains("POST /vaults")));

        let prepared = standardize_errors(&to_openapi_3_1(&sample()));
        assert_eq!(contract_violations(&prepared), Vec::<String>::new());
    }

    #[test]
    fn the_contract_flags_missing_ids_tags_duplicates_and_unpaginated_lists() {
        let mut spec = standardize_errors(&to_openapi_3_1(&sample()));
        spec["paths"]["/vaults/{id}"]["get"]["operationId"] = json!("list_vaults");
        spec["paths"]["/vaults/{id}"]["get"]["tags"] = json!([]);
        spec["paths"]["/vaults"]["post"].as_object_mut().unwrap().remove("operationId");
        spec["paths"]["/vaults"]["get"]["parameters"] = json!([]);

        let violations = contract_violations(&spec).join("\n");
        assert!(violations.contains("operationId `list_vaults` is already used"));
        assert!(violations.contains("GET /vaults/{id}: at least one tag"));
        assert!(violations.contains("POST /vaults: operationId is required"));
        assert!(violations.contains("GET /vaults: list endpoints must accept `limit` and `cursor`"));
    }

    #[test]
    fn the_postman_collection_groups_by_tag_with_paths_auth_and_bodies() {
        let collection = postman_collection(&sample());

        assert_eq!(collection["info"]["name"], "Perigee API");
        assert!(collection["info"]["schema"].as_str().unwrap().contains("v2.1.0"));
        let folder = &collection["item"][0];
        assert_eq!(folder["name"], "Vaults");
        assert_eq!(folder["item"].as_array().unwrap().len(), 3);

        let list = &folder["item"].as_array().unwrap().iter().find(|i| i["name"] == "List vaults").unwrap()["request"];
        assert_eq!(list["method"], "GET");
        assert_eq!(list["url"]["raw"], "{{baseUrl}}/vaults");
        assert_eq!(list["auth"]["type"], "bearer");
        let required = list["url"]["query"].as_array().unwrap().iter().find(|q| q["key"] == "manager_id").unwrap();
        assert_eq!(required["disabled"], false);
        let optional = list["url"]["query"].as_array().unwrap().iter().find(|q| q["key"] == "limit").unwrap();
        assert_eq!(optional["disabled"], true);

        let get = &folder["item"].as_array().unwrap().iter().find(|i| i["name"] == "Get a vault").unwrap()["request"];
        assert_eq!(get["url"]["path"], json!(["vaults", ":id"]));

        let create = &folder["item"].as_array().unwrap().iter().find(|i| i["name"] == "create_vault").unwrap()["request"];
        assert!(create["body"]["raw"].as_str().unwrap().contains("\"name\": \"demo\""));
    }

    #[test]
    fn the_redoc_page_escapes_its_inputs() {
        let page = redoc_html("Perigee \"API\"", "/api-docs/openapi.json?x=1&y=<2>");
        assert!(page.contains("<redoc spec-url=\"/api-docs/openapi.json?x=1&amp;y=&lt;2&gt;\">"));
        assert!(page.contains("<title>Perigee &quot;API&quot;</title>"));
        assert!(!page.contains("y=<2>"));
    }
}
