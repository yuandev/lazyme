use anyhow::Context;
use std::path::Path;
use std::process::Command;

/// Returns the latest remote commit hash for the deployd repo.
fn remote_head(repo: &Path, branch: &str) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(["ls-remote", "--heads", "origin", branch])
        .current_dir(repo)
        .output()
        .context("git ls-remote")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.split_whitespace().next().unwrap_or("").to_string())
}

/// Returns the local HEAD hash.
fn local_head(repo: &Path) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .context("git rev-parse")?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Check if a newer version is available on the remote.
pub fn check(repo: &Path, branch: &str) -> anyhow::Result<Option<String>> {
    let remote = remote_head(repo, branch)?;
    let local = local_head(repo)?;
    if remote.is_empty() || remote == local {
        Ok(None)
    } else {
        Ok(Some(remote))
    }
}

/// Pull latest code and rebuild. Returns the new commit hash on success.
pub fn update(repo: &Path, branch: &str) -> anyhow::Result<String> {
    let status = Command::new("git")
        .args(["pull", "--ff-only", "origin", branch])
        .current_dir(repo)
        .status()
        .context("git pull")?;
    anyhow::ensure!(status.success(), "git pull failed");

    let new_hash = local_head(repo)?;

    let status = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(repo)
        .status()
        .context("cargo build --release")?;
    anyhow::ensure!(status.success(), "cargo build failed");

    Ok(new_hash)
}

/// Restart the current binary, replacing this process.
pub fn restart() -> ! {
    let exe = std::env::current_exe().expect("cannot find current exe");
    let mut cmd = Command::new(&exe);
    // Pass through the original args (skip the first arg which is the binary name)
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        cmd.args(&args);
    }
    // Detach from parent so the new process survives
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    match cmd.spawn() {
        Ok(_) => std::process::exit(0),
        Err(e) => {
            tracing::error!("Failed to spawn new process: {e}");
            std::process::exit(1);
        }
    }
}
