# cmdname-frontend

任务 10.2：前端命令名与默认端口 6868。

## 状态

- `cd desktop && pnpm test`：16 files, 409 passed（改前 331）。
- `pnpm build`（tsc + vite build）：在仓库里**暂时过不了**，原因只有一个：`src/lib/ipc-types.ts` 还没有 Rust 线加的 `commandName`（`AppConfig`、`ConfigPatch`、`KclaudeStatus`），tsc 报 TS2339 / TS2353。这个文件不归本批改，等 Rust 线加上后重跑。
- 验证办法：把 `src/`、配置复制到 /tmp 临时目录，只在副本的 `ipc-types.ts` 里补这三个字段，副本里 `tsc --noEmit` 无报错，`vite build` 通过，`vitest run` 409 passed。副本已删除，仓库里的 `ipc-types.ts` 未动。
- 前端按 contract 使用的字段：`AppConfig.commandName: string`、`ConfigPatch.commandName?: string`、`KclaudeStatus.commandName: string`。

## Red / Green

- Red：先写 `features/command-name/view.test.ts`，没有实现时 `vitest run` 报 import 失败，`no tests`（0 个用例运行）。
- Green：实现 `view.ts` 后 46 passed。
- 随后改 `fixtures.ts`（端口 6868、`kc()` 带 `commandName`）与既有测试期望（`cclaude`），再加：
  - `gatewayView` 的 `commandName` 入参（停止态提示）；
  - `kclaudeView(status, gatewayRunning, commandName?)`；
  - `portHint(status, commandName)`、`DEFAULT_PORT = 6868`；
  - mock 后端：默认端口 6868、`commandName` 默认 `cclaude`，`config_update` 校验名字（同一纯函数）、模拟「这个名字已经被占用」（rg / fd / make），被拒的 patch 整体不生效；
  - `components/command-name.test.tsx`：命令名为 `cclaude` 与 `kclaude-test` 时，主页卡片、停止态提示、日志页空状态、设置页（输入框、移除按钮、端口提示）、状态栏都随名字变化，另一个名字不出现；设置页保存成功 / 失败的行。
- 修前（改文案前）这些用例失败 14 个（gateway / kclaude / settings 的期望），实现后全绿。

## 改动

- 新增 `src/features/command-name/view.ts` + test：`parseCommandName`（`^[A-Za-z][A-Za-z0-9_-]{1,31}$` + 保留名列表，大小写不敏感，列表含任务要求的 23 个名字）、`resolveCommandName`（config 优先，其次 status，最后 `cclaude`）、各处文案函数、`commandNameOutcome`（保存成功「已改为 X，新开一个终端即可使用」；失败显示 Rust 返回的 message，非契约形状的错误退回通用句子）、`canSaveCommandName`。
- 「已被占用」只能由 Rust 判断（要看 PATH 和启动文件），前端不重复实现，直接显示 Rust 的 message。
- `useAppState`：`commandName`（派生）、`saveCommandName`（调 `config_update({ patch: { commandName } })`，成功后用返回的 config 更新并重读 `kclaude_status`）、`commandNameResult`。
- 设置页：新增「命令名」输入框 + 保存按钮，输入时即时校验；「移除 X」按钮和名称行用当前名；端口提示用当前名。
- `docs/mockup/index.html`：cclaude、6868、设置页命令名项。`src/styles.css`：命令名输入框宽度。

## 没做 / 注意

- 托盘菜单文案在 Rust 侧（`src-tauri/src/tray`），不在本批范围；前端没有托盘文案。
- 网关运行中改端口的提示（`restartNotice`）保持不变。
