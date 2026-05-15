use crate::git;
use crate::process::{self, ManagedProcess};
use crate::queue::BuildLock;
use crate::state::StateManager;
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post, delete},
    Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
};
use tokio::sync::broadcast;

pub type SharedState = Arc<AppState>;

pub struct AppState {
    pub targets: RwLock<HashMap<String, Arc<TargetState>>>,
    pub interval: u64,
    pub tx: broadcast::Sender<WsEvent>,
    pub build_lock: Arc<BuildLock>,
    pub update_repo: String,
}

/// Per-target configuration and state.
pub struct TargetState {
    pub name: String,
    pub label: String,
    pub repo: PathBuf,
    pub remote: String,
    pub branch: Mutex<String>,
    pub build_cmd: String,
    pub artifact: Option<PathBuf>,
    pub run_cmd: Option<String>,
    pub health_url: Option<String>,
    pub health_timeout: u64,
    pub jvm_args: Mutex<Option<String>>,
    pub envs: Mutex<HashMap<String, String>>,
    pub run_mode: String,
    pub state: Mutex<StateManager>,
    pub process: Mutex<Option<ManagedProcess>>,
    pub profile: Option<String>,
    pub group: Option<String>,
    pub auto_deploy_paused: Mutex<bool>,
    pub health_status: Mutex<Option<HealthStatus>>,
    pub auto_restart: bool,
    pub cached_remote_head: Mutex<Option<String>>,
    pub cached_local_head: Mutex<Option<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthStatus {
    pub ok: bool,
    pub last_check: String,
}

fn detect_service_type(run_cmd: Option<&str>) -> String {
    let cmd = run_cmd.unwrap_or("").to_lowercase();
    if cmd.contains("java") || cmd.contains("mvn") || cmd.contains("gradle") {"Java".into()}
    else if cmd.contains("node") || cmd.contains("npm") || cmd.contains("yarn") || cmd.contains("pnpm") {"Node".into()}
    else if cmd.contains("python") || cmd.contains("uvicorn") || cmd.contains("gunicorn") {"Python".into()}
    else if cmd.contains("go") || cmd.contains("cargo") {"Go/Rust".into()}
    else {"?".into()}
}

impl TargetState {
    pub fn branch(&self) -> String {
        self.branch.lock().unwrap().clone()
    }
}

// ── WebSocket event ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsEvent {
    pub event: String,
    pub target: String,
    pub commit: Option<String>,
    pub message: Option<String>,
}

// ── Response types ──

#[derive(Serialize)]
struct TargetSummary {
    name: String,
    label: String,
    repo: String,
    branch: String,
    deployed: Option<crate::state::DeployRecord>,
    local_commit: Option<String>,
    remote_commit: Option<String>,
    process_running: bool,
    health_url: Option<String>,
    group: Option<String>,
    service_type: String,
    health_ok: Option<bool>,
}

#[derive(Serialize)]
struct TargetListResponse {
    targets: Vec<TargetSummary>,
}

#[derive(Serialize)]
struct StatusResponse {
    name: String,
    label: String,
    repo: String,
    branch: String,
    deployed: Option<crate::state::DeployRecord>,
    local_commit: Option<String>,
    remote_commit: Option<String>,
    interval_secs: u64,
    process_running: bool,
    health_url: Option<String>,
    build_cmd: String,
    run_cmd: Option<String>,
    run_mode: String,
    jvm_args: Option<String>,
    envs: HashMap<String, String>,
    auto_deploy_paused: bool,
    group: Option<String>,
    service_type: String,
    pid: Option<u32>,
    uptime_secs: Option<u64>,
    health_status: Option<HealthStatus>,
}

#[derive(Serialize)]
struct CommitsResponse {
    commits: Vec<git::CommitInfo>,
}

#[derive(Serialize)]
struct HistoryResponse {
    history: Vec<crate::state::DeployRecord>,
}

#[derive(Serialize)]
struct RollbackResponse {
    status: String,
    commit: String,
}

#[derive(Serialize)]
struct LogResponse {
    target: String,
    hash: String,
    content: String,
}

// ── WebSocket handler ──

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(s): State<SharedState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, s))
}

async fn handle_socket(mut socket: WebSocket, state: SharedState) {
    let mut rx = state.tx.subscribe();
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(event) => {
                        if let Ok(json) = serde_json::to_string(&event) {
                            if socket.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // Skip missed messages, resubscribe
                        rx = state.tx.subscribe();
                    }
                    Err(_) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(_)) => {} // ignore client messages
                    _ => break,
                }
            }
        }
    }
}

// ── API handlers ──

/// GET /api/targets
async fn list_targets(State(s): State<SharedState>) -> Json<TargetListResponse> {
    let targets = s.targets.read().unwrap();

    // Quick parallel health checks — online services respond <1ms
    let health_futures: Vec<_> = targets.iter().map(|(_, t)| {
        let t = t.clone();
        tokio::spawn(async move {
            let url = t.health_url.clone();
            let jvm_args = t.jvm_args.lock().unwrap().clone();
            if let Some(ref url) = url {
                let resolved = jvm_args.as_ref()
                    .and_then(|ja| ja.split_whitespace()
                        .find(|a| a.starts_with("-Dserver.port="))
                        .and_then(|a| a.split('=').nth(1)))
                    .map(|p| url.replace("{port}", p))
                    .unwrap_or_else(|| url.clone());
                let ok = process::health_check(&resolved, 2).await;
                *t.health_status.lock().unwrap() = Some(HealthStatus { ok, last_check: chrono::Utc::now().to_rfc3339() });
            }
        })
    }).collect();
    // Don't wait — fire and forget, results used on next refresh
    drop(health_futures);

    let summaries: Vec<TargetSummary> = targets
        .iter()
        .map(|(_, t)| {
            let st = t.state.lock().unwrap();
            let running = t.process.lock().unwrap().as_mut().map_or(false, |p| p.is_running());
            let health_ok = t.health_status.lock().unwrap().as_ref().map(|hs| hs.ok);
            TargetSummary {
                name: t.name.clone(),
                label: t.label.clone(),
                repo: t.repo.display().to_string(),
                branch: t.branch(),
                deployed: st.current().clone(),
                local_commit: t.cached_local_head.lock().unwrap().clone(),
                remote_commit: t.cached_remote_head.lock().unwrap().clone(),
                process_running: running,
                health_url: t.health_url.clone(),
                group: t.group.clone(),
                service_type: detect_service_type(t.run_cmd.as_deref()),
                health_ok,
            }
        })
        .collect();
    Json(TargetListResponse { targets: summaries })
}

/// GET /api/targets/:name/status
async fn target_status(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<StatusResponse>, StatusCode> {
    let t = s.targets.read().unwrap().get(&name).cloned().ok_or(StatusCode::NOT_FOUND)?;
    let deployed = { t.state.lock().unwrap().current().clone() };
    let cached_local = t.cached_local_head.lock().unwrap().clone();
    let cached_remote = t.cached_remote_head.lock().unwrap().clone();
    let (running, pid, uptime) = {
        let mut proc = t.process.lock().unwrap();
        if let Some(ref mut p) = *proc {
            let alive = p.is_running();
            if alive { (true, p.pid(), p.uptime_secs()) }
            else { (false, None, None) }
        } else {
            (false, None, None)
        }
    };
    // On-demand health check when viewing target detail
    let health_status = if let Some(ref url) = t.health_url {
        let resolved = {
            let proc = t.process.lock().unwrap();
            let pid = proc.as_ref().and_then(|p| p.pid());
            drop(proc);
            if let Some(port) = pid.and_then(|p| crate::process::detect_port(p)) {
                url.replace("{port}", &port.to_string())
            } else if let Some(port) = t.jvm_args.lock().unwrap().as_ref()
                .and_then(|ja| ja.split_whitespace()
                    .find(|a| a.starts_with("-Dserver.port="))
                    .and_then(|a| a.split('=').nth(1)))
            {
                url.replace("{port}", port)
            } else {
                url.clone()
            }
        };
        let ok = crate::process::health_check(&resolved, 3).await;
        let hs = Some(HealthStatus { ok, last_check: chrono::Utc::now().to_rfc3339() });
        *t.health_status.lock().unwrap() = hs.clone();
        hs
    } else {
        t.health_status.lock().unwrap().clone()
    };
    let jvm_args = t.jvm_args.lock().unwrap().clone();
    let envs = t.envs.lock().unwrap().clone();
    let auto_deploy_paused = *t.auto_deploy_paused.lock().unwrap();
    Ok(Json(StatusResponse {
        name: t.name.clone(),
        label: t.label.clone(),
        repo: t.repo.display().to_string(),
        branch: t.branch(),
        deployed,
        local_commit: cached_local.clone(),
        remote_commit: cached_remote.clone(),
        interval_secs: s.interval,
        process_running: running,
        health_url: t.health_url.clone(),
        build_cmd: t.build_cmd.clone(),
        run_cmd: t.run_cmd.clone(),
        run_mode: t.run_mode.clone(),
        jvm_args,
        envs,
        auto_deploy_paused,
        group: t.group.clone(),
        service_type: detect_service_type(t.run_cmd.as_deref()),
        pid,
        uptime_secs: uptime,
        health_status,
    }))
}

/// GET /api/targets/:name/commits
async fn target_commits(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<CommitsResponse>, StatusCode> {
    let t = s.targets.read().unwrap().get(&name).cloned().ok_or(StatusCode::NOT_FOUND)?;
    git::recent_commits(&t.repo, 20)
        .map(|commits| Json(CommitsResponse { commits }))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// GET /api/targets/:name/history
async fn target_history(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<HistoryResponse>, StatusCode> {
    let t = s.targets.read().unwrap().get(&name).cloned().ok_or(StatusCode::NOT_FOUND)?;
    let st = t.state.lock().unwrap();
    Ok(Json(HistoryResponse {
        history: st.history().to_vec(),
    }))
}

/// GET /api/targets/:name/logs/:hash
async fn target_logs(
    State(s): State<SharedState>,
    Path((name, hash)): Path<(String, String)>,
) -> Result<Json<LogResponse>, (StatusCode, String)> {
    let t = s
        .targets
        .read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;
    let log_path = t
        .repo
        .join(".deployd")
        .join("logs")
        .join(format!("{hash}.log"));
    let content =
        std::fs::read_to_string(&log_path).map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(LogResponse {
        target: name,
        hash,
        content,
    }))
}

#[derive(Deserialize)]
struct RollbackBody {
    commit: String,
}

/// POST /api/targets/:name/rollback
async fn target_rollback(
    State(s): State<SharedState>,
    Path(name): Path<String>,
    Json(body): Json<RollbackBody>,
) -> Result<Json<RollbackResponse>, (StatusCode, String)> {
    let t = s
        .targets
        .read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    git::checkout(&t.repo, &body.commit)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let artifact = t.artifact.as_deref();
    let run = t.run_cmd.as_deref();
    let health = t.health_url.as_deref();
    let health_to = t.health_timeout;

    let _ = s.tx.send(WsEvent {
        event: "rollback_started".into(),
        target: t.name.clone(),
        commit: Some(body.commit.clone()),
        message: None,
    });

    // Serialize builds
    let _guard = s.build_lock.inner.lock().await;
    s.build_lock.set_current(Some(t.name.clone()));

    let jvm_args = t.jvm_args.lock().unwrap().clone();
    let envs = t.envs.lock().unwrap().clone();

    let used_cache = try_rollback_with_cache(
        &t.repo, &body.commit, artifact, &t.state, &t.process, run, health, health_to,
        jvm_args.as_deref(),
        Some(&envs),
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !used_cache {
        let branch = t.branch();
        build_and_cache(
            &t.repo, &t.remote, &branch,
            &t.build_cmd, artifact,
            &body.commit, &t.state, &t.process, run, health, health_to,
            jvm_args.as_deref(),
            Some(&envs),
            &t.run_mode,
            Some(&s.tx), &t.name,
            Some(&t.health_status),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    s.build_lock.set_current(None);
    drop(_guard);

    // Pause auto-deploy when manually rolling back to a specific commit
    *t.auto_deploy_paused.lock().unwrap() = true;

    let _ = s.tx.send(WsEvent {
        event: "rollback_complete".into(),
        target: t.name.clone(),
        commit: Some(body.commit.clone()),
        message: None,
    });

    Ok(Json(RollbackResponse {
        status: "ok".into(),
        commit: body.commit,
    }))
}

/// POST /api/targets/:name/deploy
async fn target_deploy(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<RollbackResponse>, (StatusCode, String)> {
    let t = s
        .targets
        .read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    let branch = t.branch();
    let remote = git::remote_head(&t.repo, &t.remote, &branch)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    git::pull(&t.repo, &t.remote, &branch)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _ = s.tx.send(WsEvent {
        event: "deploy_started".into(),
        target: t.name.clone(),
        commit: Some(remote.clone()),
        message: None,
    });

    // Serialize builds
    let _guard = s.build_lock.inner.lock().await;
    s.build_lock.set_current(Some(t.name.clone()));

    let jvm_args = t.jvm_args.lock().unwrap().clone();
    let envs = t.envs.lock().unwrap().clone();

    build_and_cache(
        &t.repo, &t.remote, &branch,
        &t.build_cmd, t.artifact.as_deref(),
        &remote, &t.state, &t.process,
        t.run_cmd.as_deref(), t.health_url.as_deref(), t.health_timeout,
        jvm_args.as_deref(),
        Some(&envs),
        &t.run_mode,
        Some(&s.tx), &t.name,
        Some(&t.health_status),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    s.build_lock.set_current(None);
    drop(_guard);

    // Resume auto-deploy when deploying latest
    *t.auto_deploy_paused.lock().unwrap() = false;

    let _ = s.tx.send(WsEvent {
        event: "deploy_complete".into(),
        target: t.name.clone(),
        commit: Some(remote.clone()),
        message: None,
    });

    Ok(Json(RollbackResponse {
        status: "ok".into(),
        commit: remote,
    }))
}

// ── Build & Cache (shared) ──

pub async fn build_and_cache(
    repo: &std::path::Path,
    _remote: &str,
    _branch: &str,
    build_cmd: &str,
    artifact_rel: Option<&std::path::Path>,
    commit_hash: &str,
    state: &Mutex<StateManager>,
    process: &Mutex<Option<ManagedProcess>>,
    run_cmd: Option<&str>,
    health_url: Option<&str>,
    health_timeout: u64,
    jvm_args: Option<&str>,
    envs: Option<&HashMap<String, String>>,
    run_mode: &str,
    tx: Option<&broadcast::Sender<WsEvent>>,
    target_name: &str,
    health_status: Option<&Mutex<Option<HealthStatus>>>,
) -> anyhow::Result<()> {
    use std::process::Command;

    let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
    let flag = if cfg!(target_os = "windows") { "/C" } else { "-c" };

    let short = crate::git::short_hash(repo, commit_hash)
        .unwrap_or_else(|_| commit_hash[..7].to_string());

    let is_dev = run_mode == "dev";

    // Broadcast build started (skip build in dev mode)
    if !is_dev {
        if let Some(tx) = tx {
            let _ = tx.send(WsEvent {
                event: "build_started".into(),
                target: target_name.into(),
                commit: Some(short.clone()),
                message: None,
            });
        }
    }

    // Build step — skipped in dev mode
    let build_start = std::time::Instant::now();
    let (success, log_path, cache_path) = if is_dev {
        (true, None, None)
    } else {
        // Resolve build command placeholders
        let resolved_build = crate::project::ProjectConfig::load(target_name, repo)
            .ok()
            .flatten()
            .map(|c| {
                let mut cmd = build_cmd.to_string();
                if let Some(ref s) = c.build.maven_settings {
                    cmd = cmd.replace("{maven_settings}", s);
                }
                if let Some(ref s) = c.build.local_repo {
                    cmd = cmd.replace("{local_repo}", s);
                }
                cmd
            })
            .unwrap_or_else(|| build_cmd.to_string());

        // Send build log start event
        if let Some(tx) = tx {
            let _ = tx.send(WsEvent {
                event: "build_log_start".into(),
                target: target_name.into(),
                commit: Some(short.clone()),
                message: None,
            });
        }

        use std::io::{BufRead, BufReader};
        let mut child = Command::new(shell)
            .args([flag, &resolved_build])
            .current_dir(repo)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        // Read stdout line by line, broadcast and buffer
        let stdout = child.stdout.take().unwrap();
        let reader = BufReader::new(stdout);
        let mut log_buf = String::new();
        for line_res in reader.lines() {
            let line = line_res.unwrap_or_default();
            if let Some(tx) = tx {
                let _ = tx.send(WsEvent {
                    event: "build_output".into(),
                    target: target_name.into(),
                    commit: Some(short.clone()),
                    message: Some(line.clone()),
                });
            }
            log_buf.push_str(&line);
            log_buf.push('\n');
        }
        // Also read stderr
        if let Some(stderr) = child.stderr.take() {
            let err_reader = BufReader::new(stderr);
            for line_res in err_reader.lines() {
                let line = line_res.unwrap_or_default();
                if let Some(tx) = tx {
                    let _ = tx.send(WsEvent {
                        event: "build_output".into(),
                        target: target_name.into(),
                        commit: Some(short.clone()),
                        message: Some(line.clone()),
                    });
                }
                log_buf.push_str(&line);
                log_buf.push('\n');
            }
        }

        let status = child.wait()?;
        let success = status.success();

        // Send build log end event
        if let Some(tx) = tx {
            let _ = tx.send(WsEvent {
                event: "build_log_end".into(),
                target: target_name.into(),
                commit: Some(short.clone()),
                message: Some(format!("success={success}")),
            });
        }

        // Persist build log
        let log_dir = repo.join(".deployd").join("logs");
        let _ = std::fs::create_dir_all(&log_dir);
        let log_path = log_dir.join(format!("{short}.log"));
        let _ = std::fs::write(&log_path, &log_buf);
        let log_path = if log_path.exists() { Some(log_path) } else { None };

        let cache_path = if success {
            if let Some(artifact) = artifact_rel {
                let st = state.lock().unwrap();
                st.cache_artifact(&short, artifact).ok()
            } else {
                None
            }
        } else {
            None
        };

        (success, log_path, cache_path)
    };

    // Kill old process, start new one
    // Extract port from jvm_args for {port} placeholder
    let port = jvm_args
        .and_then(|ja| {
            ja.split_whitespace()
                .find(|a| a.starts_with("-Dserver.port="))
                .and_then(|a| a.split('=').nth(1))
                .map(|s| s.to_string())
        });

    if success {
        if let Some(ref cmd) = run_cmd {
            let mut proc = process.lock().unwrap();
            if let Some(ref mut old) = *proc {
                old.kill();
            }
            let mut resolved = if let Some(art) = artifact_rel {
                cmd.replace("{artifact}", &art.display().to_string())
            } else {
                cmd.to_string()
            };
            if let Some(ref ja) = jvm_args {
                resolved = resolved.replace("{jvm_args}", ja);
            }
            if let Some(ref p) = port {
                resolved = resolved.replace("{port}", p);
            }
            match ManagedProcess::spawn(&resolved, repo, envs) {
                Ok(p) => *proc = Some(p),
                Err(e) => tracing::warn!("Failed to spawn run command: {e}"),
            }
            drop(proc);
        }
    }

    // Broadcast build complete
    if let Some(tx) = tx {
        let _ = tx.send(WsEvent {
            event: "build_complete".into(),
            target: target_name.into(),
            commit: Some(short.clone()),
            message: Some(format!("success={success}")),
        });
    }

    // Health check — resolve {port} placeholder
    let hc_ok = if success && run_cmd.is_some() {
        if let Some(url) = health_url {
            let resolved_url = port.as_ref()
                .map(|p| url.replace("{port}", p))
                .unwrap_or_else(|| url.to_string());
            let ok = process::health_check(&resolved_url, health_timeout).await;
            if !ok {
                tracing::warn!("Health check failed for {resolved_url}");
            }
            ok
        } else { true }
    } else { false };

    if let Some(hs) = health_status {
        *hs.lock().unwrap() = Some(HealthStatus {
            ok: hc_ok,
            last_check: chrono::Utc::now().to_rfc3339(),
        });
    }

    let mut st = state.lock().unwrap();
    let duration = if is_dev { None } else { Some(build_start.elapsed().as_secs()) };
    st.record_deploy(
        commit_hash.to_string(),
        short,
        cache_path,
        log_path,
        success,
        duration,
    )?;

    Ok(())
}

pub fn try_rollback_with_cache(
    repo: &std::path::Path,
    commit_hash: &str,
    artifact_rel: Option<&std::path::Path>,
    state: &Mutex<StateManager>,
    process: &Mutex<Option<ManagedProcess>>,
    run_cmd: Option<&str>,
    _health_url: Option<&str>,
    _health_timeout: u64,
    jvm_args: Option<&str>,
    envs: Option<&HashMap<String, String>>,
) -> anyhow::Result<bool> {
    let st = state.lock().unwrap();
    let short = crate::git::short_hash(repo, commit_hash)
        .unwrap_or_else(|_| commit_hash[..7].to_string());

    let cache_hit = artifact_rel.and_then(|a| {
        let fname = a.file_name()?;
        st.find_cached_artifact(&short, fname.to_str()?)
    });

    if let Some(cache_path) = cache_hit {
        drop(st);

        // Kill old process, start new one
        if let Some(ref cmd) = run_cmd {
            let mut proc = process.lock().unwrap();
            if let Some(ref mut old) = *proc {
                old.kill();
            }
            let mut resolved = if let Some(art) = artifact_rel {
                cmd.replace("{artifact}", &art.display().to_string())
            } else {
                cmd.to_string()
            };
            if let Some(ref ja) = jvm_args {
                resolved = resolved.replace("{jvm_args}", ja);
            }
            match ManagedProcess::spawn(&resolved, repo, envs) {
                Ok(p) => *proc = Some(p),
                Err(e) => tracing::warn!("Failed to spawn run command: {e}"),
            }
            drop(proc);
        }

        let mut st = state.lock().unwrap();
        st.record_deploy(
            commit_hash.to_string(),
            short,
            Some(cache_path),
            None,
            true,
            None,
        )?;
        Ok(true)
    } else {
        Ok(false)
    }
}

// ── Branch switch ──

#[derive(Deserialize)]
struct BranchBody {
    branch: String,
}

/// POST /api/targets/:name/branch
async fn target_set_branch(
    State(s): State<SharedState>,
    Path(name): Path<String>,
    Json(body): Json<BranchBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let t = s
        .targets
        .read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    let output = std::process::Command::new("git")
        .args([
            "rev-parse",
            "--verify",
            &format!("refs/heads/{}", body.branch),
        ])
        .current_dir(&t.repo)
        .output()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !output.status.success() {
        // Create local tracking branch from remote
        let result = std::process::Command::new("git")
            .args(["branch", &body.branch, &format!("origin/{}", body.branch)])
            .current_dir(&t.repo)
            .output()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err((
                StatusCode::BAD_REQUEST,
                format!("branch '{}' not found: {}", body.branch, stderr.trim()),
            ));
        }
        // Checkout with stash protection
        git::checkout(&t.repo, &body.branch)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("checkout failed: {e}")))?;
    }

    *t.branch.lock().unwrap() = body.branch.clone();

    // Persist to project config
    if let Err(e) = crate::project::ProjectConfig::save_branch(&t.name, &body.branch) {
        tracing::warn!("Failed to save branch to config: {e}");
    }

    let _ = s.tx.send(WsEvent {
        event: "targets_changed".into(),
        target: t.name.clone(),
        commit: None,
        message: None,
    });

    Ok(Json(serde_json::json!({"status": "ok", "branch": t.branch()})))
}

// ── Branch list ──

#[derive(Serialize)]
struct BranchesResponse {
    branches: Vec<String>,
    current: String,
}

/// GET /api/targets/{name}/branches
async fn target_branches(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<BranchesResponse>, (StatusCode, String)> {
    let t = s
        .targets
        .read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    let output = std::process::Command::new("git")
        .args(["branch", "-r"])
        .current_dir(&t.repo)
        .output()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let branches: Vec<String> = stdout
        .lines()
        .map(|l| l.trim().trim_start_matches(|c: char| c == '*' || c.is_whitespace()))
        .filter(|b| !b.is_empty() && !b.contains("->"))
        .map(|b| b.trim_start_matches("origin/").to_string())
        .collect();

    Ok(Json(BranchesResponse {
        branches,
        current: t.branch(),
    }))
}

/// POST /api/targets/{name}/fetch
async fn target_fetch(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let t = s
        .targets
        .read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    let branch = t.branch();
    let remote = git::remote_head(&t.repo, &t.remote, &branch)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    git::pull(&t.repo, &t.remote, &branch)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _ = s.tx.send(WsEvent {
        event: "targets_changed".into(),
        target: t.name.clone(),
        commit: Some(remote.clone()),
        message: None,
    });

    Ok(Json(serde_json::json!({
        "status": "ok",
        "remote_head": remote,
    })))
}

// ── Clone target ──

#[derive(Deserialize)]
struct CloneBody {
    new_name: String,
    repo: Option<String>,
    #[serde(default)]
    group: Option<String>,
}

fn increment_port(s: &str) -> String {
    // Find the last port-like number (1024-65534) and increment it
    let bytes = s.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i].is_ascii_digit() {
            let end = i + 1;
            while i > 0 && bytes[i - 1].is_ascii_digit() {
                i -= 1;
            }
            if let Ok(port) = s[i..end].parse::<u16>() {
                if port >= 1024 && port < 65535 {
                    let mut result = s[..i].to_string();
                    result.push_str(&(port + 1).to_string());
                    result.push_str(&s[end..]);
                    return result;
                }
            }
            break; // only try the last number
        }
    }
    s.to_string()
}

/// POST /api/targets/{name}/clone
async fn target_clone(
    State(s): State<SharedState>,
    Path(name): Path<String>,
    Json(body): Json<CloneBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let new_name = body.new_name.trim().to_string();
    if new_name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "new_name is required".into()));
    }

    let source = s.targets.read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "source target not found".into()))?;

    if s.targets.read().unwrap().contains_key(&new_name) {
        return Err((StatusCode::CONFLICT, format!("target '{new_name}' already exists")));
    }

    // Use custom repo path or fall back to source repo
    let repo = if let Some(ref custom_repo) = body.repo {
        let p = PathBuf::from(custom_repo.trim());
        if p == source.repo {
            source.repo.clone()
        } else {
            // Create directory if needed; init git if not already a repo
            std::fs::create_dir_all(&p)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let git_dir = p.join(".git");
            if !git_dir.exists() {
                let branch = source.branch();
                std::process::Command::new("git")
                    .args(["clone", "--branch", &branch, &source.repo.display().to_string(), &p.display().to_string()])
                    .output()
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            }
            p
        }
    } else {
        source.repo.clone()
    };

    // Build cloned config with auto-incremented ports
    let cloned_run_cmd = source.run_cmd.as_ref().map(|c| increment_port(c));
    let cloned_health_url = source.health_url.as_ref().map(|u| increment_port(u));
    let cloned_jvm_args = source.jvm_args.lock().unwrap().as_ref().map(|j| increment_port(j));

    // Write new profile config
    let profile = Some(new_name.clone());
    let mut table = toml::Table::new();
    {
        let mut build = toml::Table::new();
        build.insert("command".into(), toml::Value::String(source.build_cmd.clone()));
        if let Some(ref art) = source.artifact {
            build.insert("artifact".into(), toml::Value::String(art.display().to_string()));
        }
        table.insert("build".into(), toml::Value::Table(build));
    }
    {
        let mut run = toml::Table::new();
        if let Some(ref cmd) = cloned_run_cmd {
            run.insert("command".into(), toml::Value::String(cmd.clone()));
        }
        if let Some(ref url) = cloned_health_url {
            run.insert("health_url".into(), toml::Value::String(url.clone()));
        }
        if source.health_timeout != 30 {
            run.insert("health_timeout".into(), toml::Value::Integer(source.health_timeout as i64));
        }
        if let Some(ref ja) = cloned_jvm_args {
            run.insert("jvm_args".into(), toml::Value::String(ja.clone()));
        }
        let source_envs = source.envs.lock().unwrap();
        if !source_envs.is_empty() {
            let mut env = toml::Table::new();
            for (k, v) in source_envs.iter() {
                env.insert(k.clone(), toml::Value::String(v.clone()));
            }
            table.insert("env".into(), toml::Value::Table(env));
        }
        drop(source_envs);
        run.insert("mode".into(), toml::Value::String(source.run_mode.clone()));
        table.insert("run".into(), toml::Value::Table(run));
    }
    {
        let mut watch = toml::Table::new();
        watch.insert("branch".into(), toml::Value::String(source.branch()));
        table.insert("watch".into(), toml::Value::Table(watch));
    }

    let config_path = repo.join(".deployd").join(format!("config.{new_name}.toml"));
    std::fs::create_dir_all(config_path.parent().unwrap())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    std::fs::write(&config_path, toml::to_string_pretty(&table).unwrap())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Append to registry
    let entry = crate::registry::TargetEntry {
        name: new_name.clone(),
        repo: repo.clone(),
        profile,
        group: body.group.clone().or_else(|| source.group.clone()),
        label: None,
    };
    crate::registry::append_entry(&entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Create new TargetState and insert
    let ts = Arc::new(TargetState {
        name: new_name.clone(),
        label: new_name.clone(),
        repo: repo.clone(),
        remote: source.remote.clone(),
        branch: Mutex::new(source.branch()),
        build_cmd: source.build_cmd.clone(),
        artifact: source.artifact.clone(),
        run_cmd: cloned_run_cmd,
        health_url: cloned_health_url,
        health_timeout: source.health_timeout,
        jvm_args: Mutex::new(cloned_jvm_args),
        envs: Mutex::new(source.envs.lock().unwrap().clone()),
        run_mode: source.run_mode.clone(),
        process: Mutex::new(None),
        state: Mutex::new(crate::state::StateManager::new(&repo)),
        profile: Some(new_name.clone()),
        group: body.group.clone().or_else(|| source.group.clone()),
        auto_deploy_paused: Mutex::new(false),
        health_status: Mutex::new(None),
        cached_local_head: Mutex::new(None),
        cached_remote_head: Mutex::new(None),
        auto_restart: source.auto_restart,
    });

    s.targets.write().unwrap().insert(new_name.clone(), ts.clone());

    // Spawn poll loop for the new target
    let interval = s.interval;
    let tx = s.tx.clone();
    let lock = s.build_lock.clone();
    tokio::spawn(async move { crate::poll_loop(ts, interval, tx, lock).await });

    let _ = s.tx.send(WsEvent {
        event: "targets_changed".into(),
        target: new_name.clone(),
        commit: None,
        message: None,
    });

    Ok(Json(serde_json::json!({"status": "ok", "name": new_name})))
}

// ── Config reload ──

/// POST /api/reload — reload project configs and registry
async fn reload_config(
    State(s): State<SharedState>,
) -> Json<serde_json::Value> {
    let mut updated = 0u32;
    let mut new_names: Vec<String> = Vec::new();

    // Reload registry to discover new targets
    if let Ok(registry) = crate::registry::load() {
        let targets = crate::registry::filter(registry, &[]);
        for entry in &targets {
            if let Some(t) = s.targets.read().unwrap().get(&entry.name) {
                // Reload project config for existing target
                if let Ok(Some(proj)) =
                    crate::project::ProjectConfig::load(&entry.name, &entry.repo)
                {
                    if let Some(b) = proj.watch.branch {
                        *t.branch.lock().unwrap() = b;
                        updated += 1;
                    }
                }
            } else {
                new_names.push(entry.name.clone());
            }
        }
    }

    Json(serde_json::json!({
        "status": "ok",
        "branches_updated": updated,
        "new_targets": new_names,
        "hint": "restart to pick up new targets or build/run config changes"
    }))
}

// ── Stop service ──

/// POST /api/targets/{name}/stop — stop running process
async fn target_stop(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let t = s.targets.read().unwrap().get(&name).cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;
    let mut proc = t.process.lock().unwrap();
    if let Some(ref mut p) = *proc { p.kill(); }
    *proc = None; drop(proc);
    *t.health_status.lock().unwrap() = None;
    let _ = s.tx.send(WsEvent { event: "targets_changed".into(), target: name.clone(), commit: None, message: None });
    Ok(Json(serde_json::json!({"status": "ok", "stopped": true})))
}

// ── Auto-deploy toggle ──

/// POST /api/targets/{name}/auto-deploy — toggle auto-deploy pause/resume
async fn target_auto_deploy(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let t = s
        .targets
        .read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    let mut paused = t.auto_deploy_paused.lock().unwrap();
    *paused = !*paused;
    let is_paused = *paused;
    drop(paused);

    let _ = s.tx.send(WsEvent {
        event: "auto_deploy_toggled".into(),
        target: name.clone(),
        commit: None,
        message: Some(if is_paused { "paused".into() } else { "resumed".into() }),
    });

    Ok(Json(serde_json::json!({
        "status": "ok",
        "target": name,
        "auto_deploy_paused": is_paused,
    })))
}

// ── Queue status ──

#[derive(Serialize)]
struct QueueResponse {
    building: Option<String>,
}

async fn queue_status(State(s): State<SharedState>) -> Json<QueueResponse> {
    let st = s.build_lock.status();
    Json(QueueResponse {
        building: st.current,
    })
}

// ── Version ──

/// GET /api/version
async fn version_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "version": crate::self_update::CURRENT_VERSION,
    }))
}

// ── Self-update ──

/// POST /api/self-update — check GitHub Releases, download, restart
async fn self_update_handler(
    State(s): State<SharedState>,
) -> Json<serde_json::Value> {
    let parts: Vec<&str> = s.update_repo.split('/').collect();
    if parts.len() != 2 {
        return Json(serde_json::json!({"status": "error", "message": "invalid update-repo format (expected owner/repo)"}));
    }
    let (owner, repo) = (parts[0], parts[1]);

    let tx = s.tx.clone();

    let _ = tx.send(WsEvent {
        event: "self_update_checking".into(),
        target: String::new(),
        commit: None,
        message: None,
    });

    match crate::self_update::check(owner, repo).await {
        Ok(Some(version)) => {
            let ret_version = version.clone();
            let tx2 = tx.clone();
            let owner = owner.to_string();
            let repo = repo.to_string();
            tokio::spawn(async move {
                let _ = tx2.send(WsEvent {
                    event: "self_update_pulling".into(),
                    target: String::new(),
                    commit: Some(version.clone()),
                    message: None,
                });

                match crate::self_update::update_with_progress(&owner, &repo, {
                    let tx = tx2.clone();
                    move |downloaded, total| {
                        let pct = if total > 0 { (downloaded * 100 / total) as u8 } else { 0 };
                        let _ = tx.send(WsEvent {
                            event: "self_update_progress".into(),
                            target: String::new(),
                            commit: None,
                            message: Some(format!("{pct}")),
                        });
                    }
                }).await {
                    Ok(new_version) => {
                        let _ = tx2.send(WsEvent {
                            event: "self_update_complete".into(),
                            target: String::new(),
                            commit: Some(new_version),
                            message: None,
                        });
                        tracing::info!("Self-update download complete, ready for restart.");
                    }
                    Err(e) => {
                        let _ = tx2.send(WsEvent {
                            event: "self_update_error".into(),
                            target: String::new(),
                            commit: None,
                            message: Some(e.to_string()),
                        });
                    }
                }
            });
            Json(serde_json::json!({"status": "updating", "version": ret_version}))
        }
        Ok(None) => Json(serde_json::json!({"status": "up_to_date"})),
        Err(e) => Json(serde_json::json!({"status": "error", "message": e.to_string()})),
    }
}

// ── Config file read/write ──

#[derive(Deserialize)]
struct ConfigBody {
    content: String,
}

/// GET /api/targets/{name}/config — read raw .deployd/config.toml
async fn target_get_config(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let t = s
        .targets.read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    let config = crate::project::ProjectConfig::read_config_raw(&t.name)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .unwrap_or_default();

    let path = crate::project::ProjectConfig::config_path(&t.name);

    Ok(Json(serde_json::json!({
        "target": name,
        "path": path,
        "content": config,
    })))
}

/// PUT /api/targets/{name}/config — write .deployd/config.toml
async fn target_put_config(
    State(s): State<SharedState>,
    Path(name): Path<String>,
    Json(body): Json<ConfigBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let t = s
        .targets.read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    crate::project::ProjectConfig::save_config(&t.name, &body.content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({"status": "ok", "target": name})))
}

// ── Maven settings file read/write ──

fn resolve_maven_settings_path(name: &str, repo: &PathBuf) -> Option<String> {
    crate::project::ProjectConfig::load(name, repo)
        .ok()
        .flatten()
        .and_then(|c| c.build.maven_settings)
}

/// GET /api/targets/{name}/maven-settings — read maven settings.xml
async fn target_get_maven_settings(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let t = s
        .targets.read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    let settings_path = resolve_maven_settings_path(&t.name, &t.repo);

    let (path, content) = match settings_path {
        Some(p) => {
            let content = std::fs::read_to_string(&p)
                .map_err(|e| (StatusCode::NOT_FOUND, format!("cannot read {}: {}", p, e)))?;
            (p, content)
        }
        None => (String::new(), String::new()),
    };

    Ok(Json(serde_json::json!({
        "target": name,
        "path": path,
        "content": content,
    })))
}

/// PUT /api/targets/{name}/maven-settings — write maven settings.xml
async fn target_put_maven_settings(
    State(s): State<SharedState>,
    Path(name): Path<String>,
    Json(body): Json<ConfigBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let t = s
        .targets.read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    let settings_path = resolve_maven_settings_path(&t.name, &t.repo)
        .ok_or((StatusCode::BAD_REQUEST, "maven_settings not configured in .deployd/config.toml".into()))?;

    if let Some(parent) = std::path::Path::new(&settings_path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    std::fs::write(&settings_path, &body.content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({"status": "ok", "target": name, "path": settings_path})))
}

// ── Vite config file read/write ──

/// GET /api/targets/{name}/vite-config — read {repo}/vite.config.ts
async fn target_get_vite_config(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let t = s
        .targets.read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    let path = t.repo.join("vite.config.ts");
    let content = if path.exists() {
        std::fs::read_to_string(&path)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        String::new()
    };

    Ok(Json(serde_json::json!({
        "target": name,
        "path": path,
        "content": content,
    })))
}

/// PUT /api/targets/{name}/vite-config — write {repo}/vite.config.ts
async fn target_put_vite_config(
    State(s): State<SharedState>,
    Path(name): Path<String>,
    Json(body): Json<ConfigBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let t = s
        .targets.read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    let path = t.repo.join("vite.config.ts");
    std::fs::write(&path, &body.content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({"status": "ok", "target": name, "path": path.display().to_string()})))
}

// ── Env vars & JVM args read/write ──

#[derive(Deserialize)]
struct EnvBody {
    jvm_args: Option<String>,
    envs: Option<HashMap<String, String>>,
}

/// GET /api/targets/{name}/env — read jvm_args and env vars from config
async fn target_get_env(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let t = s
        .targets.read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    let jvm_args = t.jvm_args.lock().unwrap().clone();
    let envs = t.envs.lock().unwrap().clone();
    Ok(Json(serde_json::json!({
        "target": name,
        "jvm_args": jvm_args,
        "envs": envs,
    })))
}

/// PUT /api/targets/{name}/env — update jvm_args and env vars in config.toml
async fn target_put_env(
    State(s): State<SharedState>,
    Path(name): Path<String>,
    Json(body): Json<EnvBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let t = s
        .targets.read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    // Load existing config toml, update run.jvm_args and [env], write back
    let deploy_dir = t.repo.join(".deployd");
    let config_path = if let Some(ref p) = t.profile {
        let profiled = deploy_dir.join(format!("config.{p}.toml"));
        if profiled.exists() { profiled } else { deploy_dir.join("config.toml") }
    } else {
        deploy_dir.join("config.toml")
    };

    std::fs::create_dir_all(&deploy_dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut table: toml::Table = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        toml::from_str(&content).unwrap_or_default()
    } else {
        toml::Table::new()
    };

    // Update run.jvm_args
    if let Some(ref ja) = body.jvm_args {
        let run = table.entry("run").or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if let toml::Value::Table(r) = run {
            if ja.is_empty() {
                r.remove("jvm_args");
            } else {
                r.insert("jvm_args".into(), toml::Value::String(ja.clone()));
            }
        }
    }

    // Update [env] section
    if let Some(ref envs) = body.envs {
        if envs.is_empty() {
            table.remove("env");
        } else {
            let mut env = toml::Table::new();
            for (k, v) in envs {
                env.insert(k.clone(), toml::Value::String(v.clone()));
            }
            table.insert("env".into(), toml::Value::Table(env));
        }
    }

    std::fs::write(&config_path, toml::to_string_pretty(&table).unwrap())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Update live target state
    if let Some(ref ja) = body.jvm_args {
        *t.jvm_args.lock().unwrap() = if ja.is_empty() { None } else { Some(ja.clone()) };
    }
    if let Some(ref envs) = body.envs {
        *t.envs.lock().unwrap() = envs.clone();
    }

    Ok(Json(serde_json::json!({
        "status": "ok",
        "target": name,
        "hint": "Changes saved. They will take effect on next deploy."
    })))
}

// ── Local repo path ──

fn resolve_local_repo_path(name: &str, repo: &PathBuf) -> Option<String> {
    crate::project::ProjectConfig::load(name, repo)
        .ok()
        .flatten()
        .and_then(|c| c.build.local_repo)
}

/// GET /api/targets/{name}/local-repo
async fn target_get_local_repo(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let t = s
        .targets.read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    let path = resolve_local_repo_path(&t.name, &t.repo).unwrap_or_default();

    Ok(Json(serde_json::json!({
        "target": name,
        "local_repo": path,
    })))
}

// ── Restart ──

/// POST /api/restart
async fn restart_handler() -> Json<serde_json::Value> {
    tracing::info!("Restart requested via API");
    crate::self_update::restart();
}

// ── Delete target ──

#[derive(Deserialize)]
struct DeleteBody {
    keep_files: Option<bool>,
}

/// DELETE /api/targets/{name} — stop service, verify, remove from registry
async fn target_delete(
    State(s): State<SharedState>,
    Path(name): Path<String>,
    Json(body): Json<DeleteBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let t = s
        .targets.read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    // 1. Stop the process
    let mut proc = t.process.lock().unwrap();
    if let Some(ref mut p) = *proc {
        p.kill();
        // Wait for process to actually stop
        for _ in 0..30 {
            if !p.is_running() { break; }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
    *proc = None;
    drop(proc);

    // 2. Verify process stopped
    let running = t.process.lock().unwrap().is_some();
    if running {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "failed to stop process".into()));
    }

    // 3. Remove from registry
    let keep = body.keep_files.unwrap_or(false);
    if !keep {
        let deploy_dir = t.repo.join(".deployd");
        if deploy_dir.exists() {
            let _ = std::fs::remove_dir_all(&deploy_dir);
        }
    }
    crate::registry::remove_entry(&name)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 4. Remove from in-memory targets
    s.targets.write().unwrap().remove(&name);

    let _ = s.tx.send(WsEvent {
        event: "targets_changed".into(),
        target: name.clone(),
        commit: None,
        message: None,
    });

    Ok(Json(serde_json::json!({"status": "ok", "target": name})))
}

// ── Rename target ──

#[derive(Deserialize)]
struct RenameBody {
    new_name: String,
}

/// POST /api/targets/{name}/rename
async fn target_rename(
    State(s): State<SharedState>,
    Path(name): Path<String>,
    Json(body): Json<RenameBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let new_name = body.new_name.trim().to_string();
    if new_name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "new_name is required".into()));
    }
    if new_name == name {
        return Err((StatusCode::BAD_REQUEST, "new_name must be different".into()));
    }
    if s.targets.read().unwrap().contains_key(&new_name) {
        return Err((StatusCode::CONFLICT, format!("target '{new_name}' already exists")).into());
    }

    let t = s
        .targets.read().unwrap()
        .get(&name)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    crate::registry::rename_entry(&name, &new_name)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Rename config file
    let old_path = crate::project::target_config_path(&name);
    let new_path = crate::project::target_config_path(&new_name);
    if old_path.exists() {
        std::fs::rename(&old_path, &new_path)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("failed to rename config: {e}")))?;
    }

    // Update in-memory
    let mut targets = s.targets.write().unwrap();
    let mut ts = targets.remove(&name).unwrap();
    Arc::get_mut(&mut ts).unwrap().name = new_name.clone();
    Arc::get_mut(&mut ts).unwrap().label = new_name.clone();
    targets.insert(new_name.clone(), ts);

    let _ = s.tx.send(WsEvent {
        event: "targets_changed".into(),
        target: new_name.clone(),
        commit: None,
        message: None,
    });

    Ok(Json(serde_json::json!({"status": "ok", "old_name": name, "new_name": new_name})))
}

// ── Create target ──

#[derive(Deserialize)]
struct CreateTargetBody {
    name: String,
    label: Option<String>,
    repo: String,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    git_remote: Option<String>,
    #[serde(default)]
    build_cmd: Option<String>,
    #[serde(default)]
    artifact: Option<String>,
    #[serde(default)]
    run_cmd: Option<String>,
    #[serde(default)]
    health_url: Option<String>,
    #[serde(default)]
    run_mode: Option<String>,
    #[serde(default)]
    jvm_args: Option<String>,
    #[serde(default)]
    maven_settings: Option<String>,
    #[serde(default)]
    local_repo: Option<String>,
    envs: Option<HashMap<String, String>>,
}

/// POST /api/targets — create a new target
async fn target_create(
    State(s): State<SharedState>,
    Json(body): Json<CreateTargetBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name is required".into()));
    }
    let repo = std::path::PathBuf::from(body.repo.trim());
    if body.repo.trim().is_empty() || !repo.exists() {
        return Err((StatusCode::BAD_REQUEST, "repo path does not exist".into()));
    }

    if s.targets.read().unwrap().contains_key(&name) {
        return Err((StatusCode::CONFLICT, format!("target '{name}' already exists").into()));
    }

    // Clone git repo if remote provided and directory is empty
    if let Some(ref git_url) = body.git_remote {
        if repo.read_dir().map(|mut d| d.next().is_none()).unwrap_or(true) {
            let output = std::process::Command::new("git")
                .args(["clone", git_url, &body.repo.trim()])
                .output()
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("git clone failed: {}", stderr.trim())));
            }
        }
    }

    // Write config to ~/.config/lazyme/targets/{name}.toml
    let mut config_toml = String::from("[watch]\n");
    config_toml.push_str(&format!("branch = \"{}\"\n", body.branch.as_deref().unwrap_or("main")));
    if let Some(ref cmd) = body.build_cmd {
        config_toml.push_str("\n[build]\n");
        config_toml.push_str(&format!("command = \"{}\"\n", cmd));
        if let Some(ref art) = body.artifact { config_toml.push_str(&format!("artifact = \"{}\"\n", art)); }
        if let Some(ref ms) = body.maven_settings { config_toml.push_str(&format!("maven_settings = \"{}\"\n", ms)); }
        if let Some(ref lr) = body.local_repo { config_toml.push_str(&format!("local_repo = \"{}\"\n", lr)); }
    }
    if body.run_cmd.is_some() || body.health_url.is_some() {
        config_toml.push_str("\n[run]\n");
        if let Some(ref mode) = body.run_mode { config_toml.push_str(&format!("mode = \"{}\"\n", mode)); }
        if let Some(ref cmd) = body.run_cmd { config_toml.push_str(&format!("command = \"{}\"\n", cmd)); }
        if let Some(ref url) = body.health_url { config_toml.push_str(&format!("health_url = \"{}\"\n", url)); }
        if let Some(ref ja) = body.jvm_args { config_toml.push_str(&format!("jvm_args = \"{}\"\n", ja)); }
    }
    if let Some(ref envs) = body.envs {
        if !envs.is_empty() {
            config_toml.push_str("\n[env]\n");
            for (k, v) in envs { config_toml.push_str(&format!("\"{}\" = \"{}\"\n", k, v)); }
        }
    }
    crate::project::ProjectConfig::save_config(&name, &config_toml)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Append to registry
    let entry = crate::registry::TargetEntry {
        name: name.clone(),
        repo: repo.clone(),
        profile: body.profile.clone(),
        group: body.group.clone(),
        label: body.label.clone(),
    };
    crate::registry::append_entry(&entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Build in-memory state
    let build_cmd = body.build_cmd.unwrap_or_else(|| "cargo build --release".into());
    let artifact = body.artifact.map(std::path::PathBuf::from);
    let run_cmd = body.run_cmd;
    let health_url = body.health_url;
    let run_mode = body.run_mode.unwrap_or_else(|| "deploy".into());
    let jvm_args = body.jvm_args;
    let envs = body.envs.unwrap_or_default();
    let branch = Mutex::new(body.branch.unwrap_or_else(|| "main".into()));

    let ts = Arc::new(TargetState {
        name: name.clone(),
        label: body.label.unwrap_or_else(|| name.clone()),
        repo: repo.clone(),
        remote: "origin".into(),
        branch,
        build_cmd,
        artifact,
        run_cmd,
        health_url,
        health_timeout: 30,
        jvm_args: Mutex::new(jvm_args),
        envs: Mutex::new(envs),
        run_mode,
        state: Mutex::new(crate::state::StateManager::new(&repo)),
        process: Mutex::new(None),
        profile: body.profile.clone(),
        group: body.group.clone(),
        auto_deploy_paused: Mutex::new(false),
        health_status: Mutex::new(None),
        cached_local_head: Mutex::new(None),
        cached_remote_head: Mutex::new(None),
        auto_restart: false,
    });

    s.targets.write().unwrap().insert(name.clone(), ts.clone());

    // Spawn poll loop
    let interval = s.interval;
    let tx = s.tx.clone();
    let lock = s.build_lock.clone();
    tokio::spawn(async move { crate::poll_loop(ts, interval, tx, lock).await });

    let _ = s.tx.send(WsEvent {
        event: "targets_changed".into(),
        target: name.clone(),
        commit: None,
        message: None,
    });

    Ok(Json(serde_json::json!({"status": "ok", "target": name})))
}

// ── Router ──

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .route("/api/queue", get(queue_status))
        .route("/api/targets", get(list_targets))
        .route("/api/targets/{name}/status", get(target_status))
        .route("/api/targets/{name}/commits", get(target_commits))
        .route("/api/targets/{name}/history", get(target_history))
        .route("/api/targets/{name}/logs/{hash}", get(target_logs))
        .route("/api/targets/{name}/rollback", post(target_rollback))
        .route("/api/targets/{name}/deploy", post(target_deploy))
        .route("/api/targets/{name}/branch", post(target_set_branch))
        .route("/api/targets/{name}/branches", get(target_branches))
        .route("/api/targets/{name}/fetch", post(target_fetch))
        .route("/api/targets/{name}/clone", post(target_clone))
        .route("/api/targets/{name}/rename", post(target_rename))
        .route("/api/targets/{name}", delete(target_delete))
        .route("/api/targets", post(target_create))
        .route("/api/targets/{name}/config", get(target_get_config).put(target_put_config))
        .route("/api/targets/{name}/maven-settings", get(target_get_maven_settings).put(target_put_maven_settings))
        .route("/api/targets/{name}/vite-config", get(target_get_vite_config).put(target_put_vite_config))
        .route("/api/targets/{name}/stop", post(target_stop))
        .route("/api/targets/{name}/auto-deploy", post(target_auto_deploy))
        .route("/api/targets/{name}/env", get(target_get_env).put(target_put_env))
        .route("/api/targets/{name}/local-repo", get(target_get_local_repo))
        .route("/api/reload", post(reload_config))
        .route("/api/self-update", post(self_update_handler))
        .route("/api/restart", post(restart_handler))
        .route("/api/version", get(version_handler))
        .with_state(state)
}
