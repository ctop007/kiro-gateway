# 批次 kclaude：6.1、6.2、6.3、6.4

日期：2026-10-22。只改了 `desktop/src-tauri/src/kclaude/`，没动 `main.rs`、`Cargo.toml`、`Cargo.lock`，tasks.md 勾选没改。

测试驱动方式：临时 crate `/tmp/kgw-lane-kclaude/`（`#[path]` 引入 `kclaude/mod.rs`，不开 `tauri-glue`），`CARGO_TARGET_DIR=/tmp/kgw-target-kclaude`。`commands.rs` 另用 `/tmp/kgw-lane-kclaude-glue/` 开 `tauri-glue` 做过 `cargo check`。Windows 代码用 `cargo check --target x86_64-pc-windows-msvc` 编译通过（`documents_dir`、注册表读取只在那里编译）。

最终结果：`cargo test kclaude::` 150 个测试全绿，连续跑 4 次稳定（约 10 秒，脚本测试占大头）。

## 模块布局

- `templates/kclaude.{zsh,bash,ps1}`：脚本模板，`include_str!`，占位符 `@@KCLAUDE_DATA_DIR@@` 替换成转义后的数据目录绝对路径。
- `script.rs`：渲染与安装（内容不变不写盘）；`.ps1` 写 UTF-8 BOM。
- `rc.rs`：标记段分析、`plan_install / plan_adopt / plan_remove`（纯字节函数）、`choose_rc_file`、`apply`（备份后写）。
- `ps_profile.rs`：PS5 / PS7 `CurrentUserAllHosts` 路径、执行策略判定与提示、`documents_dir`（KnownFolder，cfg(windows)）、注册表读策略（cfg(windows)）。
- `verify.rs`：`$SHELL -lic 'type kclaude'`，5 秒超时，超时杀整个进程组。
- `service.rs`：`KclaudeService`（status / install / adopt / remove / auto_install），时钟、验证、claude 查找通过 `Backend` 注入。
- `commands.rs`：`#[cfg(feature = "tauri-glue")]` 的 4 个 command 和 `auto_install(app)`。
- `types.rs`：`KclaudeStatus` 等，serde 形状照 contract。
- `fsutil.rs`：原子写（跟随符号链接、保留权限）、不覆盖旧备份的备份名。

## 6.1 脚本模板

测试：`kclaude::script_tests`（17 个，真实 `/bin/zsh -c` 与 `/bin/bash -c`，临时 HOME，数据目录含 "Application Support"，PATH 前置假 claude，`python3 -m http.server` 在 8100-8199 当 `/health`）和 `kclaude::script` 单测（8 个）。

覆盖：env 缺失；env 缺端口 / 缺密钥 / 端口非数字；网关未起（非零、不调 claude、端口上没有新进程）；端口上是别的服务（`/health` 返回 404 也算未运行）；网关起来时 `ANTHROPIC_BASE_URL / AUTH_TOKEN / OPUS / SONNET / HAIKU` 与参数透传；未加引号和双引号的 env 值；模型字段缺失用默认值；已打开的 shell 里改端口后下一次立即生效；端口改到没人监听时报未运行；`UPGRADE_REQUIRED=1` 时即使网关在也非零且不调 claude，且优先于「未运行」提示；继承的 `KIRO_GATEWAY_*` 变量不能覆盖 env 文件；kclaude 不污染调用方 shell，之后直接 `claude` 看不到网关变量；claude 退出码透传；没装 claude 的提示；路径含单引号、`$`、反引号；zsh 开了 `nounset ksharrays shwordsplit` 且 IFS 被改时仍可用。

Red：模板和 `script.rs` 先于行为测试写好，行为测试一次性写成。第一次运行 16 过 1 败（`stale_port_after_the_gateway_moved_reports_not_running`，`zsh:2: no matches found: KIRO_GATEWAY_PORT=...[1m]`），原因是测试自己拼 shell 命令时没转义 `[1m]`，修的是测试。因此 6.1 没有严格的先红后绿，用变异检查补强：
- 把两个模板里的 `/health` 探测改成 `if false` → `gateway_not_running_...`、`non_gateway_service_...`、`stale_port_...` 3 个失败。
- 把 `UPGRADE_REQUIRED` 判断条件改成 `"9"` → `upgrade_required_blocks_...`、`upgrade_required_wins_...` 2 个失败。
改回后全绿。

`kclaude.ps1` 没有跑过：本机没有 pwsh。只有 Rust 侧的渲染测试（引号、BOM、三条提示文案、无残留占位符）。需要在 Windows 真机上跑一遍（见 not_done）。

## 6.2 启动文件管理

测试：`cargo test kclaude::rc`（51 个）。

覆盖：文件不存在；已有内容只追加；无结尾换行的文件（追加后移除字节还原）；CRLF；非 UTF-8 字节原样保留；已装最新不写；标记段在文件中间；outdated（路径变了 / 正文被改）原位更新；重复标记段合并；标记残缺 / 嵌套判为 damaged 且三种操作都拒绝；foreign 的十二种写法（`source`、`.`、`[ -f ] && source`、`if...then source; fi`、`alias`、`alias -g`、`kclaude() {}`、`function kclaude`、多行函数、缩进）；注释、`kclaude_helper`、`alias kclaudex` 等不误判；foreign 优先于已有标记段；函数体里字符串和注释里的花括号不影响边界；括号不配平的函数拒绝接管；接管（旧行加 `# disabled by Kiro Gateway App: ` 前缀再加标记段，多行函数逐行；缩进行用 `: ` 保持 `if` 体合法；保留 CRLF 和无结尾换行）；install→remove 对 12 种原文逐字还原；remove 在有 foreign 时只删自己的段；`$SHELL` 映射；bash 选 `.bashrc` / `.bash_profile`（`.bash_profile` 存在且不 source `.bashrc` → 用它；source 了 → `.bashrc`；都没有时 macOS 用 `.bash_profile`，其他用 `.bashrc`）；备份内容与原文件一致、备份名不覆盖同秒旧备份（加 `-1`）、只读目录里备份失败不写（返回 `backup_failed`，原文件不变，消息不含路径）、文件不存在不备份。

`cargo test kclaude::service`（32 个）覆盖 tasks 6.2 的端到端场景：首次安装（脚本 + 启动文件）、已有内容备份、重复安装不写盘（mtime 不变）、脚本过期只重写脚本不动启动文件、数据目录搬家后更新块并备份、foreign 拒绝且不写脚本、接管只产生一个备份、接管备份失败不改动、残缺标记、移除后逐字节还原、bash / zsh / 未知 shell、`auto_install`（装 / 跳过 foreign / 用户移除后不再自动装回，显式安装清除标记）、Windows 双 profile 全部场景。

Red：`rc.rs` 先于 `rc/tests.rs` 写（测试后补），首次运行 51 个里 49 过 2 败：
- `remove_is_the_exact_inverse_of_install_for_many_shapes`：原文 `a\r\nb`（CRLF 且无结尾换行）安装再移除多出一个 `\r`（left `[97,13,10,98,13]` right `[97,13,10,98]`）。修 `removal_range`。
- `adopt_comments_out_every_line_of_a_multiline_function_and_alias`：期望值把缩进写错（测试问题），按实现的行为（缩进在前缀之前）改了期望。
`service` 的 `adopt_backup_failure_changes_nothing` 首次失败（`WriteFailed` ≠ `BackupFailed`），原因是测试没先建数据目录，脚本写入先失败；修测试。
变异检查：把 `removal_range` 的无结尾换行分支关掉 → 2 个失败；让 `apply` 在备份失败时继续写 → 2 个失败（`apply_does_not_write_when_the_backup_cannot_be_created`、`apply_with_an_unreadable_original_...`）。

## 6.3 PowerShell profile

测试：`cargo test kclaude::ps_profile`（12 个）与 service 里的 Windows 用例（7 个）。

覆盖：注入 Documents 后 PS5 / PS7 路径；OneDrive / 空格 / 中文目录；文件名是 `profile.ps1`（不是 `Microsoft.PowerShell_profile.ps1`）；策略解析大小写；只有 Restricted / AllSigned 挡 profile；四个来源的优先级（机器组策略 > 用户组策略 > 当前用户 > 本机）；Undefined 穿透；都没设按 Restricted；提示给出可复制命令 `Set-ExecutionPolicy -Scope CurrentUser -ExecutionPolicy RemoteSigned` 且 App 不执行；组策略限制时不给命令（给了也无效）；UTF-16 profile 拒绝改写；两个 profile 全有或全无（一个 foreign / UTF-16 则两个都不写）；一个 profile 缺块判 outdated 并可修复；缺 Documents 报错而不是猜路径。

Red：`ps_profile/tests.rs` 先写，首次编译 3 个 `unresolved import ExecutionPolicy`（模块是空壳）；实现后 12 个全过。

`documents_dir`（`SHGetKnownFolderPath`）、`read_policy_sources`（注册表）在 `#[cfg(windows)]`，只通过 `cargo check --target x86_64-pc-windows-msvc` 验证能编译，没有在 Windows 上跑过。

## 6.4 安装后实测

测试：`cargo test kclaude::verify`（22 个）。

覆盖：zsh / bash 的 `type` 输出解析（函数 ok；alias 盖住、同名可执行文件、找不到各自 fail 并给固定文案；启动噪音行被忽略）；超时与启动失败的文案；detail 里不回显 shell 输出（含假 token 的 alias 输出不泄露）；假 SHELL 脚本：成功（并断言收到参数 `-lic type kclaude` 且 `HOME` 是临时目录）、失败；挂住的 shell 在超时后被杀，后台子进程也被杀（进程组）；大量 stdout 不死锁；真实 `/bin/zsh -lic`、`/bin/bash -lic` 在临时 HOME 里：写了块 → ok，没写 → fail，后面又被 alias 盖住 → fail 且提示别名；PowerShell 输出解析（Function ok，找不到 fail，策略问题带 `copy_command`）。

Red：`verify/tests.rs` 先写，首次编译 31 个错误（`verify_posix`、`run_shell_with_env` 等不存在）。实现后首轮 18 过 4 败：
- `real_zsh_in_temp_home_*`、`real_zsh_where_a_later_line_*`：测试把 `ZDOTDIR` 设成空串，zsh 把空串当作已设置，找不到 `.zshrc`；改成删除该变量（`run_shell_with_env` 的参数改成 `Option<&str>`，`None` 为移除）。这也是产品行为：验证时必须去掉继承来的 `ZDOTDIR`，否则会检查用户真实的 zsh 配置目录。
- `shell_that_floods_stdout_does_not_deadlock`：实现只保留了前 256 KB，`type` 的答案在输出末尾被丢掉；改成只保留尾部。
- `hanging_shell_is_killed_...`：脚本在负载下 3 秒内没来得及启动，改成先 `--warm` 预热一次再计时。

## 实现里的取舍

- 脚本里 `kclaude` 在子 shell `( ... )` 里 source env，不改调用方环境；zsh 版加 `emulate -L zsh`。
- kclaude.env 当作 shell 文件 source（按任务给的字段）。含空格或特殊字符的值需要 app_config 写成单引号形式，见 contract_changes。
- 检测到 foreign 且标记段损坏时，`status` 报 `foreign`（contract 没有 damaged 状态），install 返回 `foreign_present`，adopt / remove 返回 `write_failed` 并提示删残缺标记行。
- `kclaude_remove` 写 `kclaude.removed` 标记文件，App 下次启动的自动安装不再装回；显式安装清除它。
- 备份文件名同秒重复时加 `-1`、`-2`，不覆盖旧备份。
- 启动文件是符号链接（dotfiles 管理器常见）时写入目标文件，链接保留。
