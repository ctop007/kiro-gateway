# 修复组 B-update

日期：2026-10-23。未 git add/commit。未碰 127.0.0.1:8000、真实 ~/.zshrc 与 App 数据目录（测试全用 tempdir / 临时 HOME）。

最终结果：`cargo test` 791 passed；`pnpm test` 328 passed；`pnpm build` 通过；`tsc --noEmit` 通过；`test_release_lib.py` 47 passed（原 36）。

## 1. [高] 下载没有超时

- 原因：`check()` 返回的 `Update.timeout` 是 `None`，builder 的 30 秒只管清单请求；下载卡住则 `check_now` 永不返回，busy 不释放。
- 修法：
  - `UpdateBackend::download` 新增 `cancel: &AtomicBool` 参数。`UpdateService::download_with_stall_guard` 把下载放到工作线程，用 `await_download`（`recv_timeout`）等；60 秒（`DOWNLOAD_STALL_TIMEOUT`）没有任何进度事件即判失败并置 cancel。每个进度事件重置计时，慢但在走的下载不会被切断。失败路径都经 `check_now` 返回，`BusyGuard` 照常释放。
  - `commands.rs`：`update.timeout = Some(30 分钟)`（`Update.timeout` 是 pub，直接设，作为整次传输的兜底），下载放进 `tauri::async_runtime::spawn`，每 250 ms 看 cancel，被取消就 `abort()`（连接随 future 释放）。
- Red：`cargo test update::` 里新测试 `stalled_background_download_fails_and_releases_busy`、`stalled_forced_download_shows_failure_and_retry_downloads_again` 失败，原因是 `check_now did not return within 5s`（假下载器 `hang_downloads` 永不报进度；测试用 `within()` 把挂起变成失败而不是卡住）。
- Green：同样两个测试通过；`slow_download_that_keeps_reporting_progress_is_not_cut_off` 保证有进度的慢下载不被误杀。
- 变异检查：把等待时间换成 3600 秒，两个 stall 测试再次失败。
- 测试：`stalled_background_download_fails_and_releases_busy`（失败后状态回 none，第二次 check 真的发起并成功）、`stalled_forced_download_shows_failure_and_retry_downloads_again`（forced 显示「下载失败」，`retry` 再次下载并安装重启）、`slow_download_that_keeps_reporting_progress_is_not_cut_off`。阈值通过 `Rig::with_stall` 注入 80 ms。

## 2. [中] 强制升级的启动顺序

- (a) `main.rs::finish_startup`：先 `spawn_scheduler`（首个 tick 就是首次清单检查，与 env 检查、kclaude 并行），再在要恢复网关处调用 `UpdateService::resume_gateway_after_startup(FIRST_CHECK_WAIT = 10 s)`。它先等启动安装结束，再等首次检查有结论（成功 / 失败 / 超时）。结论为 `Forced`，或 `guard_gateway_start` 仍拒绝（forced 状态、`upgradeRequired` 标志、lock），则不恢复；`Clear` / `Failed` / `TimedOut` 照常恢复。首次检查在恢复之后才判定 forced 的，由 `run_forced` 里原有的 `gateway.stop()` 停网关。
  - 首次检查的结论在下载开始之前记录（`record_first_check`），所以 forced 下载很慢时恢复也不用等下载。
- (b) setup 不再同步联网：`update::commands::setup` 只调用 `spawn_startup`：同步部分 `begin_startup` 只读 pending 记录（本地文件，不联网）；记录有效且版本更新则状态置为 `installing`；安装 `complete_startup_install` 在 `update-startup` 线程里做。安装未结束时：`tick()` 等待、`check_now` 直接返回、网关不恢复（最多等 `INSTALL_WAIT` 120 s）。没有待装内容时 `begin_startup` 当场对账 lock，行为同原来。
- 新增状态 `installing`（`{ state: 'installing', version }`）：Rust `UpdateState::Installing`、`desktop/src/lib/ipc-types.ts` 的 `UpdateState`、`desktop/docs/ipc-contract.md`（类型、状态说明、启动顺序、变更记录）、`contract_tests.rs`（加入该变体）已同步。**界面由另一组处理**：需要在窗口里对 `installing` 显示「正在完成更新」；目前 `view.ts` 的 switch 对未知状态走 default（显示「已是最新」），tsc 通过，但文案不对，需前端组补 `src/features/update/view.ts` 与测试。托盘 `tray/model.rs` 对 `installing` 视为非 forced、非 ready，无需改动。
- Red：测试引用的 `begin_startup` / `complete_startup_install` / `resume_gateway_after_startup` / `Installing` 不存在，编译失败；加上实现的骨架后用变异验证：
  - 把「forced 或有 lock 不恢复」条件改成无条件恢复：`first_check_forced_keeps_the_gateway_off`、`first_check_forced_is_decided_before_the_download_finishes`、`locked_version_never_resumes_the_gateway_even_if_the_check_times_out` 失败。
  - 把等待首次检查的时间改成 0：`gateway_restore_waits_for_a_first_check_that_finishes_late` 失败。
- Green：全部通过。
- 测试：`first_check_forced_keeps_the_gateway_off`、`first_check_forced_is_decided_before_the_download_finishes`、`first_check_up_to_date_resumes_the_gateway`、`first_check_failure_resumes_the_gateway`、`first_check_timeout_resumes_the_gateway_without_waiting_longer`、`locked_version_never_resumes_the_gateway_even_if_the_check_times_out`、`gateway_restore_waits_for_a_first_check_that_finishes_late`、`gateway_restore_waits_for_the_startup_install`、`setup_does_not_block_on_the_pending_install`（假 backend 记录 `install` 的线程 id，断言不是调用线程；`spawn_startup` 在安装挂起时仍立即返回，状态为 `installing`）、`installing_state_is_emitted_before_the_install_result`、`startup_discarding_a_corrupt_record_leaves_no_installing_state`。原有 `startup_*` 测试通过 `#[cfg(test)] startup()`（两阶段连续执行）继续覆盖。

## 3. [低] release.sh 失败后版本号停在新版本

- 修法：`release_lib.py` 新增 `snapshot_version_files` / `restore_version_files`（package.json、tauri.conf.json、Cargo.toml、Cargo.lock，逐字节，顺带清掉半写的 `.release-tmp`）和 CLI 子命令 `snapshot` / `restore`。`release.sh` 在 `bump` 前做快照，`trap EXIT`（并把 INT / TERM 转成退出）在发布未完成时恢复；上传成功才置 `RELEASE_DONE=1` 并删快照。
- Red（变异）：注释掉 `trap rollback_versions EXIT` 后 `test_build_failure_restores_the_version_files_and_a_rerun_is_accepted`、`test_interrupt_during_the_build_restores_too` 失败（版本停在 0.0.2）。
- Green：恢复后通过，且重跑不再报「必须高于当前版本」。
- 测试（`test_release_lib.py`）：`RollbackAndPublishedFloorTests.test_restore_puts_back_every_version_file_byte_for_byte`、`test_restore_removes_a_half_written_temp_file`；`RealReleaseScriptTests`（真实 release.sh 跑在 tempdir 里的树副本上，PATH 里放假 pnpm）：`test_build_failure_restores_the_version_files_and_a_rerun_is_accepted`、`test_interrupt_during_the_build_restores_too`。

## 4. [低] 取不到线上清单就降回 0.0.1

- 修法：`previous_min_for_release`：非 FORCE、非 `FIRST_RELEASE` 时读不到线上清单（离线、404、没有合法 `minRequiredVersion`）就抛 `ReleaseError`，文案「无法读取线上 latest.json，为避免降低最低可用版本已中止；确认是首次发布请加 FIRST_RELEASE=1」。`release.sh` 新增 `FIRST_RELEASE=0|1`，在 bump 之前用 `check-published` 先检查（真发布时），`prepare` 也传 `--first-release`。测试用 `RELEASE_MANIFEST_URL` 覆盖清单地址（只读）。`--dry-run` 不联网，行为不变。
- Red（变异）：把抛错改成 `return None` 后 `test_unreadable_live_manifest_aborts_before_anything_is_written`、`test_non_force_release_aborts_when_the_live_manifest_cannot_be_read` 失败。
- Green：恢复后通过。
- 测试：`test_non_force_release_aborts_when_the_live_manifest_cannot_be_read`、`test_first_release_and_force_skip_the_lookup`、`test_readable_manifest_returns_its_floor`、`test_unreadable_live_manifest_aborts_before_anything_is_written`（在 bump 之前中止、版本文件不变、不进入出包）、`test_unreadable_live_manifest_does_not_stop_a_forced_release`、`test_bad_first_release_value_is_rejected`。

## 改动文件

- `desktop/src-tauri/src/update/{schedule.rs,ports.rs,commands.rs,types.rs,testing.rs,schedule/tests.rs}`
- `desktop/src-tauri/src/main.rs`（`finish_startup`）
- `desktop/src-tauri/src/contract_tests.rs`（加入 `Installing` 变体；该文件不在可改清单，仅此一行，因为契约测试要覆盖每个变体）
- `desktop/docs/ipc-contract.md`、`desktop/src/lib/ipc-types.ts`（只改类型与文档）
- `desktop/scripts/{release.sh,release_lib.py,test_release_lib.py}`

## 未验证

- 真实 tauri-plugin-updater 的取消路径（`task.abort()` 关闭 socket）只做了编译，没有对着真实慢服务器跑；逻辑层的停滞检测由假下载器覆盖。
- `release.sh` 的真实上传、真实 `pnpm tauri build` 没跑，用假 pnpm 覆盖失败路径。
