# Kiro Gateway

把自己的 Kiro 账号变成一个本地的 **OpenAI / Anthropic 兼容 API**，让 Claude Code、Cursor、Cline 等工具直接使用 Kiro 里的 Claude、GPT 等模型。

本项目由组内维护，基于开源项目 [jwadow/kiro-gateway](https://github.com/jwadow/kiro-gateway) 二次开发，许可证为 AGPL-3.0（见[许可证与署名](#许可证与署名)）。

> **使用原则**：每个人在自己电脑上运行，使用**自己的** Kiro 账号。不要共用别人的账号或网关。

---

## 目录

- [快速上手](#快速上手)
- [接入客户端](#接入客户端)
- [配置说明](#配置说明)
- [常见问题](#常见问题)
- [与上游的差异](#与上游的差异)
- [开发与维护](#开发与维护)
- [许可证与署名](#许可证与署名)

---

## 快速上手

**前提**：已安装 Python 3.10+，并且已经用 kiro-cli 登录过（`kiro-cli login`）。

### 1. 安装

```bash
git clone <仓库地址> kiro-gateway
cd kiro-gateway
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
```

Windows 下把 `.venv/bin/` 换成 `.venv\Scripts\`，下同。

### 2. 配置

在项目根目录新建 `.env` 文件：

```bash
# 客户端连接网关时使用的密码，自己随便定一个（可以用 openssl rand -hex 16 生成）
PROXY_API_KEY="换成你自己的随机字符串"

# kiro-cli 的登录数据库。token、profileArn、区域都从这里自动读取，不需要另外配置
# macOS:
KIRO_CLI_DB_FILE="~/Library/Application Support/kiro-cli/data.sqlite3"
# Windows（路径待验证，把 <用户名> 换成自己的；必须用正斜杠 /，双引号里的反斜杠会被当成转义符）:
# KIRO_CLI_DB_FILE="C:/Users/<用户名>/AppData/Local/Kiro-Cli/data.sqlite3"

# 推荐：关闭伪推理，避免 Opus 5 / 5.5、Sonnet 5.5 拒答（见常见问题）
FAKE_REASONING=false
```

`.env` 已被 git 忽略，不会被提交。

### 3. 启动

```bash
.venv/bin/python main.py --host 127.0.0.1
```

看到 `Uvicorn running on http://127.0.0.1:8000` 就说明启动成功。`--host 127.0.0.1` 表示只允许本机访问；端口可以用 `--port` 修改。

网关刷新 token 后会写回 kiro-cli 的数据库，和 kiro-cli 共用登录状态，两者不会互相把对方的登录顶掉。

### 4. 验证

```bash
curl -s localhost:8000/v1/models -H "Authorization: Bearer <你的 PROXY_API_KEY>"
```

返回的是你的账号可用的模型列表。

---

## 接入客户端

> 📖 **日常使用手册**（启动/停止、kclaude、日志、额度参考）：[docs/internal/local-setup.md](docs/internal/local-setup.md)

**Claude Code 推荐用 `kclaude`**：仓库自带的 shell 函数，网关没开时会自动启动，并把 Opus/Sonnet/Haiku 映射到 Kiro 模型。在 `~/.zshrc` 里加一行即可：

```bash
source "$HOME/kiro-gateway/scripts/kclaude.zsh"
```

之后用 `kclaude` 代替 `claude`，原来的 `claude` 不受影响。

| 客户端 | 配置方式 |
|---|---|
| **Claude Code（手动配置）** | `export ANTHROPIC_BASE_URL=http://localhost:8000`<br>`export ANTHROPIC_AUTH_TOKEN=<你的 PROXY_API_KEY>`<br>模型名用短横线写法，如 `claude-opus-5-5[1m]` |
| **Cursor / Cline / Cherry Studio 等** | 选择 OpenAI 兼容，Base URL 填 `http://localhost:8000/v1`，API Key 填你的 `PROXY_API_KEY` |
| **OpenAI SDK** | `OpenAI(base_url="http://localhost:8000/v1", api_key="<PROXY_API_KEY>")` |
| **Anthropic SDK** | `Anthropic(base_url="http://localhost:8000", api_key="<PROXY_API_KEY>")` |

**接口**

| 接口 | 说明 |
|---|---|
| `POST /v1/chat/completions` | OpenAI 格式，支持流式和非流式 |
| `POST /v1/messages` | Anthropic 格式，支持流式和非流式 |
| `GET /v1/models` | 可用模型列表 |
| `GET /health` | 健康检查 |

认证方式：OpenAI 格式用 `Authorization: Bearer <key>`，Anthropic 格式用 `x-api-key: <key>`。两种请求头所有接口都接受。

**模型名**：`claude-opus-5.5`、`claude-sonnet-4-6`、`claude-sonnet-4-5-20250929`、`claude-sonnet-4-6[1m]` 这些写法都可以，网关会自动转换成 Kiro 的格式。不在列表里的模型名也会原样转给 Kiro，由 Kiro 决定能不能用。`auto` / `auto-kiro` 表示由 Kiro 自动选择模型。

---

## 配置说明

所有配置都写在 `.env` 里，修改后需要重启网关。

| 配置项 | 默认值 | 说明 |
|---|---|---|
| `PROXY_API_KEY` | — | **必填**。客户端访问网关的密码 |
| `KIRO_CLI_DB_FILE` | — | kiro-cli 登录数据库路径（推荐的登录方式） |
| `FAKE_REASONING` | `true` | 伪推理开关，**推荐设为 `false`** |
| `SERVER_HOST` / `SERVER_PORT` | `0.0.0.0` / `8000` | 监听地址和端口；命令行参数 `--host` / `--port` 优先 |
| `WEB_SEARCH_ENABLED` | `true` | 是否支持 web_search 工具 |
| `STREAM_KEEPALIVE_INTERVAL` | `10` | 流式输出长时间没有内容时，发送 keepalive 的间隔（秒），`0` 表示关闭 |
| `EXPOSE_CREDITS_USED` | `false` | 是否在响应的 usage 中附带本次消耗的 credits |
| `REQUEST_LOG_DIR` | — | 设置后，每个请求在该目录的 `requests-YYYY-MM.jsonl` 里记一行（模型、状态、耗时、credits，不含对话内容）。桌面端用它统计 credits |
| `DEBUG_MODE` | `off` | `errors`：只保存失败请求的日志；`all`：保存所有请求。日志在 `debug_logs/` 目录 |
| `LOG_LEVEL` | `INFO` | 日志级别 |
| `VPN_PROXY_URL` | — | 需要走代理时填写，如 `http://127.0.0.1:7890` |

其他认证方式（Kiro IDE 凭证文件 `KIRO_CREDS_FILE`、`REFRESH_TOKEN` 等）和完整配置项见 [`.env.example`](.env.example) 与 [上游文档](docs/upstream/README.md)。

---

## 常见问题

**模型返回空内容，或者提示模型拒绝回答**

Kiro 的 API 不支持原生的扩展思考。上游的做法是往提示词里注入标签，让模型把推理过程写出来，再转成 thinking 块（即"伪推理"）。Opus 5、Opus 5.5、Sonnet 5.5 等新模型会把这种做法识别为套取推理过程，然后拒答。

解决办法：在 `.env` 里设置 `FAKE_REASONING=false` 并重启。之后客户端里就看不到"思考过程"了，但回答质量基本不受影响，速度还会更快，消耗的 credits 也更少。

**`Invalid model ID or insufficient subscription level`**

模型名写错了，或者你的订阅等级不支持这个模型。用 `/v1/models` 查看你的账号能用哪些模型。

**HTTP 402 `Monthly request limit exceeded`**

你的 Kiro 账号本月额度已经用完，这是 Kiro 服务端的限制，和网关无关。到 Kiro / kiro-cli 里查看用量和重置时间。不同模型的消耗倍率不同，例如 Opus 系列约为 Sonnet 的 2 倍。

**`prompt is too long` / `context_length_exceeded`**

对话超出了模型的上下文窗口。Claude Code 收到这个错误后会提示你压缩上下文，执行 `/compact` 即可。

**启动时报找不到凭证，或者请求返回 401 / 403**

- 检查 `KIRO_CLI_DB_FILE` 的路径是否正确
- 重新执行 `kiro-cli login` 登录，然后重启网关

**`profileArn is required`**

kiro-cli 数据库里没有 profile 信息。先用 kiro-cli 正常对话一次，或者在 `.env` 里手动设置 `PROFILE_ARN`。

**排查其他问题**

在 `.env` 里设置 `DEBUG_MODE=errors` 并重启，复现问题后到 `debug_logs/` 目录查看请求和响应。

---

## 与上游的差异

基线：上游 `a5292ca`（v2.4）。在此基础上合并了 17 个上游尚未合并的社区 PR，并补了 4 个修复。

### 合并的上游 PR

| 类别 | PR | 作用 |
|---|---|---|
| 模型 | [#289](https://github.com/jwadow/kiro-gateway/pull/289) | 同步 Kiro 最新模型列表（Opus 5.5、Sonnet 5.5、GPT-5.6 等）；修正 1M 上下文模型的 token 计数 |
| | [#184](https://github.com/jwadow/kiro-gateway/pull/184) | 修复 `auto-kiro` 等模型别名无法使用 |
| Claude Code 兼容 | [#264](https://github.com/jwadow/kiro-gateway/pull/264) | 支持 messages 中的 `role: system`（以前直接返回 422） |
| | [#164](https://github.com/jwadow/kiro-gateway/pull/164) | 支持 `redacted_thinking` 内容块 |
| 认证 / 搜索 | [#257](https://github.com/jwadow/kiro-gateway/pull/257) + [#213](https://github.com/jwadow/kiro-gateway/pull/213) | 修复企业 SSO 账号 web_search 返回 403 |
| | [#276](https://github.com/jwadow/kiro-gateway/pull/276) | API 区域不再错误地取自凭证文件的 SSO 区域 |
| | [#188](https://github.com/jwadow/kiro-gateway/pull/188) | `/v1/models` 支持 `x-api-key` 请求头 |
| | [#183](https://github.com/jwadow/kiro-gateway/pull/183) | tokenizer 缓存到本地，网络抖动时 token 计数不会失效 |
| 流式 / 解析 | [#268](https://github.com/jwadow/kiro-gateway/pull/268) | OpenAI 流式出错时把错误写进流里返回，不再返回空回复 |
| | [#288](https://github.com/jwadow/kiro-gateway/pull/288) | 长时间无输出时发送 keepalive，防止客户端断开 |
| | [#280](https://github.com/jwadow/kiro-gateway/pull/280) | 正确解析计费事件，日志中记录每次请求消耗的 credits |
| | [#225](https://github.com/jwadow/kiro-gateway/pull/225) | 修复大参数工具调用（如大文件 Write）被误判为截断 |
| | [#149](https://github.com/jwadow/kiro-gateway/pull/149) | 输出 token 计数包含工具调用 |
| 转换 | [#286](https://github.com/jwadow/kiro-gateway/pull/286) | 合并相邻消息时不再丢失图片 |
| | [#232](https://github.com/jwadow/kiro-gateway/pull/232) + [#169](https://github.com/jwadow/kiro-gateway/pull/169) | 工具 schema 中的 `oneOf` / `anyOf` / `allOf` 和 `required: null` 不再导致 400 |

### 自有修复

1. **模型拒答时明确告知客户端**：Kiro 用 `stopReason: CONTENT_FILTERED` 表示拒答，上游会忽略它，客户端只收到一个空的成功响应。现在 Anthropic 接口返回 `stop_reason: "refusal"`，OpenAI 接口返回 `finish_reason: "content_filter"`，日志里会写明原因和解决办法。
2. **上下文超长时返回标准错误**：OpenAI 接口返回 `invalid_request_error` / `context_length_exceeded`，Anthropic 接口返回 `invalid_request_error`，并且 message 以 `prompt is too long` 开头，和官方 API 一致，便于客户端识别后压缩上下文。
3. **修复 #149 的崩溃**：工具名或参数为空时，token 计数会抛异常。
4. **修复 #225 丢数据**：工具参数同时以字符串和对象两种形式下发时，有一部分会被丢弃。
5. **结构化请求日志（可选）**：设置 `REQUEST_LOG_DIR` 后，每个请求在 `requests-YYYY-MM.jsonl` 里记一行，包括时间、模型、状态、耗时、credits，不含对话内容。OpenAI / Anthropic、流式 / 非流式四条路径行为一致，失败、拒答、中途断开都会记录。桌面端用它统计 credits。
6. **不再输出 `(empty placeholder)`**：上游会给只有工具调用或工具结果的消息填占位文本，模型在长对话里会学着说这句话。现在这类消息发送空文本，历史里模型已经说过的也会被清理。
7. **支持超过 64 个字符的工具名**（重写上游 [#182](https://github.com/jwadow/kiro-gateway/pull/182)）：Kiro 限制工具名最长 64 个字符，上游遇到超长的就直接拒绝整个请求（例如 Claude Code 的 chrome-devtools 插件）。现在发给 Kiro 时自动缩短：保留末尾真正的工具名，把前面的插件前缀压缩成哈希；Kiro 返回时再还原成原名，客户端完全感知不到。
8. **网页搜索后的下一轮不再 422**：Claude Code 会把上一轮的 `server_tool_use` / `web_search_tool_result` 原样带回，以前直接校验失败。现在接受这两种块，并把搜索词、结果标题和链接转成助手文本交给 Kiro（丢弃无意义的 `encrypted_content`）。

### 待办

- [#165](https://github.com/jwadow/kiro-gateway/pull/165)：PDF 附件支持，需要先修改
- 一键安装脚本 + 开机自启（macOS / Windows）
- 确认 Windows 上 kiro-cli 数据库的路径

---

## 开发与维护

### 运行测试

```bash
.venv/bin/pytest -q
```

所有改动都必须附带测试，并保证全量测试通过。测试与网络完全隔离，不需要真实账号。代码规范见 [AGENTS.md](AGENTS.md)。

### 同步上游

本地仓库配置了两个远程：`internal` 指向本仓库，`origin` 指向 GitHub 上游。

```bash
git remote add origin https://github.com/jwadow/kiro-gateway   # 首次需要添加

# 同步上游 main
git fetch origin
git merge origin/main
.venv/bin/pytest -q
git push internal HEAD:main

# 合并上游的某个 PR
git fetch origin pull/<编号>/head:pr-<编号>
git merge --no-ff pr-<编号>
```

合并上游后记得同步更新本文档的"与上游的差异"一节。

---

## 许可证与署名

- 原项目：[jwadow/kiro-gateway](https://github.com/jwadow/kiro-gateway)，作者 [@Jwadow](https://github.com/jwadow) 及社区贡献者（见 [CONTRIBUTORS.md](CONTRIBUTORS.md)）
- 许可证：[AGPL-3.0](LICENSE)。本项目的修改同样以 AGPL-3.0 发布；向他人提供本软件时须同时提供源码，通过本仓库分发即满足这一要求
- 上游原始文档：[docs/upstream/README.md](docs/upstream/README.md)（其他语言版本在 `docs/` 下）
