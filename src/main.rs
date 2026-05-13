mod api;
mod config;
mod git;
mod project;
mod registry;
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
};
use tracing::{error, info, warn};

#[derive(RustEmbed)]
#[folder = "frontend/dist/"]
struct Frontend;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = config::Args::parse();

    // Load target registry
    let targets = registry::load()?;
    let targets = registry::filter(targets, &args.filter);
    if targets.is_empty() {
        anyhow::bail!("No targets found. Check ~/.config/lazyme/targets.toml");
    }

    // Build per-target state
    let mut target_map = HashMap::new();
    for entry in &targets {
        let proj = project::ProjectConfig::load(&entry.repo, entry.profile.as_deref())
            .unwrap_or_default()
            .unwrap_or_default();

        let branch = proj.watch.branch.unwrap_or_else(|| "main".into());
        let build_cmd = proj.build.command.unwrap_or_else(|| "cargo build --release".into());
        let artifact = proj.build.artifact.map(std::path::PathBuf::from);
        let run_cmd = proj.run.command;

        let ts = Arc::new(TargetState {
            name: entry.name.clone(),
            repo: entry.repo.clone(),
            remote: args.remote.clone(),
            branch,
            build_cmd,
            artifact,
            run_cmd,
            state: Mutex::new(state::StateManager::new(&entry.repo)),
        });

        target_map.insert(entry.name.clone(), ts);
        info!("Target '{}' registered ({})", entry.name, entry.repo.display());
    }

    let shared = Arc::new(AppState {
        targets: target_map,
        interval: args.interval,
    });

    // Start one poller per target
    for target in shared.targets.values() {
        let t = target.clone();
        let interval = args.interval;
        tokio::spawn(async move { poll_loop(t, interval).await });
    }

    let app = api::router(shared).fallback(serve_frontend);

    let addr = format!("0.0.0.0:{}", args.port);
    info!("Web UI at http://localhost:{}", args.port);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn poll_loop(target: Arc<TargetState>, interval_secs: u64) {
    let interval = tokio::time::Duration::from_secs(interval_secs);
    tokio::time::sleep(interval).await;
    let mut timer = tokio::time::interval(interval);

    loop {
        timer.tick().await;

        let remote = match git::remote_head(&target.repo, &target.remote, &target.branch) {
            Ok(h) => h,
            Err(e) => {
                warn!("[{}] remote check failed: {e}", target.name);
                continue;
            }
        };

        if remote.is_empty() {
            continue;
        }

        let deployed = {
            let st = target.state.lock().unwrap();
            st.current().as_ref().map(|r| r.commit_hash.clone())
        };

        if deployed.as_deref() == Some(&remote) {
            continue;
        }

        info!("[{}] new commit: {remote}, pulling...", target.name);

        if let Err(e) = git::pull(&target.repo, &target.remote, &target.branch) {
            error!("[{}] pull failed: {e}", target.name);
            continue;
        }

        if let Err(e) = api::build_and_cache(
            &target.repo,
            &target.remote,
            &target.branch,
            &target.build_cmd,
            target.artifact.as_deref(),
            target.run_cmd.as_deref(),
            &remote,
            &target.state,
        )
        .await
        {
            error!("[{}] build/deploy failed: {e}", target.name);
        } else {
            info!("[{}] deployed {remote}", target.name);
        }
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
