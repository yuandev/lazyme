use crate::git;
use crate::process::{self, ManagedProcess};
use crate::queue::BuildLock;
use crate::state::StateManager;
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;

pub type SharedState = Arc<AppState>;

pub struct AppState {
    pub targets: HashMap<String, Arc<TargetState>>,
    pub interval: u64,
    pub tx: broadcast::Sender<WsEvent>,
    pub build_lock: Arc<BuildLock>,
    pub self_update_repo: PathBuf,
    pub self_update_remote: String,
}

/// Per-target configuration and state.
pub struct TargetState {
    pub name: String,
    pub repo: PathBuf,
    pub remote: String,
    pub branch: Mutex<String>,
    pub build_cmd: String,
    pub artifact: Option<PathBuf>,
    pub run_cmd: Option<String>,
    pub health_url: Option<String>,
    pub health_timeout: u64,
    pub state: Mutex<StateManager>,
    pub process: Mutex<Option<ManagedProcess>>,
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
    repo: String,
    branch: String,
    deployed: Option<crate::state::DeployRecord>,
    local_commit: Option<String>,
    remote_commit: Option<String>,
    process_running: bool,
    health_url: Option<String>,
}

#[derive(Serialize)]
struct TargetListResponse {
    targets: Vec<TargetSummary>,
}

#[derive(Serialize)]
struct StatusResponse {
    name: String,
    repo: String,
    branch: String,
    deployed: Option<crate::state::DeployRecord>,
    local_commit: Option<String>,
    remote_commit: Option<String>,
    interval_secs: u64,
    process_running: bool,
    health_url: Option<String>,
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
    let mut summaries: Vec<TargetSummary> = s
        .targets
        .iter()
        .map(|(_, t)| {
            let st = t.state.lock().unwrap();
            let running = t.process.lock().unwrap().as_mut().map_or(false, |p| p.is_running());
            TargetSummary {
                name: t.name.clone(),
                repo: t.repo.display().to_string(),
                branch: t.branch(),
                deployed: st.current().clone(),
                local_commit: git::local_head(&t.repo).ok(),
                remote_commit: git::remote_head(&t.repo, &t.remote, &t.branch()).ok(),
                process_running: running,
                health_url: t.health_url.clone(),
            }
        })
        .collect();
    summaries.sort_by(|a, b| a.name.cmp(&b.name));
    Json(TargetListResponse { targets: summaries })
}

/// GET /api/targets/:name/status
async fn target_status(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<StatusResponse>, StatusCode> {
    let t = s.targets.get(&name).ok_or(StatusCode::NOT_FOUND)?;
    let deployed = { t.state.lock().unwrap().current().clone() };
    let running = { t.process.lock().unwrap().is_some() };
    Ok(Json(StatusResponse {
        name: t.name.clone(),
        repo: t.repo.display().to_string(),
        branch: t.branch(),
        deployed,
        local_commit: git::local_head(&t.repo).ok(),
        remote_commit: git::remote_head(&t.repo, &t.remote, &t.branch()).ok(),
        interval_secs: s.interval,
        process_running: running,
        health_url: t.health_url.clone(),
    }))
}

/// GET /api/targets/:name/commits
async fn target_commits(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<CommitsResponse>, StatusCode> {
    let t = s.targets.get(&name).ok_or(StatusCode::NOT_FOUND)?;
    git::recent_commits(&t.repo, 20)
        .map(|commits| Json(CommitsResponse { commits }))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// GET /api/targets/:name/history
async fn target_history(
    State(s): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<HistoryResponse>, StatusCode> {
    let t = s.targets.get(&name).ok_or(StatusCode::NOT_FOUND)?;
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
        .get(&name)
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
        .get(&name)
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

    let used_cache = try_rollback_with_cache(
        &t.repo, &body.commit, artifact, &t.state, &t.process, run, health, health_to,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !used_cache {
        let branch = t.branch();
        build_and_cache(
            &t.repo, &t.remote, &branch,
            &t.build_cmd, artifact,
            &body.commit, &t.state, &t.process, run, health, health_to,
            Some(&s.tx), &t.name,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    s.build_lock.set_current(None);
    drop(_guard);

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
        .get(&name)
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

    build_and_cache(
        &t.repo, &t.remote, &branch,
        &t.build_cmd, t.artifact.as_deref(),
        &remote, &t.state, &t.process,
        t.run_cmd.as_deref(), t.health_url.as_deref(), t.health_timeout,
        Some(&s.tx), &t.name,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    s.build_lock.set_current(None);
    drop(_guard);

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
    tx: Option<&broadcast::Sender<WsEvent>>,
    target_name: &str,
) -> anyhow::Result<()> {
    use std::process::Command;

    let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
    let flag = if cfg!(target_os = "windows") { "/C" } else { "-c" };

    let short = crate::git::short_hash(repo, commit_hash)
        .unwrap_or_else(|_| commit_hash[..7].to_string());

    // Broadcast build started
    if let Some(tx) = tx {
        let _ = tx.send(WsEvent {
            event: "build_started".into(),
            target: target_name.into(),
            commit: Some(short.clone()),
            message: None,
        });
    }

    let output = Command::new(shell)
        .args([flag, build_cmd])
        .current_dir(repo)
        .output()?;

    let success = output.status.success();

    // Persist build log
    let log_dir = repo.join(".deployd").join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join(format!("{short}.log"));
    let _ = std::fs::write(&log_path, &output.stdout);
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

    // Kill old process, start new one
    if success {
        if let Some(ref cmd) = run_cmd {
            let mut proc = process.lock().unwrap();
            if let Some(ref mut old) = *proc {
                old.kill();
            }
            let resolved = if let Some(art) = artifact_rel {
                cmd.replace("{artifact}", &art.display().to_string())
            } else {
                cmd.to_string()
            };
            match ManagedProcess::spawn(&resolved, repo) {
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

    // Health check
    if success && run_cmd.is_some() {
        if let Some(url) = health_url {
            if !process::health_check(url, health_timeout).await {
                tracing::warn!("Health check failed for {url}");
            }
        }
    }

    let mut st = state.lock().unwrap();
    st.record_deploy(
        commit_hash.to_string(),
        short,
        cache_path,
        log_path,
        success,
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
            let resolved = if let Some(art) = artifact_rel {
                cmd.replace("{artifact}", &art.display().to_string())
            } else {
                cmd.to_string()
            };
            match ManagedProcess::spawn(&resolved, repo) {
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
        .get(&name)
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
        return Err((
            StatusCode::BAD_REQUEST,
            format!("branch '{}' not found in repo", body.branch),
        ));
    }

    *t.branch.lock().unwrap() = body.branch;
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
        .get(&name)
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
        .get(&name)
        .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    let branch = t.branch();
    let remote = git::remote_head(&t.repo, &t.remote, &branch)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    git::pull(&t.repo, &t.remote, &branch)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "remote_head": remote,
    })))
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
            if let Some(t) = s.targets.get(&entry.name) {
                // Reload project config for existing target
                if let Ok(Some(proj)) =
                    crate::project::ProjectConfig::load(&entry.repo, entry.profile.as_deref())
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

// ── Self-update ──

/// POST /api/self-update — check, pull, build, restart (runs in background)
async fn self_update_handler(
    State(s): State<SharedState>,
) -> Json<serde_json::Value> {
    let repo = s.self_update_repo.clone();
    let remote_name = s.self_update_remote.clone();
    let tx = s.tx.clone();

    let _ = tx.send(WsEvent {
        event: "self_update_checking".into(),
        target: String::new(),
        commit: None,
        message: None,
    });

    // Check first — return immediately with status
    match crate::self_update::check(&repo, &remote_name, "main") {
        Ok(Some(remote)) => {
            let return_commit = remote.clone();
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let _ = tx2.send(WsEvent {
                    event: "self_update_pulling".into(),
                    target: String::new(),
                    commit: Some(remote),
                    message: None,
                });

                match crate::self_update::update(&repo, &remote_name, "main") {
                    Ok(new_hash) => {
                        let _ = tx2.send(WsEvent {
                            event: "self_update_complete".into(),
                            target: String::new(),
                            commit: Some(new_hash),
                            message: None,
                        });
                        tracing::info!("Self-update complete, restarting...");
                        crate::self_update::restart();
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
            Json(serde_json::json!({"status": "updating", "commit": return_commit}))
        }
        Ok(None) => Json(serde_json::json!({"status": "up_to_date"})),
        Err(e) => Json(serde_json::json!({"status": "error", "message": e.to_string()})),
    }
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
        .route("/api/reload", post(reload_config))
        .route("/api/self-update", post(self_update_handler))
        .with_state(state)
}
