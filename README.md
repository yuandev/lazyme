# lazyme

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
- **Build**: runs the build command, captures stdout → `.deployd/logs/{hash}.log`
- **Cache**: copies the build artifact to `.deployd/artifacts/{short_hash}/`
- **Run**: kills old process, spawns new one, resolves `{artifact}` placeholder
- **Health**: TCP-connects to the health URL, retries until timeout

## Quick Start

### 1. Install

```bash
git clone git@github.com:yuandev/lazyme.git
cd lazyme
cargo build --release
```

Requires Rust ≥ 1.82 and Node.js (for frontend; pre-built `dist/` is embedded for release).

### 2. Register targets

Create `~/.config/lazyme/targets.toml`:

```toml
[[targets]]
name = "my-api"
repo = "/home/me/projects/my-api"
profile = "dev"   # optional: loads .deployd/config.dev.toml

[[targets]]
name = "frontend"
repo = "/home/me/projects/frontend"
```

### 3. Add per-project config (optional)

In each repo, create `.deployd/config.toml`:

```toml
[watch]
branch = "main"          # default: main

[build]
command = "cargo build --release"
artifact = "target/release/my-api"   # relative to repo root

[run]
command = "./target/release/{artifact}"   # {artifact} replaced at runtime
health_url = "http://localhost:3000/health"
health_timeout = 30    # seconds, default 30
```

All fields are optional. CLI flags override project config, project config overrides built-in defaults.

### 4. Run

```bash
# Watch all targets
lazyme

# Watch specific targets
lazyme my-api frontend

# Custom port and poll interval
lazyme --port 9090 --interval 30

# Custom remote name
lazyme --remote upstream
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

## Configuration Files

### `~/.config/lazyme/targets.toml` (required)

Multi-target registry. Each `[[targets]]` entry declares a target to watch.

| Field | Required | Description |
|-------|----------|-------------|
| `name` | yes | Unique target name (used in UI and API) |
| `repo` | yes | Absolute path to local git repository |
| `profile` | no | Load `.deployd/config.{profile}.toml` instead of `config.toml` |

### `.deployd/config.toml` (per-repo, optional)

Project-owned build/run configuration. Convention over configuration.

#### `[watch]`

| Field | Default | Description |
|-------|---------|-------------|
| `branch` | `main` | Branch to watch for new commits |

#### `[build]`

| Field | Default | Description |
|-------|---------|-------------|
| `command` | `cargo build --release` | Shell command to build |
| `artifact` | none | Path to built artifact (relative to repo root) |

#### `[run]`

| Field | Default | Description |
|-------|---------|-------------|
| `command` | none | Shell command to start the service. `{artifact}` is replaced with the actual artifact path |
| `health_url` | none | Health check endpoint (e.g. `http://localhost:3000/health`) |
| `health_timeout` | `30` | Seconds to wait for health check to pass |

## On-Disk Layout

Each watched repository accumulates state under `.deployd/`:

```
my-project/
└── .deployd/
    ├── config.toml          # project config (you create this)
    ├── state.json           # deploy history (auto-managed)
    ├── artifacts/
    │   └── a1b2c3d/
    │       └── my-binary    # cached build artifact
    └── logs/
        └── a1b2c3d.log      # build output
```

## API Reference

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/ws` | WebSocket — real-time build/deploy events |
| `GET` | `/api/targets` | List all targets with status summary |
| `GET` | `/api/targets/{name}/status` | Detailed target status |
| `GET` | `/api/targets/{name}/commits` | Recent git commits |
| `GET` | `/api/targets/{name}/history` | Deploy history (with cache/log links) |
| `GET` | `/api/targets/{name}/logs/{hash}` | Build log content |
| `POST` | `/api/targets/{name}/deploy` | Trigger manual deploy |
| `POST` | `/api/targets/{name}/rollback` | Rollback to a commit (uses cache if available) |
| `GET` | `/api/queue` | Current build status |

### WebSocket Events

```json
{"event":"build_started","target":"my-api","commit":"a1b2c3d","message":null}
{"event":"build_complete","target":"my-api","commit":"a1b2c3d","message":"success=true"}
{"event":"deploy_started","target":"my-api","commit":"a1b2c3d","message":null}
{"event":"deploy_complete","target":"my-api","commit":"a1b2c3d","message":null}
```

## Rollback

Rollback is instant when the target artifact was cached. If the cached artifact for the requested commit exists, it restarts the process immediately. Otherwise it rebuilds from that commit.

```bash
# Via API
curl -X POST http://localhost:8080/api/targets/my-api/rollback \
  -H 'Content-Type: application/json' \
  -d '{"commit":"a1b2c3d4"}'
```

Or click "rollback" in the dashboard commit list.

## Build from Source

```bash
# Backend
cargo build --release

# Frontend (only if you modified it)
cd frontend
npm install
npm run build

# The Rust binary embeds frontend/dist/ at compile time
```

## License

MIT
