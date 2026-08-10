use crate::core::PolicyState;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

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
        let data_root = data_base.join("CodexGuard");
        Self {
            state: data_root.join("state.json"),
            config: data_root.join("config.json"),
            log: data_root.join("codexguard.log"),
            session_root,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardConfig {
    #[serde(default = "default_auto_retry_enabled")]
    pub auto_retry_enabled: bool,
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self {
            auto_retry_enabled: default_auto_retry_enabled(),
        }
    }
}

fn default_auto_retry_enabled() -> bool {
    true
}

pub fn load_config(path: &PathBuf) -> GuardConfig {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save_config(path: &PathBuf, config: &GuardConfig) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(config)
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
        std::env::temp_dir().join(format!("codexguard-{name}-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn uses_codexguard_data_paths() {
        let paths = Paths::discover(PathBuf::from("sessions"));
        assert_eq!(
            paths.state.parent().and_then(Path::file_name),
            Some(std::ffi::OsStr::new("CodexGuard"))
        );
        assert_eq!(
            paths.config.file_name(),
            Some(std::ffi::OsStr::new("config.json"))
        );
        assert_eq!(
            paths.log.file_name(),
            Some(std::ffi::OsStr::new("codexguard.log"))
        );
    }

    #[test]
    fn rotates_oversized_log_and_keeps_one_backup() {
        let directory = temporary_test_directory("log-rotation");
        fs::create_dir_all(&directory).unwrap();
        let log = directory.join("guard.log");
        fs::write(&log, b"12345678").unwrap();

        rotate_log_if_needed(&log, 3, 10).unwrap();

        assert!(!log.exists());
        assert_eq!(
            fs::read(directory.join("guard.log.1")).unwrap(),
            b"12345678"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saves_and_loads_policy_state() {
        let path = std::env::temp_dir().join(format!(
            "codexguard-state-test-{}.json",
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
}
