# 批次 config-gateway：4.1、4.2、4.3、4.4、4.5（含 5.4 的日志归类）

日期：2026-10-22。只改了 `desktop/src-tauri/src/app_config/` 和 `desktop/src-tauri/src/gateway/`。tasks.md 勾选、main.rs、Cargo.toml、Cargo.lock、ipc-contract.md 没动。

## 验证方式

独立测试 crate `/tmp/kgw-lane-config-gateway/`，用 `#[path]` 引入两个 `mod.rs`，`CARGO_TARGET_DIR=/tmp/kgw-target-config-gateway`。

- `cargo test`（不开 feature）：288 passed。连续跑 25 次全绿（中途修过两处只在机器忙时才出现的时序问题，见「踩到的坑」）。
- `cargo test --features tauri-glue`：480 passed（多出的是 `env_check`、`update` 两条线自己的测试，因为 glue 要 import 它们）。tauri 2.11.1、`tauri-plugin-autostart` 2.5.1、`tauri-plugin-opener` 2.5.4、`tauri-plugin-updater` 2.11.0 编译通过，无 warning。
- `cargo check --target x86_64-pc-windows-msvc --lib`：通过，`#[cfg(windows)]` 的 Job Object、`OpenProcess`、`netstat`/`tasklist` 解析都编译过。**没有在 Windows 上运行过**。
- 联调后在 `main.rs` 里 `mod app_config; mod gateway;`，测试路径变成 `app_config::…` 和 `gateway::…`，与 tasks 里的 `cargo test gateway::env`、`gateway::port`、`gateway::redact`、`gateway::classify` 一致。tasks 写的 `gateway::state` 对应本批的 `gateway::manager`（状态机和进程监督在同一个文件里），联调时需要的话把 tasks.md 里的命令改成 `gateway::manager`，不值得为一个名字改 44 个测试的路径。

各区域用例数：

| 区域 | 数量 |
| --- | --- |
| app_config::atomic | 11 |
| app_config::secret | 12 |
| app_config::env_file | 5 |
| app_config::model | 17 |
| app_config::store | 25 |
| gateway::env | 14 |
| gateway::sidecar | 13 |
| gateway::logfile | 20 |
| gateway::redact | 22 |
| gateway::classify | 16 |
| gateway::sys | 11 |
| gateway::port | 36 |
| gateway::health | 17 |
| gateway::manager | 45 |
| gateway::service | 20 |
| gateway::real（真实 sidecar） | 4 |

## 模块

`app_config/`
- `paths.rs`：数据目录下所有路径（config.json、secret、kclaude.env、logs/gateway.log、logs/requests、runtime、gateway.pid）。
- `atomic.rs`：临时文件 + rename 的原子写；`FileMode::Private` 在**创建临时文件时**就带 0600（不是写完再 chmod），所以内容从第一个字节起就不会被别的用户读到。
- `model.rs`：`AppConfig` / `ConfigPatch` / `ModelTier`、校验、`ConfigError`（code 与 contract 的错误表一致，message 是中文且不带路径）。
- `secret.rs`：32 字节 `getrandom`，64 位小写 hex；已有文件校验后复用，权限松了就收紧到 0600，空/损坏就重新生成并返回 `Regenerated`。
- `env_file.rs`：`kclaude.env` 的六行，顺序固定。
- `store.rs`：`ConfigStore`，一把互斥锁保护三个文件。`open`（首次/损坏/复用）、`update`、`set_gateway_wanted`、`set_upgrade_required`（给 update 模块）、`sync_autostart`、`check_patch`。
- `commands.rs`（`tauri-glue`）：`setup`、`config_get`、`config_update`。

`gateway/`
- `types.rs`：contract 里的 `GatewayStatus` / `GatewayFailure` / `FailureKind` / `PortOccupant`。
- `env.rs`（4.2）：环境变量组装。
- `sidecar.rs`（4.2）：`resolve(resource_dir, KIRO_GATEWAY_SIDECAR)`；`prepare_runtime_dir`（建 `runtime/` 并删掉里面的 `.env`）。
- `logfile.rs`（4.2）：`RotatingLog`（5 MB × 3，按行轮转）、`pump`（把管道拷进日志）、`read_run_tail`（只读文件末尾 256 KB，只取最后一次启动标记之后的行）。
- `redact.rs`（4.5）：去 ANSI、替换密钥原文、遮蔽 ≥32 位 base64/hex 串。
- `classify.rs`（5.4）：从日志识别 `login_expired` / `port_in_use`。
- `port.rs`（4.4）：端口探测、`lsof` 占用者查询（Windows 用 netstat + tasklist）、`gateway.pid`、`reclaim_stale`。
- `sys.rs`：存活、可执行文件路径、SIGTERM/SIGKILL，以及 Windows 的 Job Object（`cfg(windows)`）。
- `health.rs`：`/health` 探测和 `/v1/models` 代请求（ureq 3.5.0，忽略系统代理）。
- `manager.rs`（4.3）：状态机。
- `service.rs`：`gateway_start` 周围的规则（升级门、环境门、找 sidecar、拼环境变量、`gatewayWanted`）。
- `real.rs`：用 `resources/gateway/kiro-gateway`（真实 PyInstaller 产物）跑四个测试；没构建 sidecar 时打印 SKIPPED。
- `commands.rs`（`tauri-glue`）：`setup`、`gateway_status/start/stop/models/reveal_log`、`gateway-status` event、`shutdown`、给 update 模块用的 `UpdateGatewayControl` 和 `UpdateUpgradeFlag`。

## 4.1 app_config

Red：先写了 `atomic`、`secret`、`env_file`、`model`、`store` 五个测试文件（69 个用例），模块只有空壳。`cargo test app_config`：`could not compile`，170 个错误，最多的几类：
```
31 error[E0433]: cannot find type `ConfigStore` in this scope
14 error[E0433]: cannot find type `AppConfig` in this scope
14 error[E0425]: cannot find function `is_valid_secret` in this scope
13 error[E0433]: cannot find type `FileMode` in this scope
13 error[E0425]: cannot find function `write_atomic` in this scope
```
Green：`test result: ok. 69 passed; 0 failed`。之后补了 `check_patch` 一条（先 `no method named check_patch`，再通过），`app_config` 共 70 个。

覆盖：
- 首次生成：`first_open_creates_defaults_secret_and_env_file`、`first_run_creates_the_file_and_reports_created`。
- 重启后不变：`reopening_keeps_config_and_secret_unchanged`、`second_run_returns_the_same_secret_without_rewriting`、`reopening_an_up_to_date_install_writes_nothing`（三个文件的 mtime 都不变）。
- 损坏回默认并备份：`corrupt_config_is_backed_up_and_replaced_by_defaults`（备份名 `config.json.bak-20261022-144108`）、`every_kind_of_corruption_is_handled`（空文件、截断、数组、`null`、类型错、端口越界、端口溢出、模型名为空、模型名带空格、二进制，共 10 种，备份与原文件逐字节相同）、`two_corruptions_in_the_same_second_keep_both_backups`（同一秒两次损坏不覆盖旧备份）、`corrupt_config_does_not_regenerate_a_good_secret`。
- 密钥：`created_file_is_0600`、`loose_permissions_on_an_existing_secret_are_tightened`、`empty_or_garbled_file_is_regenerated`、`non_utf8_file_is_regenerated`、`a_directory_in_place_of_the_file_is_an_error`、`private_files_are_0600_even_with_a_permissive_umask`、`env_file_and_secret_are_0600_and_config_is_not_world_writable`。
- 原子写：`failed_rename_cleans_up_and_reports_the_error`（目标是非空目录，rename 失败后临时文件被清掉、原目录不动）、`concurrent_readers_never_see_a_partial_file`（512 KB 文件，写线程反复在 A/B 两种内容之间切换，读线程每次读到的必须整份是 A 或 B）、`two_writers_in_the_same_directory_do_not_collide_on_temp_names`、`no_temporary_files_are_left_after_many_updates`。
- kclaude.env：`real_shells_read_back_exactly_the_configured_values`——用真的 `/bin/sh`、`/bin/bash`、`/bin/zsh` source 生成的文件再打印变量，值必须与配置完全一致（含 `claude-opus-5-5[1m]` 里的方括号和 `us.anthropic.claude-haiku-4-5:0` 里的冒号）。这是「不加引号也安全」这句话的证据，不是假设。
- 配置变化时 env 同步：`update_rewrites_kclaude_env_with_the_new_values`、`upgrade_required_is_written_to_both_files_and_survives_reopen`、`a_missing_or_stale_env_file_is_rewritten_on_open`。
- 校验：`validate_port_rejects_everything_outside_the_range`（-1、0、1、80、1023、65536、70000、`i64::MAX`、`i64::MIN`）、`validate_model_rejects_empty_and_shell_unsafe_names`（空、空白、空格、换行、`' " $ ; \` & | \ #`、中文、129 字符）、`apply_patch_is_all_or_nothing`、`invalid_patch_is_rejected_and_nothing_changes`。
- 前端不能清掉强制升级：`a_user_patch_cannot_clear_upgrade_required`（patch 里带 `upgradeRequired:false` 会被忽略）。
- 写失败：`write_failure_is_reported_and_memory_keeps_the_old_config`（`write_failed`，内存里仍是旧值，message 不含路径）。
- 并发：`concurrent_updates_never_lose_or_tear_the_file`（8 线程 × 10 次，最终 config.json 与内存一致，kclaude.env 的端口与最后提交的一致）。

## 4.2 环境变量、sidecar 路径、gateway.log

Red：`env`、`sidecar`、`logfile`、`redact`、`classify` 五个测试文件（85 个用例）先写，实现文件只有 `mod tests;`。`cargo test gateway::`：`could not compile`，
```
18 error[E0425]: cannot find function `classify` in this scope
15 error[E0433]: cannot find type `RotatingLog` in this scope
14 error[E0425]: cannot find function `build_env` in this scope
11 error[E0425]: cannot find function `strip_ansi` in this scope
10 error[E0425]: cannot find function `resolve` in this scope
```
Green：`test result: ok. 85 passed; 0 failed`。

覆盖（`gateway::env`，14 个）：
- 设置 design 决策 2 的全部变量：`PROXY_API_KEY`、`KIRO_CLI_DB_FILE`、`SERVER_HOST`、`SERVER_PORT`、`FAKE_REASONING=false`、`REQUEST_LOG_DIR`、`TIKTOKEN_CACHE_DIR`、`LOG_LEVEL=INFO`（`sets_every_variable_of_design_decision_2`）。
- 继承的 `ANTHROPIC_BASE_URL` 等被移除（`inherited_anthropic_variables_are_removed`），继承的 `PROXY_*` 被移除、密钥是我们的（`inherited_proxy_prefixed_variables_are_removed_and_the_key_is_ours`），大小写不敏感（Windows 的环境变量名不区分大小写）。
- `SERVER_HOST` 无论环境里写什么都是 `127.0.0.1`。
- 同时清掉 `REFRESH_TOKEN`、`KIRO_CREDS_FILE`、`ACCOUNT_SYSTEM`、`ACCOUNTS_CONFIG_FILE`、`ACCOUNTS_STATE_FILE`：它们会让网关换一条凭证来源或打开多账号系统（design 的 non-goal），是 design 决策 2 没写但同一类问题。
- 追加 `PYTHONUTF8`、`PYTHONIOENCODING`（sidecar 批次冒烟发现 Windows 重定向输出会因启动横幅里的 emoji 报 `UnicodeEncodeError`）和 `PYTHONUNBUFFERED`（崩溃前最后几行必须已经写进日志）。
- 不相关的继承变量（PATH、HOME、`HTTPS_PROXY`、LANG）原样透传。

`gateway::sidecar`（13 个）：`KIRO_GATEWAY_SIDECAR` 覆盖优先；覆盖目标不存在时报错而不是悄悄退回内置版本；空覆盖值当作没设；目录不是可执行文件；没有执行位；`tiktoken_cache/` 在可执行文件旁边时被带上；`runtime/` 里残留的 `.env` 启动前删掉（`load_dotenv()` 读不到）。

`gateway::logfile`（20 个）：5 MB × 3 常量；按行轮转（一行永不拆到两个文件）；总大小不超过 3 × 上限；超长单行照写；一份文件的配置退化为截断；重开后继续追加；运行中被外部删掉后重建；`pump` 容忍非 UTF-8 和末尾无换行、把超过 64 KB 的行切开、两路 pump 不会在一行里交错（各 500 行，逐行检查）；`read_run_tail` 只读文件末尾（6 万行的文件）、丢掉被读取窗口切断的第一行、只取最后一次启动标记之后的内容。

## 4.3 状态机与进程监督

Red：`sys`、`port`、`health`、`manager` 四个测试文件（含假 sidecar、假进程检查器、本地假 HTTP 服务）先写，实现文件只有 `mod tests;`。`cargo test --lib`：160 个编译错误：
```
50 error[E0425]: cannot find function `X` in this scope
34 error[E0433]: cannot find type `X` in this scope
34 error[E0425]: cannot find value `X` in this scope
10 error[E0432]: unresolved import `X`
```
Green：全部实现后 `262 passed`；之后加 `service`（20）、`real`（4）和 `production_settings_match_the_design`，共 288。

假子进程是临时目录里的脚本（`testing.rs`）：一个 Python 脚本，由写进脚本的模式决定行为——`serve`（监听 `SERVER_PORT`，回 `/health` 和 `/v1/models`）、`slow:N`、`never`（不监听）、`exit:N`、`crash_after:N`、`ignore_term`（忽略 SIGTERM）、`login_fail`、`bind_fail`、`echo_secret`、`many_lines:N`。其中 `never` 和 `login_fail_hang` 为了启动快用 `sh`。没用 `cp /bin/sleep`：macOS 上复制出来的系统二进制会被代码签名机制杀掉（实测退出码 137）。

tasks 4.3 的四个场景：
- **启动成功**：`start_goes_stopped_starting_running_and_health_answers`——状态序列严格是 `starting → running`，`/health` 真的可访问；`a_slow_start_stays_starting_until_health_passes`。
- **启动超时**：`a_start_that_never_becomes_healthy_fails_with_timeout_and_the_process_is_ended`——`start_failed`、`kind=timeout`、进程被结束、pid 文件被删、日志摘要里有进程输出的最后几行。
- **运行中被 kill 后 5 秒内变为意外退出**：`an_external_kill_is_noticed_within_five_seconds_and_becomes_crashed`——对运行中的假网关发 SIGKILL，断言 `crashed` 出现在 **1 秒内**（wait 线程立即发现，不靠心跳）、端口已释放、pid 文件已删、事件序列 `starting, running, crashed`。真实 sidecar 上同一场景见 `the_real_sidecar_killed_from_outside_is_reported_crashed`。
- **重复点启动不会起两个进程**：`starting_twice_does_not_launch_a_second_process`（假 sidecar 每次启动往文件里追加一行，断言只有一行）、`concurrent_starts_launch_exactly_one_process`（8 个线程同时 `start`，只启动一次，事件只有一组 `starting, running`）。

其他覆盖：
- 心跳：`three_failed_heartbeats_in_a_row_declare_a_hung_gateway_crashed`（SIGSTOP 让进程活着但不响应，3 次连续失败后 `crashed`，消息「没有响应」，进程被 SIGKILL 结束——SIGSTOP 的进程收不到 SIGTERM）；`one_or_two_failed_heartbeats_do_not_declare_the_gateway_dead`（失败必须连续，中间恢复就清零）。
- 停止：`stop_sends_sigkill_after_the_grace_period_when_sigterm_is_ignored`（忽略 SIGTERM 的进程至少等满宽限期再被 SIGKILL）；`a_cooperative_process_is_stopped_well_before_the_grace_period`；`stop_while_starting_cancels_the_start`；`a_deliberate_stop_is_not_reported_as_a_crash`；`stop_can_be_called_from_several_threads_at_once`；`the_group_of_a_stopped_gateway_leaves_no_processes_behind`（`ps -g` 查进程组里没有残留）。
- 失败原因：`a_process_that_exits_at_once_is_a_start_failure_with_its_exit_code_and_output`（`other`，消息含退出码）；`a_log_with_a_refresh_403_means_login_expired`；`login_expired_wins_over_timeout_when_the_process_hangs_after_the_error`（进程打完错误不退出也不监听，超时到了仍归为 `login_expired`）；`a_bind_error_in_the_log_means_port_in_use_with_the_occupant_looked_up`；`an_executable_that_cannot_be_started_is_a_start_failure_not_a_command_error`。
- 日志摘要：`the_failure_excerpt_is_redacted_and_the_secret_never_appears`（密钥原文、40 位 hex 都不出现，整个 `GatewayStatus` 序列化后也不含密钥）；`the_excerpt_keeps_only_the_last_twenty_lines`；`the_excerpt_of_a_second_run_does_not_contain_the_first_runs_output`。
- upgradeRequired：`upgrade_required_rejects_start_without_launching_anything`（返回 `upgrade_required`，消息「需要先升级到新版本才能使用」，没有起进程、没有发事件）。
- 事件：`consecutive_identical_statuses_are_never_emitted`；改端口只在 stopped 时影响状态；`since`/`at` 用注入的时钟。
- 默认参数：`production_settings_match_the_design` 断言 30 秒 / 500 ms / 10 秒 / 3 次 / 5 秒 / 20 行。
- `gateway_models` 的代请求：`models_come_from_the_running_gateway_with_the_key`（真的带 `Authorization: Bearer <密钥>`，错误的密钥得到 `gateway_unreachable`）、`models_while_not_running_is_gateway_not_running`（`starting` 也算没运行）。

`gateway::health`（17 个）还包括：不跟随重定向；**忽略环境里的 HTTP_PROXY**（设一个死代理，请求仍然直连成功——否则 App 会把访问密钥发给用户配置的代理）；服务端接受连接却不回应时按超时放弃；200 但不是模型列表 → `BadResponse`；条目缺 `id` 或 `id` 不是字符串 → 跳过；结果排序去重；错误信息不含 URL、端口、密钥。

`gateway::sys`（11 个）还包括：`a_killed_but_not_yet_reaped_child_is_not_alive`（见「踩到的坑」）；`kill_takes_the_whole_process_group_with_it`（sidecar 作为自己进程组的组长启动，SIGKILL 发给整组，后台孙进程一起结束）；`never_signals_pid_zero_one_or_itself`（pid 0 会变成「我自己的进程组」，pid 1 是 launchd，都拒绝）。

## 4.4 端口与遗留进程

Red/Green 同上（`port` 在 4.3 那一批里）。`gateway::port` 36 个。

tasks 4.4 的五个场景：
- **端口空闲**：`a_free_port_is_free`；探测后端口不被占用（`probing_does_not_leave_the_port_occupied`）。
- **被他人占用**：`a_loopback_listener_makes_the_port_taken…`、`a_wildcard_listener_counts_as_taken_too`（见「踩到的坑」）、占用者的名字与 pid：`the_occupant_of_a_port_is_found_by_pid_and_name`；管理器层面 `a_port_held_by_someone_else_is_reported_with_name_and_pid_and_left_alone`（占用者仍在监听，没有启动任何假 sidecar）。
- **pid 存活且是自己**：`a_leftover_gateway_of_ours_is_terminated_and_the_record_removed`（假进程检查器，断言只收到 `terminate 4242`）；`a_real_leftover_process_is_reclaimed_end_to_end`（真的子进程占着真的端口，回收后进程消失、端口释放）；管理器层面 `a_leftover_sidecar_of_ours_is_reclaimed_so_the_start_succeeds`。忽略 SIGTERM 的遗留进程在宽限期后被 SIGKILL（`a_leftover_that_ignores_sigterm_is_killed_after_the_grace_period`）。
- **pid 被复用成别的进程**：`a_pid_reused_by_another_program_is_never_signalled`、`a_real_unrelated_process_with_a_reused_pid_survives`（真的 `/bin/sleep`，回收后仍在运行）、`a_bystander_on_the_port_with_a_recorded_pid_of_another_program_is_never_killed`（管理器层面：端口被占、pid 文件指向一个实际是别的程序的进程 → `port_in_use`，旁观者进程仍在运行）。再加两条更严的：记录里的路径不是**当前** sidecar 时也不信（`a_record_written_for_another_executable_is_not_trusted`）；查不到进程路径时不动它。
- **pid 文件损坏**：`damaged_pid_files_are_reported_as_corrupt`（12 种：空、纯文本、截断、数组、缺 exe、缺 pid、pid 是字符串、负数、0、超大、exe 为空、二进制）+ 目录占了文件名。损坏记录被删除，不杀任何进程。
- 伪造记录不能让 App 杀自己或 launchd：`the_app_never_terminates_itself_or_init_through_a_forged_record`。

`lsof` 的参数是写死的 `["-nP", "-iTCP:<port>", "-sTCP:LISTEN", "-Fpc"]`，端口是 `u16`（`lsof_arguments_are_fixed_and_contain_nothing_from_outside`）。输出解析用本机实测的格式：`p84400\ncpython3.13\nf3\n`。

## 4.5 日志摘要脱敏

`gateway::redact`（22 个）。用的是 `loguru` 带颜色的原始格式（取自真实网关的输出，开了 `colorize=True`），所以先去 ANSI 再处理：
- `strips_color_codes_from_real_loguru_output`、CSI/OSC/截断序列/孤立 ESC、`\r`。
- ≥32 位 hex 或 base64 被遮蔽；**31 位不遮蔽、32 位遮蔽**（`run_of_exactly_32_is_masked_and_31_is_not`）；base64url（`-`、`_`）、带 `+` `/` 的标准 base64（AWS 密钥格式 `wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY`）；JWT 的头部段。
- 密钥原文被遮蔽，即使被拆在颜色码中间也能认出来（`the_access_key_is_masked_even_when_embedded_or_split_by_color_codes`）；手工设置的短密钥按原文匹配；空密钥不会把整行抹掉。
- 不能误伤：文件路径（`/var/folders/r9/j4988b2n0219vhfbzp5k12ch0000gn/T/…`，长度 ≥32 但按 `/` 切开后每段都短）、`_load_enterprise_device_registration` 这种每条日志都有的长标识符（只含字母和下划线，无数字）、中文。
- 取尾部：最后 20 行、空行不占名额、单行超过 400 字符截断并加 `…`（不会切坏多字节字符）。

已知的取舍，写在 `redact.rs` 的模块注释里：纯字母的 base64 token（随机 token 里约千分之一）靠形状认不出来；访问密钥本身一定按原文命中。

## 5.4 日志归类

`gateway::classify`（16 个）。**测试用的日志行是我用真实网关代码生成的**，不是凭记忆写的：
- 在 `/tmp/kgw-fixtures/gen2.py` 里用 `AccountManager._initialize_account` 加 `httpx.MockTransport`，分别让 token 端点返回 403、401、400，把 loguru 输出抓下来（该脚本在批次之外的临时目录里，不进仓库）。得到：
  ```
  ERROR | kiro.auth:_do_aws_sso_oidc_refresh:840 - AWS SSO OIDC refresh failed: status=403, body={"message":"forbidden"}
  ERROR | kiro.account_manager:_initialize_account:578 - Failed to initialize account …: Client error '403 Forbidden' for url 'https://oidc.us-east-1.amazonaws.com/token'
  ERROR | kiro.account_manager:_initialize_account:578 - Failed to initialize account …: Token expired and refresh failed. Please run 'kiro-cli login' to refresh your credentials.
  ```
  Kiro Desktop 路径的 `…/refreshToken` 同样抓过。
- 端口占用的行取自真实 uvicorn：先用另一个进程占住 8150，再启动真实网关，输出 `[Errno 48] error while attempting to bind on address ('127.0.0.1', 8150): address already in use`。

规则：
- 401/403 + 刷新失败（`AWS SSO OIDC refresh failed: status=401|403`）→ `login_expired`；
- `Client error '401|403` 且 URL 是 token 端点（`oidc.…/token` 或 `/refreshToken`）→ `login_expired`；
- `Token expired and refresh failed` + `kiro-cli login`（400 后再失败）→ `login_expired`；
- `error while attempting to bind on address` → `port_in_use`（从行里取出端口号）。

不归类（有测试）：对 `runtime.us-east-1.kiro.dev` 的 403（是请求被拒，不是登录失效）、token 端点的 404/429、`Token refresh failed with 400 after SQLite reload`（只是警告，网关会继续用旧 token）、`Run 'kiro-cli login' when convenient`（同样是警告）。刷新后又成功的（`Token refreshed via AWS SSO OIDC` / `via Kiro Desktop Auth`）取消先前的失败。最近的事件优先。

真实 sidecar 验证：`the_real_sidecar_reports_a_taken_port_in_a_way_the_classifier_understands`——真实的 PyInstaller 网关在被占用的端口上启动，退出码非 0，它的 stdout+stderr 被 `classify` 归为 `PortInUse` 且端口号正确。**这条证明了正则和打包后的真实输出对得上**。

## 真实 sidecar 实测

`desktop/src-tauri/resources/gateway/kiro-gateway`（sidecar 批次的产物，44 MB）：

- 临时 HOME、假的 kiro-cli 数据库（一年后过期的假 OIDC token，不联网）、`PROXY_API_KEY` 为我们的密钥、环境里故意放了 `ANTHROPIC_BASE_URL` / `PROXY_API_KEY=leaked-key` / 死的 `HTTPS_PROXY`，端口取 81xx。
- `the_real_sidecar_starts_serves_models_with_our_key_and_stops_cleanly`：**279 ms 内 `/health` 通过**，`/v1/models` 带正确密钥返回模型列表、错误密钥失败，`lsof` 显示只监听 `127.0.0.1`（没有 `*:` 或 `0.0.0.0`），日志里有真实横幅且不含密钥，停止后进程消失、端口释放，事件序列 `starting, running, stopped`。
- `the_real_sidecar_killed_from_outside_is_reported_crashed`：SIGKILL 后 `crashed`，摘要非空、没有密钥、没有转义字符。
- `read_run_tail_of_a_real_log_shows_the_current_run_only`：启动两次，日志尾只含最后一次的 `Starting Uvicorn server`。
- 全部用 8100–8199 的端口。8000 上用户的网关（pid 61088）全程没碰。

## 踩到的坑（都已修，测试守住）

1. **macOS 上 `kill(pid, 0)` 对僵尸进程返回成功**。`is_alive` 先用它判断，被 `kill -9` 但还没被 wait 的子进程会一直「活着」，`reclaim_stale` 等它退出直到超时，返回 `Failed`（`a_real_leftover_process_is_reclaimed_end_to_end` 失败，`left: Failed { pid }`）。先写了 `a_killed_but_not_yet_reaped_child_is_not_alive`（Red：`zombie counted as alive`），再修：macOS 上 `proc_pidinfo` 对僵尸返回 ESRCH，据此判定。（`proc_bsdinfo.pbi_status == SZOMB` 那条路走不通，实测 `got=0`。）
2. **绑定 `127.0.0.1` 不能发现 `0.0.0.0` 上的监听者**（macOS 上带 `SO_REUSEADDR` 时，Rust 的 `TcpListener::bind` 会设置它）。用户用 `python main.py` 起的网关默认监听 `0.0.0.0`，只靠 bind 探测会误判端口空闲，然后 sidecar 启动时才报错。`is_free` 先 `connect` 一次，连得上就是被占用，再 bind。实测：Python 里 `0.0.0.0` 监听 + `127.0.0.1` 带 REUSEADDR bind → 成功（漏判）；`connect_ex` → 35（EAGAIN，有人监听）。`a_wildcard_listener_counts_as_taken_too` 守住。
3. **测试夹具的时序**：`never` 假进程用 Python 写，机器忙（别的批次在同一台机器上编译）时 Python 启动要一秒多，1.2 秒的超时在假进程打出第一行之前就到了，日志摘要为空，`login_expired_wins_over_timeout…` 约 1/5 的概率失败。先加大超时仍不稳（3/15）；最后改成**让健康探测等到日志里出现目标行再返回**（`GateOnLog`），超时只在探测返回后才检查，所以不依赖机器快慢。之后 25/25 全绿。另外 `free_port` 一度会重复发出同一个端口（探测只能证明「现在」空闲，刚启动的 sidecar 可能还没绑定），改成最近 50 个不重发。
4. **重写正在被执行的脚本会得到写了一半的文件**（`import: command not found`，shell 把 Python 脚本当 sh 执行）。`write_fake_sidecar` 现在内容没变就不写，要写就先写临时文件再 rename。

## 需要联调批次做的事

### deps（加进 `src-tauri/Cargo.toml`）

已有的不重复：`serde`、`serde_json`、`chrono`、`getrandom`、`libc`、`windows-sys`、`tempfile`。新增：

- `ureq = { version = "=3.5.0", default-features = false }`：只用 HTTP（`127.0.0.1`），不要 TLS，所以关掉默认的 `rustls` 和 `gzip`。依赖树新增约 12 个 crate（`ureq-proto`、`http`、`httparse`、`utf8-zero` 等），体积增量很小。**不是** 参考项目 Cargo.lock 里已验证的版本（参考项目 用 reqwest，没有 ureq），3.5.0 是写这段时 crates.io 最新版。没选 reqwest blocking：它拉 tokio + hyper 全套，而 App 里 tauri 已经有一套，再引一套 blocking 运行时没有好处。
- `[target.'cfg(windows)'.dependencies] windows-sys` 的 features 现在是 `Win32_Foundation`、`Win32_Security`、`Win32_System_JobObjects`、`Win32_System_Threading`——本批用到的 API 都在这几个里（`CreateJobObjectW`、`SetInformationJobObject`、`AssignProcessToJobObject`、`OpenProcess`、`QueryFullProcessImageNameW`、`TerminateProcess`、`GetExitCodeProcess`、`CloseHandle`），`cargo check --target x86_64-pc-windows-msvc` 已通过，不需要再加。
- `libc` 要的 API：`kill`、`killpg`、`proc_pidpath`、`proc_pidinfo`、`proc_bsdinfo`，当前的 `=0.2.186` 够用。
- `[features]` 加 `tauri-glue = []`（本批的 `commands.rs` 还用了 `tauri-plugin-autostart` 的 `ManagerExt` 和 `tauri-plugin-opener` 的 `OpenerExt`，这两个 main.rs 已经注册了，不用新增依赖）。

### 接入 main.rs

```rust
mod app_config;
mod gateway;
// env_check、update 等由各自的批次接入
```

`setup` 里按这个顺序（有依赖）：
```rust
let (config, notes) = app_config::commands::setup(app.handle())?;           // 记录 notes（首次生成密钥 / 配置损坏已备份）
let gateway = gateway::commands::setup(app.handle(), config.clone())?;       // 注册 Arc<GatewayService>
// 恢复开关：
//   - 普通启动：gateway.resume_if_wanted()（在后台线程里调，里面会跑环境检测）
//   - update 模块：update::commands::setup(app.handle(),
//         Arc::new(gateway::commands::UpdateGatewayControl(gateway.clone())),
//         Arc::new(gateway::commands::UpdateUpgradeFlag(config.clone())))
```
`generate_handler!` 注册：`app_config::commands::{config_get, config_update}`、`gateway::commands::{gateway_status, gateway_start, gateway_stop, gateway_models, gateway_reveal_log}`。

退出：托盘「退出」和 `RunEvent::Exit` 里调 `gateway::commands::shutdown(app)`（同步，返回时进程已结束）。托盘需要启停时调 `Arc<GatewayService>` 的 `start()` / `stop()`，状态变化已经通过 `gateway-status` event 发出，托盘可以另外挂 `service.manager().add_listener(...)` 更新图标。注意回调在持有状态锁时被调用，**回调里不要再调 manager**。

`config_update` 里 `gateway.apply_port` 已经处理了「停止状态下改端口，状态里的 port 立刻变」。

### contract_changes（建议）

1. `gateway_start` 的错误表：`env_not_ready` 的 `message` 现在就是 `env_check::gateway_gate` 给的原因句（「先安装 kiro-cli 才能打开」等），前端 `startErrorView` 已经直接显示 message，不用改；建议在 contract 里写明 message 即界面文案。
2. `gateway_reveal_log` 在没有日志文件时返回 `{code:"internal", message:"还没有网关日志"}`——contract 已经这样写了，实现照办。
3. `config_update` 在系统开关开机自启失败时返回 `write_failed`（message「没能修改开机自启，请检查系统设置」），并且此时端口、模型的改动**也不会写入**（先做会失败的系统调用，再写盘），保证不会半成功。contract 的错误表里 `config_update` 的 `write_failed` 原本只写了「写文件失败」，建议补一「或开机自启开关失败」。
4. `GatewayStatus.failure.message` 的 `timeout` 文案是「30 秒内网关没有就绪，已结束这次启动」，`crashed` 的默认文案是「网关进程意外退出（退出码 N）」/「被信号 N 结束」，`hung` 是「网关进程没有响应，已结束」，`port_in_use` 是「<端口> 端口被别的程序占用」，`login_expired` 是「kiro-cli 的登录已失效」。前端 `view.ts` 对 `login_expired` 和 `port_in_use` 用自己的文案，其余显示 `failure.message`——两边一致，无需改动。
5. `PortOccupant` 在 `port_in_use` 且 lsof 找不到时是 `{name:null,pid:null}`（不是整个 `occupant: null`）。contract 写的是「仅 kind=port_in_use 时非 null」，一致；前端已处理两个字段都为 null。

### 没有被联调批次覆盖的东西

- `kclaude` 脚本读 `kclaude.env`（上一批已写好模板），本批保证文件在每次配置/密钥/升级标志变化时同步更新。
- 关于 `gateway_wanted`：`gateway_start` 在**被接受**时（升级门、环境门都过了）置 true，不管之后启动成功与否——用户要的是「开着」，端口被占或启动超时时下次启动 App 仍会重试。`gateway_stop` 置 false。`shutdown`（App 退出）和崩溃都不改，所以重启或更新后能恢复。这与 contract 一致（「意外退出不改 gatewayWanted」），`shutdown` 的情况 contract 没单独写，按 design 决策 11（更新后恢复）处理。

## 没做 / 做不到的

- **Windows 上没有运行过任何代码**。Job Object、`OpenProcess` 系列、`netstat`/`tasklist` 的调用只通过了交叉编译检查（`x86_64-pc-windows-msvc`）；`netstat -ano` 和 `tasklist /FO CSV /NH` 的**解析**有单元测试（用的是我记得的输出格式，没有在真机上对过，Windows 本地化版本的列头/`LISTENING` 字样可能不同）。tasks 9.3 的 Windows 实机验证要补这一块。
- Windows 的 Job Object 绑定在 `launch` 里是 `#[cfg(windows)]` 的 `bind_to_job`，**没有测试**（没有 Windows 环境）。
- 没有测试睡眠唤醒后的行为：10 秒心跳靠 `Condvar::wait_timeout`，睡眠期间时钟的行为取决于系统；唤醒后第一次心跳可能因为连接被系统回收而失败，一次失败不会判挂（需要连续 3 次）。
