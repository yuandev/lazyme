use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

/// Per-project configuration read from {repo}/.deployd/config.toml.
/// All fields are optional — CLI args override them, defaults fill in the rest.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProjectConfig {
    #[serde(default)]
    pub build: BuildSection,
    #[serde(default)]
    pub run: RunSection,
    #[serde(default)]
    pub watch: WatchSection,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct BuildSection {
    /// Shell command to build the project
    pub command: Option<String>,
    /// Path to the built artifact, relative to repo root
    pub artifact: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RunSection {
    /// Shell command to start the deployed service. {artifact} is replaced.
    pub command: Option<String>,
    /// Health check endpoint
    pub health_url: Option<String>,
    /// Seconds to wait for health check to pass before marking success
    #[serde(default = "default_health_timeout")]
    pub health_timeout: u64,
}

fn default_health_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WatchSection {
    /// Branch to watch (default: "main")
    pub branch: Option<String>,
}

impl ProjectConfig {
    /// Load project config. Tries config.{profile}.toml first, falls back to config.toml.
    pub fn load(repo: &Path, profile: Option<&str>) -> Result<Option<Self>> {
        let deploy_dir = repo.join(".deployd");
        let path = if let Some(p) = profile {
            let profiled = deploy_dir.join(format!("config.{p}.toml"));
            if profiled.exists() { profiled } else { deploy_dir.join("config.toml") }
        } else {
            deploy_dir.join("config.toml")
        };

        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)?;
        Ok(Some(toml::from_str(&content)?))
    }
}
