# desktop/ 规则（Kiro Gateway 桌面端）

一个装好就能用的桌面 App（macOS dmg / Windows NSIS）：把仓库里的 Python 网关打成 sidecar 放进安装包，用一个开关启停它；装上 `cclaude` 命令（名字可在设置里改，旧称 `kclaude`）；检测 kiro-cli 与 Claude Code；展示请求日志与 credits；应用内升级。仓库根 AGENTS.md 的规则同样适用。

- 行为定义：`openspec/changes/add-desktop-app/`（proposal / design / specs / tasks），归档后在 `openspec/specs/desktop/`。改动和其中的决定矛盾时，先指出哪一条，再问要不要更新。
- 界面布局、文案、配色以 `docs/mockup/index.html` 为准；`src/styles.css` 的 CSS 变量从它复制，改配色先改 mockup。
- Rust 与前端的 command / event 约定：`docs/ipc-contract.md`。两端都按它实现，要改先改它，并在文末变更记录追加一行。
- 当前版本号看 `package.json`，不要写进文档。

## 技术栈（不要扩）

- Tauri 2 + Rust；前端 Vite + React 18 + TypeScript，单窗口，状态用 `useState` / `useReducer`。
- 样式是 mockup 移植的纯 CSS（`src/styles.css` 变量 + 类名），不用 Tailwind。字体栈退回系统字体（CSP 只允许 `'self'`，不加载 Google Fonts）。
- 文案只有中文，不做 i18n。
- 测试：Vitest（前端纯函数与状态映射）+ `cargo test`（Rust 纯函数与文件操作）。
- Tauri 插件只用：single-instance（必须第一个注册）、opener、clipboard-manager、autostart，以及 update 批次加入的 updater + process；托盘用 core `tray-icon`。
- 不引入：Next.js、Zustand、Redux、react-router、framer-motion / motion、gsap、animejs、recharts 等图表库、lucide-react 等图标库、shadcn CLI、Tailwind、任何 UI 组件库。图标用内联 SVG。
- Rust 依赖固定精确版本（`"=x.y.z"`），优先选用已在其他 Tauri 2 项目中验证过的版本；npm 依赖不用 `^` / `~`。加依赖前先说明用途和体积。

## 前端结构

- `src/lib/tauri-*.ts`：`@tauri-apps/*` 的唯一出入口，每个模块一个薄封装文件（清单见 `docs/ipc-contract.md`）。其他代码不直接 import `@tauri-apps/*`。
- `src/lib/ipc-types.ts`：IPC 的 TypeScript 类型，与 `docs/ipc-contract.md` 一一对应。
- `src/features/<模块>/`：状态到文案、按钮可用性等映射写成纯函数，配 `*.test.ts`；组件只负责渲染和调用封装。
- 前端不直接访问本机网关（CSP `connect-src` 只有 IPC），要 `/v1/models` 之类的数据走 Rust command。

## 写盘范围

前端没有 fs / shell 权限，所有写盘都在 Rust command 里，范围只有：

- App 数据目录（Tauri `app_data_dir()`，即 `~/Library/Application Support/dev.kirogateway.desktop` / `%APPDATA%\dev.kirogateway.desktop`）：`config.json`、`secret`（0600，原子写）、`logs/`、`runtime/`、`gateway.pid`、`<命令名>.zsh|bash|ps1` 脚本（默认 `cclaude.*`）与 `kclaude.env`（文件名固定）、`pending-update.json`、`updates/<版本>.bin`（已下载的更新包）、`upgrade-lock.json`（强制升级必须到达的版本）、`kclaude.removed`（用户移除 kclaude 的标记）。
- 用户 shell 启动文件里带 `# >>> kiro-gateway-desktop command >>>` / `# <<< kiro-gateway-desktop command <<<` 标记的一段（旧版的 `... kclaude >>>` 段识别为本 App 的旧段，安装 / 改名时就地替换；命令名默认 `cclaude`，用户可在设置里改，规则见 `app_config/command_name.rs`）：`~/.zshrc`，或 bash 用户的 `~/.bashrc` / `~/.bash_profile`，或 Windows PowerShell 5 / 7 的 `CurrentUserAllHosts` profile。写前备份为 `<原文件名>.bak-kiro-gateway-YYYYMMDD-HHMMSS`，备份失败就不写。标记段之外的内容逐字节不动；只有用户点「改由 App 管理」时才把旧 kclaude 行注释掉。
- kiro-cli 数据库只读（`mode=ro`），读出的 token 不出 Rust 函数。
- 不碰：`claude` 命令本身、`~/.claude/` 下任何文件、全局 `ANTHROPIC_*` 环境变量、PowerShell 执行策略。
- 测试里一切写盘用临时目录或临时 HOME，绝不写开发者真实的 `~/.zshrc`、profile、`~/.claude/` 或 App 数据目录。

## 子进程规则

- 只起 Rust 侧写死的命令：网关 sidecar、`$SHELL -lic 'echo $PATH'` / `type <命令名>`（各有超时；命令名已通过格式校验才会拼进命令行），前端只传枚举，不传命令行。
- sidecar 启动前清空继承的 `ANTHROPIC_*`、`PROXY_*`，按 design 决策 2 显式设置环境变量；工作目录是 App 数据目录下的 `runtime/`；只监听 `127.0.0.1`。
- 只结束自己起的进程：`gateway.pid` 里的 pid 必须存活且可执行文件路径是本 App 的 sidecar 才回收；端口被别的进程占用时只报告进程名和 pid，绝不 kill。
- macOS 停止顺序 SIGTERM → 5 秒 → SIGKILL；Windows 用 Job Object 绑定子进程，停止用 `TerminateProcess`。App 退出时同步停网关。
- App 主线程屏蔽了 SIGTERM / SIGINT / SIGHUP（`signals.rs`），屏蔽字会被子进程继承：新增任何 `Command` 都要用 `gateway::sys::restore_default_signals` 做 `pre_exec`，否则子进程收不到 SIGTERM。
- sidecar 环境变量是白名单（`gateway/env.rs`）：网关新增配置项不会自动从 App 环境继承，需要的话在 `build_env` 里显式设置。
- `macos_state::disable_window_restoration()` 必须在 Tauri builder 之前调用，否则被 kill 后的下一次启动会卡在「重新打开窗口」模态框。
- 关窗后还要跑的周期任务（状态心跳、请求日志增量读取、更新检查）放 Rust 定时器：窗口隐藏后 webview 会被挂起。
- Windows 专属代码用 `#[cfg(windows)]` 隔离，路径计算等纯逻辑抽成跨平台函数测试。

## UI 文案

- 内部工具，文案克制：说清楚是什么、怎么修；不道歉、不营销、不用感叹号。
- 失败显示分类原因 + 能照着做的一步（复制命令、换端口、重试），不显示 URL、堆栈、备份路径。
- 不显示 token、refresh token、访问密钥的任何字符；日志摘要先脱敏再显示。
- 措辞照 mockup：「运行中 / 已停止 / 启动中 / 没能启动 / 意外退出」「新开一个终端就能用」「改由 App 管理」等。
- credits 只标 credits，不换算成金额。

## 命令（在 desktop/ 下）

- 无窗口验收：`src-tauri/target/debug/kiro-gateway-desktop --selftest`（仅 debug 构建）。在临时 HOME 里走完「无登录时拒绝启动 → 安装 kclaude → 起真 sidecar → `cclaude` 环境变量链路 → 强制升级拒绝 → 停网关无遗留 → 改名为 `kclaude-test`（新名可用、旧名消失、一段标记）→ 移除逐字节还原」，并比对真实 `~/.zshrc` 等文件的 mtime 与大小。需要先 `make sidecar`；`KGW_SELFTEST_DB` 可换成别的 kiro-cli 数据库（默认用假库，不会触发真实登录的 token 刷新）。
- 调试开关（仅 debug 构建）：`KIRO_GATEWAY_UPDATE_URL` 覆盖更新清单地址，只认回环地址，release 构建忽略（有测试）。`KIRO_GATEWAY_SIDECAR` 覆盖 sidecar 路径。
- macOS 上从 `/tmp` 直接运行 .app 会失败（`/tmp` 是符号链接，Tauri 拒绝解析资源目录）；测试用 `/private/tmp/...`。
- 环境：Rust 在 `~/.cargo/bin`（Makefile 已加进 PATH），pnpm 管理前端依赖。
- `make help`：列出全部目标。
- `make dev`：开发窗口（HMR）。
- `make test`：`pnpm test` + `cargo test`。
- `make check`：`tsc --noEmit` + `vite build`。
- `make sidecar`：调 `scripts/build-sidecar.sh` 打包本机架构的网关到 `src-tauri/resources/gateway-<arch>/`（`aarch64` 或 `x86_64`；目录 gitignore，只保留 `.gitkeep`）。`--arch arm64|x86_64` 指定架构；Apple Silicon 上打 x86_64 会自动用 `arch -x86_64` 重跑自己，并用 `uv python find cpython-3.13-macos-x86_64` 找解释器（先 `uv python install --no-bin cpython-3.13-macos-x86_64`，`--no-bin` 避免在 PATH 里留一个 x86_64 的 `python3.13`；也可用 `KGW_PYTHON` 指定），冒烟测试在 Rosetta 下照样跑。x86_64 的 venv 是 `.build-venv-x86_64`，arm64 的仍是 `.build-venv`。
- `make sidecar-universal`：两个架构各打一遍。
- `make dmg-local`：本机出未签名 dmg，**只打本机架构**的 sidecar（用 `--config` 把另一个目录从 `bundle.resources` 里去掉）。会拉起 Finder 给 dmg 窗口排版；无人值守（CI、远程会话）时用 `CI=true pnpm tauri build --bundles app,dmg`，跳过排版。
- `make dmg-universal`：出 macOS 通用（Intel + Apple 芯片）未签名 dmg，需先 `make sidecar-universal`；等价于 `CI=true pnpm tauri build --bundles app,dmg --target universal-apple-darwin`，产物在 `src-tauri/target/universal-apple-darwin/release/bundle/`。

### sidecar 目录规则（按架构）

- macOS：每个架构一个目录，`Contents/Resources/gateway-aarch64/` 与 `gateway-x86_64/`。`sidecar.rs` 按 `std::env::consts::ARCH`（App 实际运行的架构，Rosetta 下是 `x86_64`）选目录；`KIRO_GATEWAY_SIDECAR` 覆盖仍然最优先。包里没有当前架构的目录时报“这个安装包里没有适用于当前 CPU 的网关程序”，目录在但没有可执行文件时仍报“重新安装”。
- Windows：仍是单个 `gateway/`（`tauri.conf.json` 基础配置）。
- 打进哪些目录由配置决定：`tauri.conf.json` 只列 `resources/gateway/`（Windows）；`tauri.macos.conf.json`（macOS 主机自动合并，JSON merge patch）把它置 `null`，换成两个架构目录。所以 macOS 默认是 universal 布局，两个目录都必须存在（`.gitkeep` 保证目录在，但空目录会打出起不来的包，所以出 universal 包前要先 `make sidecar-universal`；CI 里两个架构的 sidecar 都是先构建再出包）。单架构构建用 `--config '{"bundle":{"resources":{"resources/gateway-<另一个>/":null}}}'`，`make dmg-local` 已经这样做。
- `make clean`：清理 dist 和 dmg 残留。

## 测试要求

- TDD：先写失败的测试再实现；不写占位实现，不留 TODO。
- 每个 Rust 模块的纯逻辑都有 `cargo test`；写盘逻辑在 tempdir 里测，覆盖文件不存在、已有内容、备份失败等分支。
- 前端每个状态映射纯函数都有 vitest，覆盖 IPC 约定里的每个枚举值。
- `src-tauri/src/scaffold_tests.rs` 守住工程配置（identity、窗口尺寸、CSP、bundle、capabilities、三处版本一致），改配置时同步改它。
- 改任何代码后 `make test` 和 `make check` 全绿再交付。不写 UI 自动化测试。

## 包体

- 包含 PyInstaller onedir 的网关 sidecar，常见的 20 MB 包体上限不适用。
- 实测 universal（`make sidecar-universal` + `CI=true pnpm tauri build --bundles app,dmg --target universal-apple-darwin`，ad-hoc 签名）：
  - dmg：48.8 MB（`Kiro Gateway_0.0.1_universal.dmg`，51125461 字节）；`.app` 97 MB（主程序 universal，`gateway-aarch64/` 44 MB + `gateway-x86_64/` 41 MB）。
  - 原生 arm64 与 `arch -x86_64`（Rosetta）下各自启动：/health 通，sidecar 分别是 `gateway-aarch64`（arm64）与 `gateway-x86_64`（x86_64），停止后无遗留进程。
- 实测单架构（macOS arm64，`pnpm tauri build --bundles app,dmg --target aarch64-apple-darwin`，ad-hoc 签名）：
  - sidecar 目录（现为 `src-tauri/resources/gateway-aarch64/`）：44 MB（`_internal` 35 MB、启动器 7.7 MB、`tiktoken_cache` 1.6 MB），打成 tar.gz 约 21 MB。
  - `Kiro Gateway.app`：49 MB（其中 sidecar 44 MB，桌面端主程序 4.9 MB）。
  - dmg：23.5 MB（`Kiro Gateway_0.0.1_aarch64.dmg`，24646084 字节）。
  - 单架构包只含本机架构的 sidecar；Intel 用户请用 universal 包。

## 不要做

- 不做 Linux 包、多账号系统配置界面、显示请求或回复内容。
- 不替用户执行 `kiro-cli login`，只给命令和复制按钮。
- 不改 `scripts/kclaude.zsh`；不改仓库根 `.gitignore`（桌面端忽略规则写在 `desktop/.gitignore`）。
- capabilities 不开 `fs:*`、`shell:*`、`http:*`。

## 发版

安装包和应用内升级用的 `latest.json` 都放 GitHub Releases（仓库 `heyuan110/kiro-gateway`）。App 读 `https://github.com/heyuan110/kiro-gateway/releases/latest/download/latest.json`，地址只在 `tauri.conf.json` 的 `plugins.updater.endpoints` 里写一次。

```bash
# 1. 改三处版本号（package.json、src-tauri/tauri.conf.json、src-tauri/Cargo.toml，Cargo.lock 里本包条目同步），提交
/opt/miniconda3/bin/python -I desktop/scripts/release_lib.py bump --root desktop --version 0.0.2
# 2. 打 annotated tag：正文就是更新说明（一行一条）
git tag -a desktop-v0.0.2 -F notes.md
# 3. 推 tag，GitHub 自动出包、生成 latest.json、发布 Release
git push internal desktop-v0.0.2
```

- 强制升级：在 `notes.md` 里单独写一行 `[force-upgrade]`，`minRequiredVersion` 就是本次版本，低于它的旧版必须升级；这一行不会出现在更新说明里。没有这一行时，`minRequiredVersion` 沿用上一个已发布 Release 的 `latest.json`（前一次的强制升级不会被悄悄解除），取不到（第一次发布）用 `0.0.1`，日志里会写明。回滚的办法是发一个版本号更高的修复版，updater 不支持降级。
- Actions（`.github/workflows/desktop-release.yml`）：macOS universal 出 dmg + 签名更新包，Windows 出 NSIS 安装器 + 签名；release job 用 `scripts/release_lib.py prepare` 把它们改成不含空格的名字（`Kiro-Gateway_<版本>_universal.dmg`、`Kiro-Gateway.app.tar.gz`、`Kiro-Gateway_<版本>_x64-setup.exe` 及各自的 `.sig`），生成 `latest.json`（URL 是 `https://github.com/<仓库>/releases/download/<tag>/<文件名>`），把这些文件原样挂到 Release，更新说明也取自 tag 正文。手动 `workflow_dispatch` 只出包不发布。
- `latest.json` 里 `manualDownload.darwin` / `.windows` 是 dmg / setup.exe 直链，强制升级下载失败时界面显示它。macOS 的 `darwin-aarch64` 与 `darwin-x86_64` 指向同一个 universal 更新包。
- 仓库私有期间，App 取不到清单（未登录访问私有仓库资产返回 404），检查更新会静默失败，不影响使用；仓库公开后生效。
- 签名私钥：`~/.kiro-gateway/desktop-updater.key`（无密码，权限 600，`*.key` 已被忽略，绝不进仓库）；Actions 用的是 Secret `TAURI_SIGNING_PRIVATE_KEY`。公钥在 `tauri.conf.json` 的 `plugins.updater.pubkey`。私钥丢了，旧版就收不到之后的更新；重新生成要换公钥并让用户手动装一次。
- 本机试跑清单生成：`release_lib.py prepare --version X --base-url https://github.com/<仓库>/releases/download/desktop-vX --tag-body body.txt --bundle-dir <mac bundle> --windows-dir <win bundle> --out <目录> [--previous-manifest 上一个 latest.json]`，上传用的文件在 `<目录>/assets/`。
- 脚本逻辑在 `scripts/release_lib.py`（只用标准库），测试：`/opt/miniconda3/bin/python -I -m unittest discover -s desktop/scripts -p 'test_release_lib.py' -v`（不联网）。
