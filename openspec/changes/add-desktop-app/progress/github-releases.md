# 应用内升级改用 GitHub Releases

日期：2026-10-23。未 git add/commit/push/tag，未调用任何会上传或删除远端内容的命令。

## 改动

- `desktop/src-tauri/tauri.conf.json`：updater endpoint 改为 `https://github.com/heyuan110/kiro-gateway/releases/latest/download/latest.json`（全仓库配置里只此一处）。
- `desktop/scripts/release_lib.py`：删除 COS（常量、凭证、`CosUploader`、HEAD 校验、上传计划、`upload`/`check-published`/`confirm-force` 等子命令）、版本文件快照回滚、`check-sidecars`、占位产物；`prepare` 改为 `--base-url`（必须 https）、`--tag-body`（tag 正文，含单独一行 `[force-upgrade]` 即强制，标记行不进说明）、`--previous-manifest`（上一个 latest.json 的本地副本，取不到用 0.0.1 并在输出里写明）；产物放进 `<out>/assets/`（不含空格的名字，加 latest.json），`<out>/notes.md` 作 Release 正文；`check_manifest_assets` 保证清单里每个 URL 都对应 assets 里的真实文件。保留 `bump` / `check-version` / `current-version` / `verify-manifest`。只用标准库。
- `.github/workflows/desktop-release.yml`：release job 增加 checkout（fetch-depth 0）、重新 fetch tag 对象读正文、`gh release download` 取上一个 latest.json、`prepare`、`verify-manifest`，最后只上传 `release-out/assets/*`。download-artifact 不再 merge，保持 `bundles-macOS/`、`bundles-Windows/` 目录。base-url 用 `${GITHUB_REPOSITORY}`，不写死。
- 删除 `desktop/scripts/release.sh`、`desktop/scripts/publish-from-github.sh`：前者只剩 COS 上传和本机构建（CI 已覆盖），后者的 `--dry-run` 价值被测试和 `prepare` 的本地试跑取代（用真实 CI 产物试跑见下）。
- 文档：`desktop/AGENTS.md`「发版」一节、`design.md` 决策 11 追加「2026-10-23 改为 GitHub Releases」、`specs/desktop/app-update/spec.md` 与 `proposal.md` 的 COS 字样、`docs/internal/local-setup.md`。
- Rust 测试：`scaffold_tests.rs` 的 `update_manifest_url_is_declared_once` 断言新地址且不含 myqcloud；`update/commands.rs` 的「debug 构建拒绝远程地址」用例改用 GitHub 地址（回环 + 许可规则不变）。

## Red / Green

- Python Red：新测试（ManifestBaseUrlTests、TagNotesTests、PreviousFloorTests、PrepareCliTests、NoCosLeftoversTests 及改过的 ManifestTests）拷到临时目录对着旧 `release_lib.py` 跑：`Ran 36 tests ... FAILED (failures=4, errors=15)`。
- Python Green：`/opt/miniconda3/bin/python -I -m unittest discover -s desktop/scripts -p 'test_*.py'`：`Ran 90 tests ... OK`（含 WorkflowTests 对 workflow 文本的守卫）。
- Rust Red：先改 `scaffold_tests.rs` 断言，`cargo test update_manifest_url` 失败（left 为 COS 地址，right 为 GitHub 地址）；改 `tauri.conf.json` 后 Green。
- `cargo test`：881 passed；`cargo build`：0 警告；`pnpm test`：16 文件 409 用例通过。
- workflow：`uv run --with pyyaml` 解析通过，jobs = build、release。

## 本机模拟 release job

输入：scratchpad `ci-bundles`（真实 CI 产物，bundles-macOS + bundles-Windows，版本 0.0.1）。`prepare --base-url https://github.com/heyuan110/kiro-gateway/releases/download/desktop-v0.0.1 --tag-body ...`：

- assets：`Kiro-Gateway_0.0.1_universal.dmg`、`Kiro-Gateway_0.0.1_x64-setup.exe`、`Kiro-Gateway_0.0.1_x64-setup.exe.sig`、`Kiro-Gateway.app.tar.gz`、`Kiro-Gateway.app.tar.gz.sig`、`latest.json`。
- 清单里 darwin-aarch64 / darwin-x86_64 / windows-x86_64 的 URL 都是 `.../releases/download/desktop-v0.0.1/<上述文件名>`，签名与对应 `.sig` 文件内容一致，`manualDownload` 指向 dmg 与 setup.exe，`verify-manifest` 通过。
- 无上一个清单：`minRequiredVersion` = 0.0.1，输出写明用默认值。tag 正文含 `[force-upgrade]`：`minRequiredVersion` = 本次版本，标记行不在 notes 里。

## 没做到

- 没有真正触发 GitHub Actions（不能 push tag）：release job 里的 `gh release download`、`git fetch` tag 对象、`softprops/action-gh-release` 只在 YAML 层面验证，第一次真实发版时要看一遍日志。
- 仓库私有期间 App 取不到清单（见 design 决策 11）。
