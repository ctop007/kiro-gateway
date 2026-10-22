# fix-C-kclaude-frontend

组 C：kclaude 脚本、env/view、请求日志、升级遮罩、样式、useAppState。每项先写测试看到失败，再修。

## 最终结果

- `cd desktop/src-tauri && cargo test`：777 passed, 0 failed
- `cd desktop && pnpm test`：14 files, 328 passed（修前 255）
- `cd desktop && pnpm build`：tsc + vite build 通过
- 没有跑 `--selftest`（需要 `make sidecar`）。install 仍强制跑登录 shell 检查，行为不变。

## 1 [高] 健康检查走代理

- Red：新增 4 个 script_tests 场景，`cargo test script_tests` 4 failed。
  - `health_probe_ignores_user_http_proxy_variables`：`http_proxy=https_proxy=http://127.0.0.1:9`，网关在 81xx 运行，zsh 报「KiroGateway 未运行」。
  - `claude_gets_loopback_in_no_proxy_without_dropping_the_user_value`、`no_proxy_is_set_when_the_user_had_none_and_does_not_leak_into_the_shell`：假 claude 看到的 NO_PROXY 不含 127.0.0.1。
  - `powershell_script_probes_without_a_proxy_and_extends_no_proxy`：模板仍是 Invoke-RestMethod。
- Green：`templates/kclaude.zsh`、`kclaude.bash`：curl 加 `--noproxy '*'`；子 shell 里 `NO_PROXY` / `no_proxy` 追加 `127.0.0.1,localhost`（保留用户原值，子 shell 内 export，不泄漏）。`kclaude.ps1`：探测改用 `System.Net.Http.HttpClient` + `HttpClientHandler.UseProxy = $false`（PS5 / PS7 都不走代理），启动 claude 前把 `NO_PROXY` / `no_proxy` 追加 `127.0.0.1,localhost`，finally 里还原。
- claude 是否走代理：Claude Code 读 `HTTP(S)_PROXY`，所以必须靠 NO_PROXY 绕开本机网关，已追加。
- PowerShell 部分只有静态断言（本机无 pwsh），未实际执行。

## 11 [高] kclaude_status 每次跑登录 shell

- Red：新增 service 测试，8 failed（连续两次 status 重复启动 shell、重启后缓存、rc 变化、损坏缓存等）。
- Green：`service.rs` 新增 `kclaude-verify.json`（App 数据目录），内含指纹（shell、各 rc 文件的 mtime 纳秒 + 大小）与上次 `KclaudeVerify`。`status()` 指纹相同就读缓存，不同才重测；`install` / `adopt` / `remove` 强制重测并写缓存。`types.rs` 给 `KclaudeVerify` 等加了 `Deserialize`。缓存损坏等同没有缓存。
- 已有测试 `status_of_an_installed_setup_runs_verify...` 改为先删缓存再断言。

## 前端（2–10）

Red：`pnpm test` 39 failed（含 race-guard 模块不存在）。Green：328 passed。

- 2：`env/view.ts` `isAbsoluteCommand` 同时剥掉开头的单、双引号。测试含空格、含 `'\''` 撇号路径、`'~/..'` 与 `'kiro-cli'` 仍判非绝对。
- 3：`UNKNOWN_MODEL_LABEL = '未知模型'` 放在 `ipc-types.ts`，前端 `UNKNOWN_MODEL` 取自它；筛选与下拉用 `modelLabel()`，null 和空串都归「未知模型」（与 Rust `model_label` 一致）。`request_log/model.rs` 未改（选择以后端为准）。测试覆盖筛选、下拉只剩项、选后端那项能筛出 null 记录。
- 4：`CONTEXT_PATTERN` 加 `context limit` 与 `exceeds model capacity`。已核对 `kiro/kiro_errors.py` 原文 "Model context limit reached. Conversation size exceeds model capacity."，以及 Anthropic 路径加前缀的 "prompt is too long: ..."。另测「Monthly request limit exceeded」不会被误判。
- 5：遮罩与设置页直接显示 Rust 的 error，不再加前缀。三种情况（下载失败、更新安装失败、具体原因）有测试。
- 6：记录进入列表时分配单调序号 `seq`（`indexSnapshot` 用于整份加载，`mergeAppended` 用于追加）；无 id 的 key 为 `no-id|ts|seq`。去重改为按记录签名计数，同一毫秒的两条无 id 记录都保留，与快照重叠的仍丢弃。
- 7：`upgradeRequired` 为真但没有 forced 状态时，遮罩显示「正在检查新版本」并给「重试」。`update_retry` 在非 forced / install_failed 时 Rust 会拒绝，所以新增 `retryCommandFor()`：forced / install_failed 用 `update_retry`，其他状态用 `update_check_now`。每次收到 update-status（以及 check / retry 返回后）都重新 `config_get`，不再由前端把 `upgradeRequired` 置为 `state === 'forced'`。无 manual_url 时只有重试。
- 8：`--faint` 浅色 #8d95a3 → #5f6877，深色 #6c7584 → #8a93a3；`--warn` 浅色 #a2650b → #935a07。新增 `src/styles.test.ts`，按 WCAG 公式读 styles.css，断言 faint 与 warn 在 window / panel / paper（warn 另含 warn-soft）、浅色、深色、系统深色三块都 ≥4.5。`--muted` 也一并断言（已达标）。
- 9：`AUTOSTART_DESCRIPTION` 常量，设置页使用。
- 10：新增 `state/race-guard.ts`（`createRaceGuard`）：请求发起时取计数，event 到达后丢弃更早发起的请求结果；gateway status（初始、visibilitychange、gateway_start / stop）与 config 都走它。8 个单测覆盖慢响应晚于 event、后发先至等。hook 本身没有 UI 自动化测试（项目约定不写）。

## 未改但需要注意

- `docs/ipc-contract.md` 里 `kclaude_status` 的「实测结果」现在是缓存值，指纹变化才重测；该文件不在本组可改范围，没有改。
- `docs/mockup/index.html` 的开机自启文案仍是旧文案，同样不在可改范围。
