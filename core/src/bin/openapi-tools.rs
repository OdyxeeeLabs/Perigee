//! Post-processes the OpenAPI document served at `/api-docs/openapi.json`.
//!
//! ```text
//! openapi-tools prepare <spec.json> <out.json> [--version v1]   3.0 -> 3.1, RFC 7807 errors, /v1 server
//! openapi-tools postman <spec.json> <out.json>                 Postman v2.1 collection
//! openapi-tools check   <spec.json>                            contract checks; exit 1 on violations
//! ```

use std::process::ExitCode;
use std::{env, fs};

use Perigee_core::rest_api::{contract_violations, postman_collection, set_version_server, standardize_errors, to_openapi_3_1};

fn read(path: &str) -> Result<serde_json::Value, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("{path} is not valid JSON: {e}"))
}

fn write(path: &str, value: &serde_json::Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(path, text + "\n").map_err(|e| format!("cannot write {path}: {e}"))
}

fn run(args: &[String]) -> Result<(), String> {
    let usage = "usage: openapi-tools prepare|postman|check <spec.json> [out.json] [--version v1]";
    match args.get(1).map(String::as_str) {
        Some("prepare") => {
            let (input, output) = (args.get(2).ok_or(usage)?, args.get(3).ok_or(usage)?);
            let version = args
                .iter()
                .position(|a| a == "--version")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str)
                .unwrap_or("v1");
            let spec = standardize_errors(&to_openapi_3_1(&read(input)?));
            write(output, &set_version_server(&spec, version))
        }
        Some("postman") => {
            let (input, output) = (args.get(2).ok_or(usage)?, args.get(3).ok_or(usage)?);
            write(output, &postman_collection(&read(input)?))
        }
        Some("check") => {
            let violations = contract_violations(&read(args.get(2).ok_or(usage)?)?);
            if violations.is_empty() {
                println!("OpenAPI contract: ok");
                Ok(())
            } else {
                Err(format!("{} contract violation(s):\n  - {}", violations.len(), violations.join("\n  - ")))
            }
        }
        _ => Err(usage.to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
