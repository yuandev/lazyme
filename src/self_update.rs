use std::path::Path;
use std::process::Command;

/// Returns the latest remote commit hash for the deployd repo.
fn remote_head(repo: &Path, remote: &str, branch: &str) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(["ls-remote", "--heads", remote, branch])
        .current_dir(repo)
        .output()
        .map_err(|e| anyhow::anyhow!("git ls-remote: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git ls-remote failed: {stderr}");
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.split_whitespace().next().unwrap_or("").to_string())
}

/// Returns the local HEAD hash.
fn local_head(repo: &Path) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .map_err(|e| anyhow::anyhow!("git rev-parse: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git rev-parse failed: {stderr}");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Check if a newer version is available on the remote.
pub fn check(repo: &Path, remote: &str, branch: &str) -> anyhow::Result<Option<String>> {
    let remote_hash = remote_head(repo, remote, branch)?;
    let local = local_head(repo)?;
    if remote_hash.is_empty() || remote_hash == local {
        Ok(None)
    } else {
        Ok(Some(remote_hash))
    }
}

/// Pull latest code and rebuild. Returns the new commit hash on success.
pub fn update(repo: &Path, remote: &str, branch: &str) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(["pull", "--ff-only", remote, branch])
        .current_dir(repo)
        .output()
        .map_err(|e| anyhow::anyhow!("git pull: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git pull failed: {stderr}");
    }

    let new_hash = local_head(repo)?;

    let output = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(repo)
        .output()
        .map_err(|e| anyhow::anyhow!("cargo build: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("cargo build failed: {stderr}");
    }

    Ok(new_hash)
}

/// Restart the current binary, replacing this process.
pub fn restart() -> ! {
    let exe = std::env::current_exe().expect("cannot find current exe");
    let mut cmd = Command::new(&exe);
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        cmd.args(&args);
    }
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
