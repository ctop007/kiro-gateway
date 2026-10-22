# 批次 integration：接线、全量验证、出包、验收、8.8

日期：2026-10-23。tasks.md 勾选未改，未 git add/commit。127.0.0.1:8000 的网关（pid 61088）全程未动；真实 `~/.zshrc`、`~/.zprofile`、`~/.bash_profile`、`~/.claude/settings.json` 的 mtime 与大小前后一致。

## 1. 核对各条线

各条线 test_command 亲自重跑：

| 线 | 结果 |
| --- | --- |
| config-gateway（临时 crate） | 288 passed |
| env-check | 99 passed |
| kclaude | 150 passed |
| request-log | 81 passed |
| update | 94 passed |
| frontend `pnpm test` / `pnpm build` | 254（现 255）passed，build 通过 |
| sidecar `python3 -I -m unittest discover` | 81 passed（报告写 47，目录下现有测试文件更多） |
| release `test_release_lib.py` | 34（现 36）passed |

Red/Green 证据：各线 progress 文件都有。需要留意的点：
- kclaude 6.1 没有严格先红后绿（自己在 progress 里说明了）。
- env-check 线报告 `agent failed`，但 `env_check/` 代码和 progress/env-check.md 已存在，99 个测试通过，按已完成对待；它没有写回报，所以 contract 缺的 `loginCommand` 是前端线发现的。
- gateway::real 的 4 个测试依赖本机已构建的 sidecar，没构建时跳过。
- `tasks.md` 4.3 的验证命令 `cargo test gateway::state` 对不上（模块名是 `gateway::manager`），需要改 tasks.md（我没改）。

发现的问题（已修）：
1. `release.sh` 导出 `TAURI_SIGNING_PRIVATE_KEY_PATH`，Tauri 2 CLI 只认 `TAURI_SIGNING_PRIVATE_KEY`，真实发版会在签名步骤失败（8.8 出包时撞上）。已改，加 `SigningEnvironmentTests`（改回旧写法时测试失败：`['export TAURI_SIGNING_PRIVATE_KEY_PATH=...']`，改正后通过）。
2. `gateway::testing` 的假 sidecar 写 dump 文件不是原子的，整套件高负载下 `the_sidecar_receives_the_stored_secret...` 出现过 1 次失败；改为临时文件加 rename，读取侧最多等 5 秒。之后整套件连续跑 3 次全绿。
3. 更新 URL 覆盖的检查最初漏掉带用户名的 URL（`http://user@evil.example@127.0.0.1/...`），测试 `debug_builds_refuse_remote_hosts_and_junk` 当场失败，已拒绝带用户名密码的 URL。
4. 网关管理器的 `tests`、`gateway::real::tests` 用了 `libc`，Windows 目标下 `cargo test` 无法编译；改为 `#[cfg(all(test, unix))]`。

## 2. 接线（main.rs 等）

- `main.rs`：声明所有模块；插件顺序 single-instance（第一个）→ opener → clipboard-manager → autostart → updater → process；注册 18 个 command；managed state；退出钩子 `RunEvent::Exit` 同步停网关；Dock 重开显示窗口；`--autostarted` 启动隐藏主窗口。
- 启动时序：setup 里同步做 读配置 → 请求日志载入 → 网关服务 → 更新服务（处理 pending，装好会直接重启）→ 托盘；慢的放后台线程 `startup`：环境检测 → 填 claude 搜索目录 → kclaude 安装 / 修复 → 按 `gatewayWanted` 恢复网关（环境未通过时不恢复）→ 请求日志轮询 → 更新调度。
- `tauri-glue` feature 去掉，`commands.rs` 变成普通代码；`update::commands::setup` 拆成 `setup` 和 `spawn_scheduler`，`request_log::commands::init` 拆成 `init` 和 `spawn_poller`，让时序可控。
- `gateway::commands::start_guarded`：`gateway_start` 与托盘共用，先问更新服务的强制升级状态。
- 托盘 `tray/`：`model.rs`（纯函数，13 个测试）+ `controller.rs`（图标、菜单重建、动作）；菜单照 mockup：状态行、今日 credits、启动 / 停止、打开主窗口、「重启以完成更新」（仅 ready 且未强制）、退出；强制升级时第一行「需要升级到 x.y.z」、启动项禁用；退出在独立线程同步停网关再 `exit(0)`；通过监听 `gateway-status` / `update-status` / `request-log-appended` 重建菜单，不回调网关管理器。
- `signals.rs`（macOS）：SIGTERM / SIGINT / SIGHUP 专线程接收，先停网关再退出。起因：打包 App 收到 `kill` 后直接死，sidecar 变孤儿占着端口（实测发现：`kill -TERM` 后 sidecar 仍在监听）。修后实测 `kill -TERM` 后 App 退出、sidecar 一并结束、端口释放。
- Cargo.toml（精确版本）：`tauri-plugin-updater =2.11.0`、`tauri-plugin-process =2.3.1`、`rusqlite =0.40.2`（bundled）、`ureq =3.5.0`（default-features = false，仅明文 127.0.0.1）、`sha2 =0.10.9`、`url =2.5.8`（已在依赖树里）；windows-sys 追加 `Win32_System_Com`、`Win32_System_Registry`、`Win32_UI_Shell`。
- 更新清单地址覆盖：debug 构建读 `KIRO_GATEWAY_UPDATE_URL`，只接受 `http(s)://` 回环地址且不带用户名密码；release 构建完全忽略。测试 `update::commands::tests`（release 忽略、debug 接受回环、拒绝远程 / 伪造主机 / 带用户名 / 非 http / 空）。
- `contract_tests.rs`（11 个）：把各返回结构序列化后和 `ipc-types.ts` 的 interface 字段名逐个比对（解析 ts 文件，含 extends、联合类型各变体），另检查 `CommandMap` 的 18 个命令与 `generate_handler!` 一致、3 个 event 名一致、Rust 能产生的错误码都在 `IPC_ERROR_CODES` 里。变异验证：把 ts 里的 `reset` 改成 `resetX`，`request_log_types_match` 失败（`left: {..."reset"...} right: {..."resetX"...}`），还原后通过。

contract_changes 合并情况：
- 已写入 `ipc-contract.md`（并在变更记录追加一行，另加第 7 节磁盘文件、第 8 节 debug 开关）：`EnvReport.loginCommand`、`RequestLogAppended.reset`、`gatewayWanted` 语义、`config_update` 自启失败语义、`env_not_ready` 的 message、失败文案、"未知模型"、`kclaude.removed`、残缺标记处理、`update_restart_now` 的 `internal` 文案、`upgrade-lock.json` 与 `manualDownload`。
- 已写入 `ipc-types.ts`：`RequestLogAppended.reset`；前端 `appendedAction()`（reset 时重新拉 snapshot），`useAppState` 使用它；`mock/backend.ts` 与测试同步（`view.test.ts` +1 个用例）。
- kclaude 线报告的 `kclaude.env` 格式（加单引号转义）与实际实现不一致：实现写的是不加引号，原因是端口 / 密钥 / 模型名都在受限字符集里（app_config 的测试用真 sh / bash / zsh 读回验证过）。以实现为准，contract 按实现写。
- 未采纳：给 `KclaudeState` 加 `damaged`（按 `foreign` 处理，已写入 contract）。

## 3. 全量验证

| 命令 | 结果 |
| --- | --- |
| `cd desktop && pnpm test` | 12 files，255 passed |
| `pnpm build` | 通过（js 182 kB，无 mock chunk） |
| `cd src-tauri && cargo test` | 750 passed，连续 2 次；`cargo build` 与 `cargo test --no-run` 均 0 警告 |
| `cargo build --release`（经 tauri build） | 0 警告 |
| 本批 Python：`discover -s desktop/scripts -p 'test_*.py'` / `test_release_lib.py` | 81 / 36 passed |
| `cargo check --target x86_64-pc-windows-msvc` | 完整 crate 被挡：`ring 0.17.14` 的 C 代码需要 MSVC 头文件，本机 clang 交叉编译失败（`uintptr_t and size_t differ`）。改为把不依赖 tauri 的六个模块（含全部 `cfg(windows)` 代码）抽到临时 crate，`cargo check --target x86_64-pc-windows-msvc --lib --tests` 通过（仅测试辅助函数的 unused 警告）。`commands.rs` 里的 Tauri 胶水和 `main.rs` 没有在 Windows 目标上检查过 |

## 4. 3.3 出包（macOS arm64，ad-hoc 签名）

`bash scripts/build-sidecar.sh`（冒烟通过）→ `CI=true pnpm tauri build --bundles app,dmg --target aarch64-apple-darwin`：
- sidecar 目录 44 MB（tar.gz 约 21 MB）；`Kiro Gateway.app` 49 MB；dmg 24643787 字节（23.5 MB）。已写入 `desktop/AGENTS.md`。
- 不加 `CI=true` 时 `bundle_dmg.sh` 通过 osascript 控制 Finder 排版，在本次无人值守会话里挂住（被我结束了），残留的 `rw.*.dmg` 已删。
- 注意：`codesign --verify --deep --strict` 报 `code has no resources but signature indicates they must be present`（ad-hoc 签名只签了主程序，Resources 没有封入签名）；本机能运行，分发前的 Apple 签名与公证属于后续 change。
- 该 .app 不能放在 `/tmp` 下运行：`/tmp` 是符号链接，Tauri 拒绝解析资源目录（`找不到应用资源目录`）；在 `/private/tmp` 或正常的 Applications 路径下没问题。已写入 AGENTS.md。

## 5. 9.1 / 9.2 自动化验收（不点 GUI）

### `--selftest`（debug 构建，`src/selftest.rs`）

`src-tauri/target/debug/kiro-gateway-desktop --selftest`：41 项全部 PASS，`RESULT passed=41 failed=0`。覆盖：
- 9.2：临时 HOME 无 kiro-cli 登录时，环境检测有失败项、`gateway_gate` 给出原因、`start()` 返回 `env_not_ready`、状态仍是 stopped 且 `gatewayWanted` 未被置位。
- 安装 kclaude 到临时 HOME 的 `.zshrc`：保留原行、只多一个标记段、写前有 `.zshrc.bak-kiro-gateway-*` 备份、新登录 shell 里 `type kclaude` 通过。
- 真 sidecar（81xx，假的 kiro-cli 库，只打 `/health`，不发模型请求）：running、`/health` 200、`gatewayWanted` 置位、`gateway.log` 不含访问密钥、`lsof` 只看到 `127.0.0.1`。
- 临时 HOME 的 zsh 里 `kclaude --version`（假 claude 在 PATH）：退出码 0、参数透传、`ANTHROPIC_BASE_URL` 是配置端口、`ANTHROPIC_AUTH_TOKEN` 是存的密钥、三档模型来自配置。
- 强制升级：kclaude 非零退出且不调用 claude、提示含「需要升级」；`start()` 返回 `upgrade_required`。
- 停网关：状态 stopped、pid 不存活、端口释放、`pgrep` 找不到 sidecar；kclaude 非零退出，提示含「未运行」。
- 移除 kclaude 后 `.zshrc` 逐字节还原。
- 真实 `~/.zshrc`、`.zprofile`、`.zshenv`、`.bashrc`、`.bash_profile`、`~/.claude/settings.json` 的 mtime 与大小前后一致（脚本内置比对，另外我手动 `stat` 前后各记录一次也一致）。
- 默认用假的 kiro-cli 数据库（一年后过期的令牌，不会触发刷新）。我没有把用户真实的 kiro-cli 数据库交给 sidecar：刷新会轮换真实的 refresh token，风险大于收益；`KGW_SELFTEST_DB` 留给你自己决定。

### 打包后的 .app 在临时 HOME 下运行（`/tmp/kgw-integration/run-app.sh`）

用 release 包里的二进制：3 秒内 `/health` 通；kclaude 自动装入临时 `.zshrc`；`gatewayWanted=true` 的配置被恢复；`kclaude -p hi` 环境变量正确；`secret` 权限 600；`kill -TERM` 修复前 sidecar 变孤儿（发现的问题 5），修复后随 App 一起结束；孤儿 sidecar 存在时下一次启动 2 秒内回收并起新的。

### 截图

`screencapture` 执行成功（退出码 0）但得到的是纯黑图（该会话没有屏幕录制权限或没有可见桌面），无信息量，已删除。9.1 的「全程截图」没有做，需要你在本机桌面会话里走一遍。

## 6. 8.8 端到端（`/tmp/kgw-integration/e2e-update.sh`）

方法：`python3 -m http.server`（127.0.0.1:8140）当 COS，用 `~/.kiro-gateway/desktop-updater.key` 通过 `pnpm tauri build --debug --config '{"version":..,"bundle":{"createUpdaterArtifacts":true}}'` 签出 0.0.1 / 0.0.2 / 0.0.3 三个包（版本只来自 `--config` 覆盖，仓库版本文件未改），自写清单；`KIRO_GATEWAY_UPDATE_URL` 指向本地服务；HOME 是临时目录；安装位置 `/private/tmp/kgw-e2e/install`。结果（完整输出 `/tmp/kgw-integration/e2e-update.txt`）：

- A1：0.0.1 启动，清单不比它新 → 不下载，无 pending；`gatewayWanted` 让网关恢复，`/health` 通。
- A2：发布 0.0.2（普通，min 0.0.1）后重启 → 静默下载（`downloading` ×4 → `ready`），写出 `pending-update.json {version 0.0.2, sha256, manualUrl, savedAt}` 与 `updates/0.0.2.bin`（36152263 字节）；仍运行 0.0.1，网关一直在跑，未被打断。
- A3：再次启动 → 日志 `[update] installing 0.0.2`，`Info.plist` 变 0.0.2，新进程自动拉起，网关恢复，pending 记录和包都清掉，`gatewayWanted` 保持 true。
- B：发布 0.0.3 且 `minRequiredVersion=0.0.3`，包被藏起来 → 0.0.2 启动后 `config.json.upgradeRequired=true`、`update-status` 进入 `forced`、网关被停、`upgrade-lock.json` 写入、`kclaude.env` 里 `KIRO_GATEWAY_UPGRADE_REQUIRED=1`；新 zsh 里 `kclaude -p hi` 退出码 1，stderr `kclaude: Kiro Gateway 需要升级，请打开 App 完成升级`，不调用 claude。
- B2：仍在强制状态下重启 0.0.2 → 在第一次成功检查之前就恢复 `forced`，`gatewayWanted` 仍为 true 但网关保持关闭（`upgrade_required` 挡住自动恢复）。
- C：把 0.0.3 包放回 → 重启 0.0.2 后无需任何点击完成安装，`Info.plist` 变 0.0.3，`upgradeRequired` 清除、锁文件删除、网关恢复、`kclaude` 重新可用。
- 收尾：无遗留进程，8125 与 8140 无监听。

没做的：
- 真实 COS（HTTPS）下载；本机测试用 http，debug 构建允许，release 构建会拒绝非 https（`dangerousInsecureTransportProtocol` 未开启）。
- 窗口里的升级遮罩 / 状态栏「立即重启」的真实渲染（无 GUI 验收）；这些只有纯函数测试。
- 下载中途断网、sha256 被篡改等路径只有单元测试（`update::pending`、`update::schedule`），没做端到端。
- `ready` 状态下点托盘「重启以完成更新」没有端到端走过。

## 7. 不能做的 / 没做的

- 9.1 的 GUI 部分（环境检测清单显示、点开关、日志页 3 秒内出现记录且与 `[Credit]` 一致、关窗后 kclaude 仍可用、托盘手动退出）：没有 GUI 权限，也不发真实模型请求（会花额度）。请求日志页与真实网关记录的一致性，由网关线 tasks 1.9 和 `request_log` 的单元测试覆盖，桌面端读真实网关产出的文件没有端到端验证。
- 9.3 Windows 实机：没有 Windows 机器。Windows 专属代码只通过了类型检查，没有运行；netstat / tasklist 解析器、`documents_dir`、执行策略读取、Job Object、NSIS 包、`build-sidecar.ps1` 都未验证。
- 9.4 对抗式审查：不在本批。
- macOS Intel / universal 包：sidecar 目前只有 arm64。
- Apple 签名与公证：后续 change。
