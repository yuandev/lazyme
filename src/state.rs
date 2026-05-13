use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployRecord {
    pub commit_hash: String,
    pub deployed_at: DateTime<Utc>,
    pub artifact_path: Option<PathBuf>,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeploymentState {
    pub current: Option<DeployRecord>,
    pub history: Vec<DeployRecord>,
}

pub struct StateManager {
    path: PathBuf,
    state: DeploymentState,
}

impl StateManager {
    pub fn new(repo_path: &std::path::Path) -> Self {
        let deploy_dir = repo_path.join(".deployd");
        let path = deploy_dir.join("state.json");
        let state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, state }
    }

    pub fn current(&self) -> &Option<DeployRecord> {
        &self.state.current
    }

    pub fn history(&self) -> &[DeployRecord] {
        &self.state.history
    }

    pub fn record_deploy(
        &mut self,
        commit_hash: String,
        artifact_path: Option<PathBuf>,
        success: bool,
    ) -> Result<()> {
        let record = DeployRecord {
            commit_hash,
            deployed_at: Utc::now(),
            artifact_path,
            success,
        };

        if self.state.current.is_some() {
            self.state
                .history
                .push(self.state.current.clone().unwrap());
        }
        self.state.current = Some(record);
        self.save()?;
        Ok(())
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.state)?;
        std::fs::write(&self.path, json)?;
        Ok(())
    }
}
