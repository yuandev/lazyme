use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// Run git command and return stdout as string
fn git(args: &[&str], repo: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .with_context(|| format!("git {} failed", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {}: {}", args.join(" "), stderr);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn remote_head(repo: &Path, remote: &str, branch: &str) -> Result<String> {
    git(
        &["ls-remote", "--heads", remote, branch],
        repo,
    )
    .map(|s| s.split_whitespace().next().unwrap_or("").to_string())
}

pub fn local_head(repo: &Path) -> Result<String> {
    git(&["rev-parse", "HEAD"], repo)
}

pub fn short_hash(repo: &Path, full_hash: &str) -> Result<String> {
    git(&["rev-parse", "--short", full_hash], repo)
}

pub fn pull(repo: &Path, remote: &str, branch: &str) -> Result<()> {
    git(&["fetch", remote], repo)?;  // fetch all remote refs
    force_checkout(repo, &format!("{remote}/{branch}"))?;
    Ok(())
}

pub fn checkout(repo: &Path, commit: &str) -> Result<()> {
    force_checkout(repo, commit)?;
    Ok(())
}

/// Checkout with dirty file protection: stash → checkout → pop stash
fn force_checkout(repo: &Path, target: &str) -> Result<()> {
    let has_changes = !git(&["status", "--porcelain"], repo)
        .unwrap_or_default().is_empty();
    if has_changes {
        let _ = git(&["stash", "push", "-m", "lazyme auto-stash before checkout"], repo);
    }
    let r = git(&["checkout", target], repo);
    if has_changes {
        let _ = git(&["stash", "pop"], repo);
    }
    r?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CommitInfo {
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    pub author: String,
    pub timestamp: String,
}

pub fn recent_commits(repo: &Path, n: usize) -> Result<Vec<CommitInfo>> {
    let fmt = "%H%x00%h%x00%s%x00%an%x00%aI";
    let output = git(&["log", &format!("-{n}"), &format!("--format={fmt}")], repo)?;

    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\x00').collect();
            if parts.len() < 5 {
                return None;
            }
            Some(CommitInfo {
                hash: parts[0].to_string(),
                short_hash: parts[1].to_string(),
                message: parts[2].to_string(),
                author: parts[3].to_string(),
                timestamp: parts[4].to_string(),
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|c| Ok(c))
        .collect()
}
