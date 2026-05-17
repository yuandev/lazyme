use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Per-target configuration stored in ~/.config/lazyme/targets/{name}.toml
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
    pub command: Option<String>,
    pub artifact: Option<String>,
    pub maven_settings: Option<String>,
    pub local_repo: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunSection {
    pub mode: Option<String>,
    pub command: Option<String>,
    pub health_url: Option<String>,
    #[serde(default = "default_health_timeout")]
    pub health_timeout: u64,
    pub jvm_args: Option<String>,
    #[serde(default)]
    pub auto_restart: Option<bool>,
    /// Graceful shutdown timeout in seconds. After SIGTERM, wait up to this
    /// many seconds for the process to exit before sending SIGKILL.
    #[serde(default = "default_kill_timeout")]
    pub kill_timeout_secs: u64,
}

fn default_health_timeout() -> u64 { 30 }
fn default_kill_timeout() -> u64 { 30 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WatchSection {
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvSection {
    #[serde(flatten)]
    pub vars: HashMap<String, String>,
}

/// Config directory base: ~/.config/lazyme/targets/
fn targets_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").ok().unwrap_or_else(|| "/tmp".into());
    Ok(PathBuf::from(home).join(".config/lazyme/targets"))
}

/// Config file path: ~/.config/lazyme/targets/{name}.toml
pub fn target_config_path(name: &str) -> PathBuf {
    targets_dir().unwrap_or_else(|_| PathBuf::from("/tmp/.config/lazyme/targets")).join(format!("{name}.toml"))
}

/// Deploy directory (runtime state) under the repo: {repo}/.deployd/
pub fn deploy_dir(repo: &Path) -> PathBuf {
    repo.join(".deployd")
}

impl ProjectConfig {
    /// Load config for a named target.
    /// Reads from ~/.config/lazyme/targets/{name}.toml, with fallback
    /// migration from legacy {repo}/.deployd/config.toml.
    pub fn load(name: &str, repo: &Path) -> Result<Option<Self>> {
        let path = target_config_path(name);

        // Check new location first
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            return Ok(Some(toml::from_str(&content)?));
        }

        // Migrate from legacy repo-based config
        let legacy = repo.join(".deployd").join("config.toml");
        if legacy.exists() {
            let content = std::fs::read_to_string(&legacy)?;
            // Write to new location
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, &content)?;
            return Ok(Some(toml::from_str(&content)?));
        }

        Ok(None)
    }

    /// Save full config as TOML string.
    pub fn save_config(name: &str, toml_str: &str) -> Result<()> {
        let path = target_config_path(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, toml_str)?;
        Ok(())
    }

    /// Save the branch name into the config file (preserves existing keys).
    pub fn save_branch(name: &str, branch: &str) -> Result<()> {
        let path = target_config_path(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
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

    /// Read raw config file content.
    pub fn read_config_raw(name: &str) -> Result<Option<String>> {
        let path = target_config_path(name);
        if !path.exists() { return Ok(None); }
        Ok(Some(std::fs::read_to_string(&path)?))
    }

    /// Get the config file path.
    pub fn config_path(name: &str) -> String {
        target_config_path(name).display().to_string()
    }
}
