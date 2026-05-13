use crate::config::Args;
use crate::git;
use crate::state::{DeployRecord, StateManager};
use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Serialize;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub type SharedState = Arc<AppState>;

pub struct AppState {
    pub state: Mutex<StateManager>,
    pub args: Args,
    pub polling: std::sync::atomic::AtomicBool,
}

#[derive(Serialize)]
struct StatusResponse {
    deployed: Option<DeployRecord>,
    local_commit: Option<String>,
    remote_commit: Option<String>,
    branch: String,
    polling: bool,
    interval_secs: u64,
}

#[derive(Serialize)]
struct CommitsResponse {
    commits: Vec<git::CommitInfo>,
}

#[derive(Serialize)]
struct HistoryResponse {
    history: Vec<DeployRecord>,
}

#[derive(Serialize)]
struct RollbackResponse {
    status: String,
    commit: String,
}

async fn status(State(s): State<SharedState>) -> Json<StatusResponse> {
    let (deployed, polling, interval_secs, repo, branch) = {
        let st = s.state.lock().unwrap();
        (
            st.current().clone(),
            s.polling.load(std::sync::atomic::Ordering::Relaxed),
            s.args.interval,
            s.args.repo.clone(),
            s.args.branch.clone(),
        )
    };

    Json(StatusResponse {
        deployed,
        local_commit: git::local_head(&repo).ok(),
        remote_commit: git::remote_head(&repo, &s.args.remote, &branch).ok(),
        branch,
        polling,
        interval_secs,
    })
}

async fn commits(State(s): State<SharedState>) -> Result<Json<CommitsResponse>, StatusCode> {
    git::recent_commits(&s.args.repo, 20)
        .map(|commits| Json(CommitsResponse { commits }))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn history(State(s): State<SharedState>) -> Json<HistoryResponse> {
    let st = s.state.lock().unwrap();
    Json(HistoryResponse {
        history: st.history().to_vec(),
    })
}

#[derive(serde::Deserialize)]
struct RollbackBody {
    commit: String,
}

async fn rollback(
    State(s): State<SharedState>,
    Json(body): Json<RollbackBody>,
) -> Result<Json<RollbackResponse>, (StatusCode, String)> {
    let repo = &s.args.repo;

    git::checkout(repo, &body.commit).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    let artifact = s.args.artifact.as_deref();
    let run = s.args.run.as_deref();

    // Try cache first
    let used_cache = try_rollback_with_cache(repo, &body.commit, artifact, run, &s.state)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Cache miss → full build
    if !used_cache {
        build_and_cache(
            repo, &s.args.remote, &s.args.branch,
            &s.args.build, artifact, run,
            &body.commit, &s.state,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(Json(RollbackResponse {
        status: "ok".into(),
        commit: body.commit,
    }))
}

async fn deploy_now(
    State(s): State<SharedState>,
) -> Result<Json<RollbackResponse>, (StatusCode, String)> {
    let repo = &s.args.repo;
    let branch = &s.args.branch;

    let remote = git::remote_head(repo, &s.args.remote, branch).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    git::pull(repo, &s.args.remote, branch).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    build_and_cache(
        repo, &s.args.remote, branch,
        &s.args.build, s.args.artifact.as_deref(), s.args.run.as_deref(),
        &remote, &s.state,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(RollbackResponse {
        status: "ok".into(),
        commit: remote,
    }))
}

/// Build, cache artifact, and record deployment. Shared by poller and manual deploy.
pub async fn build_and_cache(
    repo: &Path,
    _remote: &str,
    _branch: &str,
    build_cmd: &str,
    artifact_rel: Option<&Path>,
    run_cmd: Option<&str>,
    commit_hash: &str,
    state: &Mutex<StateManager>,
) -> anyhow::Result<()> {
    use std::process::Command;

    let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
    let flag = if cfg!(target_os = "windows") { "/C" } else { "-c" };

    let status = Command::new(shell)
        .args([flag, build_cmd])
        .current_dir(repo)
        .status()?;

    let success = status.success();

    // Cache artifact on success
    let cache_path = if success {
        if let Some(artifact) = artifact_rel {
            let st = state.lock().unwrap();
            let short = crate::git::short_hash(repo, commit_hash).unwrap_or_else(|_| commit_hash[..7].to_string());
            st.cache_artifact(&short, artifact).ok()
        } else {
            None
        }
    } else {
        None
    };

    // Run if configured
    if let Some(ref cmd) = run_cmd {
        let _ = Command::new(shell)
            .args([flag, cmd])
            .current_dir(repo)
            .spawn();
    }

    let mut st = state.lock().unwrap();
    let short = crate::git::short_hash(repo, commit_hash).unwrap_or_else(|_| commit_hash[..7].to_string());
    st.record_deploy(commit_hash.to_string(), short, cache_path, success)?;

    Ok(())
}

/// Try to rollback using cached artifact. Returns true if cache was used (no rebuild).
pub fn try_rollback_with_cache(
    repo: &Path,
    commit_hash: &str,
    artifact_rel: Option<&Path>,
    run_cmd: Option<&str>,
    state: &Mutex<StateManager>,
) -> anyhow::Result<bool> {
    use std::process::Command;

    let st = state.lock().unwrap();
    let short = crate::git::short_hash(repo, commit_hash).unwrap_or_else(|_| commit_hash[..7].to_string());

    let cache_hit = artifact_rel.and_then(|a| {
        let fname = a.file_name()?;
        st.find_cached_artifact(&short, fname.to_str()?)
    });

    if let Some(cache_path) = cache_hit {
        drop(st); // release lock before spawning

        // Run from cache
        if let Some(ref cmd) = run_cmd {
            let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
            let flag = if cfg!(target_os = "windows") { "/C" } else { "-c" };
            let _ = Command::new(shell)
                .args([flag, cmd])
                .current_dir(repo)
                .spawn();
        }

        let mut st = state.lock().unwrap();
        st.record_deploy(commit_hash.to_string(), short, Some(cache_path), true)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/api/status", get(status))
        .route("/api/commits", get(commits))
        .route("/api/history", get(history))
        .route("/api/rollback", post(rollback))
        .route("/api/deploy", post(deploy_now))
        .with_state(state)
}
