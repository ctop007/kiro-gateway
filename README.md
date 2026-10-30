# Kiro Gateway

用自己的 Kiro 账号跑 Claude Code。

## 下载

到 [Releases](https://github.com/ctop007/kiro-gateway/releases/latest) 下载：

- **macOS**：`Kiro-Gateway_<版本>_universal.dmg`（Intel 和 Apple Silicon 通用）
- **Windows**：`Kiro-Gateway_<版本>_x64-setup.exe`

## 使用

1. 先装好 [kiro-cli](https://kiro.dev/cli/) 并登录（`kiro-cli login`），再装好 Claude Code。
2. 打开 Kiro Gateway，打开开关。
3. 新开一个终端，用 `cclaude` 代替 `claude`。原来的 `claude` 不受影响。

App 会自动检查环境，有问题会告诉你怎么修。请求日志页能看到每次请求花了多少 credits。命令名和端口（默认 6868）可以在设置里改。新版本会在后台自动下载，下次打开时生效。

**第一次打开**：安装包没有商业签名。macOS 提示无法打开时，在终端运行 `xattr -dr com.apple.quarantine "/Applications/Kiro Gateway.app"`；Windows 弹出 SmartScreen 时，点「更多信息」→「仍要运行」。

## 常见问题

- **模型拒绝回答**：Kiro 不支持原生的扩展思考，App 已经默认关掉了伪推理（`FAKE_REASONING=false`）。如果还出现，换个问法或换个模型。
- **HTTP 402 `Monthly request limit exceeded`**：上游拒绝了当前请求的额度。先核对 kiro-cli 当前账号、用量和 profile 是否一致；账号页面仍有额度时，不要直接判断为整个账号额度耗尽。
- **`prompt is too long`**：对话超出了上下文窗口，在 Claude Code 里执行 `/compact`。
- **登录失效（401 / 403）**：重新 `kiro-cli login`，再打开开关。
- **原生 kiro-cli 可用，但网关报 403**：网关会先刷新一次凭证；如果 runtime 仍明确拒绝 SSO token，自动尝试不带缓存 profile 的 Q 接口。成功后仅在当前进程、当前账号内记住该路径。失败会保留真实 HTTP 状态和原因，不再误报 `504 / Unknown error`。源码运行可用 `KIRO_SSO_LEGACY_FALLBACK=false` 关闭兼容重试。

## 不用 App，直接跑网关

网关本身是一个 OpenAI / Anthropic 兼容的本地 API，Cursor、Cline 等工具也能用：

```bash
python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
cp .env.example .env    # 至少填 PROXY_API_KEY 和 KIRO_CLI_DB_FILE
.venv/bin/python main.py --host 127.0.0.1
```

接口：`POST /v1/chat/completions`（OpenAI）、`POST /v1/messages`（Anthropic）、`GET /v1/models`。配置项见 [`.env.example`](.env.example)，详细说明见 [`docs/internal/local-setup.md`](docs/internal/local-setup.md)。

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

## 开发

- 网关：`.venv/bin/pytest -q`，规范见 [AGENTS.md](AGENTS.md)
- 桌面端：见 [`desktop/AGENTS.md`](desktop/AGENTS.md)。发版：改版本号并提交，打带说明的 tag `desktop-vX.Y.Z` 推上去，GitHub Actions 自动打包并发布 Release

## 许可证与署名

- 原项目：[jwadow/kiro-gateway](https://github.com/jwadow/kiro-gateway)，作者 [@Jwadow](https://github.com/jwadow) 及社区贡献者（见 [CONTRIBUTORS.md](CONTRIBUTORS.md)）
- 许可证：[AGPL-3.0](LICENSE)。本项目的修改同样以 AGPL-3.0 发布；向他人提供本软件时须同时提供源码，通过本仓库分发即满足这一要求
- 上游原始文档：[docs/upstream/README.md](docs/upstream/README.md)（其他语言版本在 `docs/` 下）
