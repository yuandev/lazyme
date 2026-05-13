mod api;
mod config;
mod git;
mod state;

use api::AppState;
use axum::{
    body::Body,
    http::{header, StatusCode},
    response::Response,
};
use clap::Parser;
use rust_embed::RustEmbed;
use std::sync::{Arc, Mutex};
use tracing::{error, info};

#[derive(RustEmbed)]
#[folder = "frontend/dist/"]
struct Frontend;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = config::Args::parse();
    let state = Arc::new(AppState {
        state: Mutex::new(state::StateManager::new(&args.repo)),
        args: args.clone(),
        polling: std::sync::atomic::AtomicBool::new(true),
    });

    info!("Deployd started, watching {} branch {}", args.repo.display(), args.branch);

    // Start poller
    let poller_state = state.clone();
    tokio::spawn(async move {
        poll_loop(poller_state).await;
    });

    // Build router
    let app = api::router(state)
        .fallback(serve_frontend);

    let addr = format!("0.0.0.0:{}", args.port);
    info!("Web UI at http://localhost:{}", args.port);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
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

    // SPA fallback: return index.html for unmatched routes
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

async fn poll_loop(state: Arc<AppState>) {
    let interval = tokio::time::Duration::from_secs(state.args.interval);
    // Wait one interval before first poll, so the server is ready
    tokio::time::sleep(interval).await;
    let mut timer = tokio::time::interval(interval);

    loop {
        timer.tick().await;
        if !state.polling.load(std::sync::atomic::Ordering::Relaxed) {
            continue;
        }

        let repo = &state.args.repo;
        let branch = &state.args.branch;

        let remote = match git::remote_head(repo, &state.args.remote, branch) {
            Ok(h) => h,
            Err(e) => {
                error!("Failed to check remote: {e}");
                continue;
            }
        };

        if remote.is_empty() {
            continue;
        }

        let deployed_commit = {
            let st = state.state.lock().unwrap();
            st.current().as_ref().map(|r| r.commit_hash.clone())
        };

        if deployed_commit.as_deref() == Some(&remote) {
            continue; // up to date
        }

        info!("New commit detected: {remote}, pulling...");

        if let Err(e) = git::pull(repo, &state.args.remote, branch) {
            error!("Pull failed: {e}");
            continue;
        }

        // Run build
        let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
        let flag = if cfg!(target_os = "windows") { "/C" } else { "-c" };

        let status = std::process::Command::new(shell)
            .args([flag, &state.args.build])
            .current_dir(repo)
            .status();

        let success = status.map(|s| s.success()).unwrap_or(false);

        if success {
            info!("Build succeeded for {remote}");
        } else {
            error!("Build failed for {remote}");
        }

        // Run the app if configured
        if let Some(ref run_cmd) = state.args.run {
            let _ = std::process::Command::new(shell)
                .args([flag, run_cmd])
                .current_dir(repo)
                .spawn();
        }

        let mut st = state.state.lock().unwrap();
        let _ = st.record_deploy(remote, state.args.artifact.clone(), success);
    }
}
