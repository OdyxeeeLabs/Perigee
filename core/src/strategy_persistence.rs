use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const FORMAT_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyState {
    pub vault_id: String,
    pub current_phase: String,
    pub last_evaluated_at: DateTime<Utc>,
    pub last_trigger: Option<String>,
    pub evaluation_count: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedState {
    version: u8,
    states: HashMap<String, StrategyState>,
    checksum: String,
}

#[derive(Debug)]
pub struct StrategyStateManager {
    states: HashMap<String, StrategyState>,
}

impl Default for StrategyStateManager {
    fn default() -> Self {
        Self::new()
    }
}

impl StrategyStateManager {
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
        }
    }

    pub fn save_state(&mut self, state: StrategyState) {
        self.states.insert(state.vault_id.clone(), state);
    }

    pub fn load_state(&self, vault_id: &str) -> Option<&StrategyState> {
        self.states.get(vault_id)
    }

    pub fn recover_or_default(&self, vault_id: &str) -> StrategyState {
        self.states
            .get(vault_id)
            .cloned()
            .unwrap_or_else(|| StrategyState {
                vault_id: vault_id.to_string(),
                current_phase: String::from("default"),
                last_evaluated_at: Utc::now(),
                last_trigger: None,
                evaluation_count: 0,
            })
    }

    /// Serialize the current strategy state to bytes.
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` if serialization fails (e.g. a field value
    /// contains data that `serde_json` cannot represent).
    pub fn persist_to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        let state = self.serialized_state()?;
        serde_json::to_vec(&state)
    }

    pub fn restore_from_bytes(data: &[u8]) -> Result<Self, String> {
        let persisted: PersistedState = serde_json::from_slice(data).map_err(|e| e.to_string())?;
        Self::from_persisted(persisted)
    }

    pub fn persist_to_path<P: AsRef<Path>>(&self, path: P, max_backups: usize) -> io::Result<()> {
        let path = path.as_ref();
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;

        let data = self
            .persist_to_bytes()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&data)?;
        temporary.as_file().sync_all()?;
        let temporary_path = temporary.into_temp_path();

        if max_backups > 0 {
            rotate_backups(path, max_backups)?;
            if path.exists() {
                fs::copy(path, format!("{}.bak.1", path.display()))?;
            }
        }
        replace_atomically(&temporary_path, path)?;
        sync_directory(parent)
    }

    pub fn restore_from_path<P: AsRef<Path>>(path: P, max_backups: usize) -> Result<Self, String> {
        let path = path.as_ref();
        let mut last_error = None;
        for candidate in std::iter::once(path.to_path_buf()).chain(backup_paths(path, max_backups))
        {
            match fs::read(&candidate) {
                Ok(data) => match Self::restore_from_bytes(&data) {
                    Ok(state) => return Ok(state),
                    Err(error) => last_error = Some(format!("{}: {}", candidate.display(), error)),
                },
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => last_error = Some(format!("{}: {}", candidate.display(), error)),
            }
        }
        Err(last_error
            .unwrap_or_else(|| format!("strategy state file not found: {}", path.display())))
    }

    fn serialized_state(&self) -> Result<PersistedState, serde_json::Error> {
        let states = self.states.clone();
        let state_bytes = canonical_state_bytes(&states)?;
        Ok(PersistedState {
            version: FORMAT_VERSION,
            states,
            checksum: checksum(&state_bytes),
        })
    }

    fn from_persisted(persisted: PersistedState) -> Result<Self, String> {
        if persisted.version != FORMAT_VERSION {
            return Err(format!(
                "unsupported strategy state version: {}",
                persisted.version
            ));
        }
        let state_bytes = canonical_state_bytes(&persisted.states).map_err(|e| e.to_string())?;
        if checksum(&state_bytes) != persisted.checksum {
            return Err("strategy state checksum mismatch".to_string());
        }
        Ok(Self {
            states: persisted.states,
        })
    }
}

fn checksum(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

fn canonical_state_bytes(
    states: &HashMap<String, StrategyState>,
) -> Result<Vec<u8>, serde_json::Error> {
    let sorted_states: BTreeMap<_, _> = states.iter().collect();
    serde_json::to_vec(&sorted_states)
}

fn backup_paths(path: &Path, max_backups: usize) -> Vec<PathBuf> {
    (1..=max_backups)
        .map(|index| PathBuf::from(format!("{}.bak.{}", path.display(), index)))
        .collect()
}

fn rotate_backups(path: &Path, max_backups: usize) -> io::Result<()> {
    if max_backups == 0 {
        return Ok(());
    }
    for index in (1..=max_backups).rev() {
        let source = PathBuf::from(format!("{}.bak.{}", path.display(), index));
        let destination = PathBuf::from(format!("{}.bak.{}", path.display(), index + 1));
        if source.exists() {
            if index == max_backups {
                fs::remove_file(&source)?;
            } else {
                fs::rename(source, destination)?;
            }
        }
    }
    Ok(())
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(path)?.sync_all()?;
    }
    Ok(())
}

#[cfg(unix)]
fn replace_atomically(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)
}

#[cfg(windows)]
fn replace_atomically(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_and_load() {
        let mut mgr = StrategyStateManager::new();
        let state = StrategyState {
            vault_id: "v1".into(),
            current_phase: "active".into(),
            last_evaluated_at: Utc::now(),
            last_trigger: None,
            evaluation_count: 5,
        };
        mgr.save_state(state.clone());
        let loaded = mgr.load_state("v1").unwrap();
        assert_eq!(loaded.current_phase, "active");
        assert_eq!(loaded.evaluation_count, 5);
    }

    #[test]
    fn test_recover_or_default_missing() {
        let mgr = StrategyStateManager::new();
        let default = mgr.recover_or_default("missing");
        assert_eq!(default.current_phase, "default");
        assert_eq!(default.evaluation_count, 0);
    }

    #[test]
    fn test_persist_roundtrip() {
        let mut mgr = StrategyStateManager::new();
        mgr.save_state(StrategyState {
            vault_id: "v1".into(),
            current_phase: "active".into(),
            last_evaluated_at: Utc::now(),
            last_trigger: Some("crossed".into()),
            evaluation_count: 10,
        });
        let bytes = mgr.persist_to_bytes().unwrap();
        let restored = StrategyStateManager::restore_from_bytes(&bytes).unwrap();
        let s = restored.load_state("v1").unwrap();
        assert_eq!(s.evaluation_count, 10);
        assert_eq!(s.last_trigger.as_deref(), Some("crossed"));
    }

    #[test]
    fn test_checksum_rejects_modified_state() {
        let mut mgr = StrategyStateManager::new();
        mgr.save_state(StrategyState {
            vault_id: "v1".into(),
            current_phase: "active".into(),
            last_evaluated_at: Utc::now(),
            last_trigger: None,
            evaluation_count: 1,
        });
        let mut bytes = mgr.persist_to_bytes().unwrap();
        let checksum_byte = bytes.len() - 3;
        bytes[checksum_byte] = b'0';

        let error = StrategyStateManager::restore_from_bytes(&bytes).unwrap_err();
        assert!(error.contains("checksum") || error.contains("JSON"));
    }

    #[test]
    fn test_disk_persistence_is_atomic_and_keeps_backups() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("strategy-state.json");
        let mut mgr = StrategyStateManager::new();
        mgr.save_state(StrategyState {
            vault_id: "v1".into(),
            current_phase: "bull".into(),
            last_evaluated_at: Utc::now(),
            last_trigger: None,
            evaluation_count: 1,
        });
        mgr.persist_to_path(&path, 2).unwrap();

        mgr.states.get_mut("v1").unwrap().current_phase = "bear".into();
        mgr.persist_to_path(&path, 2).unwrap();
        assert_eq!(
            StrategyStateManager::restore_from_path(&path, 2)
                .unwrap()
                .load_state("v1")
                .unwrap()
                .current_phase,
            "bear"
        );
        assert!(path.with_file_name("strategy-state.json.bak.1").exists());
        assert_eq!(
            StrategyStateManager::restore_from_bytes(
                &fs::read(path.with_file_name("strategy-state.json.bak.1")).unwrap()
            )
            .unwrap()
            .load_state("v1")
            .unwrap()
            .current_phase,
            "bull"
        );
    }

    #[test]
    fn test_disk_load_falls_back_to_valid_backup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("strategy-state.json");
        let mut mgr = StrategyStateManager::new();
        mgr.save_state(StrategyState {
            vault_id: "v1".into(),
            current_phase: "bull".into(),
            last_evaluated_at: Utc::now(),
            last_trigger: None,
            evaluation_count: 1,
        });
        mgr.persist_to_path(&path, 1).unwrap();
        mgr.states.get_mut("v1").unwrap().current_phase = "bear".into();
        mgr.persist_to_path(&path, 1).unwrap();
        fs::write(&path, b"corrupt").unwrap();

        let restored = StrategyStateManager::restore_from_path(&path, 1).unwrap();
        assert_eq!(restored.load_state("v1").unwrap().current_phase, "bull");
    }
}
