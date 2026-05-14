use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Per-project configuration read from {repo}/.deployd/config.toml.
/// All fields are optional — CLI args override them, defaults fill in the rest.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectConfig {
    #[serde(default)]
    pub build: BuildSection,
    #[serde(default)]
    pub run: RunSection,
    #[serde(default)]
    pub watch: WatchSection,
    #[serde(default)]
    pub env: Option<EnvSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BuildSection {
    /// Shell command to build the project
    pub command: Option<String>,
    /// Path to the built artifact, relative to repo root
    pub artifact: Option<String>,
    /// Path to custom Maven settings.xml
    pub maven_settings: Option<String>,
    /// Path to local Maven repository
    pub local_repo: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunSection {
    /// Run mode: "dev" or "deploy". dev skips build & cache, just pulls and restarts run command.
    pub mode: Option<String>,
    /// Shell command to start the deployed service. {artifact}, {jvm_args} are replaced.
    pub command: Option<String>,
    /// Health check endpoint
    pub health_url: Option<String>,
    /// Seconds to wait for health check to pass before marking success
    #[serde(default = "default_health_timeout")]
    pub health_timeout: u64,
    /// JVM arguments
    pub jvm_args: Option<String>,
}

fn default_health_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WatchSection {
    /// Branch to watch (default: "main")
    pub branch: Option<String>,
}

/// Environment variables as key-value pairs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvSection {
    #[serde(flatten)]
    pub vars: HashMap<String, String>,
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

    /// Save the branch name into the project config file (preserves existing keys).
    pub fn save_branch(repo: &Path, profile: Option<&str>, branch: &str) -> Result<()> {
        let deploy_dir = repo.join(".deployd");
        std::fs::create_dir_all(&deploy_dir)?;
        let path = if let Some(p) = profile {
            deploy_dir.join(format!("config.{p}.toml"))
        } else {
            deploy_dir.join("config.toml")
        };

        let mut table: toml::Table = if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            toml::from_str(&content).unwrap_or_default()
        } else {
            toml::Table::new()
        };

        let watch = table.entry("watch").or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if let toml::Value::Table(w) = watch {
            w.insert("branch".into(), toml::Value::String(branch.to_string()));
        }

        std::fs::write(&path, toml::to_string_pretty(&table)?)?;
        Ok(())
    }

    /// Save full config as TOML string (merge-preserving unknown keys).
    pub fn save_config(repo: &Path, profile: Option<&str>, toml_str: &str) -> Result<()> {
        let deploy_dir = repo.join(".deployd");
        std::fs::create_dir_all(&deploy_dir)?;
        let path = if let Some(p) = profile {
            deploy_dir.join(format!("config.{p}.toml"))
        } else {
            deploy_dir.join("config.toml")
        };
        std::fs::write(&path, toml_str)?;
        Ok(())
    }

    /// Read raw config file content.
    pub fn read_config_raw(repo: &Path, profile: Option<&str>) -> Result<Option<String>> {
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
        Ok(Some(std::fs::read_to_string(&path)?))
    }

    /// Get the path to the project config file (does not guarantee it exists).
    pub fn config_path(repo: &Path, profile: Option<&str>) -> std::path::PathBuf {
        let deploy_dir = repo.join(".deployd");
        if let Some(p) = profile {
            let profiled = deploy_dir.join(format!("config.{p}.toml"));
            if profiled.exists() { profiled } else { deploy_dir.join("config.toml") }
        } else {
            deploy_dir.join("config.toml")
        }
    }

    /// Get the deploy directory path.
    pub fn deploy_dir(repo: &Path) -> std::path::PathBuf {
        repo.join(".deployd")
    }
}
