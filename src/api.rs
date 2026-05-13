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

    run_build_and_record(&s, &body.commit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

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

    run_build_and_record(&s, &remote)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(RollbackResponse {
        status: "ok".into(),
        commit: remote,
    }))
}

async fn run_build_and_record(s: &SharedState, commit: &str) -> anyhow::Result<()> {
    use std::process::Command;

    let repo = &s.args.repo;
    let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
    let flag = if cfg!(target_os = "windows") { "/C" } else { "-c" };

    let status = Command::new(shell)
        .args([flag, &s.args.build])
        .current_dir(repo)
        .status()?;

    let success = status.success();

    // Kill old process if running
    if let Some(ref run_cmd) = s.args.run {
        // Simple: kill the old process by name or just start a new one
        // For MVP, just start the new process
        let _child = Command::new(shell)
            .args([flag, run_cmd])
            .current_dir(repo)
            .spawn();
    }

    let mut st = s.state.lock().unwrap();
    st.record_deploy(
        commit.to_string(),
        s.args.artifact.clone(),
        success,
    )?;

    Ok(())
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
