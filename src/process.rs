use std::collections::HashMap;
use std::path::Path;
use std::process::{Child, Command};
use std::time::Instant;

pub struct ManagedProcess {
    child: Option<Child>,
    started_at: Option<Instant>,
}

impl ManagedProcess {
    pub fn spawn(cmd: &str, repo: &Path, envs: Option<&HashMap<String, String>>) -> anyhow::Result<Self> {
        let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
        let flag = if cfg!(target_os = "windows") { "/C" } else { "-c" };
        let mut c = Command::new(shell);
        // Prepend env vars via `env` to handle dotted names like spring.application.group
        let resolved = if let Some(envs) = envs {
            let prefix: String = envs.iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ");
            if prefix.is_empty() { cmd.to_string() } else { format!("env {prefix} {cmd}") }
        } else {
            cmd.to_string()
        };
        c.args([flag, &resolved]).current_dir(repo);
        let child = c.spawn()?;
        Ok(Self { child: Some(child), started_at: Some(Instant::now()) })
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.id())
    }

    pub fn uptime_secs(&self) -> Option<u64> {
        self.started_at.map(|t| t.elapsed().as_secs())
    }

    pub fn kill(&mut self) {
        if let Some(ref mut child) = self.child {
            // Kill process group to ensure subprocesses (e.g. java) die too
            #[cfg(unix)]
            unsafe {
                let pid = child.id() as i32;
                libc::killpg(pid, libc::SIGTERM);
                // Give a moment for graceful shutdown, then force
                std::thread::sleep(std::time::Duration::from_millis(500));
                libc::killpg(pid, libc::SIGKILL);
            }
            #[cfg(not(unix))]
            {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        self.child = None;
    }

    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(Some(_)) => {
                    self.child = None;
                    false
                }
                Ok(None) => true,
                Err(_) => {
                    self.child = None;
                    false
                }
            }
        } else {
            false
        }
    }
}

/// Parse host:port from a URL string. Handles http://, https://, or bare host:port/path.
fn parse_host_port(url: &str) -> Option<(&str, u16)> {
    let url = url.trim();
    let without_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let host_port = without_scheme.split('/').next()?;
    let mut parts = host_port.split(':');
    let host = parts.next()?;
    let port: u16 = parts.next().unwrap_or("80").parse().ok()?;
    Some((host, port))
}

/// Health check: try TCP connect to host:port from a URL string.
/// Returns true if connection succeeds within timeout_secs.
pub async fn health_check(url: &str, timeout_secs: u64) -> bool {
    let (host, port) = match parse_host_port(url) {
        Some(hp) => hp,
        None => return false,
    };

    let addr = format!("{host}:{port}");
    let deadline = tokio::time::Duration::from_secs(timeout_secs);
    let start = tokio::time::Instant::now();

    while start.elapsed() < deadline {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    false
}
