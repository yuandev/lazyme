use std::collections::HashMap;
use std::path::Path;
use std::process::{Child, Command};
use std::time::Instant;

pub struct ManagedProcess {
    child: Option<Child>,
    started_at: Option<Instant>,
}

impl Drop for ManagedProcess {
    fn drop(&mut self) {
        if self.child.is_some() {
            self.kill();
        }
    }
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
        self.kill_with_timeout(30);
    }

    /// Kill with a configurable graceful timeout in seconds.
    /// Sends SIGTERM, polls is_running every 500ms up to timeout_secs, then SIGKILL.
    pub fn kill_with_timeout(&mut self, timeout_secs: u64) {
        if let Some(ref mut child) = self.child {
            // Kill process group to ensure subprocesses (e.g. java) die too
            #[cfg(unix)]
            unsafe {
                let pid = child.id() as i32;
                libc::killpg(pid, libc::SIGTERM);
                // Poll for graceful shutdown
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
                loop {
                    match child.try_wait() {
                        Ok(Some(_)) => break, // process exited
                        Ok(None) => {
                            if std::time::Instant::now() >= deadline {
                                libc::killpg(pid, libc::SIGKILL);
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(500));
                        }
                        Err(_) => {
                            libc::killpg(pid, libc::SIGKILL);
                            break;
                        }
                    }
                }
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

/// Detect the actual listening TCP port for a given PID.
/// Linux: reads /proc/<pid>/net/tcp
/// macOS: uses lsof as fallback
pub fn detect_port(pid: u32) -> Option<u16> {
    // Linux: /proc/net/tcp
    if let Some(port) = detect_port_linux(pid) {
        return Some(port);
    }
    // macOS / BSD: lsof fallback
    if let Some(port) = detect_port_lsof(pid) {
        return Some(port);
    }
    None
}

fn detect_port_linux(pid: u32) -> Option<u16> {
    let path = format!("/proc/{pid}/net/tcp");
    let content = std::fs::read_to_string(&path).ok()?;
    for line in content.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 && parts[3] == "0A" {
            if let Some(addr) = parts.get(1) {
                if let Some(port_hex) = addr.split(':').nth(1) {
                    if let Ok(port) = u16::from_str_radix(port_hex, 16) {
                        return Some(port);
                    }
                }
            }
        }
    }
    None
}

fn detect_port_lsof(pid: u32) -> Option<u16> {
    let output = std::process::Command::new("lsof")
        .args(["-i", "-P", "-n", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() { return None; }
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Parse lines like: java  12345  user  123u  IPv6  0x...  TCP *:8080 (LISTEN)
    for line in stdout.lines() {
        if line.contains("(LISTEN)") {
            // Extract port from the last field, format: *:8080 or 127.0.0.1:8080
            if let Some(name) = line.split_whitespace().last() {
                // Remove the "(LISTEN)" suffix if present
                let addr = name.strip_suffix(" (LISTEN)").unwrap_or(name);
                if let Some(port_str) = addr.rsplit(':').next() {
                    if let Ok(port) = port_str.parse::<u16>() {
                        return Some(port);
                    }
                }
            }
        }
    }
    None
}
