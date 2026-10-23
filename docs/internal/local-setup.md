# 本机使用手册

在自己的 Mac 上运行 kiro-gateway，并让 Claude Code 通过它使用 Kiro 的模型。

首次安装见 [README 快速上手](../../README.md#快速上手)。本文记录日常怎么启动、怎么配置、怎么排查问题。

---

## 一、日常使用：kclaude

`kclaude` 是一个 shell 函数，用法和 `claude` 完全一样，只是请求走 Kiro：

```bash
kclaude                 # 交互模式
kclaude -p "..."        # 非交互模式
claude                  # 不受影响，仍然走原来的账号
```

它会自动完成三件事：

1. 从 `~/kiro-gateway/.env` 读取网关密码 `PROXY_API_KEY`
2. 网关没在运行时，自动在后台启动它（只监听本机）。所以电脑重启后直接用 `kclaude` 即可，不需要手动启动网关
3. 把 Claude Code 的三档模型映射到 Kiro 的模型：

| Claude Code 里选 | 实际调用 | 上下文 |
|---|---|---|
| Opus（默认） | claude-opus-5.5 | 1M |
| Sonnet | claude-sonnet-5.5 | 1M |
| Haiku（后台小任务，比如生成标题、做摘要） | claude-haiku-4.5 | 200K |

在 Claude Code 里可以用 `/model sonnet`、`/model opus` 切换模型。

### 安装 kclaude

脚本在仓库的 [`scripts/kclaude.zsh`](../../scripts/kclaude.zsh)。在 `~/.zshrc` 里加一行，然后执行 `source ~/.zshrc`：

```bash
source "$HOME/kiro-gateway/scripts/kclaude.zsh"
```

如果 `~/.zshrc` 末尾有要求必须放在最后的配置（例如 Kiro CLI 的 post block），这一行要加在它上面。

### 自定义

在 `source` 那一行之前设置下列环境变量即可覆盖默认值：

| 变量 | 默认值 | 说明 |
|---|---|---|
| `KIRO_GATEWAY_DIR` | `~/kiro-gateway` | 项目目录 |
| `KIRO_GATEWAY_PORT` | `8000` | 网关端口 |
| `KIRO_GATEWAY_LOG` | `~/Library/Logs/kiro-gateway.log` | 日志文件 |
| `KCLAUDE_OPUS_MODEL` | `claude-opus-5-5[1m]` | Opus 档对应的模型 |
| `KCLAUDE_SONNET_MODEL` | `claude-sonnet-5-5[1m]` | Sonnet 档对应的模型 |
| `KCLAUDE_HAIKU_MODEL` | `claude-haiku-4-5` | Haiku 档对应的模型 |

例如想把默认模型换成 Sonnet 来节省额度：

```bash
export KCLAUDE_OPUS_MODEL="claude-sonnet-5-5[1m]"
source "$HOME/kiro-gateway/scripts/kclaude.zsh"
```

**模型名的写法**：这里要用 Claude Code 认识的短横线写法（`claude-opus-5-5`），不要用 Kiro 的带点写法（`claude-opus-5.5`）。用带点的写法，Claude Code 会提示不认识这个模型，并按 200K 上下文处理，导致过早压缩对话。`[1m]` 后缀用来告诉 Claude Code 这个模型支持 1M 上下文。网关会自动把这些写法转换成 Kiro 的模型 ID。

### 你会看到的提示

`claude.ai connectors are disabled...`：这是正常的。用 kclaude 时认证走的是网关，你在 claude.ai 账号里配置的连接器不会加载。普通的 `claude` 不受影响。

---

## 二、手动启动和停止网关

一般不需要手动操作，kclaude 会自动启动网关。需要手动操作时：

```bash
# 启动（前台运行，Ctrl+C 停止）
cd ~/kiro-gateway && .venv/bin/python main.py --host 127.0.0.1

# 停止后台运行的网关
pkill -f "main.py --host 127.0.0.1"

# 检查是否在运行
curl -s localhost:8000/health        # 返回 "healthy" 即正常
```

**修改 `.env` 之后必须重启网关才会生效**：先停止，再执行一次 `kclaude`，它会自动重新启动网关。

---

## 三、配置（`.env`）

`.env` 放在项目根目录，已被 git 忽略。推荐配置：

```bash
PROXY_API_KEY="自己定的随机字符串"
KIRO_CLI_DB_FILE="~/Library/Application Support/kiro-cli/data.sqlite3"
FAKE_REASONING=false
```

| 配置项 | 作用 |
|---|---|
| `PROXY_API_KEY` | 客户端访问网关的密码，可以随便定。kclaude 会自动读取，改了不用改别处 |
| `KIRO_CLI_DB_FILE` | kiro-cli 的登录数据库。token、profileArn、区域都从这里自动读取。网关刷新 token 后会写回这个文件，和 kiro-cli 共用登录状态 |
| `FAKE_REASONING=false` | 关闭伪推理。不关的话，Opus 5、Opus 5.5、Sonnet 5.5 会拒答（详见第五节） |

也可以改用 Kiro IDE 的凭证文件，把 `KIRO_CLI_DB_FILE` 换成 `KIRO_CREDS_FILE="~/.aws/sso/cache/kiro-auth-token.json"`。但企业 SSO 账号用这种方式时，需要另外配置 `PROFILE_ARN`。用 kiro-cli 数据库则不需要。

全部配置项见 [`.env.example`](../../.env.example)。

---

## 四、日志：怎么确认请求走了网关

由 kclaude 启动的网关，日志写在 `~/Library/Logs/kiro-gateway.log`。

实时查看请求：

```bash
tail -f ~/Library/Logs/kiro-gateway.log | grep --line-buffered -E "Request to|Credit|HTTP [0-9]{3} -"
```

每次请求在日志里对应三行：

```
Request to /v1/messages (model=claude-opus-5-5, stream=True)        ← 请求到了网关，后面是所用的模型
[Credit] req=msg_... model=claude-opus-5-5 credits=0.72 calls=1     ← Kiro 这次扣了多少额度
HTTP 200 - POST /v1/messages (streaming) - completed                ← 成功；出错时这里是 4xx/5xx 和原因
```

其他确认方式：在 kclaude 里执行 `/status`，看到 Base URL 是 `http://127.0.0.1:8000` 就对了。

日志只记录请求概况，不记录对话内容。文件会一直变大，偶尔清空一下：`: > ~/Library/Logs/kiro-gateway.log`。

需要排查某个请求的完整内容时，在 `.env` 里加上 `DEBUG_MODE=errors` 并重启网关，失败请求的完整内容会保存到 `~/kiro-gateway/debug_logs/`。

---

## 五、常见问题

| 现象 | 原因和处理 |
|---|---|
| 回复为空，或者提示模型拒绝回答 | 伪推理导致新模型拒答。在 `.env` 里设置 `FAKE_REASONING=false` 后重启。日志里会有一行 `Model refused to respond ... REASONING_EXTRACTION` |
| HTTP 402 `Monthly request limit exceeded` | Kiro 账号本月额度用完了，这是 Kiro 服务端的限制，与网关无关。到 Kiro / kiro-cli 里查看用量 |
| `Invalid model ID or insufficient subscription level` | 模型名写错了，或者订阅不支持这个模型。用 `curl -s localhost:8000/v1/models -H "Authorization: Bearer <密码>"` 查看可用模型 |
| `prompt is too long` | 超出上下文窗口了，在 Claude Code 里执行 `/compact` |
| 401 / 403，或者启动时报找不到凭证 | 执行 `kiro-cli login` 重新登录，然后重启网关 |
| Claude Code 提示不认识某个模型 | 模型名用了带点的写法，改成短横线写法（见第一节） |
| 报 `Tool name(s) exceed Kiro API limit of 64 characters` | 网关是旧版本。更新代码后重启网关即可，超长工具名现在会自动缩短 |
| 回复里出现 `(empty placeholder)` | 网关是旧版本。更新代码后重启网关即可 |
| kclaude 提示网关启动失败 | 查看 `~/Library/Logs/kiro-gateway.log` 的最后几行 |

---

## 六、额度消耗参考

Kiro 按 credits 计费，不同模型的倍率不同。Claude Code 每次请求都会带上很长的系统提示词，Kiro 也会附加它自己的系统提示，所以哪怕只问一句话，开销也不小。下面是在 Claude Code 里只发一句话的实测消耗：

| 模型 | 每次请求约消耗 |
|---|---|
| claude-haiku-4.5 | 0.04 credits |
| claude-sonnet-4.6 | 0.16 credits |
| claude-sonnet-5.5 | 0.29 credits |
| claude-opus-5.5 | 0.45～0.7 credits |

日常写代码用 Sonnet 就够了，复杂任务再切到 Opus。每次请求实际扣了多少，可以在日志的 `[Credit]` 行里看到。

---

## 七、仓库与分支

| 位置 | 说明 |
|---|---|
| 内部仓库 | 组内 Git 仓库（本地远程名为 `internal`） |
| 上游 | `https://github.com/jwadow/kiro-gateway`（本地远程名为 `origin`） |
| 本地工作分支 | `local-merged`，对应内部仓库的 `main` |

同步上游、合并上游 PR 的步骤见 [README 开发与维护](../../README.md#开发与维护)。

---

## 八、后续计划（尚未实施）

- **一键安装脚本 + 开机自启**（macOS / Windows）：自动探测 kiro-cli 数据库、生成 `.env`、注册后台服务
- **服务器部署，多人共用**：可行。用户、key、用量和计费交给上层的 API 网关平台管理，kiro-gateway 只作为它的一个上游。建议每个 Kiro 账号跑一个网关实例，由上层平台负责负载均衡。kiro-gateway 自带的多账号模式只做故障切换（用完一个再换下一个），不做负载均衡。上线前需要先确认公司的 Kiro 订阅允许多人共用账号

---

## 九、桌面端（Kiro Gateway App）

桌面端把上面手动做的事装进一个 App：内置网关、一个开关启停、自动装好 `kclaude` 命令、检测 kiro-cli 与 Claude Code、展示请求日志与 credits、应用内升级。代码在 [`desktop/`](../../desktop/)，规则见 [`desktop/AGENTS.md`](../../desktop/AGENTS.md)，行为定义见 `openspec/changes/add-desktop-app/`。

### 安装与启动

出包（macOS，需要 Rust 1.99、pnpm、Python 3.10+，Rust 在 `~/.cargo/bin`）：

```bash
cd desktop
make sidecar            # 用 PyInstaller 把网关打成 sidecar，输出到 src-tauri/resources/gateway/
make dmg-local          # 出未签名 dmg：src-tauri/target/release/bundle/dmg/
```

`make dmg-local` 会拉起 Finder 排版 dmg 窗口；在无人值守的环境里用 `CI=true pnpm tauri build --bundles app,dmg` 跳过排版。未签名的 App 被 Gatekeeper 拦住时：

```bash
xattr -dr com.apple.quarantine "/Applications/Kiro Gateway.app"
```

打开 App 后：环境检测四项（kiro-cli 已安装、已登录、登录有效、Claude Code 已安装）全部通过，才能打开开关。开关打开后网关监听 `127.0.0.1:<端口>`，关闭窗口只是隐藏，托盘菜单可以启停网关、重启以完成更新、退出；从托盘退出时网关一起停止。

### 和手动网关、`scripts/kclaude.zsh` 的关系

| | 手动方式（前面几节） | 桌面端 |
|---|---|---|
| 网关 | 仓库里 `python main.py`，配置在 `~/kiro-gateway/.env` | 内置 sidecar，配置在 App 数据目录，不读 `.env` |
| 访问密钥 | `.env` 里的 `PROXY_API_KEY` | App 首次启动随机生成，存在数据目录的 `secret`（权限 0600），不需要手填 |
| 默认端口 | 8000 | 6868，可在设置里改（1024–65535） |
| 命令名 | `kclaude` | 默认 `cclaude`，可在设置里改名，所以和手动的 `kclaude` 可以同时存在 |
| 命令 | `source scripts/kclaude.zsh`，网关没起会自动后台启动 | 标记段 `source` 数据目录下的 `<命令名>.zsh`，网关没起只提示，不替你启动 |

App 数据目录：`~/Library/Application Support/dev.kirogateway.desktop/`。里面有 `config.json`、`secret`、`kclaude.env`（文件名固定，内部文件）、`<命令名>.zsh`（默认 `cclaude.zsh`）、`logs/gateway.log`、`logs/requests/`（请求日志）、`runtime/`、`gateway.pid`。

**默认端口改成了 6868，不再和手动网关的 8000 冲突。** 以前用过 App 的人，`config.json` 里已经保存的端口（比如 8000）保持不变，只有新装的才默认 6868。如果还是遇到「端口已被占用」，App 会给出占用者的进程名和 pid，不会去结束它。处理办法：先关掉对方，或者在 App 设置里换一个端口（改端口后需要重启网关才生效）。

### 命令的启动文件管理

App 启动时会在 `~/.zshrc`（bash 用户是 `~/.bashrc` / `~/.bash_profile`）里追加一个带标记的段，只有这一段归 App 管：

```
# >>> kiro-gateway-desktop command >>>
[ -r '…/cclaude.zsh' ] && source '…/cclaude.zsh'
# <<< kiro-gateway-desktop command <<<
```

- 写之前先备份成 `<原文件名>.bak-kiro-gateway-YYYYMMDD-HHMMSS`，备份失败就不写；标记段之外的内容逐字节不动。
- **改命令名**：设置里把 `cclaude` 改成别的名字（例如测试时用 `kclaude-test`）。名字只能用字母、数字、`-`、`_`，以字母开头，2 到 32 个字符；不能是 `claude` 或常见系统命令（`ls`、`git`、`node` 等）；PATH 里已有同名可执行文件、或启动文件里已有同名函数 / alias / `source` 时提示「这个名字已经被占用」。改完后启动文件里仍然只有一段，指向新名字的脚本，旧名字的脚本文件删除，新开的终端里旧名字不再存在。
- 旧版本（命令固定叫 `kclaude`）写的标记段是 `# >>> kiro-gateway-desktop kclaude >>>`，App 认得它：启动时就地换成新标记段，并删掉旧的 `kclaude.zsh`。
- **你现在的 `~/.zshrc` 里有 `source "$HOME/kiro-gateway/scripts/kclaude.zsh"`**（手动方式）。默认命令名是 `cclaude`，所以它不再算「已有同名命令」：App 照常装上 `cclaude`，你那一行原样不动，两个命令可以同时用。只有把 App 的命令名也改成 `kclaude` 时才会提示被占用。
- 已经打开的终端不会自动生效，新开一个终端。命令脚本每次运行都会读 `kclaude.env`，所以在 App 里改端口、模型映射后，已打开的终端里的命令下一次运行就生效；改命令名则需要新开终端。
- 移除后，App 下次启动不会再自动装回（数据目录里有 `kclaude.removed` 标记；重新点「安装」会清除它）。移除状态下改命令名只保存名字，不写启动文件。
- App 不碰 `claude` 命令本身、`~/.claude/` 和全局 `ANTHROPIC_*` 环境变量。

### 升级

升级包放在腾讯云 COS，App 启动时和之后每 60 分钟读一次 `latest.json`：

- 普通更新：静默下载并校验签名，下次启动 App 时自动安装；也可以在状态栏或托盘点「重启以完成更新」。
- 强制升级（清单里 `minRequiredVersion` 高于当前版本）：停网关，显示无法关闭的升级遮罩；期间开关和托盘启动项不可用，`kclaude` 提示「需要升级」并以非零状态退出。下载失败时遮罩上有重试和安装包直链。

发版用 `desktop/scripts/release.sh`（流程、签名私钥位置、`--dry-run` 见 `desktop/AGENTS.md` 的「发版」一节）。

### 排查

- 网关日志：`~/Library/Application Support/dev.kirogateway.desktop/logs/gateway.log`（5 MB × 3 份轮转），界面上有「打开完整日志」。
- 请求日志在 `logs/requests/requests-YYYY-MM.jsonl`，只保留本月和上月，界面上可以清空。
- 登录失效：界面会给出 `<kiro-cli 绝对路径> login` 的复制按钮；App 不会替你执行它。
- 无窗口验收：debug 构建的二进制支持 `--selftest`，在临时 HOME 里走一遍安装 kclaude、起真 sidecar、跑 `kclaude`、停网关、移除的流程，不碰真实的启动文件（见 `desktop/AGENTS.md`）。
