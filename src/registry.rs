use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct TargetEntry {
    pub name: String,
    pub repo: PathBuf,
    #[serde(default)]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TargetsFile {
    targets: Vec<TargetEntry>,
}

/// Load targets from ~/.config/lazyme/targets.toml
pub fn load() -> Result<Vec<TargetEntry>> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let path = PathBuf::from(home).join(".config/lazyme/targets.toml");

    if !path.exists() {
        return Err(anyhow::anyhow!(
            "No targets file found at {}. Create it with [[targets]] entries.",
            path.display()
        ));
    }

    let content = std::fs::read_to_string(&path)?;
    Ok(toml::from_str::<TargetsFile>(&content)?.targets)
}

/// Append a target entry to the registry file.
pub fn append_entry(entry: &TargetEntry) -> Result<()> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let path = PathBuf::from(home).join(".config/lazyme/targets.toml");
    std::fs::create_dir_all(path.parent().unwrap())?;

    let mut content = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };

    let new_entry = format!(
        "\n[[targets]]\nname = \"{}\"\nrepo = \"{}\"\nprofile = \"{}\"\n",
        entry.name,
        entry.repo.display(),
        entry.profile.as_deref().unwrap_or("")
    );

    content.push_str(&new_entry);
    std::fs::write(&path, &content)?;
    Ok(())
}

/// Filter targets by name. If names is empty, return all.
pub fn filter(targets: Vec<TargetEntry>, names: &[String]) -> Vec<TargetEntry> {
    if names.is_empty() {
        return targets;
    }
    targets
        .into_iter()
        .filter(|t| names.iter().any(|n| n == &t.name))
        .collect()
}
