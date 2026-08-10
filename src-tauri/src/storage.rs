use crate::core::PolicyState;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
pub const DEFAULT_AUTOMATIC_CHAIN_LIMIT: u32 = 10;
pub const MIN_AUTOMATIC_CHAIN_LIMIT: u32 = 1;
pub const MAX_AUTOMATIC_CHAIN_LIMIT: u32 = 100;

#[derive(Debug, Clone)]
pub struct Paths {
    pub state: PathBuf,
    pub config: PathBuf,
    pub log: PathBuf,
    pub session_root: PathBuf,
}

impl Paths {
    pub fn discover(session_root: PathBuf) -> Self {
        let data_base = dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let data_root = data_base.join("TurnMender");
        Self {
            state: data_root.join("state.json"),
            config: data_root.join("config.json"),
            log: data_root.join("turnmender.log"),
            session_root,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinuationConfig {
    #[serde(default = "default_auto_retry_enabled")]
    pub auto_retry_enabled: bool,
    #[serde(default = "default_automatic_chain_limit")]
    pub automatic_chain_limit: u32,
}

impl Default for ContinuationConfig {
    fn default() -> Self {
        Self {
            auto_retry_enabled: default_auto_retry_enabled(),
            automatic_chain_limit: default_automatic_chain_limit(),
        }
    }
}

impl ContinuationConfig {
    pub fn normalized(mut self) -> Self {
        self.automatic_chain_limit = self
            .automatic_chain_limit
            .clamp(MIN_AUTOMATIC_CHAIN_LIMIT, MAX_AUTOMATIC_CHAIN_LIMIT);
        self
    }
}

fn default_auto_retry_enabled() -> bool {
    true
}

fn default_automatic_chain_limit() -> u32 {
    DEFAULT_AUTOMATIC_CHAIN_LIMIT
}

pub fn load_config(path: &PathBuf) -> ContinuationConfig {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ContinuationConfig>(&bytes).ok())
        .unwrap_or_default()
        .normalized()
}

pub fn save_config(path: &PathBuf, config: &ContinuationConfig) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(&config.clone().normalized())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    atomic_write(path, &bytes)
}

pub fn load_policy(path: &PathBuf) -> PolicyState {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save_policy(path: &PathBuf, policy: &mut PolicyState) -> io::Result<()> {
    policy.trim(1000);
    let bytes = serde_json::to_vec_pretty(policy)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    atomic_write(path, &bytes)
}

fn atomic_write(path: &PathBuf, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)?;
    #[cfg(target_os = "windows")]
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&temporary, path)?;
    Ok(())
}

pub fn append_log(path: &PathBuf, message: &str) -> io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let line = format!("{} {}\n", chrono_like_timestamp(), message);
    rotate_log_if_needed(path, line.len() as u64, MAX_LOG_BYTES)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

fn rotate_log_if_needed(path: &Path, incoming_bytes: u64, max_bytes: u64) -> io::Result<()> {
    let current_bytes = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if current_bytes.saturating_add(incoming_bytes) <= max_bytes {
        return Ok(());
    }

    let mut backup_name = path.file_name().unwrap_or_default().to_os_string();
    backup_name.push(".1");
    let backup = path.with_file_name(backup_name);
    if backup.exists() {
        fs::remove_file(&backup)?;
    }
    fs::rename(path, backup)
}

fn chrono_like_timestamp() -> String {
    chrono::Local::now()
        .format("[%Y-%m-%d %H:%M:%S]")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::EventKey;

    fn temporary_test_directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("turnmender-{name}-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn uses_turnmender_data_paths() {
        let paths = Paths::discover(PathBuf::from("sessions"));
        assert_eq!(
            paths.state.parent().and_then(Path::file_name),
            Some(std::ffi::OsStr::new("TurnMender"))
        );
        assert_eq!(
            paths.config.file_name(),
            Some(std::ffi::OsStr::new("config.json"))
        );
        assert_eq!(
            paths.log.file_name(),
            Some(std::ffi::OsStr::new("turnmender.log"))
        );
    }

    #[test]
    fn rotates_oversized_log_and_keeps_one_backup() {
        let directory = temporary_test_directory("log-rotation");
        fs::create_dir_all(&directory).unwrap();
        let log = directory.join("turnmender.log");
        fs::write(&log, b"12345678").unwrap();

        rotate_log_if_needed(&log, 3, 10).unwrap();

        assert!(!log.exists());
        assert_eq!(
            fs::read(directory.join("turnmender.log.1")).unwrap(),
            b"12345678"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saves_and_loads_policy_state() {
        let path = std::env::temp_dir().join(format!(
            "turnmender-state-test-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut policy = PolicyState::default();
        policy.mark_processed(EventKey {
            task_id: "task-1".into(),
            turn_id: "turn-1".into(),
        });
        policy.note_automatic_turn("task-1", "turn-2");
        save_policy(&path, &mut policy).unwrap();

        let loaded = load_policy(&path);
        assert!(loaded.is_processed(&EventKey {
            task_id: "task-1".into(),
            turn_id: "turn-1".into(),
        }));
        assert!(loaded.is_automatic_turn("turn-2"));
        assert_eq!(loaded.chain_failures("task-1"), 1);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn loads_legacy_config_with_default_chain_limit() {
        let path = std::env::temp_dir().join(format!(
            "turnmender-config-test-{}.json",
            uuid::Uuid::new_v4()
        ));
        fs::write(&path, br#"{"auto_retry_enabled":false}"#).unwrap();

        let config = load_config(&path);

        assert!(!config.auto_retry_enabled);
        assert_eq!(config.automatic_chain_limit, DEFAULT_AUTOMATIC_CHAIN_LIMIT);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn normalizes_chain_limit_when_loading_and_saving_config() {
        let path = std::env::temp_dir().join(format!(
            "turnmender-config-limit-test-{}.json",
            uuid::Uuid::new_v4()
        ));
        let config = ContinuationConfig {
            auto_retry_enabled: true,
            automatic_chain_limit: u32::MAX,
        };

        save_config(&path, &config).unwrap();
        let loaded = load_config(&path);

        assert_eq!(loaded.automatic_chain_limit, MAX_AUTOMATIC_CHAIN_LIMIT);
        fs::remove_file(path).unwrap();
    }
}
