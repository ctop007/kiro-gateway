# 批次 update：8.1、8.2、8.3、8.4、8.5（Rust 部分）、8.7

日期：2026-10-22。tasks.md 勾选未改。未 git add/commit。未真实上传 COS。

测试入口：
- Rust：临时 crate `/tmp/kgw-lane-update`，`#[path]` 引入 `desktop/src-tauri/src/update/mod.rs`，`CARGO_TARGET_DIR=/tmp/kgw-target-update cargo test --offline`，94 个用例通过；`--features tauri-glue` 的 `cargo build` 通过（`commands.rs` 对着 tauri 2.11.1 / tauri-plugin-updater 2.11.0 编译过）。
- Python：`/opt/miniconda3/bin/python -I -m unittest discover -s desktop/scripts -p 'test_release_lib.py' -v`，34 个用例通过。

## 8.1 密钥与公钥

- 执行：`pnpm tauri signer generate -w ~/.kiro-gateway/desktop-updater.key --ci -p ""`（文件原本不存在；已存在则复用，不覆盖）。私钥 `~/.kiro-gateway/desktop-updater.key` 权限 600，目录 700；`desktop/.gitignore` 已有 `*.key`。公钥写入 `tauri.conf.json` 的 `plugins.updater.pubkey`（只改了这一个键）。
- 用该私钥对测试文件签名成功（`pnpm tauri signer sign`），签名文件已删。
- `package.json` 加 `@tauri-apps/plugin-updater 2.11.0`、`@tauri-apps/plugin-process 2.3.1`（与 参考项目 一致，精确版本），`pnpm add --save-exact` 更新了 `pnpm-lock.yaml`。
- 测试：`test_release_lib.UpdaterKeyTests`（公钥是 minisign 公钥格式；与本机 .pub 一致；desktop/ 下没有私钥文件且 .gitignore 含 `*.key`）。
- 三处版本一致的测试已有（`scaffold_tests::version_is_consistent_across_three_files`），未改。

## 8.2 decide_update

- 文件：`update/decide.rs` + `update/decide/tests.rs`。`decide_offer` 返回 `Decision::{None, Background(UpdateOffer), Forced(UpdateOffer)}`，`decide_update` 返回 contract 的 `UpdateState`（`downloading` 带空进度 / `forced` 带 `manualUrl`）；另有 `assess`（区分「清单有问题」和「已是最新」，调度器用它判断检查是否成功）。
- Red：先写了 `decide/tests.rs` 再没有实现，`cargo test` 报 `cannot find type Decision in this scope` 等 75 个编译错误。
- Green：实现后 `28 passed`（含 types 6 个）。覆盖：有新版、版本相等、当前更新、低于 min、恰好等于 min（background）、低于 min 一位（forced）、缺 min（不强制）、min 无法解析 → none、min 高于发布版本（矛盾）→ none、缺平台、平台项 url/signature 为空或类型错、各种坏版本串（`1.2`、`v1.2.3`、`1.2.3-beta`、前导零、空白、溢出）、清单不是 JSON/缺字段、`0.0.9 → 0.0.10` 数字比较、manualUrl 按平台选择与回退。
- 取舍：版本串严格 `x.y.z`，不接受前导零和预发布后缀；`minRequiredVersion` 高于 `version` 视为清单不一致 → none。

## 8.3 调度器

- 文件：`update/schedule.rs`（`UpdateService`）、`update/ports.rs`（注入点：`Clock`、`UpdateBackend`、`PackageInstaller`、`UpgradeFlag`、`GatewayControl`、`AppControl`、`EventSink`）、`update/store.rs`（pending 落盘）、`update/types.rs`（IPC 类型，serde 形状对着 contract）、`update/commands.rs`（`tauri-glue`）。
- `tick()` 每 5 分钟由调用方触发，内部比较注入的墙钟：从未检查或距上次尝试 ≥60 分钟或时钟回拨 → 检查。睡了 9 小时后第一次 tick 只补查一次。失败（断网、坏清单、下载失败）静默，状态不变，不发事件，下一个 60 分钟再试（不是每个 tick 都试）。同一版本在 `downloading`/`ready` 时不重复下载；有更新的版本出现时替换 pending。进度事件最多每 250 ms 一次（末尾 100% 必发）。
- Red（先写测试）：`schedule/tests.rs` 在 `UpdateService` 不存在时编译失败（`unresolved import`）；mutation 检查：把 60 改成 90，`check_runs_again_after_60_minutes`、`failed_check_is_silent_and_retried_after_60_minutes_not_every_tick` 失败；改回后 94 passed。
- 覆盖的用例：首次检查、60 分钟周期、59 分钟不查、唤醒补查一次、时钟回拨、连续失败不报错、坏清单当失败、已是最新记 `lastCheckedAt`、下载成功 → ready、同一版本不重复下载、新版本替换旧 pending、ready 经过失败检查和最新清单仍保留、后台下载失败静默并重试、进度节流、`restart_now`（无 pending / 成功 / 安装失败 / 安装器暂不可用 / 文件被删）、`retry`（非法状态 / 安装失败后重试）。

### 下载 / 安装与 pending 方案（需要补进 design）

tauri-plugin-updater 2.11.0 的 API（读 `~/.cargo/registry/src/*/tauri-plugin-updater-2.11.0/src/updater.rs`）：
- `app.updater_builder().timeout(..).build()?.check().await` → `Option<Update>`；`Update.raw_json` 是完整清单 JSON（含我们加的 `minRequiredVersion`、`manualDownload`），`Update.version`、`download_url`、`signature` 是该平台项。`check()` 在清单版本不比当前新时返回 `None`；不带自定义字段的处理，所以 `minRequiredVersion` 要从 `raw_json` 里取，由 `decide::assess` 判断。
- `Update::download(on_chunk(usize, Option<u64>), on_finish) -> Result<Vec<u8>>`：下载到内存并验证 minisign 签名（对着 `plugins.updater.pubkey`）。
- `Update::install(bytes)`：macOS 解压 `.app.tar.gz` 替换当前 .app；Windows 启动 NSIS 并退出本进程；需要 `Update` 句柄（取目标和当前 .app 路径）。
- 方案：下载成功（签名已验）后，把 bytes 落盘到 `<app_data>/updates/<version>.bin`，同时写 `pending-update.json`：`{version, sha256, manualUrl, savedAt}`（原子写，先包后记录）。下次启动 `process_pending` 读出记录，校验 sha256（防磁盘损坏/篡改），再调 `Update::install(bytes)`。启动时没有现成的 `Update` 句柄，用 `updater_builder().version_comparator(|_, _| true)` 重新 `check()` 取句柄（只用于 install，不用它的下载）；离线时拿不到句柄 → `InstallError::Unavailable`，包保留，状态显示 `ready`，下次再装。安装器真的失败（磁盘满、权限）→ 删包和记录，状态 `install_failed`，带 `manualUrl`。
- 记录里的版本只接受 `x.y.z`，包路径由版本推出（不从记录读路径），防路径穿越。
- 强制升级另写 `<app_data>/upgrade-lock.json`：`{version, manualUrl}`。启动时若运行版本 < lock.version，直接进 `forced`（不用等清单），`upgradeRequired=true`；运行版本 ≥ lock.version 才清标志并删 lock。只有 `upgradeRequired` 标志、没有 lock 时，持续拒绝启动网关，直到一次有效清单不再要求升级才放开。

## 8.4 强制升级不可绕过

- 文件：`update/forced.rs`（`guard_gateway_start(state, flag)`）、`UpdateService::guard_gateway_start()`。`gateway_start` 与托盘「启动网关」都应调它，错误码 `upgrade_required`。
- `forced` 流程（`run_forced`）：置 `UpgradeFlag`、写 lock、停网关（只停一次，重试不重复停）、下载（已有同版本已验证包则复用）、`process_pending` 安装，成功后 `AppControl::restart()`；下载失败 → `forced{error:"下载失败"}`；安装失败 → `forced{error:"安装失败"}`，都带 `manualUrl`，不回到可用界面。
- 启动清标志：`startup()` 在新版本启动时（运行版本 ≥ lock）调 `UpgradeFlag::set(false)` 并删 lock。
- Red/Green：`update::forced::tests`（3 个）、`schedule::tests::forced_*` 与 `startup_*`（共 13 个）。mutation：去掉 `|| flag.is_set()`，`persisted_flag_blocks_start_before_first_check` 失败。
- 依赖注入：`app_config` 当时只有空壳，没有 `upgradeRequired` 接口，所以用 `ports::UpgradeFlag { is_set(), set(bool) }`，联调时由 `app_config` 实现；`GatewayControl { stop(), resume_if_wanted() }` 由 gateway 实现。
- 待补：kclaude 脚本读 `upgradeRequired` 并非零退出（6.1 脚本测试）属于 kclaude 批次，本批没碰。

## 8.5 启动时 pending（Rust）

- 文件：`update/pending.rs` + `pending/tests.rs`，`process_pending(dir, current, installer) -> StartupOutcome`。
- Red：写测试后在函数不存在时编译失败；Green：`pending` 13 个用例通过。覆盖：正常安装并清理、没有 pending、孤立包文件被清掉、文件丢失、记录损坏（坏 JSON）、sha256 不符（内容被改、被截断）、版本等于当前、版本低于当前、当前版本无法解析（绝不安装）、安装器失败（删包 + 返回 manualUrl，错误文案不含底层细节）、安装器暂不可用（保留包）、记录版本含 `../` 不能越出 `updates/`。
- mutation：跳过 sha256 校验，3 个用例失败；`>` 改成 `>=`，`version_equal_to_current_is_discarded_as_not_newer` 失败。

## 8.7 release.sh

- 文件：`desktop/scripts/release.sh`、`desktop/scripts/release_lib.py`、`desktop/scripts/test_release_lib.py`；`desktop/AGENTS.md` 末尾追加「发版」一节。
- 流程：校验 VERSION（`x.y.z` 且高于当前，三处版本必须一致）→ `--dry-run` 外改 package.json / tauri.conf.json / Cargo.toml / Cargo.lock 的本包条目（先算好全部新内容再写，原子替换）→ `pnpm tauri build --target universal-apple-darwin --bundles app,dmg --config '{"bundle":{"createUpdaterArtifacts":true}}'`（因为 `tauri.conf.json` 没有 `createUpdaterArtifacts`，本批只能改 pubkey，所以由命令行覆盖；联调批次可把它写进配置，脚本里的 `--config` 可去掉）→ 生成 `latest.json`（含 `manualDownload`、`minRequiredVersion`）→ 上传。
- 上传顺序：dmg → `Kiro-Gateway.app.tar.gz` → `.sig` → 对每个文件 HEAD（200 且大小一致）→ `latest.json`（`Cache-Control: no-cache`）。HEAD 失败、大小不符、上传异常时清单都不会被上传（`UploadOrderTests` 5 个用例）。上传时文件名规整为无空格的 `Kiro-Gateway_<v>_universal.dmg`、`Kiro-Gateway.app.tar.gz`。
- `FORCE=1`：`minRequiredVersion=VERSION`，需把版本号再输入一遍，不一致退出码 1 且不生成清单；不带 FORCE 时沿用线上清单的 `minRequiredVersion`（读不到用 `0.0.1`），防止悄悄解除上一次强制升级。
- 上传用 `/opt/miniconda3/bin/python -I` 调 `qcloud_cos.CosS3Client.upload_file`；凭证读 `COS_SECRET_ID`/`COS_SECRET_KEY` 或 `~/.tccli/default.credential`，不打印（测试断言输出不含 `AKID`/`secretKey`）。本批没有调用上传。
- `--dry-run` 实测（VERSION=0.0.2）输出上传计划 4 步并生成清单，`jq -e '.version and .minRequiredVersion and .notes and .pub_date and (.platforms|length==2) and .manualDownload.darwin'` 通过；三处版本号未被改动（测试 tearDown 断言）。dry-run 清单的签名是占位值，仅用于校验字段。
- Windows：`WINDOWS_BUNDLE_DIR` 指向 Windows 机器上出的 `bundle` 目录时，清单加 `windows-x86_64` 与 `manualDownload.windows`；本批没有 Windows 实机，只测了清单逻辑。

## 联调批次需要做的

- `src-tauri/Cargo.toml` 加依赖（见报告 deps），`main.rs` 注册 `tauri_plugin_updater::Builder::new().build()` 与 `tauri_plugin_process::init()`；`mod update;` 并启用 `tauri-glue` 对应的代码（`commands.rs` 里的 `setup` 和四个 command）。
- capabilities 增加 `updater:default`、`process:allow-restart`（前端目前不直接调这两个插件，所有升级动作都走 Rust command；如不需要可不加）。
- `app_config` 实现 `UpgradeFlag`；gateway 实现 `GatewayControl`；`gateway_start` 与托盘启动项调 `UpdateService::guard_gateway_start()`；托盘/状态栏用 `update_status`/`update-status`。
- 把 pending 方案补进 design 决策 11。
