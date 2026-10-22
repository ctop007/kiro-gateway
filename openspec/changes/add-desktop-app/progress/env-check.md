# 批次 env-check：5.1、5.2（含 EnvReport 组合）

日期：2026-10-22。只改了 `desktop/src-tauri/src/env_check/` 和本文件。没动 main.rs、Cargo.toml、Cargo.lock，tasks.md 的勾选没改。

## 文件

- `env_check/mod.rs`：模块说明、`Platform`、`UserDirs`（home / LOCALAPPDATA / APPDATA，只认绝对路径）
- `env_check/which.rs`（5.1）：`find_tool`、`default_dirs`、`executable_names`、`probe_shell_path`、`PathCache`、`locate`
- `env_check/login.rs`（5.2）：`database_candidates`、`find_database`、`sqlite_read_only_uri`、`open_read_only`、`probe_database`、`LoginInfo` / `LoginProbe`
- `env_check/report.rs`：`build_report`（EnvReport）、`gateway_gate`、`kclaude_unavailable_reason`、`login_command`、`display_path`、`run_env_check`
- `env_check/commands.rs`：`#[cfg(feature = "tauri-glue")]`，`env_check` command，无逻辑

## 测试方式

独立临时 crate `/tmp/kgw-lane-env-check/`（`#[path]` 引入 env_check/mod.rs），`CARGO_TARGET_DIR=/tmp/kgw-target-env-check cargo test`。
`cargo check --features tauri-glue` 也通过（用 desktop 的 Cargo.lock 副本解析 tauri 2.11.1）。
测试里的 sqlite 全在 tempdir 里建，没有碰真实的 kiro-cli 数据库、`~/.zshrc` 或 App 数据目录。

## 5.1 Red / Green（`env_check::which`）

Red：只有测试、没有实现时 `cargo test` 报
`error[E0425]: cannot find function find_tool`（14 处）、`probe_shell_path`（7 处）、`default_dirs`（5 处）、`executable_names`（2 处）、
`error[E0422]: cannot find struct SearchInputs`、`error[E0433]: cannot find type PathCache` / `Platform`，lib test 编译失败。

Green：`27 passed; 0 failed`。四种情况对应的测试：
- 只在默认位置：`find_tool_only_in_default_location`
- 只在 PATH：`find_tool_only_in_path`、`find_tool_in_process_path_when_shell_path_unavailable`
- 都没有：`find_tool_nowhere_returns_none`
- shell 超时：`probe_shell_path_times_out_and_kills_the_shell`（300ms 超时，5 秒内返回 None）、`shell_timeout_still_finds_tool_in_default_location`（端到端）
- 其他边界：PATH 优先于默认位置、登录 shell PATH 优先于进程 PATH、相对路径 / 空项 / `.` 被忽略、同名目录被忽略、无执行位文件被忽略、claude 与 kiro-cli 不串、目录含空格、rc 文件打印横幅时取最后一行、`-lic` 参数顺序、shell 不存在、退出码非零无输出、缓存只探测一次且失败也缓存
- Windows 规则（纯函数，在 macOS 上跑）：`windows_default_dirs_under_localappdata_programs_and_userprofile`、`windows_default_dirs_skip_missing_env_dirs`、`windows_executable_names_cover_exe_cmd_bat`、`windows_find_tool_uses_exe_and_cmd_names_without_exec_bit`
- macOS 默认位置：`macos_default_dirs_are_local_bin_homebrew_and_usr_local`、`macos_default_dirs_without_home_keep_system_dirs`

## 5.2 Red / Green（`env_check::login`）

Red：`cannot find function probe_database`（30 处）、`database_candidates`、`find_database`、`sqlite_read_only_uri`、`open_read_only`，`cannot find type LoginInfo / LoginProbe / LoginMethod`、`cannot find value BUSY_TIMEOUT`，编译失败。

Green：`47 passed`（login 过滤）。任务要求的场景：
- 无库：`probe_missing_database_is_no_database`、`probe_database_path_that_is_a_directory_is_no_database`
- 无表：`probe_database_without_auth_kv_table_is_no_credentials`、`probe_empty_file_without_schema_is_no_credentials`
- 无 key：`probe_database_without_token_keys_is_no_credentials`
- 过期但有 refresh（通过）：`expired_with_refresh_token_is_valid`
- 过期无 refresh（失败）：`expired_without_refresh_token_is_invalid`、`expired_with_empty_or_null_refresh_token_is_invalid`
- value 不是 JSON：`probe_value_that_is_not_json_is_corrupt`、`probe_json_values_of_the_wrong_shape_are_corrupt`、`probe_null_value_is_corrupt`
- 另一个连接持有写事务：`probe_reads_while_another_connection_holds_a_write_transaction`（rollback journal，BEGIN IMMEDIATE）、`probe_reads_while_a_wal_mode_writer_holds_a_transaction`（WAL）
- 排他锁（BEGIN EXCLUSIVE）读不了时不挂死：`probe_gives_up_with_unreadable_when_the_writer_holds_an_exclusive_lock`
- 不泄露：`serialized_result_contains_no_token_material`（序列化成 JSON 并取 Debug，断言不含假 access / refresh / client secret 字符串，键集合恰为 expiresAt / method / refreshable）、`corrupt_and_unreadable_results_do_not_echo_the_raw_value`
- 只读：`probe_leaves_the_database_untouched`（字节不变、目录里不出现 -journal / -wal / -shm）、`probe_cannot_write_even_through_a_crafted_state`（DELETE 报 readonly）
- 优先级：`social_wins_over_odic_and_odic_wins_over_legacy`、`first_key_that_exists_decides_even_if_it_is_corrupt`（与 kiro/auth.py 的 SQLITE_TOKEN_KEYS 一致）
- expires_at：Z、`+08:00`、9 位纳秒（Z 和偏移两种）、无偏移按 UTC、乱码 / 缺失 / 非字符串为 None
- URI 与路径：空格、`%`、`?`、`#`、非 ASCII 的真实文件都能打开；`uri_escapes_*`、Windows 盘符 URI
- 候选路径：macOS 三个、Windows 两个（注释写明未经真机确认）、`find_database_*`

## 报告组合 Red / Green（`env_check::report`）

Red：`cannot find function build_report`（20 处）、`gateway_gate`、`login_command`、`display_path`、`kclaude_unavailable_reason`，`cannot find type EnvReport / CheckId / CheckStatus`，编译失败。

Green：首次运行 98 passed、1 failed（`every_probe_without_a_usable_row_means_not_logged_in`：Unreadable 时 detail 前面多一句「读不到 kiro-cli 的登录数据。」，测试期望完全相等，是测试写得过严）。把断言改成 `ends_with` 后全部通过。
最终：`99 passed; 0 failed`（which 27 + login 47 + report 25）。

覆盖：固定 4 项且顺序正确；全通过；没装 kiro-cli → 后两项 skipped；未登录（无库 / 无凭证 / 损坏 / 读不到四种探测结果）；过期有 refresh 通过；过期无 refresh 失败；fix 只在 fail 时非 null；Claude Code 未装独立失败；`gateway_gate` 三项任一非 ok 即 Blocked（含 skipped、残缺报告），Claude 状态不影响；JSON 形状与 ipc-contract 一致（camelCase，null 不省略）；`login_command` 路径含空格加引号（POSIX 单引号、含 `'` 转义、Windows 双引号）、用绝对路径不用 `~`；`display_path` 按路径分量缩写（`/Users/meow` 不被 `/Users/me` 吞）；报告 JSON 不含 token 相关字样。

## 需要加进 src-tauri/Cargo.toml 的依赖

- `rusqlite = { version = "=0.40.2", features = ["bundled"] }`（libsqlite3-sys 0.38.2，bundled 会编译 SQLite C 源码，增加编译时间和约 1 MB 体积）
- 已有且够用：`serde =1.0.228`（derive）、`serde_json =1.0.149`、`chrono =0.4.44`；dev-dependencies 的 `tempfile =3.27.0`
- 新增 feature：`tauri-glue = []`（commands.rs 用 `#[cfg(feature = "tauri-glue")]`，接入 main.rs 时需要在 Cargo.toml 声明并在 `generate_handler!` 里注册 `env_check::commands::env_check`）。参考项目 的 Cargo.lock 没有 rusqlite，0.40.2 是 crates.io 当前最新版，未经 参考项目 验证。

## 未验证

- Windows 的默认安装位置和 kiro-cli 数据库位置没有真机确认（代码注释已写明）；只验证了规则的纯函数。
- `CLAUDE_CODE_INSTALL_URL`（`https://docs.claude.com/en/docs/claude-code/setup`）本会话不能联网，没有打开确认；kiro-cli 安装页 `https://kiro.dev/cli/` 取自仓库 README。
- `clippy` 未安装，没跑。
- 没用真实 kiro-cli 数据库跑（按要求）。
