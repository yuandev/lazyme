use serde::Deserialize;
use std::io::Write;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const CURRENT_TARGET: &str = env!("TARGET");

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn asset_name() -> String {
    format!("deployd-{CURRENT_TARGET}")
}

/// Check if a newer release is available on GitHub.
pub async fn check(owner: &str, repo: &str) -> anyhow::Result<Option<String>> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
    let client = reqwest::Client::builder()
        .user_agent("deployd")
        .build()?;
    let release: Release = client.get(&url).send().await?.json().await?;
    let latest = release.tag_name.strip_prefix('v').unwrap_or(&release.tag_name);
    if latest != CURRENT_VERSION {
        Ok(Some(latest.to_string()))
    } else {
        Ok(None)
    }
}

/// Download the latest binary with progress, replace this binary.
pub async fn update_with_progress(
    owner: &str,
    repo: &str,
    on_progress: impl Fn(u64, u64) + Send + 'static,
) -> anyhow::Result<String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
    let client = reqwest::Client::builder()
        .user_agent("deployd")
        .build()?;
    let release: Release = client.get(&url).send().await?.json().await?;

    let name = asset_name();
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| anyhow::anyhow!("no asset found for {CURRENT_TARGET} (expected: {name})"))?;

    let current_exe = std::env::current_exe()?;
    let tmp = current_exe.with_extension("tmp");

    // Stream download with progress
    let mut resp = client.get(&asset.browser_download_url).send().await?;
    let total = resp.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(&tmp)?;
    let mut downloaded: u64 = 0;
    let mut buf = vec![0u8; 65536];

    while let Some(chunk) = resp.chunk().await? {
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        on_progress(downloaded, total);
    }
    drop(file);

    // Make executable on unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }

    std::fs::rename(&tmp, &current_exe)?;

    let new_version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name)
        .to_string();
    Ok(new_version)
}

/// Restart the current binary, replacing this process.
pub fn restart() -> ! {
    let exe = std::env::current_exe().expect("cannot find current exe");
    let mut cmd = std::process::Command::new(&exe);
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        cmd.args(&args);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
        cmd.stdin(std::process::Stdio::null());
    }
    tracing::info!("Spawning new process: {} {:?}", exe.display(), args);
    match cmd.spawn() {
        Ok(child) => {
            tracing::info!("New process spawned (pid={}), exiting", child.id());
            std::process::exit(0);
        }
        Err(e) => {
            tracing::error!("Failed to spawn new process: {e}");
            std::process::exit(1);
        }
    }
}
