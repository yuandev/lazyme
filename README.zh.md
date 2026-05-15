！！！
这一段是用户邀请保留的readme说明，请不要修改和删除用三个感叹号开始 三个感叹号结束
AI写的，问题很多，用不了就下源码用ai启动
！！！

# lazyme

[English](README.md)

单二进制、多目标的本地测试环境部署守护进程。自动监听 git 分支、拉取新提交、构建、缓存产物、管理运行中的进程，并提供一个暗色主题的 React 管理界面。

## 工作原理

```
targets.toml        .deployd/config.toml
     │                      │
     ▼                      ▼
┌─────────┐   轮询   ┌──────────┐   构建   ┌──────────┐
│ 注册表  │ ──────▶  │ 轮询循环 │ ───────▶ │ 构建缓存 │
│ (名称)  │          │ 每目标   │          │          │
└─────────┘          └──────────┘          └──────────┘
                                                 │
                           ┌─────────────────────┤
                           ▼                     ▼
                    ┌──────────┐          ┌──────────┐
                    │ 产物缓存 │          │ 运行命令 │
                    └──────────┘          └──────────┘
                                               │
                                    健康检查 (TCP)
```

- **轮询**: 每个目标按配置的间隔检查 `git ls-remote`
- **拉取**: 发现新提交时 fetch + checkout 远程分支
- **构建**: 执行构建命令，stdout 写入 `.deployd/logs/{hash}.log`（dev 模式下跳过）
- **缓存**: 构建产物复制到 `.deployd/artifacts/{short_hash}/`
- **运行**: 终止旧进程，启动新进程，替换 `{artifact}` 和 `{jvm_args}` 占位符
- **健康检查**: TCP 连接健康检查地址，超时前不断重试

## 快速开始

### 1. 下载

从 [GitHub Releases](https://github.com/yuandev/lazyme/releases) 下载预编译二进制：

- `deployd-x86_64-unknown-linux-gnu` — Linux x86_64
- `deployd-aarch64-apple-darwin` — macOS Apple Silicon

```bash
# macOS 示例
curl -L -o deployd https://github.com/yuandev/lazyme/releases/latest/download/deployd-aarch64-apple-darwin
chmod +x deployd
./deployd
```

无需安装任何依赖，前端已嵌入二进制文件中。

### 2. 注册目标

创建 `~/.config/lazyme/targets.toml`：

```toml
[[targets]]
name = "my-api"
repo = "/home/me/projects/my-api"
profile = "dev"   # 可选：加载 .deployd/config.dev.toml

[[targets]]
name = "frontend"
repo = "/home/me/projects/frontend"
```

### 3. 项目配置（可选）

在每个仓库中创建 `.deployd/config.toml`：

```toml
[watch]
branch = "main"

[build]
command = "mvn package -DskipTests"
artifact = "target/my-api.jar"
maven_settings = "/path/to/settings.xml"   # 可选
local_repo = "/path/to/maven/repo"         # 可选

[run]
mode = "deploy"              # "deploy"（默认，构建+运行）或 "dev"（仅运行，跳过构建）
command = "java {jvm_args} -jar {artifact}"
jvm_args = "-Xmx512m -Dserver.port=8080"
health_url = "http://localhost:8080/health"
health_timeout = 30

[env]
JAVA_HOME = "/usr/lib/jvm/java-17"
SPRING_PROFILES_ACTIVE = "prod"
```

所有字段均可选。CLI 参数覆盖项目配置，项目配置覆盖内置默认值。

### 4. 运行

```bash
# 监听所有目标
./deployd

# 监听指定目标
./deployd my-api frontend

# 自定义端口和轮询间隔
./deployd --port 9090 --interval 30

# 自定义远程名称
./deployd --remote upstream
```

打开 **http://localhost:8080** 查看管理界面。

## CLI 参数

```
lazyme [OPTIONS] [FILTER]...

参数:
  -R, --remote <REMOTE>    远程名称 [默认: origin]
  -i, --interval <SECS>    轮询间隔秒数 [默认: 60]
  -p, --port <PORT>        Web 界面端口 [默认: 8080]
  <FILTER>...              要监听的目标名称（为空表示全部）
```

## 功能

### Web 管理界面

- **状态页**: 实时展示已部署提交、git 状态、构建/运行命令、JVM 参数、环境变量
- **提交页**: 最近提交列表，支持回滚和查看日志
- **历史页**: 部署历史记录，显示缓存状态
- **配置页**: 编辑 `.deployd/config.toml`、Maven 配置、`vite.config.ts`、JVM 参数和环境变量
- **分支切换**: 下拉框展示所有远程分支，切换后自动持久化
- **克隆目标**: 复制目标配置到自定义路径，独立运行，互不影响
- **打开服务**: 一键在浏览器中打开已部署的服务
- **中英文切换**: header 右侧语言切换按钮

### 自更新

- 管理界面 header 处检查 GitHub Releases 新版本
- 流式下载带进度条显示
- 下载完成后手动重启（不会在更新过程中关闭界面）

### Dev 模式

在 `[run]` 中设置 `mode = "dev"` 跳过构建步骤。发现新提交时直接 pull + 重启运行命令，适用于 Node.js、Python 等无需编译的项目。

## 配置文件

### `~/.config/lazyme/targets.toml`（必需）

| 字段 | 必填 | 说明 |
|------|------|------|
| `name` | 是 | 唯一目标名称 |
| `repo` | 是 | 本地 git 仓库绝对路径 |
| `profile` | 否 | 加载 `.deployd/config.{profile}.toml` |

### `.deployd/config.toml`（按仓库配置，可选）

#### `[watch]`

| 字段 | 默认值 | 说明 |
|------|--------|------|
| `branch` | `main` | 监听的分支 |

#### `[build]`

| 字段 | 默认值 | 说明 |
|------|--------|------|
| `command` | `cargo build --release` | 构建命令 |
| `artifact` | 无 | 构建产物路径（相对于仓库根目录） |
| `maven_settings` | 无 | Maven settings.xml 路径 |
| `local_repo` | 无 | 本地 Maven 仓库路径 |

#### `[run]`

| 字段 | 默认值 | 说明 |
|------|--------|------|
| `mode` | `deploy` | `"deploy"`（构建+运行）或 `"dev"`（仅运行） |
| `command` | 无 | 启动命令，`{artifact}` 和 `{jvm_args}` 运行时替换 |
| `jvm_args` | 无 | JVM 启动参数 |
| `health_url` | 无 | 健康检查地址 |
| `health_timeout` | `30` | 健康检查超时秒数 |

#### `[env]`

键值对形式的环境变量，启动进程时注入。

## 磁盘布局

```
my-project/
└── .deployd/
    ├── config.toml          # 项目配置
    ├── state.json           # 部署历史（自动管理）
    ├── artifacts/
    │   └── a1b2c3d/
    │       └── my-binary    # 缓存的产物
    └── logs/
        └── a1b2c3d.log      # 构建日志
```

## API 参考

| 方法 | 路径 | 说明 |
|------|------|------|
| `GET` | `/ws` | WebSocket 事件 |
| `GET` | `/api/targets` | 列出所有目标 |
| `GET` | `/api/targets/{name}/status` | 目标状态 |
| `GET` | `/api/targets/{name}/commits` | 最近提交 |
| `GET` | `/api/targets/{name}/history` | 部署历史 |
| `GET` | `/api/targets/{name}/logs/{hash}` | 构建日志 |
| `POST` | `/api/targets/{name}/deploy` | 手动部署 |
| `POST` | `/api/targets/{name}/rollback` | 回滚到指定提交 |
| `GET` | `/api/targets/{name}/branches` | 列出远程分支 |
| `POST` | `/api/targets/{name}/branch` | 切换分支 |
| `POST` | `/api/targets/{name}/fetch` | 拉取最新代码 |
| `POST` | `/api/targets/{name}/clone` | 克隆目标 |
| `GET/PUT` | `/api/targets/{name}/config` | 读写 config.toml |
| `GET/PUT` | `/api/targets/{name}/vite-config` | 读写 vite.config.ts |
| `GET/PUT` | `/api/targets/{name}/env` | 读写 JVM 参数和环境变量 |
| `POST` | `/api/self-update` | 检查并下载更新 |
| `POST` | `/api/restart` | 重启服务 |
| `GET` | `/api/version` | 当前版本 |
| `GET` | `/api/queue` | 构建队列状态 |
| `POST` | `/api/reload` | 重新加载配置 |

## 从源码构建

```bash
git clone git@github.com:yuandev/lazyme.git
cd lazyme

# 后端（需要 Rust ≥ 1.82）
cargo build --release

# 前端（仅修改前端时需要，预编译的 dist/ 已提交）
cd frontend
npm install
npm run build
```

## 许可证

MIT
