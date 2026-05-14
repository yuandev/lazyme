mod api;
mod config;
mod git;
mod process;
mod project;
mod queue;
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
use tokio::sync::broadcast;

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

        let branch = Mutex::new(proj.watch.branch.unwrap_or_else(|| "main".into()));
        let build_cmd = proj.build.command.unwrap_or_else(|| "cargo build --release".into());
        let artifact = proj.build.artifact.map(std::path::PathBuf::from);
        let run_cmd = proj.run.command;
        let health_url = proj.run.health_url;
        let health_timeout = proj.run.health_timeout;

        let ts = Arc::new(TargetState {
            name: entry.name.clone(),
            repo: entry.repo.clone(),
            remote: args.remote.clone(),
            branch,
            build_cmd,
            artifact,
            run_cmd,
            health_url,
            health_timeout,
            process: Mutex::new(None),
            state: Mutex::new(state::StateManager::new(&entry.repo)),
        });

        target_map.insert(entry.name.clone(), ts);
        info!("Target '{}' registered ({})", entry.name, entry.repo.display());
    }

    let (tx, _rx) = broadcast::channel::<api::WsEvent>(64);
    let build_lock = Arc::new(queue::BuildLock::new());

    let shared = Arc::new(AppState {
        targets: target_map,
        interval: args.interval,
        tx,
        build_lock: build_lock.clone(),
    });

    // Start one poller per target
    for target in shared.targets.values() {
        let t = target.clone();
        let interval = args.interval;
        let tx = shared.tx.clone();
        let lock = build_lock.clone();
        tokio::spawn(async move { poll_loop(t, interval, tx, lock).await });
    }

    let app = api::router(shared).fallback(serve_frontend);

    // Try ports starting from the configured port, auto-increment if in use
    let mut port = args.port;
    let listener = loop {
        let addr = format!("0.0.0.0:{port}");
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => break l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                warn!("Port {port} in use, trying {}...", port + 1);
                port += 1;
            }
            Err(e) => return Err(e.into()),
        }
    };
    info!("Web UI at http://localhost:{port}");
    axum::serve(listener, app).await?;

    Ok(())
}

async fn poll_loop(
    target: Arc<TargetState>,
    interval_secs: u64,
    tx: broadcast::Sender<api::WsEvent>,
    build_lock: Arc<queue::BuildLock>,
) {
    let interval = tokio::time::Duration::from_secs(interval_secs);
    tokio::time::sleep(interval).await;
    let mut timer = tokio::time::interval(interval);

    loop {
        timer.tick().await;

        let branch = target.branch();

        let remote = match git::remote_head(&target.repo, &target.remote, &branch) {
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

        if let Err(e) = git::pull(&target.repo, &target.remote, &branch) {
            error!("[{}] pull failed: {e}", target.name);
            continue;
        }

        // Acquire build lock to serialize builds across targets
        let _guard = build_lock.inner.lock().await;
        build_lock.set_current(Some(target.name.clone()));

        if let Err(e) = api::build_and_cache(
            &target.repo,
            &target.remote,
            &branch,
            &target.build_cmd,
            target.artifact.as_deref(),
            &remote,
            &target.state,
            &target.process,
            target.run_cmd.as_deref(),
            target.health_url.as_deref(),
            target.health_timeout,
            Some(&tx),
            &target.name,
        )
        .await
        {
            error!("[{}] build/deploy failed: {e}", target.name);
        } else {
            info!("[{}] deployed {remote}", target.name);
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
