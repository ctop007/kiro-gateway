# 批次 request-log：7.1、7.2、7.3

日期：2026-10-22。只改了 `desktop/src-tauri/src/request_log/`。tasks.md 勾选、main.rs、Cargo.toml、Cargo.lock 没动。

验证方式：独立测试 crate `/tmp/kgw-lane-request-log/`，用 `#[path]` 引入 `request_log/mod.rs`，`CARGO_TARGET_DIR=/tmp/kgw-target-request-log`。
- `cargo test`：78 passed（不开 feature）。
- `cargo test --features tauri-glue`：81 passed（多 3 个 commands.rs 的测试），tauri 2.11.1 编译通过。
- 联调后在 `src-tauri/src/main.rs` 里 `mod request_log;` 即可，测试路径会变成 `request_log::reader` / `request_log::agg` / `request_log::retention`，与 tasks 的验证命令一致。
- 数据形态取自 `kiro/request_log.py::_serialize`：测试里用 `2026-10-22T20:51:03.713+08:00`、`credits 0.006173584610281925`、`duration_ms 5894`、`id msg_01`。没有读真实日志目录，没有碰 8000 端口的网关。

## 模块

- `model.rs`：`RequestRecord`（camelCase）、`parse_line`、`YearMonth`（文件名往返）、`RequestLogSnapshot` / `RequestLogAppended` / `UsageSummary`。
- `reader.rs`（7.1）：`LogReader`，每个月份文件一个游标（偏移、inode、前 64 字节指纹）。
- `agg.rs`（7.2）：`summarize`（时区注入）、`recent`（倒序，最多 N 条）、`RecordFilter`。
- `retention.rs`（7.3）：`prune_old_months`、`clear_all`。
- `store.rs`：`LogStore`，把三者接起来；`open` / `tick` / `snapshot` / `clear`，当前时间作为参数传入。
- `commands.rs`（`tauri-glue`）：`request_log_snapshot`、`request_log_clear`、1 秒轮询线程；只做转发。

## 7.1 增量读取（reader）

Red：先只写了测试模块，`cargo test reader` 报 `error[E0433]: cannot find type LogReader in this scope`，5 处，`could not compile`。
Green：实现后 `cargo test reader`：`24 passed; 0 failed`。

覆盖的用例：
- 首次读全部完整行、顺序正确：`reads_every_complete_line_of_a_new_file_in_order`
- 追加只返回新行、没有新数据返回空：`second_poll_returns_only_appended_lines`、`poll_without_new_data_returns_nothing`
- 半行：`half_written_last_line_waits_for_its_newline`（半行不返回、不计入坏行，补齐并换行后才读到）、`half_line_ending_inside_a_multibyte_character_is_reassembled`、`unterminated_final_line_is_never_returned`
- 坏行：`bad_lines_are_skipped_and_counted_while_good_lines_survive`（非 JSON、缺字段、NUL 行）、`invalid_utf8_line_is_skipped`、`blank_lines_and_crlf_endings_are_tolerated`（空行不算坏行）
- 截断 / 重建：`truncated_file_is_read_again_from_the_start_and_flagged`、`file_emptied_to_zero_bytes_resets_with_no_records`、`rewritten_in_place_to_a_longer_file_is_detected_by_its_head`（同 inode、变长，靠前 64 字节指纹发现）、`file_replaced_by_a_new_inode_is_read_again_from_the_start`、`deleted_file_resets_and_a_recreated_file_is_read_from_the_start`
- 跨月：`months_are_tracked_independently`、`a_month_file_that_appears_later_is_read_from_its_first_line`
- 其他：文件和目录不存在、目录占了文件名（返回错误且不动游标）、`forget`、`reset_all`、2 万行一次读完。

store 层的启动加载与跨月：`open_loads_current_and_previous_month_and_deletes_older_files`、`crossing_into_a_new_month_resets_the_month_and_prunes_the_oldest_file`、`a_late_line_written_to_last_months_file_after_rollover_is_still_picked_up`、`first_request_of_a_new_month_is_read_from_the_new_file`。

变异检查：把 `tick` 里轮询范围从「上月 + 本月」改成只轮询本月，`open_loads_current_and_previous_month...` 和 `a_late_line_written...` 两个测试失败，说明它们真的守着这个行为；已还原。

## 7.2 聚合（agg）

Red：只写测试时 `cargo test agg`：`cannot find function summarize` 19 处、`recent` 12 处、`RecordFilter` 12 处，`could not compile`。
Green：实现后第一次跑 19 passed、1 failed：`by_model_groups_sorted_by_credits_descending...`。原因是我的测试数据让 haiku（0.1+0.1）和 sonnet（0.2）并列，排序规则（credits 降序，再请求数降序）把 haiku 排前面，是测试数据写错，把 sonnet 改成 0.25 后 `20 passed; 0 failed`。

覆盖的用例（固定时区 +08:00，另有 -05:00 和 UTC）：
- 23:59:00 与 00:00:00 分属两天，月合计不变：`one_minute_before_and_after_local_midnight_land_on_different_days`
- 月末 23:59:59 与下月 00:00:00：`last_day_of_month_rolls_over_into_a_fresh_month_total`；年末：`year_end_rolls_over_to_january`
- 不同偏移的记录按注入时区归日：`records_written_in_another_offset_are_bucketed_in_the_injected_timezone`
- 失败 / 拒答 / 断开的 credits 计入：`failed_refused_and_disconnected_requests_still_count_their_credits`；0 credits 的请求仍计请求数
- 按模型分组之和等于合计（逐位相等，不是近似）：`by_model_groups_sorted_by_credits_descending_and_sum_to_the_month_total`、`group_sum_equals_total_for_awkward_floats`（500 条非整数 credits）。合计就是按分组顺序求和，所以一定相等。
- null 和空字符串模型归「未知模型」；并列时按请求数、名称排序，输出稳定；上月记录不进本月分组
- 最近 N 条倒序：`recent_returns_newest_first_and_at_most_limit`、按时刻而不是文本排序、时刻相同时后到的在前
- 筛选：仅失败（非 ok 全部算）、指定模型（含「未知模型」）、两者叠加；`filtering_the_list_never_changes_the_summary`

## 7.3 保留与清空（retention）

Red：只写测试时 `cannot find function prune_old_months` 8 处、`clear_all` 4 处，`could not compile`。
Green：实现后 `cargo test retention`：`8 passed; 0 failed`（含只读目录下删除失败的两个测试，root 下自动跳过）。

覆盖：保留本月和上月、删更早的；1 月保留上年 12 月；未来月份、`.bak`、月份 13、非文件（目录）一律不动；目录不存在 / 为空是空操作；删不掉的文件记入 `failed` 且不影响其他文件；`clear_all` 只删月份文件、不动 `gateway.log` 等；store 层 `clear_removes_all_files_resets_the_view_and_later_requests_are_recorded`（清空后新写入的第一行不丢）和 `clear_failure_keeps_showing_what_is_still_on_disk`。

## 没验证到的

- 轮询线程和两个 command 只做了编译检查（`--features tauri-glue`）和三个纯函数测试，没有在真实窗口里跑。
- Windows 路径：`file_identity` 的非 unix 分支用创建时间，没有在 Windows 上编译或运行。
- 没装 clippy，没跑。
