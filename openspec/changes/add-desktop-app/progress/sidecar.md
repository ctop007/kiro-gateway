# 批次 sidecar：3.1、3.2、3.4

日期：2026-10-22。只改了 `desktop/scripts/` 下的文件，产物在 `desktop/src-tauri/resources/gateway/`（gitignore）。tasks.md 的勾选没改。3.3（实测包体写进 AGENTS.md）不在本批，数字见文末。

## 文件

- `desktop/scripts/build-sidecar.sh`、`build-sidecar.ps1`：入口。
- `desktop/scripts/sidecar/build_helpers.py`：纯标准库的辅助（生产依赖过滤、hidden-imports 解析与 PyInstaller 参数、cl100k 下载校验）和给两个脚本用的命令行。
- `desktop/scripts/sidecar/smoke_sidecar.py`：构建后冒烟。
- `desktop/scripts/sidecar/hidden-imports.txt`：hidden-import 清单（每条带原因）。
- `desktop/scripts/test_smoke_sidecar.py`：47 个 unittest。上一轮的残留文件按新设计重写（上一轮假设 `kirocli:social:token` 和「无凭证即退出」，与本批要求不符）。
- 运行测试：`python3 -I -m unittest discover -s desktop/scripts -p 'test_*.py' -v`

## 3.1 构建脚本

- 构建 venv：`desktop/.build-venv`，用系统 Python（本机选中 `/opt/miniconda3/bin/python3.13`，≥3.10 才接受，可用 `KGW_PYTHON` 指定）。已存在就复用。
- 依赖：`requirements.txt` 在 `# Testing dependencies` 注释之前的部分（fastapi、uvicorn[standard]、httpx、loguru、python-dotenv、tiktoken），加 `pyinstaller==6.22.3`（固定在 `build_helpers.PYINSTALLER_VERSION`）。生产依赖没有再固定版本，沿用仓库 requirements 的写法；本次构建实际装到的版本与仓库 `.venv` 相同（fastapi 0.159.1、uvicorn 0.54.0、httpx 0.28.1、tiktoken 0.14.0）。
- tiktoken：构建时下载 `cl100k_base.tiktoken` 到产物 `tiktoken_cache/`，文件名为 URL 的 sha1（`9b5ad71b2ce5302211f9c61530b329a4922fc6a4`，对照 `tiktoken/load.py` 的 `read_file_cached`），下载后校验 sha256，写临时名再 rename。
- PyInstaller：`--onedir --console --name kiro-gateway`，入口仓库根 `main.py`，`--exclude-module tkinter`。参数来自 `hidden-imports.txt`：
  - `collect-submodules uvicorn`：uvicorn 用字符串在运行时选 loop / http / lifespan 实现。
  - `collect-submodules kiro`：`grep` 与 ast 扫描确认 kiro 里有 20 多处函数体内 `from kiro.x import`（routes、streaming、exceptions、debug_middleware、mcp_tools 等）；整包收集最稳，并由单测守住。
  - `hidden-import main`：`main.py` 末尾 `uvicorn.run("main:app", ...)` 和日志配置里的 `main.InterceptHandler` 都是字符串导入，冻结后入口脚本是 `__main__`，不加就找不到 `main`。
  - `hidden-import tiktoken_ext` / `tiktoken_ext.openai_public`：tiktoken 用命名空间包插件机制发现编码。
  - 仓库里没有 `importlib` / `__import__` 动态导入。
- 产物先组装在临时目录，冒烟通过后才替换 `src-tauri/resources/gateway/`（保留 `.gitkeep`）；冒烟失败时输出目录不动。
- 产物：`gateway/kiro-gateway`（arm64 Mach-O，minos 11.0）、`gateway/_internal/`、`gateway/tiktoken_cache/`。

## 3.2 冒烟

`smoke_sidecar.py`：临时 HOME、临时 cwd（空目录，`load_dotenv` 读不到 .env）、81xx 空闲端口（8100–8199，bind 探测）。环境只继承 PATH/LANG/TMPDIR 等少数变量，其余（`ANTHROPIC_*`、`PROXY_*`、`KIRO_*`、`REFRESH_TOKEN`）全部丢弃，再设置：`PROXY_API_KEY=test`、`KIRO_CLI_DB_FILE`（临时 sqlite）、`SERVER_HOST=127.0.0.1`、`SERVER_PORT`、`FAKE_REASONING=false`、`TIKTOKEN_CACHE_DIR`、`LOG_LEVEL=DEBUG`、`HTTP(S)_PROXY`/`ALL_PROXY=http://127.0.0.1:9`、`NO_PROXY=127.0.0.1,localhost`、`PYTHONUTF8=1`。

假数据库：`auth_kv` 表，`kirocli:odic:token`（假 access_token / refresh_token，`expires_at` 一年后，region us-east-1）和 `kirocli:odic:device-registration`（假 client_id / client_secret）。网关因此走 AWS SSO OIDC 路径，token 不会过期所以不刷新；runtime 端点用静态模型列表，不会联网。

检查项（任一失败整个脚本失败，并打出 sidecar 输出末尾）：
1. 启动期间进程没退出，`/health` 在超时内返回 200。
2. `/v1/models`（`Authorization: Bearer test`）返回 200 且列表非空，这证明账号初始化成功。
3. `/v1/messages/count_tokens`（`x-api-key: test`）返回 200，且日志出现 `Initialized tiktoken`、不出现 `Failed to initialize tiktoken`。这一条是为了抓住 tiktoken 缺插件时 `kiro/tokenizer.py` 悄悄退回估算、`/health` 却仍然 200 的情况。
4. 进程组 SIGTERM，5 秒后 SIGKILL；临时目录全部删除。运行前先检查缓存文件存在、名字正确、不小于 1 MB。

开关：`build-sidecar.sh --negative-check <模块>` 把该模块从 PyInstaller 参数里去掉，构建到临时目录，只有冒烟失败才算成功，输出目录不动。模块名不在清单里（拼写错误）时报错而不是静默通过。

### Red / Green

先写测试后写实现。

- Red：`python3 -I -m unittest discover -s desktop/scripts -p 'test_*.py'` → `ModuleNotFoundError: No module named 'build_helpers'`，`Ran 1 test ... FAILED (errors=1)`（测试文件在 `sidecar/` 目录有任何实现之前就导入失败）。
- Green（build_helpers + smoke_sidecar 写完后）：`Ran 37 tests in 14.4s OK`；后续补 CLI 与下载校验后 `Ran 47 tests OK`。
- 追加一轮小的 Red/Green：测试里加了 `PYTHONUTF8=1` / `PYTHONIOENCODING=utf-8` 断言（启动横幅里有 emoji，Windows 上 stdout 重定向到文件时会用 ANSI 代码页报 `UnicodeEncodeError`），先 `KeyError: 'PYTHONUTF8'` 失败，改 `build_environment` 后 47 个全过。

覆盖的用例（节选）：
- build_helpers：Testing 注释截断、无标记、空结果报错、大小写、真实 requirements.txt 恰好六个生产包、spec 解析（注释/空行/未知类型/缺值/多值）、omit 只去掉一项、omit 拼写错误报错、真实清单含 uvicorn/kiro/main/tiktoken_ext、**ast 扫描 kiro/*.py 的函数体内 `from kiro...` 导入且都被清单覆盖**、缓存文件名等于 sha1(url)、URL 与已安装 tiktoken 一致、CLI 各子命令、下载哈希不符不落盘、网络失败有提示、已有正确文件不联网、损坏文件被替换。
- smoke（用假 sidecar 可执行文件驱动全部失败分支）：成功路径；环境隔离（泄漏的 ANTHROPIC_BASE_URL / PROXY_API_KEY / KIRO_CREDS_FILE / REFRESH_TOKEN / HTTPS_PROXY 不会进入子进程，HOME 与 cwd 是新目录，代理指向死端口）；临时目录清理；启动即崩溃并带出输出；/health 503；从不监听；/v1/models 500；count_tokens 500；tiktoken 回退日志；没有初始化日志；找不到可执行文件；缓存目录缺失、为空、被截断、文件名不对；失败和成功后都不遗留进程；假 DB 的表结构、key、一年后过期、registration 字段；端口范围、占用跳过、全占用报错。

### 真实构建与反向验证

- `./scripts/build-sidecar.sh`：首次含建 venv 和装依赖约 58 秒，之后约 25 秒。最终输出：`冒烟通过：/health、/v1/models、count_tokens 都正常` → `完成：.../resources/gateway/kiro-gateway (44M)`。
- 反向验证（每次脚本退出码 0，表示冒烟如预期失败）：
  - `--negative-check main`：冒烟失败，sidecar 输出 `ModuleNotFoundError: No module named 'main'` / `ValueError: Cannot resolve 'main.InterceptHandler'`，进程在启动阶段退出。
  - `--negative-check tiktoken_ext.openai_public`：`/health`、`/v1/models` 都 200，`count_tokens` 也 200，但日志 `Unknown encoding cl100k_base. Plugins found: []`，被第 3 项检查抓住。这说明只打 `/health` 是不够的。
  - `--negative-check uvicorn`：冒烟**仍然通过**（脚本报「冒烟测试挡不住缺失的 hidden-import」，退出非 0）。推测原因是 PyInstaller 自带的 uvicorn hook 已经覆盖了 `uvicorn.protocols.*`、`uvicorn.loops.*`（没有去读 hook 源码确认）；这一条 `collect-submodules uvicorn` 是冗余保险，不是必需。任务要求的是「去掉一个必要的 hidden-import」，`main` 和 `tiktoken_ext.openai_public` 才是必要的，已验证；`uvicorn` 这条保留（体积只多几百 KB，任务明确要求加）。
  - `--negative-check mian`（拼写错）：报 `hidden-imports.txt 里没有 'mian'`，不会误判为通过。
  - 验证后恢复：`hidden-imports.txt` 没有被改动（负向检查靠命令行 omit，不改文件），随后重新跑了正常构建，结果同上。
- 冒烟全程没有占用 8000；8100–8199 上构建结束后无残留监听，也没有遗留 sidecar 进程。

## 3.4 build-sidecar.ps1

- 逻辑与 .sh 一致：bootstrap（`KGW_PYTHON`，否则 `py -3.13..-3.10`，再 `python`）、`.build-venv\Scripts\python.exe`、同一个 `build_helpers.py` 命令行取依赖/参数/下载 tiktoken、PyInstaller、冒烟（`smoke_sidecar.py`，POSIX 与 Windows 分支：`terminate/kill` 与 `killpg`）、`-NegativeCheck <模块>`、冒烟通过后才替换输出目录。
- 文件是 UTF-8 带 BOM + CRLF：Windows PowerShell 5.1 对无 BOM 文件按 ANSI 代码页读，中文提示会乱码。
- 语法检查：本机没有 `pwsh` / `powershell`（`which` 均未找到），**没有做真正的 PowerShell 语法检查**。只做了括号配对的机械检查（`{}` 35/35、`()` 65/65、`[]` 12/12）和 BOM 检查，没有实机或 pwsh 验证。放到 9.3（Windows 实机或 CI windows runner）：跑 `build-sidecar.ps1` 并检查 `[System.Management.Automation.Language.Parser]::ParseFile`。
- 未验证的 Windows 细节：PyInstaller 产物名 `kiro-gateway.exe`；`py` launcher 的选择；`*> $PyiLog` 重定向在 5.1 上的编码（只用于出错时看日志）。

## 包体（供 3.3 使用）

- `du -sh desktop/src-tauri/resources/gateway`：**44M**（`_internal` 35M，`kiro-gateway` 启动器 7.7M，`tiktoken_cache` 1.6M）。
- `tar czf` 压缩后 21M（dmg 内会再压缩，dmg 实测留给 3.3）。
- 低于 design 预估的 25–45 MB 压缩后范围。arm64 only；universal2 需要在 x86_64 上另外构建或用 universal2 Python，本批没做。
