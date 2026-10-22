# Kiro Gateway（内部版本）

> 这是 [jwadow/kiro-gateway](https://github.com/jwadow/kiro-gateway) 的内部维护版本，在上游 `a5292ca` 的基础上合并了 17 个尚未被上游合并的社区 PR，并补了 4 个修复。
> 本节为内部说明，**分隔线以下是上游原始 README**，保持原样以便同步。

它能把你的 Kiro 账号变成一个本地的 OpenAI / Anthropic 兼容 API，供 Claude Code、Cursor 等工具使用。**每个人用自己的 Kiro 账号在本机运行**，不要共用别人的账号。

## 快速上手

前提：已安装 Python 3.10+，并且已经用 kiro-cli 登录过（`kiro-cli login`）。

**1. 安装**

```bash
git clone <仓库地址>
cd kiro-gateway
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt        # Windows: .venv\Scripts\pip install -r requirements.txt
```

**2. 在项目根目录新建 `.env`**

```bash
# 客户端连网关用的密码，自己随便定一个（可以用 openssl rand -hex 16 生成）
PROXY_API_KEY="换成你自己的随机字符串"

# kiro-cli 的登录数据库，token、profileArn、区域都会从这里自动读取，不需要再配 PROFILE_ARN
# macOS:
KIRO_CLI_DB_FILE="~/Library/Application Support/kiro-cli/data.sqlite3"
# Windows（路径待验证，请把 <用户名> 换成自己的；务必用正斜杠 /，双引号里的反斜杠会被当成转义符）:
# KIRO_CLI_DB_FILE="C:/Users/<用户名>/AppData/Local/Kiro-Cli/data.sqlite3"

# 推荐：关闭伪推理，避免 Opus 5 / 5.5、Sonnet 5.5 拒答（见下文）
FAKE_REASONING=false
```

> ⚠️ 上游文档写的默认路径 `~/.local/share/kiro-cli/data.sqlite3` 在 macOS 上是错的，新版 kiro-cli 在 macOS 上把数据库放在 `~/Library/Application Support/kiro-cli/`。

**3. 启动**

```bash
.venv/bin/python main.py --host 127.0.0.1        # 默认端口 8000；--host 127.0.0.1 表示只允许本机访问
```

看到 `Uvicorn running on http://127.0.0.1:8000` 就说明启动成功。网关刷新 token 后会写回 kiro-cli 的数据库，和 kiro-cli 共用登录状态，不会互相顶掉。

**4. 接入客户端**

| 客户端 | 配置 |
|---|---|
| Claude Code | `export ANTHROPIC_BASE_URL=http://localhost:8000`<br>`export ANTHROPIC_API_KEY=<你的 PROXY_API_KEY>` |
| Cursor / Cline / Cherry Studio 等 | 选 OpenAI 兼容，Base URL 填 `http://localhost:8000/v1`，API Key 填你的 `PROXY_API_KEY` |

查看自己账号可用的模型：

```bash
curl -s localhost:8000/v1/models -H "Authorization: Bearer <你的 PROXY_API_KEY>"
```

模型名写成 `claude-opus-5.5`、`claude-sonnet-4-6`、`claude-sonnet-4-5-20250929` 这些格式都可以，网关会自动转换。

## 推荐配置：关闭伪推理

Kiro 的 API 不支持原生的扩展思考，上游默认会往提示词里注入标签，让模型把推理过程写出来，再转成 thinking 块。
Opus 5、Opus 5.5、Sonnet 5.5 等新模型会把这种做法识别为套取推理过程，直接拒答。设置 `FAKE_REASONING=false` 后：

- 新模型可以正常回答，回答质量基本不受影响
- 客户端里看不到"思考过程"
- 输出更少，速度更快，消耗的 credits 也更少

即使不关闭，本版本也会把拒答明确报告给客户端（见下方"自有修复"第 1 条），不会再返回莫名其妙的空回复。

## 与上游的差异

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

以上改动都附带单元测试，全量测试共 1936 个，均通过。

### 未合并的 PR

- 重复的 PR：同一问题只保留质量最好的一个
- 范围过大或夹带无关改动的 PR，例如 #244、#235、#228
- 会改动用户内容的 PR，例如 #215、#216、#238
- 与本组环境无关的 PR，例如 Docker、Codex 相关
- 待处理：#182（超长 MCP 工具名，方向正确但实现需要重写）、#165（PDF 附件，需要先修改）

## 维护

```bash
# 同步上游更新（origin 指向 GitHub 上游，internal 指向内部仓库）
git fetch origin
git merge origin/main
.venv/bin/pytest -q            # 必须全部通过
git push internal HEAD:main

# 合并上游的某个 PR
git fetch origin pull/<编号>/head:pr-<编号>
git merge --no-ff pr-<编号>
```

许可证：本项目使用 AGPL-3.0，内部分发时须同时提供源码。通过本仓库分发即可满足这一要求。

---

<div align="center">

# 👻 Kiro Gateway

**Proxy gateway for Kiro API (Amazon Q Developer / AWS CodeWhisperer)**

🇬🇧 English • [🇷🇺 Русский](docs/ru/README.md) • [🇨🇳 中文](docs/zh/README.md) • [🇪🇸 Español](docs/es/README.md) • [🇮🇩 Indonesia](docs/id/README.md) • [🇧🇷 Português](docs/pt/README.md) • [🇯🇵 日本語](docs/ja/README.md) • [🇰🇷 한국어](docs/ko/README.md)

Made with ❤️ by [@Jwadow](https://github.com/jwadow)

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)
[![Python 3.10+](https://img.shields.io/badge/python-3.10+-blue.svg)](https://www.python.org/downloads/)
[![FastAPI](https://img.shields.io/badge/FastAPI-0.100+-green.svg)](https://fastapi.tiangolo.com/)
[![Sponsor](https://img.shields.io/badge/💖_Sponsor-Support_Development-ff69b4)](#-support-the-project)

*Use Claude models from Kiro with Claude Code, OpenCode, OpenClaw, Claw Code, Codex app, Cursor, Cline, Roo Code, Kilo Code, Obsidian, OpenAI SDK, LangChain, Continue and other OpenAI or Anthropic compatible tools*

[Models](#-supported-models) • [Features](#-features) • [Quick Start](#-quick-start) • [Configuration](#%EF%B8%8F-configuration) • [💖 Sponsor](#-support-the-project)

</div>

---

## 🤖 Available Models (Free List)

> ⚠️ **Important:** Model availability depends on your Kiro tier (free/paid). The gateway provides access to whatever models are available in your IDE or CLI based on your subscription. The list below shows models commonly available on the **free tier**.

> 🔒 **Claude Opus 4.5** was removed from the free tier on January 17, 2026. It may be available on paid tiers — check your IDE/CLI model list.

> 💎 **Paid tiers** add newer models such as **Claude Opus 5.5**, **Claude Sonnet 5.5**, **Claude Opus 5**, **Claude Sonnet 5**, **GPT-5.6** (Sol, Terra, Luna) and **Kimi K3**, with up to 1M context. Availability depends on your plan and region — check your IDE/CLI model list.

> 📅 **Claude Sonnet 4** reached end of life in Kiro on October 14, 2026 and is no longer available.

🚀 **Claude Sonnet 4.5** — Balanced performance. Great for coding, writing, and general-purpose tasks.

⚡ **Claude Haiku 4.5** — Lightning fast. Perfect for quick responses, simple tasks, and chat.

💤 **GLM-5** — Open MoE model (744B params, 40B active). Advanced model for complex systems engineering and long-horizon agentic tasks.

🐋 **DeepSeek-V3.2** — Open MoE model (685B params, 37B active). Balanced performance for coding, reasoning, and general tasks.

🧩 **MiniMax M2.5** — Open MoE model (230B params, 10B active). Enhanced version with improved reasoning and task handling.

🧩 **MiniMax M2.1** — Open MoE model (230B params, 10B active). Great for complex tasks, planning, and multi-step workflows.

🤖 **Qwen3-Coder-Next** — Open MoE model (80B params, 3B active). Coding-focused. Excellent for development and large projects.

> 💡 **Smart Model Resolution:** Use any model name format — `claude-sonnet-4-5`, `claude-sonnet-4.5`, or even versioned names like `claude-sonnet-4-5-20250929`. The gateway normalizes them automatically.

---

## ✨ Features

| Feature | Description |
|---------|-------------|
| 🔌 **OpenAI-compatible API** | Works with any OpenAI-compatible tool |
| 🔌 **Anthropic-compatible API** | Native `/v1/messages` endpoint |
| 🔀 **Multi-Account Support** | Intelligent failover between multiple accounts |
| 🌐 **VPN/Proxy Support** | HTTP/SOCKS5 proxy for restricted networks |
| 🧠 **Extended Thinking** | Reasoning is exclusive to our project |
| 👁️ **Vision Support** | Send images to model |
| 🔍 **Web Search** | Search the web for current information |
| 🛠️ **Tool Calling** | Supports function calling |
| 💬 **Full message history** | Passes complete conversation context |
| 📡 **Streaming** | Full SSE streaming support |
| 🔄 **Retry Logic** | Automatic retries on errors (403, 429, 5xx) |
| 📋 **Extended model list** | Including versioned models |
| 🔐 **Smart token management** | Automatic refresh before expiration |

---

## 🚀 Quick Start

**Choose your deployment method:**
- 🐍 **Native Python** - Full control, easy debugging
- 🐳 **Docker** - Isolated environment, easy deployment → [jump to Docker](#-docker-deployment)

### Prerequisites

- Python 3.10+
- One of the following:
  - [Kiro IDE](https://kiro.dev/) with logged in account, OR
  - [Kiro CLI](https://kiro.dev/cli/) with AWS SSO (AWS IAM Identity Center, OIDC) - free Builder ID or corporate account

### Installation

```bash
# Clone the repository (requires Git)
git clone https://github.com/Jwadow/kiro-gateway.git
cd kiro-gateway

# Or download ZIP: Code → Download ZIP → extract → open kiro-gateway folder

# Install dependencies
pip install -r requirements.txt

# Configure (see Configuration section)
cp .env.example .env
# Copy and edit .env with your credentials

# Start the server
python main.py

# Or with custom port (if 8000 is busy)
python main.py --port 9000
```

The server will be available at `http://localhost:8000`

---

## ⚙️ Configuration

> 💡 **Advanced users:** Looking for multi-account support? See [Account System](#-account-system-advanced) below.

### Option 1: JSON Credentials File (Kiro IDE / Enterprise)

Specify the path to the credentials file:

Works with:
- **Kiro IDE** (standard) - for personal accounts
- **Enterprise** - for corporate accounts with SSO

```env
KIRO_CREDS_FILE="~/.aws/sso/cache/kiro-auth-token.json"

# Password to protect YOUR proxy server (make up any secure string)
# You'll use this as api_key when connecting to your gateway
PROXY_API_KEY="my-super-secret-password-123"
```

<details>
<summary>📄 JSON file format</summary>

```json
{
  "accessToken": "eyJ...",
  "refreshToken": "eyJ...",
  "expiresAt": "2025-01-12T23:00:00.000Z",
  "profileArn": "arn:aws:codewhisperer:us-east-1:...",
  "region": "us-east-1",
  "clientIdHash": "abc123..."  // Optional: for corporate SSO setups
}
```

> **Note:** If you have two JSON files in `~/.aws/sso/cache/` (e.g., `kiro-auth-token.json` and a file with a hash name), use `kiro-auth-token.json` in `KIRO_CREDS_FILE`. The gateway will automatically load the other file.

</details>

### Option 2: Environment Variables (.env file)

Create a `.env` file in the project root:

```env
# Required
REFRESH_TOKEN="your_kiro_refresh_token"

# Password to protect YOUR proxy server (make up any secure string)
PROXY_API_KEY="my-super-secret-password-123"

# Optional
PROFILE_ARN="arn:aws:codewhisperer:us-east-1:..."
KIRO_REGION="us-east-1"
```

### Option 3: AWS SSO Credentials (kiro-cli / Enterprise)

If you use `kiro-cli` or Kiro IDE with AWS SSO (AWS IAM Identity Center), the gateway will automatically detect and use the appropriate authentication.

Works with both free Builder ID accounts and corporate accounts.

```env
KIRO_CREDS_FILE="~/.aws/sso/cache/your-sso-cache-file.json"

# Password to protect YOUR proxy server
PROXY_API_KEY="my-super-secret-password-123"

# Note: PROFILE_ARN is NOT needed for AWS SSO (Builder ID and corporate accounts)
# The gateway will work without it
```

<details>
<summary>📄 AWS SSO JSON file format</summary>

AWS SSO credentials files (from `~/.aws/sso/cache/`) contain:

```json
{
  "accessToken": "eyJ...",
  "refreshToken": "eyJ...",
  "expiresAt": "2025-01-12T23:00:00.000Z",
  "region": "us-east-1",
  "clientId": "...",
  "clientSecret": "..."
}
```

**Note:** AWS SSO (Builder ID and corporate accounts) users do NOT need `profileArn`. The gateway will work without it (if specified, it will be ignored).

</details>

<details>
<summary>🔍 How it works</summary>

The gateway automatically detects the authentication type based on the credentials file:

- **Kiro Desktop Auth** (default): Used when `clientId` and `clientSecret` are NOT present
  - Endpoint: `https://prod.{region}.auth.desktop.kiro.dev/refreshToken`
  
- **AWS SSO (OIDC)**: Used when `clientId` and `clientSecret` ARE present
  - Endpoint: `https://oidc.{region}.amazonaws.com/token`

No additional configuration is needed — just point to your credentials file!

</details>

### Option 4: kiro-cli SQLite Database

If you use `kiro-cli` and prefer to use its SQLite database directly:

```env
KIRO_CLI_DB_FILE="~/.local/share/kiro-cli/data.sqlite3"

# Password to protect YOUR proxy server
PROXY_API_KEY="my-super-secret-password-123"

# Note: PROFILE_ARN is NOT needed for AWS SSO (Builder ID and corporate accounts)
# The gateway will work without it
```

<details>
<summary>📄 Database locations</summary>

| CLI Tool | Database Path |
|----------|---------------|
| kiro-cli | `~/.local/share/kiro-cli/data.sqlite3` |
| amazon-q-developer-cli | `~/.local/share/amazon-q/data.sqlite3` |

The gateway reads credentials from the `auth_kv` table which stores:
- `kirocli:odic:token` or `codewhisperer:odic:token` — access token, refresh token, expiration
- `kirocli:odic:device-registration` or `codewhisperer:odic:device-registration` — client ID and secret

Both key formats are supported for compatibility with different kiro-cli versions.

</details>

### Getting Credentials

**For Kiro IDE users:**
- Log in to Kiro IDE and use Option 1 above (JSON credentials file)
- The credentials file is created automatically after login

**For Kiro CLI users:**
- Log in with `kiro-cli login` and use Option 3 or Option 4 above
- No manual token extraction needed!

<details>
<summary>🔧 Advanced: Manual token extraction</summary>

If you need to manually extract the refresh token (e.g., for debugging), you can intercept Kiro IDE traffic:
- Look for requests to: `prod.us-east-1.auth.desktop.kiro.dev/refreshToken`

</details>

---

## 🔀 Account System (Advanced)

Account System is a way to manage multiple Kiro accounts with automatic failover. In the future, this system will replace `.env` file for credential configuration, but currently it's optional and intended for those who want to use multiple accounts.

### Why You Need This

If you have multiple Kiro accounts, the gateway can automatically switch between them when account is temporarily unavailable.

The system works with a single account too — just without switching.

### How to Enable

Add to your `.env`:

```env
ACCOUNT_SYSTEM=true
```

**What happens:**
- On first startup, your credentials from `.env` are automatically migrated to `credentials.json` (one-time)
- After that, all account and region settings from `.env` are ignored
- Account management only through `credentials.json`

<details>
<summary>📄 Configuration Examples</summary>

**Single account:**
```json
[
  {
    "type": "json",
    "path": "~/.aws/sso/cache/kiro-auth-token.json"
  }
]
```

**Multiple accounts:**
```json
[
  {
    "type": "json",
    "path": "~/.aws/sso/cache/kiro-auth-token.json"
  },
  {
    "type": "sqlite",
    "path": "~/.local/share/kiro-cli/data.sqlite3"
  },
  {
    "type": "refresh_token",
    "refresh_token": "eyJhbGc...",
    "profile_arn": "arn:aws:codewhisperer:us-east-1:..."
  }
]
```

**Folder with files:**
```json
[
  {
    "type": "json",
    "path": "C:\\MyAccs\\kiro67"
  }
]
```

The gateway will scan all files in the folder and add them as separate accounts.

</details>

### How Failover Works

When one account returns an error (429 rate limit, 402 quota exceeded), the gateway automatically tries the next account from the list. If an account fails several times in a row, the gateway temporarily stops using it and periodically checks if it has recovered.

For a single account, failover doesn't work — you get the original error from Kiro API.

For complete configuration examples (including per-account region settings), see [`credentials.json.example`](credentials.json.example).

---

## 🐳 Docker Deployment

> **Docker-based deployment.** Prefer native Python? See [Quick Start](#-quick-start) above.

### Quick Start

```bash
# 1. Clone and configure
git clone https://github.com/Jwadow/kiro-gateway.git
cd kiro-gateway
cp .env.example .env
# Edit .env with your credentials

# 2. Run with docker-compose
docker-compose up -d

# 3. Check status
docker-compose logs -f
curl http://localhost:8000/health
```

### Docker Run (Without Compose)

<details>
<summary>🔹 Using Environment Variables</summary>

```bash
docker run -d \
  -p 8000:8000 \
  -e PROXY_API_KEY="my-super-secret-password-123" \
  -e REFRESH_TOKEN="your_refresh_token" \
  --name kiro-gateway \
  ghcr.io/jwadow/kiro-gateway:latest
```

</details>

<details>
<summary>🔹 Using Credentials File</summary>

**Linux/macOS:**
```bash
docker run -d \
  -p 8000:8000 \
  -v ~/.aws/sso/cache:/home/kiro/.aws/sso/cache:ro \
  -e KIRO_CREDS_FILE=/home/kiro/.aws/sso/cache/kiro-auth-token.json \
  -e PROXY_API_KEY="my-super-secret-password-123" \
  --name kiro-gateway \
  ghcr.io/jwadow/kiro-gateway:latest
```

**Windows (PowerShell):**
```powershell
docker run -d `
  -p 8000:8000 `
  -v ${HOME}/.aws/sso/cache:/home/kiro/.aws/sso/cache:ro `
  -e KIRO_CREDS_FILE=/home/kiro/.aws/sso/cache/kiro-auth-token.json `
  -e PROXY_API_KEY="my-super-secret-password-123" `
  --name kiro-gateway `
  ghcr.io/jwadow/kiro-gateway:latest
```

</details>

<details>
<summary>🔹 Using .env File</summary>

```bash
docker run -d -p 8000:8000 --env-file .env --name kiro-gateway ghcr.io/jwadow/kiro-gateway:latest
```

</details>

### Docker Compose Configuration

Edit `docker-compose.yml` and uncomment volume mounts for your OS:

```yaml
volumes:
  # Kiro IDE credentials (choose your OS)
  - ~/.aws/sso/cache:/home/kiro/.aws/sso/cache:ro              # Linux/macOS
  # - ${USERPROFILE}/.aws/sso/cache:/home/kiro/.aws/sso/cache:ro  # Windows
  
  # kiro-cli database (choose your OS)
  - ~/.local/share/kiro-cli:/home/kiro/.local/share/kiro-cli  # Linux/macOS
  # - ${USERPROFILE}/.local/share/kiro-cli:/home/kiro/.local/share/kiro-cli  # Windows
  
  # Debug logs (optional)
  - ./debug_logs:/app/debug_logs
```

### Management Commands

```bash
docker-compose logs -f      # View logs
docker-compose restart      # Restart
docker-compose down         # Stop
docker-compose pull && docker-compose up -d  # Update
```

<details>
<summary>🔧 Building from Source</summary>

```bash
docker build -t kiro-gateway .
docker run -d -p 8000:8000 --env-file .env kiro-gateway
```

</details>

---

## 🌐 VPN/Proxy Support

**For users in China, corporate networks, or regions with connectivity issues to AWS services.**

The gateway supports routing all Kiro API requests through a VPN or proxy server. This is essential if you experience connection problems to AWS endpoints or need to use a corporate proxy.

### Configuration

Add to your `.env` file:

```env
# HTTP proxy
VPN_PROXY_URL=http://127.0.0.1:7890

# SOCKS5 proxy
VPN_PROXY_URL=socks5://127.0.0.1:1080

# With authentication (corporate proxies)
VPN_PROXY_URL=http://username:password@proxy.company.com:8080

# Without protocol (defaults to http://)
VPN_PROXY_URL=192.168.1.100:8080
```

### Supported Protocols

- ✅ **HTTP** — Standard proxy protocol
- ✅ **HTTPS** — Secure proxy connections
- ✅ **SOCKS5** — Advanced proxy protocol (common in VPN software)
- ✅ **Authentication** — Username/password embedded in URL

### When You Need This

| Situation | Solution |
|-----------|----------|
| Connection timeouts to AWS | Use VPN/proxy to route traffic |
| Corporate network restrictions | Configure your company's proxy |
| Regional connectivity issues | Use a VPN service with proxy support |
| Privacy requirements | Route through your own proxy server |

### Popular VPN Software with Proxy Support

Most VPN clients provide a local proxy server you can use:
- **Sing-box** — Modern VPN client with HTTP/SOCKS5 proxy
- **Clash** — Usually runs on `http://127.0.0.1:7890`
- **V2Ray** — Configurable SOCKS5/HTTP proxy
- **Shadowsocks** — SOCKS5 proxy support
- **Corporate VPN** — Check your IT department for proxy settings

Leave `VPN_PROXY_URL` empty (default) if you don't need proxy support.

---

## 📡 API Reference

### Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/` | GET | Health check |
| `/health` | GET | Detailed health check |
| `/v1/models` | GET | List available models |
| `/v1/chat/completions` | POST | OpenAI Chat Completions API |
| `/v1/messages` | POST | Anthropic Messages API |

---

## 💡 Usage Examples

### OpenAI API

<details>
<summary>🔹 Simple cURL Request</summary>

```bash
curl http://localhost:8000/v1/chat/completions \
  -H "Authorization: Bearer my-super-secret-password-123" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-5",
    "messages": [{"role": "user", "content": "Hello!"}],
    "stream": true
  }'
```

> **Note:** Replace `my-super-secret-password-123` with the `PROXY_API_KEY` you set in your `.env` file.

</details>

<details>
<summary>🔹 Streaming Request</summary>

```bash
curl http://localhost:8000/v1/chat/completions \
  -H "Authorization: Bearer my-super-secret-password-123" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-5",
    "messages": [
      {"role": "system", "content": "You are a helpful assistant."},
      {"role": "user", "content": "What is 2+2?"}
    ],
    "stream": true
  }'
```

</details>

<details>
<summary>🛠️ With Tool Calling</summary>

```bash
curl http://localhost:8000/v1/chat/completions \
  -H "Authorization: Bearer my-super-secret-password-123" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-5",
    "messages": [{"role": "user", "content": "What is the weather in London?"}],
    "tools": [{
      "type": "function",
      "function": {
        "name": "get_weather",
        "description": "Get weather for a location",
        "parameters": {
          "type": "object",
          "properties": {
            "location": {"type": "string", "description": "City name"}
          },
          "required": ["location"]
        }
      }
    }]
  }'
```

</details>

<details>
<summary>🐍 Python OpenAI SDK</summary>

```python
from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:8000/v1",
    api_key="my-super-secret-password-123"  # Your PROXY_API_KEY from .env
)

response = client.chat.completions.create(
    model="claude-sonnet-4-5",
    messages=[
        {"role": "system", "content": "You are a helpful assistant."},
        {"role": "user", "content": "Hello!"}
    ],
    stream=True
)

for chunk in response:
    if chunk.choices[0].delta.content:
        print(chunk.choices[0].delta.content, end="")
```

</details>

<details>
<summary>🦜 LangChain</summary>

```python
from langchain_openai import ChatOpenAI

llm = ChatOpenAI(
    base_url="http://localhost:8000/v1",
    api_key="my-super-secret-password-123",  # Your PROXY_API_KEY from .env
    model="claude-sonnet-4-5"
)

response = llm.invoke("Hello, how are you?")
print(response.content)
```

</details>

### Anthropic API

<details>
<summary>🔹 Simple cURL Request</summary>

```bash
curl http://localhost:8000/v1/messages \
  -H "x-api-key: my-super-secret-password-123" \
  -H "anthropic-version: 2023-06-01" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-5",
    "max_tokens": 1024,
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

> **Note:** Anthropic API uses `x-api-key` header instead of `Authorization: Bearer`. Both are supported.

</details>

<details>
<summary>🔹 With System Prompt</summary>

```bash
curl http://localhost:8000/v1/messages \
  -H "x-api-key: my-super-secret-password-123" \
  -H "anthropic-version: 2023-06-01" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-5",
    "max_tokens": 1024,
    "system": "You are a helpful assistant.",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

> **Note:** In Anthropic API, `system` is a separate field, not a message.

</details>

<details>
<summary>📡 Streaming</summary>

```bash
curl http://localhost:8000/v1/messages \
  -H "x-api-key: my-super-secret-password-123" \
  -H "anthropic-version: 2023-06-01" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-5",
    "max_tokens": 1024,
    "stream": true,
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

</details>

<details>
<summary>🐍 Python Anthropic SDK</summary>

```python
import anthropic

client = anthropic.Anthropic(
    api_key="my-super-secret-password-123",  # Your PROXY_API_KEY from .env
    base_url="http://localhost:8000"
)

# Non-streaming
response = client.messages.create(
    model="claude-sonnet-4-5",
    max_tokens=1024,
    messages=[{"role": "user", "content": "Hello!"}]
)
print(response.content[0].text)

# Streaming
with client.messages.stream(
    model="claude-sonnet-4-5",
    max_tokens=1024,
    messages=[{"role": "user", "content": "Hello!"}]
) as stream:
    for text in stream.text_stream:
        print(text, end="", flush=True)
```

</details>

---

## 🔧 Debugging

Debug logging is **disabled by default**. To enable, add to your `.env`:

```env
# Debug logging mode:
# - off: disabled (default)
# - errors: save logs only for failed requests (4xx, 5xx) - recommended for troubleshooting
# - all: save logs for every request (overwrites on each request)
DEBUG_MODE=errors
```

### Debug Modes

| Mode | Description | Use Case |
|------|-------------|----------|
| `off` | Disabled (default) | Production |
| `errors` | Save logs only for failed requests (4xx, 5xx) | **Recommended for troubleshooting** |
| `all` | Save logs for every request | Development/debugging |

### Debug Files

When enabled, requests are logged to the `debug_logs/` folder:

| File | Description |
|------|-------------|
| `request_body.json` | Incoming request from client (OpenAI format) |
| `kiro_request_body.json` | Request sent to Kiro API |
| `response_stream_raw.txt` | Raw stream from Kiro |
| `response_stream_modified.txt` | Transformed stream (OpenAI format) |
| `app_logs.txt` | Application logs for the request |
| `error_info.json` | Error details (only on errors) |

---

## 🔧 Troubleshooting

### Connection Issues

**Error: "Name or service not known" or DNS resolution failed**

The Q API endpoint may not be publicly resolvable in your region. Use a VPN or proxy:

```env
VPN_PROXY_URL=http://127.0.0.1:7890
```

See [VPN/Proxy Support](#-vpnproxy-support) for details.

---

**Error: "503 Service Unavailable" through proxy**

The Q API endpoint exists in specific regions only. Try a different region:

```env
KIRO_API_REGION="eu-central-1"  # or us-east-1
```

Commonly reachable regions: `us-east-1`, `eu-central-1`

---

**OIDC works but Q API fails**

Your SSO region may differ from the Q API region. The gateway auto-detects this from credentials, but you can override:

```env
KIRO_API_REGION="eu-central-1"
```

---

## 📜 License

This project is licensed under the **GNU Affero General Public License v3.0 (AGPL-3.0)**.

This means:
- ✅ You can use, modify, and distribute this software
- ✅ You can use it for commercial purposes
- ⚠️ **You must disclose source code** when you distribute the software
- ⚠️ **Network use is distribution** — if you run a modified version on a server and let others interact with it, you must make the source code available to them
- ⚠️ Modifications must be released under the same license

See the [LICENSE](LICENSE) file for the full license text.

### Why AGPL-3.0?

AGPL-3.0 ensures that improvements to this software benefit the entire community. If you modify this gateway and deploy it as a service, you must share your improvements with your users.

### Contributor License Agreement (CLA)

By submitting a contribution to this project, you agree to the terms of our [Contributor License Agreement (CLA)](CLA.md). This ensures that:
- You have the right to submit the contribution
- You grant the maintainer rights to use and relicense your contribution
- The project remains legally protected

---

## 💖 Support the Project

<div align="center">

<img src="https://raw.githubusercontent.com/Tarikul-Islam-Anik/Animated-Fluent-Emojis/master/Emojis/Smilies/Smiling%20Face%20with%20Hearts.png" alt="Love" width="80" />

**If this project saved you time or money, consider supporting it!**

Every contribution helps keep this project alive and growing

<br>

### 🤑 Donate

[**☕ One-time Support**](https://app.lava.top/products/b4e34d12-3b6b-49b7-be50-50b6a20ed262/f3ea941f-de73-4ad1-bbb6-f82042ef8132)

<br>

### 🪙 Or send crypto

| Currency | Network | Address |
|:--------:|:-------:|:--------|
| **USDT** | TRC20 | `TSVtgRc9pkC1UgcbVeijBHjFmpkYHDRu26` |
| **BTC** | Bitcoin | `12GZqxqpcBsqJ4Vf1YreLqwoMGvzBPgJq6` |
| **ETH** | Ethereum | `0xc86eab3bba3bbaf4eb5b5fff8586f1460f1fd395` |
| **SOL** | Solana | `9amykF7KibZmdaw66a1oqYJyi75fRqgdsqnG66AK3jvh` |
| **TON** | TON | `UQBVh8T1H3GI7gd7b-_PPNnxHYYxptrcCVf3qQk5v41h3QTM` |

</div>

---

## ⚠️ Disclaimer

This project is not affiliated with, endorsed by, or sponsored by Amazon Web Services (AWS), Anthropic, or Kiro IDE. Use at your own risk and in compliance with the terms of service of the underlying APIs.

---

<div align="center">

**[⬆ Back to Top](#-kiro-gateway)**

</div>
