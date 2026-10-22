# Tasks

## 1. 网关：结构化请求日志（gateway/request-log）

- [x] 1.1 在 `kiro/config.py` 新增 `REQUEST_LOG_DIR`（默认空），同步 `.env.example` 和 README 配置表；验证：`tests/unit/test_config.py::test_request_log_dir_defaults_to_empty`、`test_request_log_dir_reads_env` 通过
- [x] 1.2 新建 `kiro/request_log.py`（`RequestRecord`、`finish()`、按月文件名、线程锁、每次 open-append-close、`OSError` 只告警一次、error 截断 200 字符）；验证：新建 `tests/unit/test_request_log.py`，覆盖未配置不建文件、自动建目录、按月分文件（月末 23:59 与次月 00:00）、文件被删后重建、只写一次（重复 finish）、无写权限不抛异常且只告警一次、20 线程并发每行可解析、记录不含 content/headers 字段，全部通过
- [x] 1.3 OpenAI 流式层透传 `request_record`：`stream_kiro_to_openai_internal` 在 `[Credit]` 处写 credits 与 completion_id、在 `is_refusal` 处标 refused；`collect_stream_response` 透传；验证：`tests/unit/test_streaming_openai.py` 新增 `test_request_record_credits_match_credit_log`、`test_request_record_marks_refusal`、`test_request_record_optional_param_keeps_behavior`，通过
- [x] 1.4 Anthropic 流式层透传 `request_record`（`streaming_anthropic.py` 549 行流式、798 行非流式两处，以及 refusal 分支）；验证：`tests/unit/test_streaming_anthropic.py` 新增同名三组测试（流式、非流式各一套），通过
- [x] 1.5 `routes_openai.py` 接入：入口建 record，流式 `stream_wrapper.finally`、非流式返回、FATAL 上游错误、`HTTPException`、兜底 `Exception`、全部账号失败的最终返回，账号系统与 legacy 两个分支都写；外层 `try/finally` 兜底补写并打 debug；验证：`tests/unit/test_routes_openai.py` 新增 `TestRequestLog`，覆盖流式成功、非流式成功、上游 402、网络错误 502、流式中途出错（credits 为出错前累计值）、客户端断开、兜底从不触发，通过
- [x] 1.6 `routes_anthropic.py` 接入，范围同 1.5；验证：`tests/unit/test_routes_anthropic.py` 新增 `TestRequestLog`，用例同 1.5，通过
- [x] 1.7 `validation_exception_handler` 对 `LOGGED_ENDPOINTS` 写 422 记录，`/v1/models`、`/health`、`count_tokens` 不写；验证：`tests/unit/test_exceptions.py::test_validation_error_writes_request_record`、`tests/unit/test_routes_openai.py::test_models_endpoint_not_logged`、`tests/unit/test_routes_anthropic.py::test_count_tokens_not_logged` 通过
- [x] 1.8 四路径一致性：同一组 mock 上游事件分别走 OpenAI/Anthropic × 流式/非流式，断言记录除 `api`/`stream`/`id`/`ts`/`duration_ms` 外完全相同；验证：`tests/integration/test_full_flow.py::test_request_log_consistent_across_four_paths` 通过
- [x] 1.9 全量回归与真机冒烟：`.venv/bin/pytest -q` 全绿；在 8100 端口带 `REQUEST_LOG_DIR=/tmp/kgw-reqlog` 起一个临时实例，用 haiku 各打一次 OpenAI 和 Anthropic 的流式与非流式请求，确认写出 4 行且 credits 与 `[Credit]` 日志一致，然后关掉这个实例
- [x] 1.10 README「配置说明」和「与上游的差异 › 自有修复」补上请求日志；验证：文档里写的配置名与代码一致（`grep REQUEST_LOG_DIR`）

## 2. 桌面端工程骨架

- [x] 2.1 确认 Rust 工具链（本机已有 `~/.cargo/bin`，1.99，未在 PATH；补 `x86_64-apple-darwin` target 以便出 universal 包）；验证：`cargo --version`、`rustup target list --installed`
- [x] 2.2 创建 `desktop/`：Tauri 2 + Vite + React 18 + TS + Vitest，productName "Kiro Gateway"，identifier `dev.kirogateway.desktop`，版本 0.0.1，单窗口，CSP 只放 `ipc:`；只注册 single-instance（第一个）、opener、clipboard-manager、autostart、updater、process 六个插件；托盘带占位图标；capabilities 不开放 fs/shell；验证：`cd desktop && pnpm tauri dev` 能出窗口，`cargo test` 和 `pnpm test` 能跑空用例
- [x] 2.3 写 `desktop/AGENTS.md`（技术栈边界、写盘范围、进程规则、文案原则、命令）和 `desktop/Makefile`（dev / test / check / sidecar / dmg-local / help），`.gitignore` 加上 `desktop/src-tauri/resources/gateway/`、`target/`、`node_modules/`、`dist/`；验证：`make help` 列出全部目标

## 3. 网关 sidecar 打包

- [x] 3.1 写 `desktop/scripts/build-sidecar.sh`：建独立构建 venv、安装 requirements 的生产依赖和 PyInstaller（版本固定）、预下载 tiktoken BPE、PyInstaller `--onedir`（加 `--collect-submodules uvicorn` 等 hidden-import），产物输出到 `src-tauri/resources/gateway/`；验证：脚本跑完后产物目录存在可执行文件
- [x] 3.2 构建后冒烟：脚本末尾在随机端口用临时空 HOME 起 sidecar，`PROXY_API_KEY=test`、不设任何凭证，打 `/health` 返回 200，然后结束进程；冒烟失败时构建失败；验证：故意删掉一个 hidden-import 时冒烟失败
- [x] 3.3 实测包体：记录 sidecar 目录大小和 dmg 大小，写进 `desktop/AGENTS.md`；验证：文档里有实测数字
- [x] 3.4 写 `desktop/scripts/build-sidecar.ps1`（Windows 版，逻辑同 3.1/3.2）；验证：脚本语法检查通过（Windows 实机验证放 8.3）

## 4. 桌面端：网关进程控制（desktop/gateway-control）

- [x] 4.1 Rust `app_config`：读写 App 数据目录下的 `config.json`（端口默认 8000、模型映射默认值与 `scripts/kclaude.zsh` 一致）；首次运行生成 32 字节随机密钥写入 `secret`（0600，原子写）；验证：`cargo test app_config::` 覆盖首次生成、重启后不变、config 损坏时回到默认值并备份、`secret` 权限为 0600
- [x] 4.2 Rust `gateway`：环境变量组装（清空继承的 `ANTHROPIC_*`/`PROXY_*`，显式设置 design 决策 2 里的全部变量）、cwd 设为 `runtime/`、stdout/stderr 写入轮转的 `gateway.log`；验证：`cargo test gateway::env` 断言环境变量集合，含"继承的 ANTHROPIC_BASE_URL 被移除"
- [x] 4.3 状态机与启停：已停止 → 启动中 → 运行中 / 启动失败（30 秒超时）→ 意外退出；wait 线程加 10 秒心跳（连续 3 次失败判定挂掉）；macOS SIGTERM 5 秒后 SIGKILL，Windows 用 Job Object；状态变化发 event；验证：`cargo test gateway::state` 用假的子进程（一个 sleep 或立即退出的脚本）覆盖启动成功、启动超时、运行中被 kill 后 5 秒内变为意外退出、重复点启动不会起两个进程
- [x] 4.4 端口与遗留进程：bind 探测；`gateway.pid` 加可执行文件路径校验后回收自己遗留的进程；其他占用者绝不 kill，返回占用进程名；验证：`cargo test gateway::port` 覆盖端口空闲、被他人占用、pid 存活且是自己、pid 被复用成别的进程、pid 文件损坏
- [x] 4.5 日志摘要脱敏：取 `gateway.log` 最后 20 行，过滤 ≥32 位 base64/hex 串和密钥原文；验证：`cargo test gateway::redact` 用含假 token 和假密钥的日志行验证
- [x] 4.6 托盘与窗口：关窗只隐藏窗口；托盘显示状态，菜单有「打开主窗口」「启动 / 停止网关」「退出」；退出时同步停网关；single-instance 把已有窗口拉到前台；验证：`tauri dev` 下手动走一遍并截图，`ps` 确认退出后没有遗留进程
- [x] 4.7 前端主页开关区：开关、状态徽标、失败原因和日志摘要、端口被占用时的改端口入口；验证：`pnpm test` 覆盖状态到界面文案的纯函数映射（五种状态 + 端口占用）

## 5. 桌面端：环境检测（desktop/environment-check）

- [x] 5.1 Rust PATH 解析：`$SHELL -lic 'echo $PATH'`（3 秒超时，结果缓存）加默认位置列表，查找 `kiro-cli`、`claude`；验证：`cargo test env_check::which` 用临时目录模拟只在默认位置、只在 PATH、都没有、shell 超时四种情况
- [x] 5.2 Rust kiro-cli 登录检测：rusqlite `mode=ro` 加 `busy_timeout`，按三个 key 的优先级读取，只返回登录方式、过期时间、是否可刷新；数据库路径按平台候选列表查找；验证：`cargo test env_check::login` 用临时 SQLite 覆盖无库、无表、无 key、过期但有 refresh、过期且无 refresh、value 不是 JSON、数据库被写锁占用，并断言返回结构里不含任何 token 字段
- [x] 5.3 前端检测清单：四项状态，每项的修复提示和按钮（安装页、复制 `<kiro-cli 绝对路径> login`、Claude Code 安装链接），「重新检测」，三项 kiro-cli 检查任一未通过时禁用开关并说明原因；验证：`pnpm test` 覆盖检测结果 → 开关可用性的纯函数，以及复制命令用的是绝对路径
- [x] 5.4 网关启动时的 401/403 归类：解析 sidecar 日志里的 token 刷新失败，状态原因显示"登录已失效，请运行 kiro-cli login"；验证：`cargo test gateway::classify` 用真实格式的日志行覆盖

## 6. 桌面端：kclaude 命令（desktop/kclaude-command）

- [x] 6.1 脚本模板：`kclaude.zsh`/`kclaude.bash`/`kclaude.ps1` 每次运行读 `config.json` 和 `secret`，探测 `/health`，失败时提示"KiroGateway 未运行，请在 Kiro Gateway App 里打开开关"并以非零状态退出，成功则设置 `ANTHROPIC_BASE_URL`、`ANTHROPIC_AUTH_TOKEN` 和三档模型后 `command claude "$@"`；验证：在临时 HOME 下 `zsh -c` / `bash -c` 跑脚本，用假 `claude`（打印环境变量）断言网关未起时退出码非零且不调用 claude、网关起了时环境变量正确、改端口后立刻生效
- [x] 6.2 Rust 启动文件管理：按 `$SHELL` 选择 `.zshrc` 或 `.bashrc`/`.bash_profile`；标记段的增、查、删；写前备份（备份失败就不写）；检测到不带标记的已有 kclaude（含 `source .../scripts/kclaude.zsh`）时不写，返回 `foreign`；「改由 App 管理」时注释掉旧行再加标记段；验证：`cargo test kclaude::rc` 覆盖文件不存在、已有内容、已装最新（不写）、foreign、接管、移除后其余内容逐字节不变、备份失败
- [x] 6.3 Windows profile：计算 PS5 和 PS7 的 `CurrentUserAllHosts` 路径（Documents 用 KnownFolder），两处都写入；验证：`cargo test kclaude::ps_profile`（路径计算用注入的 Documents 目录测试）
- [x] 6.4 安装后实测：用新登录 shell `type kclaude` 确认可用，结果显示在界面上；验证：`cargo test kclaude::verify` 用临时 HOME 加假 shell 输出覆盖成功和失败
- [x] 6.5 前端 kclaude 区：状态（已安装 / 未安装 / foreign / 需先装 Claude Code）、「改由 App 管理」「移除」按钮、"新开一个终端"文案、模型映射下拉（来源 `/v1/models`，网关未运行时禁用）；验证：`pnpm test` 覆盖状态 → 文案和按钮的纯函数
- [x] 6.6 文档：`docs/internal/local-setup.md` 补上桌面端安装方式，说明桌面端和 `scripts/kclaude.zsh` 的关系；验证：文中的命令和路径与实现一致

## 7. 桌面端：请求日志页（desktop/request-log-viewer）

- [x] 7.1 Rust 增量读取：按月份文件维护偏移，每 1 秒 stat，没有换行结尾的半行留到下一轮，坏行跳过；启动时加载本月和上月；验证：`cargo test request_log::reader` 覆盖追加、半行、坏行、文件被截断或重建（偏移回零）、跨月切换
- [x] 7.2 Rust 聚合：今日、本月合计（credits、请求数），本月按模型分组，按本机时区算自然日和自然月，失败请求的 credits 也计入；验证：`cargo test request_log::agg` 用固定时区覆盖 23:59 / 00:00、月末跨月、按模型分组之和等于合计
- [x] 7.3 清理与清空：启动时删除早于上上个月的文件；「清空日志」删除全部文件并重置偏移；验证：`cargo test request_log::retention`
- [x] 7.4 前端日志页：汇总卡片（标注 credits）、按模型分组、倒序列表（本机时区时间）、筛选（仅失败 / 模型，汇总不受筛选影响）、失败展开和常见错误建议（402 / 上下文超长 / 401·403 / 拒答）、空状态、清空确认；有新记录时由 event 推送；验证：`pnpm test` 覆盖错误 → 建议映射、筛选、时间格式化

## 8. 桌面端：应用内升级（desktop/app-update）

- [x] 8.1 版本号固定为 0.0.1（`package.json`、`tauri.conf.json`、`Cargo.toml` 三处一致），生成 minisign 密钥对，公钥写进 `tauri.conf.json`，私钥存本机并加入 `.gitignore`；验证：`cargo test version::consistent` 断言三处版本一致
- [x] 8.2 Rust `update::decide`：解析清单，`decide_update(current, manifest, platform)` 返回 none / background / forced；验证：`cargo test update::decide` 覆盖有新版、版本相等、低于 minRequiredVersion、恰好等于 minRequiredVersion、缺少当前平台、版本号格式错、清单不是 JSON、`minRequiredVersion` 缺失
- [x] 8.3 Rust 检查调度：启动时检查一次，之后每 60 分钟检查一次（按墙钟判断，睡眠唤醒后补查），失败时静默跳过；`background` 时下载并校验，写 `pending-update.json`，发 event；`forced` 时停网关、置 `upgrade_required`（开关与托盘启动命令拒绝执行，`config.json` 写 `upgradeRequired: true`）并发 event；验证：`cargo test update::schedule` 用注入时钟和假清单源覆盖首次检查、60 分钟周期、唤醒补查、连续失败不报错、同一版本不重复下载
- [x] 8.4 强制升级不可绕过：`gateway_start` 在 `upgrade_required` 时返回错误；kclaude 脚本读到 `upgradeRequired: true` 时提示升级并非零退出；验证：`cargo test update::forced_blocks_start`，以及 6.1 的脚本测试新增「强制升级时退出码非零且不调用 claude」
- [x] 8.5 启动时安装待更新：有 `pending-update.json` 且文件校验通过就先安装再进主界面，重启后恢复开关状态；安装失败时以当前版本启动并标记失败；验证：`cargo test update::pending` 覆盖正常、文件丢失、记录损坏、版本已不比当前新
- [x] 8.6 前端：状态栏和托盘的「新版本已下载，立即重启」、设置页版本区（当前版本、检查更新、失败提示和手动下载入口）、强制升级遮罩（进度、失败重试、安装包直链，不能关闭，期间开关与托盘启动项不可用）；验证：`pnpm test` 覆盖 update 状态 → 界面元素的纯函数映射
- [x] 8.7 `desktop/scripts/release.sh`：出包、签名、按「安装包 → .sig → HEAD 校验 → 覆盖 latest.json」的顺序上传 COS，`FORCE=1` 时二次确认；文档写进 `desktop/AGENTS.md` 发版一节；验证：`--dry-run` 模式打印上传计划并生成清单，用 `jq` 校验字段齐全
- [x] 8.8 端到端：本机用一个临时 HTTP 服务模拟 COS，先装 0.0.1，发布 0.0.2（普通）确认静默下载后重启生效，再发布 0.0.3 且 `minRequiredVersion=0.0.3`，确认 0.0.2 打开即进入强制升级并自动完成

## 9. 集成验收

- [ ] 9.1 macOS 本机出包：`make sidecar && make dmg-local`，安装 dmg 后在你的机器上走一遍：环境检测全部通过 → 打开开关 → 新开终端运行 `kclaude -p "hi"`（用 haiku）→ 日志页 3 秒内出现记录，credits 与 `gateway.log` 里的 `[Credit]` 一致 → 关闭窗口后 kclaude 仍可用 → 托盘退出后没有遗留进程 → `claude` 本身不受影响；全程截图
- [x] 9.2 在临时 HOME 下（`HOME=$(mktemp -d)`）跑一遍首次安装流程，确认没有 kiro-cli 登录时的提示和禁用开关的行为正确，并且不碰你真实的 `~/.zshrc`
- [ ] 9.3 Windows 实机（或 CI windows runner）：跑 `build-sidecar.ps1`、出 NSIS 包、确认 kiro-cli 数据库路径并更新候选列表、走一遍 9.1 的流程；没有条件时把这一项标为阻塞，并在 README 待办里写明
- [x] 9.4 对抗式审查（大功能）：按模块派独立审查者（网关请求日志四路径、sidecar 生命周期、kclaude 写盘、日志聚合、升级与强制升级），按 dev-workflow 的要求出报告，确认的高 / 中问题修完

## Workflow follow-up

- verify 通过后 archive，把 delta specs 合并到 `openspec/specs/`。
- 后续 change：Apple 签名公证与 Windows Authenticode、Windows CI 出包。
