mod api;
mod config;
mod git;
mod process;
mod project;
mod queue;
mod registry;
mod self_update;
mod state;

use api::{AppState, TargetState};
use axum::{
    body::Body,
    http::{header, StatusCode},
    response::Response,
};
use clap::Parser;
use rust_embed::RustEmbed;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tracing::{error, info, warn};
use tokio::sync::broadcast;

#[derive(RustEmbed)]
#[folder = "frontend/dist/"]
struct Frontend;

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = config::Args::parse();

    info!("lazyme v{} starting...", self_update::CURRENT_VERSION);
    info!(
        "config: port={}, interval={}s, remote={}, filter={}",
        args.port,
        args.interval,
        args.remote,
        if args.filter.is_empty() { "all".to_string() } else { args.filter.join(",") }
    );

    // Self-update check (warn if newer version available)
    let parts: Vec<&str> = args.update_repo.split('/').collect();
    if parts.len() == 2 {
        info!("Checking for updates from {}/{}...", parts[0], parts[1]);
        match self_update::check(parts[0], parts[1]).await {
            Ok(Some(version)) => {
                warn!("lazyme update available: v{version}. POST /api/self-update to update.");
            }
            Ok(None) => {
                info!("lazyme is up to date");
            }
            Err(e) => {
                warn!("self-update check failed: {e}");
            }
        }
    }

    // Load target registry
    info!("Loading target registry from ~/.config/lazyme/targets.toml");
    let targets = registry::load()?;
    let targets = registry::filter(targets, &args.filter);
    if targets.is_empty() {
        anyhow::bail!("No targets found. Check ~/.config/lazyme/targets.toml");
    }
    info!("Loaded {} target(s)", targets.len());

    // Build per-target state
    let mut target_map = HashMap::new();
    for entry in &targets {
        let proj = project::ProjectConfig::load(&entry.name, &entry.repo)
            .unwrap_or_default()
            .unwrap_or_default();

        let branch = Mutex::new(proj.watch.branch.unwrap_or_else(|| "main".into()));
        let build_cmd = proj.build.command.unwrap_or_else(|| "cargo build --release".into());
        let artifact = proj.build.artifact.map(std::path::PathBuf::from);
        let run_cmd = proj.run.command;
        let health_url = proj.run.health_url;
        let health_timeout = proj.run.health_timeout;
        let jvm_args = proj.run.jvm_args;
        let envs = proj.env.map(|e| e.vars).unwrap_or_default();
        let auto_restart = proj.run.auto_restart.unwrap_or(false);
        let run_mode = proj.run.mode.unwrap_or_else(|| "deploy".into());
        let run_mode_display = run_mode.clone();

        let kill_timeout = proj.run.kill_timeout_secs;
        let build_timeout = proj.run.build_timeout_secs;
        let webhook_url = proj.run.webhook_url.clone();
        let pre_deploy_cmd = proj.run.pre_deploy_cmd.clone();
        let post_deploy_cmd = proj.run.post_deploy_cmd.clone();
        let ts = Arc::new(TargetState {
            name: entry.name.clone(),
            label: entry.label.clone().unwrap_or_else(|| entry.name.clone()),
            repo: entry.repo.clone(),
            remote: args.remote.clone(),
            branch,
            build_cmd,
            artifact,
            run_cmd,
            health_url,
            health_timeout,
            jvm_args: Mutex::new(jvm_args),
            envs: Mutex::new(envs),
            run_mode,
            process: Mutex::new(None),
            state: Mutex::new(state::StateManager::new(&entry.name)),
            profile: entry.profile.clone(),
            group: entry.group.clone(),
            auto_deploy_paused: Mutex::new(false),
            health_status: Mutex::new(None),
            auto_restart,
            kill_timeout_secs: kill_timeout,
            build_timeout_secs: build_timeout,
            webhook_url,
            pre_deploy_cmd,
            post_deploy_cmd,
            cached_local_head: Mutex::new(None),
            cached_remote_head: Mutex::new(None),
        });

        let branch_val = ts.branch();
        target_map.insert(entry.name.clone(), ts);
        info!(
            "Target '{}' registered (repo={}, branch={}, mode={}, group={})",
            entry.name,
            entry.repo.display(),
            branch_val,
            run_mode_display,
            entry.group.as_deref().unwrap_or("-"),
        );
    }

    let (tx, _rx) = broadcast::channel::<api::WsEvent>(64);
    let build_lock = Arc::new(queue::BuildLock::new());

    let shared = Arc::new(AppState {
        targets: std::sync::RwLock::new(target_map),
        interval: args.interval,
        tx,
        build_lock: build_lock.clone(),
        update_repo: args.update_repo.clone(),
        token: args.token.clone(),
    });

    // Start one poller per target, each with random jitter per tick
    let n = shared.targets.read().unwrap().len();
    info!("Starting {} poll loop(s)...", n);
    for target in shared.targets.read().unwrap().values() {
        let t = target.clone();
        let interval = args.interval;
        let tx = shared.tx.clone();
        let lock = build_lock.clone();
        let name = t.name.clone();
        tokio::spawn(async move {
            info!("[{name}] poll loop started (interval={interval}s)");
            poll_loop(t, interval, tx, lock).await;
        });
    }

    let app = api::router(shared).fallback(serve_frontend);

    // Try ports starting from the configured port, auto-increment if in use
    let mut port = args.port;
    let listener = loop {
        let addr = format!("0.0.0.0:{port}");
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => break l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                warn!("Port {port} in use, trying next...");
                port += 1;
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(e) => return Err(e.into()),
        }
    };
    info!("Web UI listening at http://localhost:{port}");
    axum::serve(listener, app).await?;

    Ok(())
}

/// Resolve the TCP port a target should be listening on.
/// Checks jvm_args (-Dserver.port=) first, then health_url.
fn resolve_target_port(target: &TargetState) -> Option<u16> {
    if let Some(s) = target.jvm_args.lock().unwrap().as_ref()
        .and_then(|ja| ja.split_whitespace()
            .find(|a| a.starts_with("-Dserver.port="))
            .and_then(|a| a.split('=').nth(1)))
    {
        if let Ok(p) = s.parse() { return Some(p); }
    }
    if let Some(ref url) = target.health_url {
        if let Some((_, p)) = process::parse_host_port(url) {
            return Some(p);
        }
    }
    None
}

pub async fn poll_loop(
    target: Arc<TargetState>,
    interval_secs: u64,
    tx: broadcast::Sender<api::WsEvent>,
    build_lock: Arc<queue::BuildLock>,
) {
    loop {
        // 60 ± random: add ±8s jitter per tick so no two targets sync up
        let jitter: i64 = rand::random::<u64>() as i64 % 17 - 8;
        let sleep_secs = (interval_secs as i64 + jitter).max(1) as u64;
        tokio::time::sleep(tokio::time::Duration::from_secs(sleep_secs)).await;

        // Recover process after restart: if handle is gone, detect via
        // multi-factor: (1) configured port, (2) artifact keyword
        {
            let mut proc = target.process.lock().unwrap();
            let gone = proc.as_mut().map_or(true, |p| !p.is_running());
            if gone {
                let recovered = resolve_target_port(&target)
                    .and_then(process::pid_by_port)
                    .or_else(|| {
                        target.artifact.as_ref().and_then(|a| {
                            let keyword = a.file_name()?.to_str()?;
                            process::pid_by_keyword(keyword)
                        })
                    });
                if let Some(pid) = recovered {
                    info!("[{}] recovered process (pid={pid})", target.name);
                    *proc = Some(process::ManagedProcess::from_recovered(pid));
                }
            }
        }

        // Periodic health check (runs every poll interval, not just on deploy)
        if let Some(ref url) = target.health_url {
            // Resolve port: actual PID port > jvm_args port > static URL
            let pid_for_port = {
                let proc = target.process.lock().unwrap();
                proc.as_ref().and_then(|p| p.pid())
            };
            let resolved_url = if let Some(port) = pid_for_port
                .and_then(|p| process::detect_port(p))
            {
                url.replace("{port}", &port.to_string())
            } else if let Some(port) = target.jvm_args.lock().unwrap().as_ref()
                .and_then(|ja| ja.split_whitespace()
                    .find(|a| a.starts_with("-Dserver.port="))
                    .and_then(|a| a.split('=').nth(1)))
            {
                url.replace("{port}", port)
            } else {
                url.clone()
            };
            let ok = process::health_check(&resolved_url, 5).await;
            *target.health_status.lock().unwrap() = Some(api::HealthStatus {
                ok,
                last_check: chrono::Utc::now().to_rfc3339(),
            });
        }

        let branch = target.branch();

        // Cache local head — use spawn_blocking to keep worker threads free for HTTP
        let repo = target.repo.clone();
        let repo2 = repo.clone();
        let remote = target.remote.clone();

        let lh = tokio::task::spawn_blocking(move || git::local_head(&repo)).await.unwrap_or_else(|e| {
            warn!("[{}] spawn_blocking local_head panicked: {e}", target.name);
            Err(anyhow::anyhow!("panic"))
        });
        if let Ok(ref h) = lh {
            *target.cached_local_head.lock().unwrap() = Some(h.clone());
        }

        let remote_head_result = {
            let repo = repo2.clone();
            let remote = remote.clone();
            let branch = branch.clone();
            let name = target.name.clone();
            match tokio::time::timeout(
                std::time::Duration::from_secs(20),
                tokio::task::spawn_blocking(move || {
                    git::remote_head(&repo, &remote, &branch)
                })
            ).await {
                Ok(Ok(Ok(h))) => Ok(h),
                Ok(Ok(Err(e))) => Err(e),
                Ok(Err(e)) => {
                    warn!("[{name}] spawn_blocking remote_head panicked: {e}");
                    Err(anyhow::anyhow!("panic"))
                }
                Err(_) => {
                    warn!("[{name}] remote_head timed out");
                    Err(anyhow::anyhow!("timeout"))
                }
            }
        };

        let remote_head = match remote_head_result {
            Ok(h) => {
                *target.cached_remote_head.lock().unwrap() = Some(h.clone());
                h
            }
            Err(e) => {
                warn!("[{}] remote check failed: {e}", target.name);
                continue;
            }
        };

        if remote_head.is_empty() {
            continue;
        }

        let deployed = {
            let st = target.state.lock().unwrap();
            st.current().as_ref().map(|r| r.commit_hash.clone())
        };

        // Auto-restart crashed process (independent of new commits)
        if target.auto_restart {
            let is_running = target.process.lock().unwrap().as_mut().map_or(false, |p| p.is_running());
            let has_deployed = deployed.is_some();
            if has_deployed && !is_running {
                warn!("[{}] process died, auto-restarting...", target.name);
                let jvm_args = target.jvm_args.lock().unwrap().clone();
                let envs = target.envs.lock().unwrap().clone();
                if let Some(ref run_cmd) = target.run_cmd {
                    let mut proc = target.process.lock().unwrap();
                    let mut resolved = if let Some(ref art) = target.artifact {
                        run_cmd.replace("{artifact}", &art.display().to_string())
                    } else {
                        run_cmd.to_string()
                    };
                    if let Some(ref ja) = jvm_args {
                        resolved = resolved.replace("{jvm_args}", ja);
                    }
                    match process::ManagedProcess::spawn(&resolved, &target.repo, Some(&envs)) {
                        Ok(p) => {
                            *proc = Some(p);
                            info!("[{}] auto-restarted", target.name);
                        }
                        Err(e) => error!("[{}] auto-restart failed: {e}", target.name),
                    }
                    drop(proc);
                }
            }
        }

        if deployed.as_deref() == Some(&remote_head) {
            continue;
        }

        // Skip if auto-deploy is paused
        if *target.auto_deploy_paused.lock().unwrap() {
            continue;
        }

        info!("[{}] new commit: {remote_head}, pulling...", target.name);

        {
            let repo = repo2.clone();
            let remote = remote.clone();
            let branch = branch.clone();
            let name = target.name.clone();
            let pull_result = match tokio::time::timeout(
                std::time::Duration::from_secs(30),
                tokio::task::spawn_blocking(move || {
                    git::pull(&repo, &remote, &branch)
                })
            ).await {
                Ok(Ok(Ok(()))) => Ok(()),
                Ok(Ok(Err(e))) => Err(e),
                Ok(Err(e)) => {
                    warn!("[{name}] spawn_blocking pull panicked: {e}");
                    Err(anyhow::anyhow!("panic"))
                }
                Err(_) => {
                    warn!("[{name}] git pull timed out");
                    Err(anyhow::anyhow!("timeout"))
                }
            };
            if let Err(e) = pull_result {
                error!("[{}] pull failed: {e}", target.name);
                continue;
            }
        }

        // Acquire build lock to serialize builds across targets
        let _guard = build_lock.inner.lock().await;
        build_lock.set_current(Some(target.name.clone()));

        let jvm_args = target.jvm_args.lock().unwrap().clone();
        let envs = target.envs.lock().unwrap().clone();

        if let Err(e) = api::build_and_cache(
            &target.repo,
            &target.remote,
            &branch,
            &target.build_cmd,
            target.artifact.as_deref(),
            &remote_head,
            &target.state,
            &target.process,
            target.run_cmd.as_deref(),
            target.health_url.as_deref(),
            target.health_timeout,
            target.kill_timeout_secs,
            target.build_timeout_secs,
            target.webhook_url.as_deref(),
            target.pre_deploy_cmd.as_deref(),
            target.post_deploy_cmd.as_deref(),
            jvm_args.as_deref(),
            Some(&envs),
            &target.run_mode,
            Some(&tx),
            &target.name,
            Some(&target.health_status),
        )
        .await
        {
            error!("[{}] build/deploy failed: {e}", target.name);
        } else {
            info!("[{}] deployed {remote_head}", target.name);
        }

        build_lock.set_current(None);
    }
}

async fn serve_frontend(uri: axum::http::Uri) -> Response<Body> {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if let Some(file) = Frontend::get(path) {
        let mime = mime_guess::from_path(&path).first_or_octet_stream();
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime.as_ref())
            .body(Body::from(file.data))
            .unwrap();
    }

    if let Some(file) = Frontend::get("index.html") {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html")
            .body(Body::from(file.data))
            .unwrap();
    }

    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("Not found"))
        .unwrap()
}
