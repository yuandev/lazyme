use std::path::Path;
use std::process::{Child, Command};

pub struct ManagedProcess {
    child: Option<Child>,
}

impl ManagedProcess {
    pub fn spawn(cmd: &str, repo: &Path) -> anyhow::Result<Self> {
        let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
        let flag = if cfg!(target_os = "windows") { "/C" } else { "-c" };
        let child = Command::new(shell)
            .args([flag, cmd])
            .current_dir(repo)
            .spawn()?;
        Ok(Self { child: Some(child) })
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.id())
    }

    pub fn kill(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
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
