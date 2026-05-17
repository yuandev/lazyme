use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployRecord {
    pub commit_hash: String,
    pub short_hash: String,
    pub deployed_at: DateTime<Utc>,
    pub cache_path: Option<PathBuf>,
    pub log_path: Option<PathBuf>,
    pub success: bool,
    #[serde(default)]
    pub build_duration_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeploymentState {
    pub current: Option<DeployRecord>,
    pub history: Vec<DeployRecord>,
}

pub struct StateManager {
    state_path: PathBuf,
    data_dir: PathBuf,
    state: DeploymentState,
}

impl StateManager {
    pub fn new(target_name: &str) -> Self {
        let data_dir = data_dir(target_name);
        let state_path = data_dir.join("state.json");
        let state = std::fs::read_to_string(&state_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { state_path, data_dir, state }
    }

    pub fn current(&self) -> &Option<DeployRecord> {
        &self.state.current
    }

    pub fn history(&self) -> &[DeployRecord] {
        &self.state.history
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn cache_dir(&self, short_hash: &str) -> PathBuf {
        self.data_dir.join("artifacts").join(short_hash)
    }

    pub fn find_cached_artifact(&self, short_hash: &str, artifact_name: &str) -> Option<PathBuf> {
        let path = self.cache_dir(short_hash).join(artifact_name);
        if path.exists() { Some(path) } else { None }
    }

    pub fn cache_artifact(&self, short_hash: &str, artifact_rel: &Path) -> Result<PathBuf> {
        let cache_dir = self.cache_dir(short_hash);
        std::fs::create_dir_all(&cache_dir)?;
        let fname = artifact_rel.file_name().context("artifact has no filename")?;
        let dst = cache_dir.join(fname);
        std::fs::copy(&artifact_rel, &dst)
            .with_context(|| format!("copy artifact to {}", dst.display()))?;
        Ok(dst)
    }

    pub fn record_deploy(
        &mut self,
        commit_hash: String,
        short_hash: String,
        cache_path: Option<PathBuf>,
        log_path: Option<PathBuf>,
        success: bool,
        build_duration_secs: Option<u64>,
    ) -> Result<()> {
        let record = DeployRecord {
            commit_hash,
            short_hash,
            deployed_at: Utc::now(),
            cache_path,
            log_path,
            success,
            build_duration_secs,
        };
        if self.state.current.is_some() {
            self.state.history.push(self.state.current.clone().unwrap());
        }
        self.state.current = Some(record);
        self.save()?;
        Ok(())
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.state_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.state)?;
        let tmp = self.state_path.with_extension("tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.state_path)?;
        Ok(())
    }
}

/// XDG data directory for a target: ~/.local/share/lazyme/{name}/
fn data_dir(target_name: &str) -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".local/share")
        });
    let dir = base.join("lazyme").join(target_name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}
