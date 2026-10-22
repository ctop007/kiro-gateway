# Design

## Context

- 动机和范围见 proposal.md，行为定义见 `specs/`。
- 网关现状：
  - 每个请求的 credits 只出现在 loguru 文本日志里，格式是 `[Credit] req=<id> model=<m> credits=<x> calls=<n>`。
  - 写这行日志的地方有三处：`streaming_openai.py:296`（OpenAI 的流式和非流式都经过这里，非流式由 `collect_stream_response` 复用内部流）、`streaming_anthropic.py:549`（Anthropic 流式）、`streaming_anthropic.py:798`（Anthropic 非流式）。
  - 拒答由 `UpstreamStop.is_refusal` 判定。
  - `routes_openai.py` 和 `routes_anthropic.py` 各有两套分支：账号系统的故障切换分支和 legacy 分支。每套分支里都有流式 `stream_wrapper`、非流式、上游错误、`HTTPException`、兜底 `Exception` 这几个出口，HTTP 状态行目前只是 `logger.info/error` 的文本。
  - 422 由 `kiro/exceptions.py` 的 `validation_exception_handler` 返回，根本走不到路由函数。
  - `DebugLoggerMiddleware` 已经限定只处理 `/v1/chat/completions` 和 `/v1/messages` 两个端点。
- kiro-cli 数据库：
  - macOS 路径是 `~/Library/Application Support/kiro-cli/data.sqlite3`。
  - 登录凭证在 `auth_kv` 表里，key 为 `kirocli:social:token`、`kirocli:odic:token` 或 `codewhisperer:odic:token`，value 是 JSON，包含 `access_token`、`refresh_token`、`expires_at` 等字段。
  - Windows 上的路径还没确认，README 待办里有这一条。
- 网关读配置的方式：`kiro/config.py` 在 import 时执行 `load_dotenv()`，从当前目录读 `.env`，再从环境变量读。sidecar 可以完全靠环境变量传配置，不需要写 `.env`。
- 参考实现：参考项目 `参考项目`，用的是 Tauri 2 + React 18 + TS + Vitest，带 single-instance、tray、autostart、updater，shell rc 写入时带标记并备份。本项目照它的结构和约束来，但不照搬它的业务模块。
- 本机还没装 Rust 工具链。

## Goals / Non-Goals

**Goals:**

- 同事装完 App、打开开关，新开终端就能用 `kclaude`，不用接触 Python、`.env` 和命令行配置。
- 每个请求的 credits 记录准确，日志页里的数字和 `[Credit]` 日志一致。
- 网关这边的改动不影响不用桌面端的用户：新功能默认关闭，所有现有测试都要保持通过。

**Non-Goals：**

- 第一版不做 Apple 签名公证与 Windows Authenticode 签名（同事首次安装要手动放行一次，见 README）。应用内静默升级和强制升级第一版就做，见决策 11。
- 不做 Linux 包。
- 不在 App 里替用户执行 `kiro-cli login`。登录需要浏览器交互，App 只给命令和复制按钮。
- 不显示请求和回复的内容。
- 不支持网关的多账号系统（`ACCOUNT_SYSTEM`）配置界面。sidecar 固定走单账号的 kiro-cli 数据库。
- 不替换 `scripts/kclaude.zsh`。不用桌面端的人照旧用它，两者共存的规则见决策 6。

## Decisions

### 1. 网关以 PyInstaller onedir 打包为 sidecar，由 Rust 管理进程

- 用 PyInstaller 的 `--onedir` 模式，把 `main.py` 连同依赖打成一个目录，作为 Tauri 资源（`bundle.resources`）放进安装包，由 Rust 用 `std::process::Command` 启动和管理。
- 不选 `--onefile`：它每次启动都要先解压到临时目录，冷启动多 1 到 3 秒。Windows 上杀毒软件经常拦截临时目录里的 exe，macOS 公证时内嵌的二进制也不好签名。onedir 启动快，签名可以逐个文件处理。
- 不用 Tauri 的 `externalBin` sidecar 机制：它只支持单个可执行文件，跟 onedir 不兼容。
- tiktoken 的 BPE 文件在构建时预先下载，放进包内。启动时通过 `TIKTOKEN_CACHE_DIR` 指向它，避免首次运行时去联网下载（`kiro/tokenizer.py` 已经支持这个环境变量）。
- 包体：Python 3.13 运行时加 FastAPI、uvicorn、httpx、tiktoken，预计压缩后 25 到 45 MB，tasks 里有一项实测。参考项目 定的 20 MB 上限这里不适用。

### 2. 配置全部通过环境变量传给 sidecar

Rust 启动 sidecar 时用**白名单**：先清空环境，只放行系统必要变量（`PATH`、`HOME`、`USER`、`LOGNAME`、`TMPDIR`、`LANG`、`LC_*`、`SHELL`、Windows 的 `SYSTEMROOT`/`WINDIR`/`APPDATA`/`LOCALAPPDATA`/`USERPROFILE`/`TEMP`/`TMP`/`COMSPEC`/`PATHEXT`、`SSL_CERT_FILE`/`SSL_CERT_DIR`，以及 `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`/`NO_PROXY` 及其小写形式——代理要保留，国内用户靠它访问 Kiro），其余继承来的变量（包括 `ANTHROPIC_*`、`PROXY_*`、`DEBUG_MODE`、`VPN_PROXY_URL`、`REFRESH_TOKEN` 等所有网关配置项）一律不传，再显式设置以下内容：

- `PROXY_API_KEY`：App 自己生成的密钥。
- `KIRO_CLI_DB_FILE`：环境检测时找到的路径。
- `SERVER_HOST=127.0.0.1`，以及 `SERVER_PORT`。
- `FAKE_REASONING=false`，原因见 README 常见问题。
- `REQUEST_LOG_DIR`：App 数据目录下的 `logs/requests`。
- `TIKTOKEN_CACHE_DIR`。
- `LOG_LEVEL=INFO`。

sidecar 的工作目录设成 App 数据目录下一个空的 `runtime/`，保证 `load_dotenv()` 读不到任何意外的 `.env`。stdout 和 stderr 写入 `logs/gateway.log`，按大小轮转，最多保留 5 MB × 3 份。

### 3. App 数据目录与配置文件

- 位置：用 Tauri 的 `app_data_dir()`。macOS 是 `~/Library/Application Support/<identifier>`，Windows 是 `%APPDATA%\<identifier>`。identifier 暂定为 `dev.kirogateway.desktop`。
- 目录结构：
  - `config.json`：端口、模型映射、kclaude 管理状态。
  - `secret`：访问密钥，权限 0600。
  - `logs/requests/`
  - `logs/gateway.log`
  - `runtime/`
  - `gateway.pid`
- 密钥不放系统钥匙串，原因是 `kclaude` 每次运行都要读它。钥匙串在 macOS 上会弹授权框，Windows 的 PowerShell 读取起来也麻烦。而这个密钥只能访问本机 127.0.0.1 上的网关，泄露的影响仅限本机，用一个 0600 权限的文件就够了。

### 4. 网关新增 `kiro/request_log.py`，在路由层收口，credits 从流式层带出

`RequestRecord` 是一个 dataclass。请求开始时（路由函数入口）创建，记下 `ts`、`api`、`model`、`stream`、`t0`。各个出口负责填 `status`、`outcome`、`error`，最后调用 `finish(record)`。`finish` 的规则：

- 每个请求只写一次，用 `finished` 标志防止重复写。
- 用一把 `threading.Lock`（模块级）保护写入，以 `open(path, "a")` 写一行后立刻关闭文件，保证并发时行与行不交错，外部删除文件后下次写入会重新创建。
- 写入失败时捕获 `OSError`，只在第一次失败时打一条 warning。
- `REQUEST_LOG_DIR` 为空时 `finish` 直接返回。

credits 和响应 ID 由流式层带出：三处 `[Credit]` 日志的位置旁边，把 `credits_used` 和 `message_id`/`completion_id` 写回调用方传进来的 `record`，即新增一个可选参数 `request_record: Optional[RequestRecord] = None`。拒答也在这里标记为 `outcome="refused"`，位置是 `is_refusal` 分支旁边。不新增参数的调用方行为不变。

四条路径各自的改法：

- **OpenAI 非流式**：`collect_stream_response` 透传 `request_record` 给 `stream_kiro_to_openai_internal`，路由在 `JSONResponse` 返回前 `finish`。
- **OpenAI 流式**：`stream_wrapper` 的 `finally` 里根据 `streaming_error` / `client_disconnected` 设置 outcome，然后 `finish`。credits 由内部流在结束或出错前写入 record。
- **Anthropic 非流式**：`collect_anthropic_response` 在 798 行附近写入 record，路由在返回前 `finish`。
- **Anthropic 流式**：与 OpenAI 流式对称，549 行附近写入，`stream_wrapper` 的 `finally` 里 `finish`。

错误出口：账号系统分支和 legacy 分支里的 FATAL 上游错误、`HTTPException`、兜底 `Exception`、全部账号失败后的最终返回，每个出口都在返回或 raise 之前 `finish`，status 填实际返回的状态码。为避免漏掉出口，路由函数体外包一层 `try/finally`：如果 `finally` 时 record 还没写，并且不是流式请求已经交给 `stream_wrapper` 接管的情况，就以 `status=500, outcome=error` 补写，同时打一条 debug 日志标明有出口漏了。测试里会对这个兜底断言"从不触发"。

422：`validation_exception_handler` 只在 path 属于 `LOGGED_ENDPOINTS` 时写一条记录，`model` 尽量从原始 body 里取，取不到就写 null，`stream` 写 false。

`error` 字段截取用户可见错误信息的前 200 字符。错误信息一律从已经整理过的 `last_error_message` / `str(e)` 来，不直接用上游原始 body。

被否决的方案：
- 解析 loguru 文本日志：格式随意，失败请求的状态行和 `[Credit]` 行之间没有共同 ID，跨行关联不可靠。
- 用 ASGI 中间件统一记录：中间件拿不到 credits；流式响应要包住 body iterator 才知道什么时候结束，跟现有 `stream_wrapper` 的断开检测重复。

### 5. 桌面端读取请求日志：Rust 增量读取 + 内存聚合

Rust 侧维护每个月份文件的读取偏移，每 1 秒 `stat` 一次当月文件，有新增就读出新的行并解析。解析失败的行跳过；行尾没有换行符的半行先不处理，等下一轮。

- 聚合（今日、本月、按模型）在 Rust 内存里做。前端通过 command 拿到最近 N 条记录和聚合结果，有更新时 Rust 发 event 通知前端。
- App 启动时读取本月和上月两个文件完成初始化。
- 每天启动时删除早于上上个月的文件，因为网关每次写入都会重新打开文件，删除是安全的。
- 文件规模估算：每条约 250 字节，重度用户一个月 5 万条约 12 MB，全量读一次在 100 ms 级别，不需要 SQLite。

### 6. kclaude 的实现：启动文件里只放一行引用，逻辑放在 App 管理的脚本里

- macOS：
  - App 把 `kclaude.zsh` 写到 App 数据目录（bash 用户写 `kclaude.bash`）。
  - 在 `~/.zshrc` 末尾追加一段，用 `# >>> kiro-gateway-desktop kclaude >>>` 和 `# <<< kiro-gateway-desktop kclaude <<<` 包住，中间只有一行 source。
  - 脚本每次运行时读 `config.json` 和 `secret`，先探测 `/health`，失败就提示用户并退出。
  - 写入前先备份成 `.zshrc.bak-kiro-gateway-YYYYMMDD-HHMMSS`，备份失败就不写。
- Windows：
  - 写 `kclaude.ps1` 到 App 数据目录。
  - 对 Windows PowerShell 5 和 PowerShell 7 各自的 `$PROFILE.CurrentUserAllHosts`，用 `# >>> kiro-gateway-desktop kclaude >>>` 标记追加一行 dot-source。
  - profile 路径由 Rust 按已知规则计算：`Documents\WindowsPowerShell\profile.ps1` 和 `Documents\PowerShell\profile.ps1`，Documents 用 `SHGetKnownFolderPath`，以处理 OneDrive 重定向。不调用 powershell 进程。
- 只升级脚本内容（比如改了提示文案）时，不需要再动启动文件。
- 发现启动文件里已有不带本标记的 `kclaude`（比如用户手动 source 了 `scripts/kclaude.zsh`，你现在的 `~/.zshrc` 就是这样）时，不写入，界面显示「检测到已有 kclaude」，并提供「改由 App 管理」按钮。点了以后注释掉旧行，再加上本标记段，同样先备份。
- 已运行的终端不会自动生效，界面文案明确写「新开一个终端」。
- 模型映射的默认值和 `scripts/kclaude.zsh` 保持一致：opus 用 `claude-opus-5-5[1m]`，sonnet 用 `claude-sonnet-5-5[1m]`，haiku 用 `claude-haiku-4-5`。

### 7. 环境检测在 Rust 侧做，SQLite 只读

- 用 `rusqlite`（bundled），以 `mode=ro` 打开数据库，设 `busy_timeout`，只读 `auth_kv` 里那三个 token key，判断是否存在、`expires_at`、是否有 `refresh_token`。读到的值不出 Rust 函数，只返回结论（登录方式、过期时间、是否可刷新）。
- kiro-cli 和 claude 的查找顺序：
  1. 登录 shell 的 PATH。macOS 上 GUI 进程的 PATH 不全，启动时用 `$SHELL -lic 'echo $PATH'` 取一次并缓存，超时 3 秒，失败就退回默认位置。
  2. 已知默认位置。macOS 查 `~/.local/bin/`、`/opt/homebrew/bin`、`/usr/local/bin`。Windows 查 `%LOCALAPPDATA%` 下各工具的安装位置，以及 `%USERPROFILE%\.local\bin`。
- Windows 的 kiro-cli 数据库路径要在真机上确认，tasks 里有一项专门做这件事。确认之前，按候选路径依次尝试。

### 8. 进程生命周期与端口

- 启动前先检查端口：尝试 bind `127.0.0.1:<port>`，失败就是被占用。
- 如果 `gateway.pid` 里的进程还活着，并且它的可执行文件路径是本 App 的 sidecar，就认定是本 App 上次遗留的网关，先结束它。否则一律视为别的程序，绝不 kill。
- 启动 sidecar 后每 500 ms 探测一次 `/health`，30 秒超时。
- 运行期间单独开一个线程 `wait` 子进程，进程退出就立刻发事件把状态改为「意外退出」，带上 `gateway.log` 最后 20 行的摘要（先过滤掉 token 样式的字符串）。
- 停止：macOS 先发 SIGTERM，等 5 秒没退再 SIGKILL。Windows 直接 `TerminateProcess`，因为子进程没有控制台，收不到 Ctrl+C。
- App 退出时（托盘「退出」、`RunEvent::Exit`）同步停止网关。
- Windows 下用 Job Object 绑定子进程，保证 App 崩溃时子进程也跟着结束。macOS 依靠下次启动时检查 pid 文件来回收。

### 9. 前端界面与技术栈

- 技术栈沿用 参考项目 的约束：React 18 + TS + Vite、纯 CSS 变量、`useState`/`useReducer`、Vitest，不引入状态库、路由库或图表库。
- 单窗口两个页签：
  - 「主页」：环境检测清单、网关开关和状态、kclaude 状态和模型映射。
  - 「请求日志」：汇总卡片、按模型分组、列表和筛选。
- 文案只做中文，暂不做 i18n，因为目前只给组内用。
- 用到的 Tauri 插件：single-instance（第一个注册）、opener（打开安装页）、clipboard-manager（复制 `kiro-cli login`）、autostart（设置里提供开关，默认关闭）。
- 所有文件读写和进程管理都在 Rust command 里完成，前端不开 fs、shell 权限。

### 10. 仓库与构建

- 目录结构：
  - `desktop/`：Tauri 工程。
  - `desktop/scripts/build-sidecar.(sh|ps1)`：用仓库根目录的网关代码和独立的构建 venv 跑 PyInstaller，产物输出到 `desktop/src-tauri/resources/gateway/`，这个目录 gitignore。
  - `desktop/Makefile`：提供 `dev`、`test`、`check`、`sidecar`、`dmg-local` 这几个目标。
  - `desktop/AGENTS.md`：桌面端规则。
- Windows 包只能在 Windows 上构建。第一版先在 macOS 本机出 dmg。Windows 包等 CI 或 Windows 机器再出，tasks 里单独列一项。

### 11. 应用内升级：COS 上的静态清单 + Tauri updater 签名包

- 发布物全部放腾讯云 COS（公共读）：
  - 版本清单 `kiro-gateway/desktop/latest.json`
  - mac 更新包 `Kiro-Gateway.app.tar.gz` 与 `.sig`、dmg（首次安装用）
  - Windows NSIS 安装器 `*-setup.exe` 与 `.sig`（Windows 的更新包就是安装器本身）
- 清单格式在 Tauri updater 的 static JSON 基础上加一个 `minRequiredVersion`：
  ```json
  {
    "version": "0.0.2",
    "minRequiredVersion": "0.0.1",
    "notes": "一行一条的更新说明",
    "pub_date": "2026-11-01T10:00:00+08:00",
    "platforms": {
      "darwin-aarch64": { "url": "https://<bucket>.cos.<region>.myqcloud.com/kiro-gateway/desktop/0.0.2/Kiro-Gateway.app.tar.gz", "signature": "<.sig 内容>" },
      "darwin-x86_64":  { "url": "...", "signature": "..." },
      "windows-x86_64": { "url": ".../Kiro-Gateway_0.0.2_x64-setup.exe", "signature": "..." }
    }
  }
  ```
- 决定动作的纯函数 `decide_update(current, manifest, platform)`，结果三选一：
  - `none`：没有更新的版本，或清单不可用。
  - `background`：有新版本，当前版本不低于 `minRequiredVersion`。
  - `forced`：当前版本低于 `minRequiredVersion`。
  - 版本号按 `x.y.z` 数字逐段比较，任何一方解析失败都算 `none`，宁可不升也不误升。
- 检查放在 Rust 定时器里：启动时查一次，之后每 60 分钟查一次。窗口隐藏后 webview 会被挂起，JS 定时器会停，参考项目 也是因为这个原因把检查放在 Rust。用 `tauri-plugin-updater` 的 `check()` 读清单，`download()` 下载并校验 minisign 签名。
- `background`：下载到 updater 的缓存后先不安装，在 App 数据目录记一个 `pending-update.json`。下次启动时先安装再进主界面，用户也可以在状态栏或托盘点「立即重启」。安装前先停网关，重启后按 `config.json` 里记的开关状态恢复。
- `forced`：立即停网关，Rust 侧置 `upgrade_required` 标志（开关和托盘的启动命令都直接拒绝，`config.json` 写入 `upgradeRequired: true` 让 kclaude 脚本给出升级提示），前端显示不可关闭的升级遮罩，下载并安装完成后 `restart()`。下载失败时遮罩上显示「重试」和 dmg / exe 的直链，不回退到可用界面。
- 签名私钥：minisign 私钥只存在维护者本机（比如 `~/.kiro-gateway/desktop-updater.key`）和 CI Secret 里，公钥写进 `tauri.conf.json`。私钥丢了，就再也发不出能被旧版接受的更新。
- 发布：用 `desktop/scripts/release.sh VERSION=x.y.z NOTES=notes.md [FORCE=1]` 出包、签名、上传 COS，最后才覆盖 `latest.json`。上传顺序保证清单永远不会指向还没传完的文件。`FORCE=1` 时把 `minRequiredVersion` 设为本次版本。上传用 `coscli`，凭证从本机配置或环境变量读取，不进仓库。
- 被否决的方案：参考项目 的后端发布通道。这个项目没有后端，COS 静态文件已经够用，而且 Tauri updater 原生就支持 static JSON。

## Risks / Trade-offs

- [PyInstaller 漏收动态导入的模块，比如 uvicorn 的 `uvicorn.protocols.*`、`uvicorn.loops.*`] → 构建脚本显式加上 `--collect-submodules uvicorn` 并指定 hidden-import。构建完成后自动做冒烟测试：起 sidecar，打 `/health` 和 `/v1/models`（这一步用 mock 的 Kiro 环境变量，不联网）。
- [包体比 参考项目 大] → 接受。实测之后记进 `desktop/AGENTS.md`。
- [Windows 杀毒软件误报 PyInstaller 产物] → onedir 比 onefile 好一些，但仍可能误报。在 README 里写明处理办法。签名放到后续 change。
- [网关 `[Credit]` 计费和 record 写入路径出现分叉] → record 的 credits 和 `[Credit]` 日志在同一位置、读同一个变量。测试断言两者相等。
- [两个网关同时刷新 kiro-cli token：桌面端 sidecar 和用户手动起的 8000 网关] → 端口冲突时不启动，降低同时运行的概率。token 写回本来就是现有行为，这次不改。
- [未签名的 dmg 在同事机器上被 Gatekeeper 拦截] → 第一版在 README 里写 `xattr -dr com.apple.quarantine` 的操作说明，签名公证放到后续 change。updater 替换 .app 时不会重新打上 quarantine，所以只有首次安装需要这一步。
- [发错清单导致全员强制升级到坏版本] → `release.sh` 在 `FORCE=1` 时要求二次确认。清单覆盖前先上传新包，并用 HEAD 请求校验 URL 可访问。回滚的办法是发一个版本号更高的修复版，Tauri updater 不支持降级。

## 事前验尸

假设上线一个月后出了事，最可能的几种情况如下。

1. **同事的 `kclaude` 用不了，界面却显示"已安装"。**
   - 怎么出事：用户的默认 shell 是 bash，或者 `.zshrc` 在 App 写入的那段之后又 `unset` 了函数，或者用的是 oh-my-zsh 的 alias 覆盖。也可能用户一直在用 App 安装前就打开的终端。
   - 后果：用户以为装坏了，或者跑去改全局配置，结果把 `claude` 也弄坏了。
   - 防法：安装后用一个新的登录 shell（`$SHELL -lic 'type kclaude'`）实测命令是否可用，结果显示在界面上。界面文案明确写"新开终端"。bash 用户写 `.bashrc`/`.bash_profile`。测试在临时 HOME 下覆盖 zsh、bash 两种情况，以及启动文件已有内容、不存在两种情况。

2. **界面显示「运行中」，实际网关已经挂了，kclaude 报连接失败。**
   - 怎么出事：sidecar 因为 token 刷新失败或者端口冲突退出了，状态没有更新。或者 macOS 睡眠唤醒后进程被系统回收。
   - 后果：用户看到的状态和实际不一致，不知道问题在哪。
   - 防法：用 wait 线程即时发现进程退出，再加每 10 秒一次 `/health` 心跳作为第二道保险。连续 3 次失败就标为「意外退出」，附日志摘要。测试 kill 子进程后，状态在 5 秒内变化。

3. **登录失效时，提示让用户去做一件做不到的事。**
   - 怎么出事：kiro-cli 的 refresh token 过期了，界面提示"运行 kiro-cli login"，但用户的终端里没有 kiro-cli（GUI 能找到是因为查了默认位置，终端 PATH 里却没有）。
   - 后果：用户卡住。
   - 防法：复制按钮复制 kiro-cli 的绝对路径加 `login`，不依赖终端的 PATH。网关启动时如果 401/403，原因明确写"登录已失效"。测试覆盖：找到的 kiro-cli 不在 PATH 里时，提示用的是绝对路径。

4. **credits 统计不准，同事拿日志页的数字去对 Kiro 账单，对不上。**
   - 怎么出事：某个出口（比如 legacy 分支的网络错误、流式中途断开）没写 record。或者一个请求写了两条，重复计算。或者跨月、跨时区归错了天。
   - 后果：统计不可信，功能等于白做。
   - 防法：record 带 `finished` 标志，保证只写一次。路由函数外层 `finally` 兜底，测试断言兜底从不触发。四条路径分别和 `[Credit]` 日志值做相等断言。聚合按本机时区，测试用固定时区覆盖月末 23:59、次月 00:00 两个时间点。

5. **App 崩溃或者被强制退出后，8000 端口被遗留的网关占着，再开 App 就提示"端口被占用"，没有出路。**
   - 怎么出事：macOS 上用户强制退出了 App，子进程没跟着结束。
   - 后果：用户只能重启电脑，或者自己去 `lsof`。
   - 防法：pid 文件加可执行文件路径校验，识别出是自己遗留的就回收（决策 8）。识别不出时，界面给出"改用其他端口"的出路，并显示占用端口的进程名。测试覆盖 pid 文件存在且进程存活、pid 被别的进程复用、pid 文件损坏三种情况。

6. **强制升级把所有人卡在遮罩里。**
   - 怎么出事：清单里 `minRequiredVersion` 写高了，或者新包的 URL 404，或者签名不对，所有旧版打开都在遮罩上反复下载失败。
   - 后果：全组都用不了 kclaude。
   - 防法：`release.sh` 上传后先 HEAD 校验每个平台的 URL 和 `.sig`，再覆盖清单。强制发布时二次确认。遮罩上始终提供安装包直链，即使 updater 失败，用户也能手动装新版。测试覆盖 `decide_update` 的各种边界：版本相等、低于最低版本、缺少平台、版本号格式错，以及下载失败时遮罩给出的出路。

7. **访问密钥或 token 泄露到日志或界面。**
   - 怎么出事：意外退出时把 `gateway.log` 的尾部直接显示出来，里面可能有调试信息。或者请求日志的 error 字段带出了上游回显的内容。
   - 后果：凭证泄露。虽然只影响本机，但同事截图发群里的风险是真实存在的。
   - 防法：日志摘要显示前先过滤掉疑似 token 的长串（≥ 32 位的 base64/hex），并且不显示 `PROXY_API_KEY`。网关的请求记录里 error 字段只取已整理过的错误信息，截到 200 字符。测试用含假 token 的日志行和错误信息验证过滤生效。

已检查、确认没有问题的：
- 绕过限制：网关只监听 127.0.0.1，并且需要密钥。
- 数据损坏：半行 JSON 跳过，见 spec。

### 12. 实施中补充的决定

- macOS 被信号结束（`kill`、注销、updater 重启）后，AppKit 下次启动会在 `finishLaunching` 里弹「是否重新打开窗口」的模态框，主线程卡在这里，网关恢复和托盘都起不来。`ApplePersistenceIgnoreState` 只从 user defaults 读，写进 Info.plist 无效，所以启动最早处用 `registerDefaults:` 在易失的注册域里设为 YES（`src-tauri/src/macos_state.rs`），只作用于本进程、不落盘。
- sidecar 的 `pre_exec` 恢复默认信号处置并清空信号屏蔽字：App 为了统一处理 SIGTERM 在主线程屏蔽了它，屏蔽字会被子进程继承。
- 测试用的假 sidecar 端口改为由系统分配临时端口，不再和 8100–8199 共用，避免并行测试互相应答 `/health`。

## Migration Plan

- 网关改动默认关闭，合并后不用桌面端的用户不受影响。
- 你本机现在的 `~/.zshrc` 里已经手动 source 了 `scripts/kclaude.zsh`。装上 App 后会走决策 6 里"检测到已有 kclaude"的分支，由你决定是否交给 App 管理。不点的话两者互不干扰，但 kclaude 仍是旧脚本，会自己在后台启动网关。
- 回滚：卸载 App，并在界面上点「移除 kclaude」（或者按标记手动删掉那一段），网关代码不需要回滚。

## Open Questions

- Windows 上 kiro-cli 的数据库路径和可执行文件位置：要等 Windows 真机确认。只影响候选路径列表，不影响整体方案。
- App 名称和图标：暂定 "Kiro Gateway"，图标先用占位图。

## 联调批次补充的决策（只补充，不改上面的内容）

### 12. 启动顺序与线程

`setup` 里同步做的事要快，慢的放后台线程，窗口和托盘先可用：

1. 读配置与访问密钥（`app_config`）→ 载入请求日志（只读文件，不起轮询）。
2. 建网关服务；建更新服务并立即处理 `pending-update.json`（装好会直接重启 App，所以排在其他一切之前）。
3. 建托盘（订阅 `gateway-status`、`update-status`、`request-log-appended` 三个事件，不回调网关管理器，避免在它持锁时重入）。
4. 后台线程：环境检测（顺带填好登录 shell 的 PATH 缓存）→ 安装 / 修复 kclaude → 按 `gatewayWanted` 恢复网关 → 启动请求日志轮询（1 秒）→ 启动更新调度（5 分钟一跳，60 分钟一次检查）。
- 环境检测没通过时，`gatewayWanted` 保留为 true 但不启动网关；下次环境恢复后第一次打开 App 再恢复。
- `autostart` 启动的实例（带 `--autostarted`）隐藏主窗口，只留托盘。
- macOS 上 SIGTERM / SIGINT / SIGHUP 专门线程接收（其余线程屏蔽），先停网关再退出；否则 `kill <App 的 pid>` 会留下孤儿网关占着端口，要等下次启动才被回收。

### 13. 强制升级的拒绝点

`gateway_start` 与托盘的「启动网关」都走同一个 `start_guarded`：先问更新服务（内存状态 `forced`），再由网关服务查持久化的 `upgradeRequired`，任一为真就返回 `upgrade_required`，什么都不启动。持久化标志在 `gatewayWanted=true` 的重启场景里同样挡住自动恢复。

### 14. 更新的下载与安装

- 检查：`Updater::check()` 读清单；`Update::raw_json` 带完整清单（含插件会忽略的 `minRequiredVersion`、`manualDownload`），决策函数 `assess` 吃的是它。
- 下载：`Update::download` 返回已校验签名的字节；写 `updates/<version>.bin` 与 `pending-update.json {version, sha256, manualUrl, savedAt}`（先包后记录，原子写）。包路径由校验过的版本号推出，不读记录里的路径。
- 安装：下次启动 `process_pending` 先验记录、版本更新、sha256，再调 `Update::install(bytes)`（macOS 替换 `.app`，Windows 启动 NSIS 并退出）。离线拿不到更新句柄时保留包（`Deferred`，状态显示 `ready`）；安装器真的失败则删包并给 `install_failed` 与 `manualUrl`。
- 强制升级写 `upgrade-lock.json {version, manualUrl}`：启动时若运行版本低于它，不等第一次检查就进入遮罩；运行版本达到后才清除 `upgradeRequired`。有标志但没有锁记录时，继续挡住，直到一次有效清单不再要求升级。
- 调试：debug 构建认 `KIRO_GATEWAY_UPDATE_URL`，只接受回环地址、不带用户名密码；release 构建完全忽略。
- 签名：Tauri 2 CLI 读 `TAURI_SIGNING_PRIVATE_KEY`（值可以是密钥文本或文件路径）；`TAURI_SIGNING_PRIVATE_KEY_PATH` 不被识别。`release.sh` 已改正并有回归测试。

### 15. 前端与 Rust 的契约守卫

`src-tauri/src/contract_tests.rs` 把各 command / event 的返回结构序列化成 JSON，与 `src/lib/ipc-types.ts` 里对应 interface 的字段名逐个比对（读 ts 文件解析），并检查 `CommandMap` 的 18 个命令与 `generate_handler!` 完全一致、Rust 能产生的错误码都在前端的 `IPC_ERROR_CODES` 里。改任何一端的字段而不改另一端，这组测试会失败。
