# 批次 scaffold：2.1、2.2、2.3 和 IPC 约定

日期：2026-10-22。只改了 `desktop/` 下的文件，没动 `desktop/docs/mockup/`。tasks.md 的勾选没改。

## 2.1 Rust 工具链

- `cargo 1.99.0 (5f94df478 2026-08-27)`，`rustc 1.99.0 (b940084d7 2026-09-28)`，位置 `~/.cargo/bin`。
- 执行了 `rustup target add x86_64-apple-darwin`。现在 `rustup target list --installed` 列出 `aarch64-apple-darwin`、`x86_64-apple-darwin`、`x86_64-pc-windows-msvc`。
- `cargo build --target x86_64-apple-darwin` 通过，为出 universal 包做准备。

## 2.2 Tauri 2 工程

- 手写，没用 create-tauri-app。结构对齐 参考项目 desktop-app：`package.json`、`pnpm-lock.yaml`、`pnpm-workspace.yaml`（`allowBuilds: esbuild: true`）、`vite.config.ts`（开发端口 1430，避开 参考项目 用的 1420）、`tsconfig.json`、`vitest.config.ts`、`index.html`、`src/`、`src-tauri/`。
- 版本 0.0.1 写在三处：`package.json`、`tauri.conf.json`、`Cargo.toml`。由 `scaffold_tests::version_is_consistent_across_three_files` 守住。
- 依赖全部是精确版本，与 参考项目 的 Cargo.lock / pnpm-lock 中已验证的版本一致：
  - Rust：tauri =2.11.1、tauri-build =2.6.1、single-instance =2.4.5、opener =2.5.4、clipboard-manager =2.3.2、autostart =2.5.1、serde =1.0.228、serde_json =1.0.149。
  - Cargo.lock 以 参考项目 的为种子，cargo 自动裁掉了没用到的包。
- 插件注册顺序：single-instance（第一个，回调把主窗口拉到前台）→ opener → clipboard-manager → autostart（LaunchAgent，带启动参数 `--autostarted`）。updater 和 process 没注册，留给 update 批次。
- capabilities：只有 core window 的 hide/show/focus/unminimize、`clipboard-manager:allow-write-text`，以及仅限 `https://*` 的 `opener:allow-open-url`。没有 fs、shell、http，也没有 autostart（原因见下文「补充与偏差」）。
- CSP：`connect-src ipc: http://ipc.localhost`，不允许连 https、http、localhost。
- bundle：targets 是 `["dmg","nsis"]`；`resources` 为 `{"resources/gateway/": "gateway/"}`；`resources/gateway/.gitkeep` 已建。
- 窗口：label `main`，880×600，可调大小，最小 760×560。关窗时 `prevent_close` 并隐藏窗口；macOS 下点 Dock 图标（`RunEvent::Reopen`）重新显示。
- 更新清单 URL 写在 `tauri.conf.json` 的 `plugins.updater.endpoints`，整个 desktop/ 只出现这一次。测试只校验 URL 形状，不重复写完整地址。updater 插件这一批还没注册，配置先放着；`cargo build` 不受影响。
- 图标：用 Python 生成一张 1024×1024 的占位 PNG（深色圆角方块加绿色开关），再用 `pnpm tauri icon` 生成 icns、ico 和各尺寸 png。删掉了 android/ios 目录，`source.png` 保留在 icons/。
- 前端：`App.tsx` 是最小可运行的外壳，包括标题栏、三个页签（主页 / 请求日志 / 设置，支持左右方向键切换和 tablist 无障碍属性）、空白视图和状态栏版本号。`styles.css` 复制了 mockup 的 CSS 变量，含深色模式两种写法，以及外壳用到的类。薄封装有 `src/lib/tauri-opener.ts`、`src/lib/tauri-clipboard.ts`。

### Red / Green

前端 `src/features/shell/tabs.test.ts`（6 个用例：页签顺序和文案、左右循环、版本号格式）：
- Red：`pnpm test` 报 `Failed to resolve import "./tabs"`，结果为 `Test Files 1 failed (1)`、`Tests no tests`。
- Green：`Test Files 1 passed (1)`、`Tests 6 passed (6)`。

Rust `src/window.rs` 的测试（`closing_main_window_hides_it`、`closing_other_windows_closes_them`、`label_match_is_exact`），以及 `src/scaffold_tests.rs` 的 8 个用例：
- Red：`cargo test` 报 `error[E0425]: cannot find value MAIN_WINDOW_LABEL` 和 `cannot find function close_action`、`cannot find type CloseAction`，编译失败。
- Green：`test result: ok. 11 passed; 0 failed`。
- 对配置守卫做了变异测试，确认它们真能失败：
  - 把窗口宽度改成 900，`single_main_window_with_required_geometry` 失败（`left: Number(900) right: 880`）。
  - 往 connect-src 加 `http://127.0.0.1:8000`，`csp_allows_no_remote_or_localhost_connections` 失败。
  - 往 capabilities 加 `shell:allow-open`，`tauri-build` 直接报 `Permission shell:allow-open not found`，构建失败。
  - 三处改动都已还原，还原后 11 个用例全部通过。

## 2.3 规则、Makefile、.gitignore

- `desktop/AGENTS.md`：技术栈边界、不准引入的依赖清单（照 参考项目，另外加了 Tailwind 和 UI 组件库）、前端结构、写盘范围、子进程规则、文案原则、命令、测试要求；包体一节写明「待 3.3 实测」。
- `desktop/Makefile`：help、dev、test（pnpm test + cargo test）、check（pnpm build，即 tsc + vite build）、sidecar（调 `scripts/build-sidecar.sh`，脚本不存在时给出中文提示并失败）、dmg-local、clean。PATH 已自动加上 `~/.cargo/bin`。`make help` 列出全部 7 个目标。
- `desktop/.gitignore`：node_modules/、dist/、src-tauri/target/、src-tauri/gen/、`src-tauri/resources/gateway/*` 加 `!…/.gitkeep`、.build-venv/、*.key。用 `git check-ignore -v` 验证过：`.gitkeep` 被反向规则保留，`resources/gateway/x`、node_modules、target、dist 都被忽略。仓库根的 .gitignore 没改。

## IPC 约定

已写 `desktop/docs/ipc-contract.md`，覆盖配置、网关、环境检测、kclaude、请求日志、升级六个模块。每个模块写了 command 名、参数、返回的 TS 形状、错误码，以及 event 和触发时机。另外规定了统一错误形状 `{ code, message }`，前端封装集中在 `src/lib/tauri-*.ts`，状态映射写成纯函数。

## 最终验证

```
cd desktop && pnpm install       → Already up to date
pnpm test                        → Test Files 1 passed (1) / Tests 6 passed (6)
pnpm build                       → tsc 通过；dist/assets/index-*.js 145.04 kB（gzip 46.81 kB），✓ built
cd src-tauri && cargo build      → Finished `dev` profile
cargo test                       → test result: ok. 11 passed; 0 failed
cargo build --target x86_64-apple-darwin → Finished
make help                        → 列出 7 个目标
```

- 没运行 `tauri dev`（无头环境开窗没有意义）。tasks 2.2 原写的验证「`pnpm tauri dev` 能出窗口」需要派活方在有图形界面的环境里补做。
- `cargo check --target x86_64-pc-windows-msvc` 被工具链挡住：`tauri-winres` 需要 `llvm-rc`，报 `NotAttempted("llvm-rc")`。失败发生在 build script 阶段，不是代码问题；本机装 LLVM 或在 Windows / CI 上可以验证。
- 本机没装 clippy 组件（`cargo clippy` 报未安装），所以没跑 clippy。

## 补充与偏差（需要派活方确认）

1. **autostart 由 Rust 持有**：前端不注册 autostart 权限，也没有 `tauri-autostart.ts` 封装。`config_update { patch: { autostart } }` 在 Rust 里调插件，返回系统实际状态。原因是 config.json 里的偏好和系统状态应该只有一个所有者。插件照常在第一批注册。
2. **IPC 约定比任务描述多出的部分**（已写进 ipc-contract.md 的变更记录）：
   - `gateway_reveal_log`：mockup 上的「打开完整日志」按钮需要它。
   - `AppConfig.gatewayWanted`：重启或更新后据此恢复网关；`upgradeRequired`：只读。
   - `ConfigUpdateResult.restartRequired`。
   - `EnvReport.kiroCliPath` / `claudePath` / `loginExpiresAt`。
   - `KclaudeVerify.fix`：Windows 执行策略禁止加载 profile 时，给出可复制的修复命令。
   - `request-log-appended` 在跨日、跨月时推送一次空记录，让汇总归零。
3. tasks 2.2 原文写了六个插件和托盘，本批按派活说明只做了四个插件，没做托盘。updater、process 和托盘分别由 update 批次和 4.6 完成。
4. 本机没有 `tauri dev` 截图，也没有 Windows check，原因见上文。
