# 批次 frontend：4.7、5.3、6.5、7.4、8.6

日期：2026-10-22。只改了 `desktop/src/`（`index.html` 未改）。tasks.md 的勾选没改。验证命令：`cd desktop && pnpm test && pnpm build`，最终 12 个测试文件、254 个用例通过，`tsc --noEmit` 与 `vite build` 通过。

## 结构

- `src/lib/tauri-core.ts`：`invoke`/`listen` 的唯一出入口，命令名和类型来自 `ipc-types.ts`；rejection 统一经 `ipc-error.ts` 归一为 `{code,message}`，非契约形状一律换成通用中文句子（不透出 URL、堆栈）。
- `src/lib/tauri-{config,gateway,env,kclaude,request-log,update}.ts`：每个模块一个薄封装；`tauri-opener.ts`、`tauri-clipboard.ts` 保留。
- 纯函数：`features/{env,gateway,kclaude,request-log,update,settings}/view.ts`，组件（`src/components/`）只渲染和调用封装。`state/useAppState.ts` 只负责加载、订阅 event、转发 handler。
- `src/mock/backend.ts`：内存假后端实现全部 18 个命令和 3 个 event，场景可由 URL 参数或 `window.__kgwMock.setScenario()` 切换。
- 样式：mockup 的 CSS 移植进 `styles.css`；左侧原型控制栏和假红绿灯没有移植。

## 任务证据

### 4.7 主页开关区
- Red：`features/gateway/view.test.ts`、`features/env/view.test.ts` 先跑，报 `Cannot find module './view'`，`Test Files 3 failed`。
- Green：实现后 `gatewayView` 覆盖五种状态（stopped/starting/running/start_failed/crashed）加端口占用（有进程名、只有 pid、完全未知、occupant 为 null）、登录失效、failure kind=timeout、环境禁用（运行中不被禁用）、强制升级禁用、状态未到达；`startErrorView` 覆盖 env_not_ready/upgrade_required/其他。
- 端口占用不提供结束占用进程的动作（测试 `never offers to kill the occupying process`）。

### 5.3 检测清单
- Red：同上（`env/view.test.ts` 缺模块）。
- Green：`switchGate`（三项 kiro-cli 检查任一 fail/skipped/缺失即禁用，claude 不影响，首次报告前等待）；`isAbsoluteCommand`（POSIX、盘符、UNC、带引号；`~`、相对路径、空串拒绝）；`checkRows` 固定顺序、丢弃非绝对路径的复制命令、只放行 https 链接、非 fail 行不显示修复；`loginCommandFor`。

### 6.5 kclaude 区
- Red：`features/kclaude/view.test.ts` 缺模块。
- Green：`kclaudeView` 覆盖 installed（网关运行/停止）、installed 但缺 Claude Code、foreign（含两个 profile、无 rc 文件）、not_installed、outdated、null、verify 失败；`modelPicker`（网关未运行禁用并提示「启动网关后可选择模型」、保留已存值、去重去空、列表为空）；`displayModel`；`rcDescription`。

### 7.4 日志页
- Red：`features/request-log/view.test.ts` 缺模块。
- Green：`errorAdvice`（402、400 上下文超长的 3 种措辞、401/403、429、5xx、拒答、未知 4xx、缺错误文本）；`applyFilters`（仅失败、模型、组合、未知模型、不改入参；汇总函数不接收筛选）；`summaryView`/`modelBars`（零总额、占比下限、超 100% 夹紧）；`rowView` 复用 `format.ts` 的本机时区格式化；`mergeAppended`（置顶、去重、截断、空推送清零汇总、乱序）；`emptyState`、`effectiveFilters`。清空日志有二次确认；失败行 `tabIndex=0`、`aria-expanded`、Enter/Space 展开。

### 8.6 升级
- Red：`features/update/view.test.ts` 缺模块。
- Green：`statusBarUpdate`（ready 出现「立即重启」）、`settingsVersion`（none/downloading/ready/install_failed/forced，手动下载只放行 https）、`forcedOverlay`（进度已知/未知/未开始、失败带重试和直链、无关闭字段）、`isUpgradeForced`（config 标志或 update 状态）。遮罩 `role=dialog aria-modal`，Escape 被吞，背后三块区域设 `inert`。

## 假后端隔离
- `src/mock/mock-isolation.test.ts`：（1）静态检查只有 `lib/` 下的封装以 `import.meta.env.DEV && VITE_MOCK_IPC==='1'` 守卫动态 import；（2）用 `vite.build`（`VITE_MOCK_IPC=1`、production）打包，断言产物不含 `__kgwMock`、`createMockBackend`、假数据，且没有 backend chunk。
- 变异验证：删掉 `tauri-opener.ts` 里的 `import.meta.env.DEV &&` 守卫，两个用例都失败；还原后通过。
- 另外 `pnpm build` 后 `grep -c __kgwMock dist/assets/*.js` 为 0，`dist/assets` 只有一个 js 和一个 css。
- 开发期 `vite --port 1432` 下 `/src/main.tsx`、`/src/App.tsx`、`/src/mock/backend.ts` 都返回 200。

## 没做到的
- 没有在真实浏览器里截图或手动操作界面：chrome-devtools 的浏览器实例被占用（`browser is already running`）。界面只由 vitest 纯函数测试、一次服务端渲染测试（`App.test.tsx`）和类型检查覆盖。组件交互（键盘展开、遮罩焦点）未做运行时验证。
