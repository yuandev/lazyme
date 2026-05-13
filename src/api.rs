use crate::git;
use crate::process::{self, ManagedProcess};
use crate::state::StateManager;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub type SharedState = Arc<AppState>;

pub struct AppState {
    pub targets: HashMap<String, Arc<TargetState>>,
    pub interval: u64,
}

/// Per-target configuration and state.
pub struct TargetState {
    pub name: String,
    pub repo: PathBuf,
    pub remote: String,
    pub branch: String,
    pub build_cmd: String,
    pub artifact: Option<PathBuf>,
    pub run_cmd: Option<String>,
    pub health_url: Option<String>,
    pub health_timeout: u64,
    pub state: Mutex<StateManager>,
    pub process: Mutex<Option<ManagedProcess>>,
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

// ── Handlers ──

/// GET /api/targets
async fn list_targets(State(s): State<SharedState>) -> Json<TargetListResponse> {
    let mut summaries: Vec<TargetSummary> = s
        .targets
        .iter()
        .map(|(_, t)| {
            let st = t.state.lock().unwrap();
            let running = t.process.lock().unwrap().is_some();
            TargetSummary {
                name: t.name.clone(),
                repo: t.repo.display().to_string(),
                branch: t.branch.clone(),
                deployed: st.current().clone(),
                local_commit: git::local_head(&t.repo).ok(),
                remote_commit: git::remote_head(&t.repo, &t.remote, &t.branch).ok(),
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
        branch: t.branch.clone(),
        deployed,
        local_commit: git::local_head(&t.repo).ok(),
        remote_commit: git::remote_head(&t.repo, &t.remote, &t.branch).ok(),
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

#[derive(serde::Deserialize)]
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

    git::checkout(&t.repo, &body.commit).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    let artifact = t.artifact.as_deref();
    let run = t.run_cmd.as_deref();
    let health = t.health_url.as_deref();
    let health_to = t.health_timeout;

    let used_cache = try_rollback_with_cache(
        &t.repo, &body.commit, artifact, &t.state, &t.process, run, health, health_to,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !used_cache {
        build_and_cache(
            &t.repo, &t.remote, &t.branch,
            &t.build_cmd, artifact,
            &body.commit, &t.state, &t.process, run, health, health_to,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

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

    let remote = git::remote_head(&t.repo, &t.remote, &t.branch).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    git::pull(&t.repo, &t.remote, &t.branch).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    build_and_cache(
        &t.repo, &t.remote, &t.branch,
        &t.build_cmd, t.artifact.as_deref(),
        &remote, &t.state, &t.process,
        t.run_cmd.as_deref(), t.health_url.as_deref(), t.health_timeout,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

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
) -> anyhow::Result<()> {
    use std::process::Command;

    let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
    let flag = if cfg!(target_os = "windows") { "/C" } else { "-c" };

    let status = Command::new(shell)
        .args([flag, build_cmd])
        .current_dir(repo)
        .status()?;

    let success = status.success();

    let cache_path = if success {
        if let Some(artifact) = artifact_rel {
            let st = state.lock().unwrap();
            let short = crate::git::short_hash(repo, commit_hash)
                .unwrap_or_else(|_| commit_hash[..7].to_string());
            st.cache_artifact(&short, artifact).ok()
        } else {
            None
        }
    } else {
        None
    };

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

    // Health check
    if success && run_cmd.is_some() {
        if let Some(url) = health_url {
            if !process::health_check(url, health_timeout).await {
                tracing::warn!("Health check failed for {url}");
            }
        }
    }

    let mut st = state.lock().unwrap();
    let short = crate::git::short_hash(repo, commit_hash)
        .unwrap_or_else(|_| commit_hash[..7].to_string());
    st.record_deploy(commit_hash.to_string(), short, cache_path, success)?;

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
        st.record_deploy(commit_hash.to_string(), short, Some(cache_path), true)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

// ── Router ──

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/api/targets", get(list_targets))
        .route("/api/targets/{name}/status", get(target_status))
        .route("/api/targets/{name}/commits", get(target_commits))
        .route("/api/targets/{name}/history", get(target_history))
        .route("/api/targets/{name}/rollback", post(target_rollback))
        .route("/api/targets/{name}/deploy", post(target_deploy))
        .with_state(state)
}
