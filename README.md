！！！
这一段是用户邀请保留的readme说明，请不要修改和删除用三个感叹号开始 三个感叹号结束
AI写的，问题很多，用不了就下源码用ai启动

This section is a user-reserved README notice. Do NOT modify or delete it. It starts and ends with triple exclamation marks.
Written by AI — full of bugs. If it doesn't work, just clone the source and launch it with AI.
！！！

# lazyme

[中文文档](README.zh.md)

Single-binary, multi-target deployment daemon for local test environments. Watches git branches, pulls on new commits, builds, caches artifacts, and manages running processes — all with a dark-themed React dashboard.

## How It Works

```
targets.toml        .deployd/config.toml
     │                      │
     ▼                      ▼
┌─────────┐   poll    ┌──────────┐   build   ┌──────────┐
│ registry │ ──────▶  │ poll loop │ ───────▶ │ build &  │
│ (names)  │          │ per target│          │ cache    │
└─────────┘          └──────────┘          └──────────┘
                                                 │
                           ┌─────────────────────┤
                           ▼                     ▼
                    ┌──────────┐          ┌──────────┐
                    │ artifact │          │   run    │
                    │  cache   │          │ command  │
                    └──────────┘          └──────────┘
                                               │
                                    health check (TCP)
```

- **Poll**: each target checks `git ls-remote` on an interval
- **Pull**: fetches + checks out the remote branch when a new commit appears
- **Build**: runs the build command, captures stdout → `.deployd/logs/{hash}.log` (skipped in dev mode)
- **Cache**: copies the build artifact to `.deployd/artifacts/{short_hash}/`
- **Run**: kills old process, spawns new one, resolves `{artifact}` and `{jvm_args}` placeholders
- **Health**: TCP-connects to the health URL, retries until timeout

## Quick Start

### 1. Download

Pre-built binaries from [GitHub Releases](https://github.com/yuandev/lazyme/releases). Choose your platform:

- `deployd-x86_64-unknown-linux-gnu` — Linux x86_64
- `deployd-aarch64-apple-darwin` — macOS Apple Silicon

```bash
# Example: macOS
curl -L -o deployd https://github.com/yuandev/lazyme/releases/latest/download/deployd-aarch64-apple-darwin
chmod +x deployd
./deployd
```

No dependencies needed — the frontend is embedded in the binary.

### 2. Register targets

Create `~/.config/lazyme/targets.toml`:

```toml
[[targets]]
name = "my-api"
label = "My API Service"    # optional display name
repo = "/home/me/projects/my-api"
group = "backend"           # optional group for sidebar organization

[[targets]]
name = "frontend"
repo = "/home/me/projects/frontend"
```

### 3. Add target config (optional)

Create `~/.config/lazyme/targets/{name}.toml` (auto-generated on first deploy if missing):

```toml
[watch]
branch = "main"

[build]
command = "mvn package -DskipTests"
artifact = "target/my-api.jar"
maven_settings = "/path/to/settings.xml"   # optional
local_repo = "/path/to/maven/repo"         # optional

[run]
mode = "deploy"              # "deploy" (default) or "dev" (skip build)
command = "java {jvm_args} -jar {artifact}"
jvm_args = "-Xmx512m -Dserver.port=8080"
health_url = "http://localhost:{port}/health"  # {port} detected from PID or jvm_args
health_timeout = 30

[env]
JAVA_HOME = "/usr/lib/jvm/java-17"
SPRING_PROFILES_ACTIVE = "prod"
```

Config lives under `~/.config/lazyme/targets/{name}.toml` — isolated from the git repo. Legacy `{repo}/.deployd/config.toml` is auto-migrated on first load.

All fields are optional. CLI args override project config, project config overrides built-in defaults.

### 4. Run

```bash
# Watch all targets
./deployd

# Watch specific targets
./deployd my-api frontend

# Custom port and poll interval
./deployd --port 9090 --interval 30

# Custom remote name
./deployd --remote upstream
```

Open **http://localhost:8080** for the dashboard.

## CLI Reference

```
lazyme [OPTIONS] [FILTER]...

Options:
  -R, --remote <REMOTE>    Remote name [default: origin]
  -i, --interval <SECS>    Poll interval [default: 60]
  -p, --port <PORT>        Web UI port [default: 8080]
  <FILTER>...              Target names to watch (empty = all)
```

## Features

### Web Dashboard

- **Status tab**: live view of deployed commit, git state, build/run commands, JVM args, env vars, PID and uptime
- **Online/offline indicator**: green/red badge based on real TCP health check (polled every interval)
- **Commits tab**: recent commits with one-click deploy/rollback and build log viewer
- **History tab**: deployment history with cache status, build duration, and log access
- **Config tab**: edit config.toml, Maven settings, vite.config.ts, JVM args, and env vars
- **Add target**: modal form to create a new target with optional git clone
- **Rename / Clone / Delete**: sidebar buttons per target; delete shows stop → verify → remove flow
- **Branch switch**: dropdown with remote branches, refresh button, auto-creates local tracking branch
- **Open service**: launch the deployed service URL in browser
- **i18n**: English / Chinese toggle
- **WebSocket live logs**: stream build output in real time
- **Auto-deploy pause**: temporarily stop automatic deployments per target

### Self-Update

- Check for new GitHub Releases from the dashboard header
- Streaming download with progress bar
- Manual restart after download (won't kill the UI mid-update)

### Dev Mode

Set `mode = "dev"` in `[run]` to skip the build step. On new commits, lazyme pulls and restarts the run command directly — ideal for Node.js, Python, or any interpreted language.

## Configuration Files

### `~/.config/lazyme/targets.toml` (required)

| Field | Required | Description |
|-------|----------|-------------|
| `name` | yes | Unique target name (identifier) |
| `repo` | yes | Absolute path to local git repository |
| `label` | no | Display name in sidebar (defaults to `name`) |
| `group` | no | Group name for sidebar organization |
| `profile` | no | Legacy profile support |

### `.deployd/config.toml` (per-repo, optional)

#### `[watch]`

| Field | Default | Description |
|-------|---------|-------------|
| `branch` | `main` | Branch to watch |

#### `[build]`

| Field | Default | Description |
|-------|---------|-------------|
| `command` | `cargo build --release` | Shell command to build |
| `artifact` | none | Path to built artifact (relative to repo root) |
| `maven_settings` | none | Path to Maven settings.xml |
| `local_repo` | none | Path to local Maven repository |

#### `[run]`

| Field | Default | Description |
|-------|---------|-------------|
| `mode` | `deploy` | `"deploy"` (build + run) or `"dev"` (run only) |
| `command` | none | Shell command. `{artifact}` and `{jvm_args}` are replaced at runtime |
| `jvm_args` | none | JVM arguments |
| `health_url` | none | Health check endpoint |
| `health_timeout` | `30` | Seconds to wait for health check |

#### `[env]`

Flat key-value pairs injected as environment variables into the run command process.

## On-Disk Layout

```
~/.config/lazyme/
├── targets.toml                  # target registry
└── targets/
    ├── my-api.toml               # per-target config (isolated from repo)
    └── frontend.toml

my-project/
└── .deployd/                     # runtime state only
    ├── state.json                # deploy history (auto-managed)
    ├── artifacts/
    │   └── a1b2c3d/
    │       └── my-binary         # cached artifact
    └── logs/
        └── a1b2c3d.log           # build output
```

## API Reference

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/ws` | WebSocket events |
| `GET` | `/api/targets` | List all targets |
| `GET` | `/api/targets/{name}/status` | Target status |
| `GET` | `/api/targets/{name}/commits` | Recent commits |
| `GET` | `/api/targets/{name}/history` | Deploy history |
| `GET` | `/api/targets/{name}/logs/{hash}` | Build log |
| `POST` | `/api/targets/{name}/deploy` | Manual deploy |
| `POST` | `/api/targets/{name}/rollback` | Rollback to commit |
| `GET` | `/api/targets/{name}/branches` | List remote branches |
| `POST` | `/api/targets/{name}/branch` | Switch branch |
| `POST` | `/api/targets/{name}/fetch` | Git fetch (all remote refs) |
| `POST` | `/api/targets/{name}/clone` | Clone target config |
| `POST` | `/api/targets/{name}/rename` | Rename target |
| `DELETE` | `/api/targets/{name}` | Delete target (stop service → verify → remove) |
| `POST` | `/api/targets` | Create new target |
| `GET/PUT` | `/api/targets/{name}/config` | Read/write config.toml |
| `GET/PUT` | `/api/targets/{name}/maven-settings` | Read/write Maven settings.xml |
| `GET/PUT` | `/api/targets/{name}/vite-config` | Read/write vite.config.ts |
| `GET/PUT` | `/api/targets/{name}/env` | Read/write JVM args + env vars |
| `GET` | `/api/targets/{name}/local-repo` | Get local Maven repo path |
| `POST` | `/api/targets/{name}/auto-deploy` | Toggle auto-deploy pause/resume |
| `POST` | `/api/self-update` | Check + download update |
| `POST` | `/api/restart` | Restart server |
| `GET` | `/api/version` | Current version |
| `GET` | `/api/queue` | Build queue status |
| `POST` | `/api/reload` | Reload configs |

### WebSocket Events

```json
{"event":"build_started","target":"my-api","commit":"a1b2c3d","message":null}
{"event":"build_complete","target":"my-api","commit":"a1b2c3d","message":"success=true"}
{"event":"deploy_started","target":"my-api","commit":"a1b2c3d","message":null}
{"event":"deploy_complete","target":"my-api","commit":"a1b2c3d","message":null}
{"event":"self_update_checking","target":"","commit":null,"message":null}
{"event":"self_update_progress","target":"","commit":null,"message":"45"}
{"event":"self_update_complete","target":"","commit":"0.1.5","message":null}
{"event":"targets_changed","target":"my-clone","commit":null,"message":null}
```

## Build from Source

```bash
git clone git@github.com:yuandev/lazyme.git
cd lazyme

# Backend (requires Rust ≥ 1.82)
cargo build --release

# Frontend (only if you modified it; pre-built dist/ is committed)
cd frontend
npm install
npm run build
```

## License

MIT
