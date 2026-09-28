use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing_subscriber::{
    filter::{EnvFilter, LevelFilter},
    reload,
};

#[derive(Clone)]
pub struct RuntimeLogController {
    state: Arc<RuntimeLogState>,
}

pub type LogLevelManager = RuntimeLogController;

struct RuntimeLogState {
    handle: reload::Handle<EnvFilter, tracing_subscriber::Registry>,
    base_directives: String,
    settings: RwLock<LogSettings>,
}

#[derive(Clone, Default)]
struct LogSettings {
    global_level: Option<String>,
    modules: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLevelUpdate {
    pub module: String,
    pub level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLevelSnapshot {
    pub base_directives: String,
    pub global_level: Option<String>,
    pub modules: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum LogLevelError {
    #[error("invalid log module: {0}")]
    InvalidModule(String),
    #[error("invalid log level: {0}")]
    InvalidLevel(String),
    #[error("invalid log filter: {0}")]
    InvalidFilter(String),
    #[error("log filter reload failed: {0}")]
    Reload(String),
    #[error("log level configuration file could not be read: {0}")]
    Io(#[from] io::Error),
    #[error("log level configuration is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("log level state lock is poisoned")]
    LockPoisoned,
}

impl RuntimeLogController {
    pub fn new(
        handle: reload::Handle<EnvFilter, tracing_subscriber::Registry>,
        base_directives: impl Into<String>,
    ) -> Self {
        let base_directives = base_directives.into();
        let base_directives = if base_directives.trim().is_empty() {
            "info".to_string()
        } else {
            base_directives
        };
        Self {
            state: Arc::new(RuntimeLogState {
                handle,
                base_directives,
                settings: RwLock::new(LogSettings::default()),
            }),
        }
    }

    pub fn set_module(&self, module: &str, level: &str) -> Result<LogLevelSnapshot, LogLevelError> {
        let module = validate_module(module)?;
        let level = normalize_level(level)?;
        let handle = self.state.handle.clone();
        let base_directives = self.state.base_directives.clone();
        {
            let mut settings = self
                .state
                .settings
                .write()
                .map_err(|_| LogLevelError::LockPoisoned)?;
            let mut next = settings.clone();
            next.modules.insert(module, level);
            reload_settings(&handle, &base_directives, &next)?;
            *settings = next;
        }
        self.snapshot()
    }

    pub fn remove_module(&self, module: &str) -> Result<LogLevelSnapshot, LogLevelError> {
        let module = validate_module(module)?;
        let handle = self.state.handle.clone();
        let base_directives = self.state.base_directives.clone();
        {
            let mut settings = self
                .state
                .settings
                .write()
                .map_err(|_| LogLevelError::LockPoisoned)?;
            let mut next = settings.clone();
            next.modules.remove(&module);
            reload_settings(&handle, &base_directives, &next)?;
            *settings = next;
        }
        self.snapshot()
    }

    pub fn set_global_level(&self, level: &str) -> Result<LogLevelSnapshot, LogLevelError> {
        let level = normalize_level(level)?;
        let handle = self.state.handle.clone();
        let base_directives = self.state.base_directives.clone();
        {
            let mut settings = self
                .state
                .settings
                .write()
                .map_err(|_| LogLevelError::LockPoisoned)?;
            let mut next = settings.clone();
            next.global_level = Some(level);
            reload_settings(&handle, &base_directives, &next)?;
            *settings = next;
        }
        self.snapshot()
    }

    pub fn clear_global_level(&self) -> Result<LogLevelSnapshot, LogLevelError> {
        let handle = self.state.handle.clone();
        let base_directives = self.state.base_directives.clone();
        {
            let mut settings = self
                .state
                .settings
                .write()
                .map_err(|_| LogLevelError::LockPoisoned)?;
            let mut next = settings.clone();
            next.global_level = None;
            reload_settings(&handle, &base_directives, &next)?;
            *settings = next;
        }
        self.snapshot()
    }

    pub fn snapshot(&self) -> Result<LogLevelSnapshot, LogLevelError> {
        let base_directives = self.state.base_directives.clone();
        let settings = self
            .state
            .settings
            .read()
            .map_err(|_| LogLevelError::LockPoisoned)?;
        Ok(LogLevelSnapshot {
            base_directives,
            global_level: settings.global_level.clone(),
            modules: settings.modules.clone(),
        })
    }

    pub fn load_file(&self) -> Result<(), LogLevelError> {
        let path = match env::var("PERIGEE_LOG_LEVELS_FILE")
            .or_else(|_| env::var("LOG_LEVELS_FILE"))
        {
            Ok(path) if !path.trim().is_empty() => path,
            _ => return Ok(()),
        };
        let contents = fs::read_to_string(path)?;
        let updates: Vec<LogLevelUpdate> = serde_json::from_str(&contents)?;
        for update in updates {
            let module = update.module.trim();
            if module == "*" {
                self.set_global_level(&update.level)?;
            } else {
                self.set_module(module, &update.level)?;
            }
        }
        Ok(())
    }

}

fn reload_settings(
    handle: &reload::Handle<EnvFilter, tracing_subscriber::Registry>,
    base_directives: &str,
    settings: &LogSettings,
) -> Result<(), LogLevelError> {
    let mut values = vec![base_directives.to_string()];
    if let Some(level) = &settings.global_level {
        values.push(level.clone());
    }
    values.extend(
        settings
            .modules
            .iter()
            .map(|(module, level)| format!("{module}={level}")),
    );
    let directives = values.join(",");
    let filter = EnvFilter::builder()
        .with_regex(false)
        .with_default_directive(LevelFilter::INFO.into())
        .parse(&directives)
        .map_err(|error| LogLevelError::InvalidFilter(error.to_string()))?;
    handle
        .reload(filter)
        .map_err(|error| LogLevelError::Reload(error.to_string()))
}

fn validate_module(module: &str) -> Result<String, LogLevelError> {
    let module = module.trim();
    if module.is_empty() || module.len() > 200 {
        return Err(LogLevelError::InvalidModule(module.to_string()));
    }
    if !module
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_.:-".contains(character))
    {
        return Err(LogLevelError::InvalidModule(module.to_string()));
    }
    Ok(module.to_string())
}

fn normalize_level(level: &str) -> Result<String, LogLevelError> {
    let level = level.trim().to_ascii_lowercase();
    if matches!(level.as_str(), "trace" | "debug" | "info" | "warn" | "error" | "off") {
        Ok(level)
    } else {
        Err(LogLevelError::InvalidLevel(level))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_normalized_and_validated() {
        assert_eq!(normalize_level(" DEBUG ").unwrap(), "debug");
        assert!(normalize_level("verbose").is_err());
        assert!(validate_module("Perigee_core::simulation").is_ok());
        assert!(validate_module("bad,module").is_err());
    }
}
