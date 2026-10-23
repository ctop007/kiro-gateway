# macOS universal 安装包

## 方案
- sidecar 分架构：`resources/gateway-aarch64/`、`resources/gateway-x86_64/`（Windows 仍是 `gateway/`）。
- `build-sidecar.sh --arch arm64|x86_64`；x86_64 在 Apple Silicon 上自动 `arch -x86_64` 重跑，解释器取 uv 的 `cpython-3.13-macos-x86_64`，venv `.build-venv-x86_64`。
- `sidecar.rs::resolve(resource_dir, override, arch)` 按 `std::env::consts::ARCH` 选目录；缺目录 -> `ArchMissing`（提示 CPU / 重新下载），目录在但无可执行文件 -> `Missing`。
- `tauri.conf.json` 保持 Windows 布局；`tauri.macos.conf.json` 用 merge patch 把 `resources/gateway/` 置 null 并加两个架构目录。单架构：`--config` 置 null 另一个（`make dmg-local`）。
- `release.sh` 在改版本号前调 `release_lib.py check-sidecars`（两目录都在、Mach-O CPU 类型正确），构建命令本来就是 `--target universal-apple-darwin --bundles app,dmg`，与新流程一致。

## Red
- Rust：先写 `sidecar/tests.rs`（`resource_subdir`、按 arch 选目录、`ArchMissing`、`is_gateway_dir_name`）、`port/tests.rs`（识别 `gateway-<arch>`）、`scaffold_tests.rs`（`tauri.macos.conf.json`），`cargo test` 编译失败（`cannot find function resource_subdir`、`resolve` 参数个数不符）；scaffold 两条因 `cannot read tauri.macos.conf.json` 失败。
- Python：`CheckSidecarsTests` 与 `RealReleaseScriptTests` 新增用例是在实现之后补的；用把 `release.sh` 里的 `check-sidecars` 临时换成 `true` 的方式验证它们会挡住回归：`test_missing_sidecar_aborts_before_versions_or_build_are_touched`、`test_wrong_architecture_sidecar_aborts_too` 两条 FAIL（59 条里 2 条失败），恢复后全过。

## Green
- `cargo test`：881 passed（改前 879），`cargo build` 0 警告。
- `pnpm test`：409 passed。
- `desktop/scripts` 全部 Python 测试：106 OK（`test_release_lib.py` 59 条）。

## 构建与验证
- x86_64 sidecar：`scripts/build-sidecar.sh --arch x86_64`，Rosetta 下冒烟通过（/health、/v1/models、count_tokens），`file`：Mach-O 64-bit executable x86_64。arm64 sidecar：`file` arm64。
- `CI=true pnpm tauri build --bundles app,dmg --target universal-apple-darwin`：`lipo -info` = x86_64 arm64；两份 sidecar 架构正确。
- dmg：`src-tauri/target/universal-apple-darwin/release/bundle/dmg/Kiro Gateway_0.0.1_universal.dmg`，51125461 字节（48.8 MB）；`.app` 97 MB。
- 单架构 `--config` 置 null：`--bundles app` 只带 `gateway-aarch64`，主程序 arm64 非 fat，配置合并顺序（macos 配置在前，`--config` 在后）有效。
- 运行验证（临时 HOME 在 /private/tmp，端口 8100 / 8101，脚本 scratchpad 下 run-app-universal.sh）：
  - 原生 arm64：/health 通；sidecar `.../Resources/gateway-aarch64/kiro-gateway`，file = arm64；SIGTERM 主程序后子进程退出、端口关闭。
  - `arch -x86_64`：/health 通（约 8 秒）；sidecar `.../Resources/gateway-x86_64/kiro-gateway`，file = x86_64（Rosetta）；停止后无遗留。
