# Proposal

## Why

现在要用 Kiro Gateway，得先 clone 仓库、建 venv、手写 `.env`、在 `~/.zshrc` 里 source `scripts/kclaude.zsh`。这一套只有会折腾的人走得通，组里其他同事（尤其是 Windows 用户）很难自己装起来。做一个双击就能装的桌面 App（macOS + Windows），让同事装好后直接在终端里敲 `kclaude` 就能用 Kiro 跑 Claude Code，原来的 `claude` 完全不受影响。

## What Changes

- 新增 `desktop/`：Tauri 2 + Rust 桌面壳，前端 React + TypeScript，技术栈与 参考项目 `desktop-app` 保持一致。出两种包：macOS dmg、Windows NSIS 安装器。
- Python 网关用 PyInstaller 打成独立目录，放进安装包，由 App 作为子进程拉起和停止。用户机器上不需要装 Python。
- **一个开关**：启动 / 停止 KiroGateway，并实时显示状态（已停止 / 启动中 / 运行中 / 启动失败及原因）。网关只监听 `127.0.0.1`。
- **kclaude 命令**：App 启动时检查 `kclaude` 是否已安装，没有就自动装上。macOS 往 `~/.zshrc` 加一段带标记的 source，Windows 往 PowerShell profile 里加。只新增 `kclaude`，不改 `claude` 本身，也不改 Claude Code 的任何配置文件。
- **环境检测**：自动检查 kiro-cli 是否安装、是否已登录、登录是否还有效，以及 Claude Code 是否安装。哪一项没通过，就给出一句能照着做的修复提示；检测不通过时开关不能打开。
- **请求日志页**：每次请求一行，显示时间、接口、模型、成功或失败、耗时、credits；顶部显示今日和本月的 credits 合计。只记元数据，不记对话内容。
- **网关新增结构化请求日志**（可选开启，默认关闭）：每个 `/v1/chat/completions`、`/v1/messages` 请求结束时写一行 JSONL，内容包括时间、接口、模型、是否流式、状态码、耗时、credits、错误摘要。四条路径都要覆盖：OpenAI / Anthropic × 流式 / 非流式。桌面端开启它并读取。网关直接运行（不经过桌面端）时行为不变。

## Capabilities

### New Capabilities

- `gateway/request-log`：网关的结构化请求日志，每个请求一条记录，含 credits，四条路径行为一致，可通过配置开启。
- `desktop/gateway-control`：桌面端对内置网关进程的启停、状态展示、端口与密钥管理、退出清理。
- `desktop/environment-check`：桌面端对 kiro-cli 安装、登录及登录有效性和 Claude Code 安装的检测与修复提示。
- `desktop/kclaude-command`：桌面端安装、检测和更新 `kclaude` 命令（macOS zsh、Windows PowerShell），且不影响现有 `claude`。
- `desktop/request-log-viewer`：桌面端请求日志页，含每次请求的 credits 以及今日、本月合计。

### Modified Capabilities

无（`openspec/specs/` 目前为空）。

## Impact

- **新增目录**：`desktop/`（Tauri 工程、前端、sidecar 打包脚本、规则文件 `desktop/AGENTS.md`）。
- **网关代码**：新增 `kiro/request_log.py`；在 `kiro/config.py` 新增配置项；`kiro/routes_openai.py`、`kiro/routes_anthropic.py`、`kiro/streaming_openai.py`、`kiro/streaming_anthropic.py` 接入请求记录；`.env.example` 和 README 配置表同步。默认关闭，不影响现有用户。
- **新依赖**：Rust 工具链（本机尚未安装，需 rustup）、Tauri 2 及插件、PyInstaller（只在打包时用）。
- **用户机器上的写盘**：App 数据目录；`~/.zshrc` 或 PowerShell profile 里带标记的一段（写前备份）。
- **不涉及**：kiro-cli 本身（只读它的数据库，网关刷新 token 时会写回，这是现有行为）；Claude Code 的配置文件；`origin` 上游仓库。
