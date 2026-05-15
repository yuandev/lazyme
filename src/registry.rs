use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct TargetEntry {
    pub name: String,
    pub repo: PathBuf,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
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

    let group_line = if let Some(ref g) = entry.group {
        format!("group = \"{}\"\n", g)
    } else {
        String::new()
    };
    let new_entry = format!(
        "\n[[targets]]\nname = \"{}\"\nrepo = \"{}\"\nprofile = \"{}\"\n{}",
        entry.name,
        entry.repo.display(),
        entry.profile.as_deref().unwrap_or(""),
        group_line,
    );

    content.push_str(&new_entry);
    std::fs::write(&path, &content)?;
    Ok(())
}

/// Remove a target entry by name from the registry file.
pub fn remove_entry(name: &str) -> Result<bool> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let path = PathBuf::from(home).join(".config/lazyme/targets.toml");
    if !path.exists() { return Ok(false); }
    let content = std::fs::read_to_string(&path)?;
    // Parse entries, filter out the one to remove, rebuild
    let entries: TargetsFile = toml::from_str(&content)?;
    let original_len = entries.targets.len();
    let filtered: Vec<TargetEntry> = entries.targets.into_iter()
        .filter(|e| e.name != name)
        .collect();
    if filtered.len() == original_len { return Ok(false); }
    // Rebuild TOML manually to preserve formatting
    let mut out = String::new();
    for e in &filtered {
        out.push_str("\n[[targets]]\n");
        out.push_str(&format!("name = \"{}\"\n", e.name));
        out.push_str(&format!("repo = \"{}\"\n", e.repo.display()));
        if let Some(ref p) = e.profile { out.push_str(&format!("profile = \"{}\"\n", p)); }
        if let Some(ref g) = e.group { out.push_str(&format!("group = \"{}\"\n", g)); }
        if let Some(ref l) = e.label { out.push_str(&format!("label = \"{}\"\n", l)); }
    }
    std::fs::write(&path, out)?;
    Ok(true)
}

/// Rename a target entry in the registry file.
pub fn rename_entry(old_name: &str, new_name: &str) -> Result<()> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let path = PathBuf::from(home).join(".config/lazyme/targets.toml");
    let content = std::fs::read_to_string(&path)?;
    let entries: TargetsFile = toml::from_str(&content)?;
    let mut out = String::new();
    for e in &entries.targets {
        out.push_str("\n[[targets]]\n");
        let n = if e.name == old_name { new_name } else { &e.name };
        out.push_str(&format!("name = \"{}\"\n", n));
        out.push_str(&format!("repo = \"{}\"\n", e.repo.display()));
        if let Some(ref p) = e.profile { out.push_str(&format!("profile = \"{}\"\n", p)); }
        if let Some(ref g) = e.group { out.push_str(&format!("group = \"{}\"\n", g)); }
        if let Some(ref l) = e.label { out.push_str(&format!("label = \"{}\"\n", if e.name == old_name { l.replace(old_name, new_name) } else { l.clone() })); }
    }
    std::fs::write(&path, out)?;
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
