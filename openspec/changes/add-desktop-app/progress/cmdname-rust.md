# 批次 cmdname-rust：10.1（Rust 侧）

日期：2026-10-23。只改了 `desktop/src-tauri/src/**`、`desktop/src/lib/ipc-types.ts`（只加 `commandName` 三处）、`docs/internal/local-setup.md`、`desktop/AGENTS.md`。没碰 tasks.md 勾选、`.env`、`credentials.json`、`state.json`、真实 `~/.zshrc` 与真实 App 数据目录；没有 git add / commit / push。`scaffold_tests.rs`、`tray/controller.rs`、`icons/` 的改动是并行那一条线的，不是本批的。

最终结果：`cargo test` 869 全绿（上批 793 + 本批新增），`cargo build` 0 警告，`--selftest` 52 项全过（原 41 项 + 11 项改名与默认名）。

## 做了什么

- **默认值**：`DEFAULT_PORT` 8000 → 6868；`AppConfig.commandName` 默认 `cclaude`。旧 `config.json` 缺 `commandName` 走 serde 默认值，不当成损坏；已保存的 `port: 8000` 原样保留（有测试）。
- **`app_config/command_name.rs`（新）**：名字规则 `^[A-Za-z][A-Za-z0-9_-]{1,31}$` 与保留名清单（要求的 23 个 + shell 内建 / 关键字 / 常用工具，大小写不敏感），`NameProblem` 三种原因对应三条中文文案。
- **`kclaude.env`** 末尾加 `KIRO_GATEWAY_COMMAND=<名字>`（脚本不依赖它，函数名和报错前缀在渲染时写死；真 sh / bash / zsh 读回验证）。
- **模板**：`templates/kclaude.*` 改名为 `command.*`，占位符 `@@KCLAUDE_COMMAND@@`（函数名、注释、报错前缀）。`script::render` 对不合规的名字回退成默认名，不会把任意字符拼进脚本。文件名 `<名字>.zsh|bash|ps1`，`script::remove_all(名字)` 只删这三个文件（名字先过格式校验，`../x` 之类删不到别处）。
- **`rc.rs`**：新标记段 `# >>> kiro-gateway-desktop command >>>`；旧 `... kclaude >>>` 段识别为本 App 的旧段（状态 `Outdated`，安装时就地替换为新段，写前备份）；新旧标记混搭判为 `Damaged`。foreign 检测按当前命令名，`source` 行改成「整词匹配」（`kclaude-test.zsh` 不算 `kclaude`，`work/scripts/kclaude.zsh` 算 `kclaude`）。
- **`service.rs`**：`KclaudeEnv.command`；`status` 带 `commandName`；`verify` 用 `type <名字>`（`verify::login_type_args`），提示文案带名字；verify 缓存指纹加入 `cmd=<名字>`；`name_problem`（格式 → 保留名 → PATH 同名可执行 → 启动文件同名定义）；`rename_from(old)`：旧名处于 foreign / 标记残缺 / 用户已移除时只保存不写，否则先写新脚本、替换标记段、最后删旧脚本。旧版 `kclaude.*` 脚本在标记迁移后删除（命令名仍为 `kclaude` 时保留）。
- **`kclaude/rename.rs`（新）**：`check_name` 与 `apply_update`。先校验（失败不写盘），再 `ConfigStore::update`，再重写；重写失败则把 config 回滚到旧名并清掉新名脚本，返回 `write_failed`。
- **`config_update`**：带 `commandName` 时在自动启动开关之前先做占用检查（被拒的补丁什么都不改），通过后走 `apply_update`；`#[tauri::command(async)]` 避免登录 shell 探测阻塞主线程。`env_and_backend` 先刷新登录 shell PATH 缓存。
- **`KclaudeStatus.commandName`**；`ipc-types.ts` 的 `AppConfig` / `ConfigPatch` / `KclaudeStatus` 各加 `commandName`；`contract_tests` 对照更新（含 ConfigPatch 双向键对照、默认值 6868 / cclaude）。
- **selftest**：默认名 cclaude、`cclaude.zsh` 存在且 `kclaude.zsh` 不存在、改名为 `kclaude-test`（新名是函数、旧名消失、一段标记、旧脚本删除、报错前缀）、改名为 `claude` 被拒；端口仍用 8100–8199，不碰 6868。
- 文档：`docs/internal/local-setup.md`（默认端口、命令名、新标记段、与手动 kclaude 共存、旧段迁移）、`desktop/AGENTS.md`（写盘范围、子进程规则、selftest 说明）。

## Red / Green

先写测试，再实现；每一块都先看到编译失败或断言失败。

- app_config：先写 `model/tests.rs`、`store/tests.rs`、`env_file/tests.rs`、`command_name/tests.rs`。Red：`cargo test app_config` 因 `ConfigPatch` 缺 `command_name` 编译失败（E0063）；实现后 Green：89 个 app_config 测试全过。
- kclaude：先写 `rc/tests.rs`（新增 11 个）、`verify/tests.rs`（新增 4 个）、`script.rs` 内联测试、`script_tests.rs`（新增 5 个）。Red：`cargo build` 在 `selftest.rs`、`contract_tests.rs` 报 `make_env` / `KclaudeStatus.command_name` / `skipped()` 参数缺失（E0061 / E0063）。Green：`cargo test kclaude::` 219 全过。一处旧测试 `file_without_trailing_newline_...` 因断言里写死旧标记失败，改成新标记。
- `rename/tests.rs`（28 个）一次性写好后全绿，没有先红后绿，用变异检查补强（改完已全部还原）：
  - 改名后不删旧脚本 → 3 个失败（来回改名、真 zsh / bash 改名、Windows 改名）。
  - 来源 / 别名匹配从整词换成子串 → `a_name_that_merely_contains_another_name_is_not_a_match` 失败。
  - 不认旧标记 → 12 个失败（rc 7 个、rename 5 个）。

## 测试覆盖对照（任务要求）

- 改名：临时 HOME 下真 zsh 与真 bash，新名 `type` 是函数、旧名 `type` 找不到、rc 里一段标记、有备份且备份里是旧段、旧脚本删除、`status.verify` 为 ok、报错前缀是新名（`rename_with_real_zsh_and_bash_...`、`the_renamed_command_prints_its_own_name_...`）。
- 非法名不写盘：15 种非法名（保留名含大小写、数字开头、空格、空串、单字符、注入字符、中文、33 个字符），逐个断言 rc / config.json / kclaude.env / 脚本 / 备份全部不变（`invalid_names_are_refused_...`）。
- PATH 占用：`a_name_that_is_an_executable_on_the_path_is_taken`（无可执行位的文件和相对路径目录不算占用）。
- rc 同名 alias / function / `function` 关键字 / source 占用：`a_name_defined_in_the_startup_file_is_taken_...`（5 种写法）；注释行和相似名不占用。
- 旧 kclaude 标记段迁移：就地替换、文件末尾无换行、新旧段并存合并、移除、bash / zsh 真 shell 下 `kclaude` 消失 `cclaude` 可用、`auto_install` 启动迁移、命令名仍为 `kclaude` 时保留脚本。
- **用户手动 `source .../scripts/kclaude.zsh` 共存**：`manual_source_of_scripts_kclaude_zsh_does_not_block_cclaude_and_stays_untouched`（zsh 与 bash）——状态是 `not_installed` 而非 `foreign`，`cclaude` 正常安装，rc 里旧行逐字节在原位、没有 `disabled by` 前缀，zsh 下 `cclaude` 与 `kclaude` 同时是函数。反向：App 命令名设成 `kclaude` 而手动那行还在 → 「这个名字已经被占用」。
- 旧 config 无 `commandName`：`old_config_json_without_command_name_is_not_corrupt_and_keeps_the_users_port`（不产生损坏备份，port 8000 保留，env 文件补 `KIRO_GATEWAY_PORT=8000` 与 `KIRO_GATEWAY_COMMAND=cclaude`）。
- contract_tests、selftest：见上。
- 另有：改名回滚（`only 备份失败` → config 与 env 回到旧名，旧脚本保留，新脚本清掉）、foreign / 用户已移除 / 标记残缺时只保存、端口与名字同补丁、同名不写盘、Windows 两个 profile 同步改写并换 `.ps1`、verify 缓存随名字失效。

## 没做 / 没验证

- **PowerShell 未实机运行**：`command.ps1` 与 Windows 两个 profile 的改名只有 Rust 侧文件级测试（和上批一样，本机没有 pwsh）。`cargo check --target x86_64-pc-windows-msvc` 本机跑不了（`ring` 需要 MSVC 的 C 头文件，报 `assert.h` not found），Windows 专属分支（`documents_dir`、注册表）本批没改，但没有重新交叉编译验证。
- **保留名清单是人定的**：要求的 23 个全在，另外加了常见 shell 内建和工具；清单外的系统命令靠 PATH 占用检查兜底。PATH 检查用登录 shell 的 PATH，GUI 启动时短 PATH 会先被 `refresh_search_dirs` 补上；别名 / 函数只检查目标启动文件，不检查 `.zprofile` 等其他文件。
- 前端设置页的改名输入、错误展示不在本批（前端线）。`config_update` 在改名时会跑登录 shell 验证（最长 5 秒），前端需要在提交时显示等待状态。
- 手动网关文档 `scripts/kclaude.zsh` 没改（AGENTS.md 规定不改）。
