# IPC 约定（Rust ⇄ 前端）

创建：2026-10-22 · 状态：已定稿（v1，随实现补充时在文末「变更记录」追加）

本文件是 Rust command / event 与前端之间的唯一约定。实现与本文不一致时，以本文为准修实现；确实需要改约定时，先改本文再改两端，并在变更记录里写一行。

## 通用规则

- **命名**：command 用 `snake_case`，以模块名开头（`config_*`、`gateway_*`、`env_*`、`kclaude_*`、`request_log_*`、`update_*`）。event 用 `kebab-case`，以模块名开头。
- **JSON 形状**：所有参数和返回值的字段名用 `camelCase`（Rust 侧 `#[serde(rename_all = "camelCase")]`）。枚举值用 `snake_case` 字符串。可选字段缺失时为 `null`，不省略键。
- **时间**：一律是带时区偏移的 ISO 8601 字符串（本机时区，例如 `2026-10-22T14:41:08+08:00`）。时长用毫秒整数，字段名以 `Ms` 结尾。
- **路径**：返回给前端用于展示的路径一律以 `~` 缩写用户目录（`displayPath`）；前端拿到的路径只用于显示和复制，不会再传回 Rust 当参数。唯一例外是修复命令里的 kiro-cli 绝对路径（见环境检测）。
- **错误**：command 失败时 promise reject，值固定为：
  ```json
  { "code": "port_in_use", "message": "6868 端口已被占用" }
  ```
  - `code` 是下表中的稳定枚举，前端按它分支，不解析 `message`。
  - `message` 是可以直接展示的中文一句话，不含 URL、堆栈、token、访问密钥。

  | code | 含义 |
  | --- | --- |
  | `invalid_argument` | 参数不合法（端口越界、模型名为空、limit 越界等） |
  | `upgrade_required` | 处于强制升级状态，拒绝启动网关等操作 |
  | `env_not_ready` | 环境检测未通过（kiro-cli 三项任一未通过），拒绝启动网关 |
  | `gateway_not_running` | 需要网关运行的操作在网关未运行时调用 |
  | `gateway_unreachable` | 网关进程在，但本机请求失败或超时 |
  | `foreign_present` | 启动文件里已有不带本 App 标记的 kclaude，需要先「改由 App 管理」 |
  | `backup_failed` | 写启动文件前备份失败，未写入 |
  | `write_failed` | 写文件失败（权限、磁盘满） |
  | `no_pending_update` | 没有已下载的更新却调用了重启安装 |
  | `internal` | 其他未分类错误 |

  `env_not_ready` 的 `message` 就是环境检测给出的原因句子（例如「先登录 kiro-cli 才能打开」），前端原样显示。

  注意：网关「启动失败」不是 command 错误，而是状态（见 `GatewayStatus.failure`）。`gateway_start` 只在启动请求本身被拒绝时 reject（`upgrade_required` / `env_not_ready` / `internal`）。

- **前端封装**：所有 `@tauri-apps/*` 调用集中在 `src/lib/tauri-*.ts` 薄封装，每个模块一个文件：`tauri-config.ts`、`tauri-gateway.ts`、`tauri-env.ts`、`tauri-kclaude.ts`、`tauri-request-log.ts`、`tauri-update.ts`，以及插件封装 `tauri-opener.ts`、`tauri-clipboard.ts`。共享类型放 `src/lib/ipc-types.ts`（不 import `@tauri-apps/*`）。状态到界面文案、按钮可用性的映射写成纯函数放 `src/features/<模块>/`，用 vitest 测。
- **前端不直接连网关**：CSP 的 `connect-src` 只有 `ipc:` 和 `http://ipc.localhost`。凡是要访问本机网关（`/health`、`/v1/models`）的都由 Rust command 代办。

## 1. 配置（config）

### 类型

```ts
type ModelTier = 'opus' | 'sonnet' | 'haiku'

interface AppConfig {
  port: number                         // 1024..=65535，默认 6868
  commandName: string                  // kclaude 命令名，默认 cclaude；规则见第 4 节
  models: Record<ModelTier, string>    // 默认 opus=claude-opus-5-5[1m]、sonnet=claude-sonnet-5-5[1m]、haiku=claude-haiku-4-5
  autostart: boolean                   // 开机自启，默认 false；值为系统实际状态
  gatewayWanted: boolean               // 上次用户是否打开了开关（重启 / 更新后据此恢复）
  upgradeRequired: boolean             // 强制升级标志（只读，由 update 模块写）
}

interface ConfigPatch {
  port?: number
  commandName?: string
  models?: Partial<Record<ModelTier, string>>
  autostart?: boolean
}

interface ConfigUpdateResult {
  config: AppConfig
  restartRequired: boolean             // 网关运行中改了端口时为 true；界面提示「重启网关生效」
}
```

### Commands

| command | 参数 | 返回 | 错误 |
| --- | --- | --- | --- |
| `config_get` | 无 | `AppConfig` | `internal` |
| `config_update` | `{ patch: ConfigPatch }` | `ConfigUpdateResult` | `invalid_argument`、`write_failed` |

- `config_update` 只改 patch 里出现的字段，写盘是原子写（写临时文件再 rename）。
- 开机自启的系统调用在写盘之前执行；系统调用失败时整个 patch 不生效（端口、模型都不变），返回 `write_failed`，`message` 为「没能修改开机自启，请检查系统设置」。
- `autostart` 由 Rust 调 `tauri-plugin-autostart` 的 Rust API 开关，返回值反映操作后的系统实际状态；前端没有 autostart 权限。
- 访问密钥（`secret`）永远不经 IPC 返回。
- 改 `commandName` 时 Rust 先校验（错误码 `invalid_argument`，`message` 写明原因：格式不对 / 保留名 / 已被占用），通过后写 config，再按新名字重写脚本与启动文件的标记段（等同一次 `kclaude_install`，写前备份）；返回的 `ConfigUpdateResult.config` 是改名完成后的状态。kclaude 处于 `foreign` 或被用户移除时只保存名字，不写启动文件。

## 2. 网关（gateway）

### 类型

```ts
type GatewayState = 'stopped' | 'starting' | 'running' | 'start_failed' | 'crashed'

type FailureKind = 'port_in_use' | 'login_expired' | 'timeout' | 'other'

interface PortOccupant {
  name: string | null                  // 进程名，例如 "python3.13"；查不到为 null
  pid: number | null
}

interface GatewayFailure {
  kind: FailureKind
  message: string                      // 一句中文原因，例如「6868 端口被别的程序占用」
  occupant: PortOccupant | null        // 仅 kind=port_in_use 时非 null
  logExcerpt: string[]                 // gateway.log 最后最多 20 行，已脱敏（≥32 位 base64/hex 串、访问密钥原文替换为 "***"）；无日志时为 []
  at: string                           // 失败或退出发生的时间
}

interface GatewayStatus {
  state: GatewayState
  port: number                         // 当前（或将要）使用的端口
  pid: number | null                   // starting / running 时非 null
  since: string | null                 // 进入当前状态的时间；running 时界面据此算「已运行 2 小时 14 分」
  failure: GatewayFailure | null       // 仅 start_failed / crashed 时非 null
}

interface GatewayModel {
  id: string                           // /v1/models 返回的 id
}
```

`GatewayFailure.message` 的实际措辞（前端对 `login_expired` 和 `port_in_use` 用自己的文案，其余直接显示 `message`）：`timeout` 「30 秒内网关没有就绪，已结束这次启动」；`other` 「网关进程在就绪前退出了（退出码 N）」或「被信号 N 结束」；`crashed` 「网关进程意外退出（退出码 N）」；挂死「网关进程没有响应，已结束」；`port_in_use` 「<端口> 端口被别的程序占用」；`login_expired` 「kiro-cli 的登录已失效」；找不到内置网关「没找到内置的网关程序，请重新安装 Kiro Gateway」。查不到占用者时 `occupant` 为 `{ name: null, pid: null }`（`kind=port_in_use` 时 `occupant` 恒非 null）。

失败原因归类规则：

- `port_in_use`：启动前 bind `127.0.0.1:<port>` 失败，且不是本 App 遗留的 sidecar。
- `login_expired`：启动期间 sidecar 日志出现 token 刷新 401 / 403。`message` 为「kiro-cli 的登录已失效」，界面配复制 `<kiro-cli 绝对路径> login`。
- `timeout`：30 秒内 `/health` 一直不通，进程已被结束。
- `other`：其余情况（进程立即退出、可执行文件缺失等），`message` 尽量具体。
- `crashed` 状态下 `failure.kind` 通常是 `other`；若日志能归到 `login_expired` 则用它。

### Commands

| command | 参数 | 返回 | 错误 |
| --- | --- | --- | --- |
| `gateway_status` | 无 | `GatewayStatus` | 无 |
| `gateway_start` | 无 | `GatewayStatus`（通常是 `starting`） | `upgrade_required`、`env_not_ready`、`internal` |
| `gateway_stop` | 无 | `GatewayStatus`（`stopped`） | `internal` |
| `gateway_models` | 无 | `GatewayModel[]` | `gateway_not_running`、`gateway_unreachable` |
| `gateway_reveal_log` | 无 | `null` | `internal`（日志文件不存在时 message 为「还没有网关日志」，前端直接显示这句话） |

- `gateway_start` 幂等：已在 `starting` / `running` 时直接返回当前状态，不起第二个进程。调用时 Rust 会先重新跑一次环境检测（spec：每次打开开关前检测）。
- `gatewayWanted`：请求被接受（强制升级检查和环境检测都通过）时置 true，不管随后启动成功还是进入 `start_failed`，因为用户要的是「网关开着」；`gateway_stop` 置 false；意外退出、App 退出、更新重启都不改它，下次启动 App 或更新重启后据此恢复网关。
- `gateway_stop` 在 `stopped` / `start_failed` / `crashed` 时也成功，并把状态清回 `stopped`（清掉 failure）。
- `gateway_models` 由 Rust 带访问密钥请求 `http://127.0.0.1:<port>/v1/models`，超时 5 秒，按 id 排序去重后返回。
- `gateway_reveal_log` 用系统默认程序打开 `logs/gateway.log`（Rust 侧 opener），前端不拿到路径。

### Event

| event | payload | 何时发 |
| --- | --- | --- |
| `gateway-status` | `GatewayStatus` | 每次状态迁移（stopped→starting→running/start_failed、running→crashed、任意→stopped），以及 `port` 变化时。不发重复的相同状态。 |

前端在页面挂载时先调 `gateway_status` 拿初值，再监听事件；两者以事件为准覆盖。

## 3. 环境检测（env）

### 类型

```ts
type CheckId = 'kiro_cli_installed' | 'kiro_cli_logged_in' | 'login_valid' | 'claude_installed'
type CheckStatus = 'ok' | 'fail' | 'skipped'

type FixAction =
  | { kind: 'open_url'; label: string; url: string }          // 例如「打开安装页面」
  | { kind: 'copy_command'; label: string; command: string }  // 例如「复制」+ "/Users/x/.local/bin/kiro-cli login"

interface CheckItem {
  id: CheckId
  status: CheckStatus
  title: string        // 「kiro-cli 已安装」「没找到 kiro-cli」……（照 mockup 文案）
  detail: string       // 第二行说明；ok 时可为路径（~ 缩写）或登录方式，例如「AWS SSO 登录」
  fix: FixAction | null  // 仅 status=fail 时可能非 null
}

interface EnvReport {
  checkedAt: string
  items: CheckItem[]   // 固定 4 项，顺序即 CheckId 的顺序
  kiroCliPath: string | null   // 找到的 kiro-cli，~ 缩写，仅展示
  claudePath: string | null    // 找到的 claude，~ 缩写，仅展示
  loginExpiresAt: string | null // access token 过期时间；不可得为 null
  loginCommand: string | null   // `<kiro-cli 绝对路径> login`；找到 kiro-cli 时非 null，路径含空格时按 shell 规则加引号
}
```

规则：

- 固定返回 4 项，顺序为 `kiro_cli_installed`、`kiro_cli_logged_in`、`login_valid`、`claude_installed`。
- 前一项未通过时后续依赖项为 `skipped`：没装 kiro-cli → 后两项 `skipped`；未登录 → `login_valid` `skipped`。`claude_installed` 与前三项独立。
- `copy_command` 里的 kiro-cli 一律是**绝对路径**（不缩写 `~`），保证用户终端 PATH 里没有 kiro-cli 时也能执行。
- 检测结果不含任何 token、refresh token、client secret 片段；登录方式与过期时间是最多能展示的信息。
- 开关是否可用由前端纯函数从 `items` 算出（三项 kiro-cli 检查任一 `fail` 或 `skipped` 即禁用）；Rust 在 `gateway_start` 里用同一规则再校验一次。

### Commands

| command | 参数 | 返回 | 错误 |
| --- | --- | --- | --- |
| `env_check` | 无 | `EnvReport` | `internal` |

- 每次调用都重新检测（不复用上次结果），登录 shell PATH 的探测结果可以缓存。耗时上限约 3 秒（受 `$SHELL -lic` 超时约束）。
- 无 event：检测只在 App 启动、点「重新检测」、打开开关前发生，都由调用方主动发起。

## 4. kclaude（kclaude）

### 类型

```ts
type KclaudeState = 'installed' | 'not_installed' | 'foreign' | 'outdated'
type ShellKind = 'zsh' | 'bash' | 'powershell'

interface KclaudeVerify {
  status: 'ok' | 'fail' | 'skipped'   // 新登录 shell 里 `type kclaude` 的实测结果；未安装时 skipped
  detail: string                       // 失败时一句原因，例如「新开的终端里找不到 kclaude」
  fix: FixAction | null                // Windows 执行策略禁止加载 profile 时给 copy_command（不擅自改策略）
}

interface KclaudeStatus {
  commandName: string        // 当前命令名（与 AppConfig.commandName 一致）
  state: KclaudeState
  shell: ShellKind
  rcFiles: string[]          // 涉及的启动文件，~ 缩写，例如 ["~/.zshrc"]；Windows 为两个 profile
  claudeInstalled: boolean   // 为 false 时界面显示「已安装，但要先装好 Claude Code 才能用」
  verify: KclaudeVerify
}
```

- `installed`：启动文件里有本 App 标记段，App 数据目录里的脚本内容是最新的。
- `not_installed`：没有标记段，也没有别的 kclaude。
- `foreign`：启动文件里有不带本标记的 kclaude（例如 `source .../scripts/kclaude.zsh`），App 不动它。
- `outdated`：有标记段，但标记段或脚本内容不是当前版本生成的（脚本内容过期时 App 启动会自动重写脚本，所以界面上通常只会短暂看到它）。

### Commands

| command | 参数 | 返回 | 错误 |
| --- | --- | --- | --- |
| `kclaude_status` | 无 | `KclaudeStatus`（`verify` 取自缓存 `kclaude-verify.json`：只在安装 / 接管 / 移除后或 rc 文件 mtime、大小变化时重跑登录 shell） | `internal` |
| `kclaude_install` | 无 | `KclaudeStatus` | `foreign_present`、`backup_failed`、`write_failed` |
| `kclaude_adopt` | 无 | `KclaudeStatus` | `backup_failed`、`write_failed`；非 `foreign` 时调用为 `invalid_argument` |
| `kclaude_remove` | 无 | `KclaudeStatus`（`not_installed`） | `backup_failed`、`write_failed` |

- `kclaude_install` 幂等：已是 `installed` 时不写盘，直接返回。App 启动时 Rust 自动调用一次安装逻辑（`foreign` 时跳过）。
- `kclaude_adopt`：注释掉旧的 kclaude 行，再追加标记段，写前备份。
- `kclaude_remove`：只删标记段，其余内容逐字节不变，写前备份。
- 每次写盘成功后都重新判定状态并跑一次 `verify`，返回的是写后的真实状态。
- `kclaude_remove` 之后，App 下次启动不再自动装回：数据目录下写 `kclaude.removed` 标记文件，显式的 `kclaude_install` / `kclaude_adopt` 清除它。
- 启动文件里标记残缺（只有开始或结束标记）时，`KclaudeState` 没有对应值，按 `foreign` 处理：`kclaude_install` 返回 `foreign_present`，`adopt` / `remove` 返回 `write_failed`，`message` 提示删掉残缺的标记行。
- `verify.fix` 目前只会出现 `copy_command`。
- `kclaude.env` 的格式（App 写、脚本读）：每行 `KEY=value`，值不加引号；字段 `KIRO_GATEWAY_PORT`、`KIRO_GATEWAY_KEY`、`KCLAUDE_OPUS_MODEL`、`KCLAUDE_SONNET_MODEL`、`KCLAUDE_HAIKU_MODEL`、`KIRO_GATEWAY_UPGRADE_REQUIRED`（0 或 1）。不加引号是安全的：端口是数字，密钥只含 `[A-Za-z0-9._-]`，模型名在 `config_update` 里按同类字符集校验（带 `[1m]` 的名字也能被 sh / bash / zsh 原样读回，app_config 的测试用真 shell 验证过）。PowerShell 脚本按行解析同一格式，也接受带引号的值。
- 无 event：状态只因用户操作或 App 启动而变。

## 5. 请求日志（request_log）

### 类型

```ts
type ApiKind = 'openai' | 'anthropic'
type Outcome = 'ok' | 'error' | 'refused' | 'client_disconnected'

interface RequestRecord {             // 对应网关 JSONL 一行，字段转为 camelCase
  ts: string
  id: string | null
  api: ApiKind
  model: string | null
  stream: boolean
  status: number
  outcome: Outcome
  durationMs: number
  credits: number                     // 未计费为 0
  error: string | null                // ≤200 字符，网关已截断
}

interface UsageTotal {
  credits: number
  requests: number
}

interface ModelUsage extends UsageTotal {
  model: string                       // model 为 null 或空串的记录归到 "未知模型"
}

interface UsageSummary {
  today: UsageTotal                   // 本机时区自然日
  month: UsageTotal & { month: string }  // 本机时区自然月，month 形如 "2026-10"
  byModel: ModelUsage[]               // 本月，按 credits 降序；各项 credits 之和等于 month.credits
}

interface RequestLogSnapshot extends UsageSummary {
  records: RequestRecord[]            // 按 ts 倒序，最多 limit 条
  generatedAt: string
}

interface RequestLogAppended extends UsageSummary {
  records: RequestRecord[]            // 本轮新读到的记录，按 ts 倒序
  reset: boolean                      // 某个月份文件被外部截断、替换或删除时为 true，此时 records 为空
}
```

- 失败请求的 credits 也计入合计。
- 坏行和没有换行结尾的半行不出现在任何结果里。
- 筛选（仅失败 / 按模型）在前端对 `records` 做，汇总不受筛选影响。

### Commands

| command | 参数 | 返回 | 错误 |
| --- | --- | --- | --- |
| `request_log_snapshot` | `{ limit?: number }`，默认 200，范围 1..=1000 | `RequestLogSnapshot` | `invalid_argument` |
| `request_log_clear` | 无 | `RequestLogSnapshot`（全部归零、`records` 为空） | `write_failed` |

- `request_log_clear` 删除全部月份文件并重置读取偏移；确认弹框由前端负责。目录删不掉时 reject 为 `write_failed`，`message` 是不含路径的中文一句话。

### Event

| event | payload | 何时发 |
| --- | --- | --- |
| `request-log-appended` | `RequestLogAppended` | Rust 每 1 秒检查当月文件，读到至少一条新记录时发一次（一轮只发一次，多条合并）。跨日 / 跨月时即使没有新记录，也发一次 `records: []` 让汇总归零。 |

`reset=true` 时前端不合并，改为重新调用 `request_log_snapshot` 刷新列表（持有的列表里可能有已不存在的记录）。

`reset=false` 时前端收到后把 `records` 插到列表顶部，并用 payload 里的汇总整体替换当前汇总；列表长度超过当前 limit 时截掉尾部。

## 6. 升级（update）

### 类型

```ts
interface DownloadProgress {
  downloadedBytes: number
  totalBytes: number | null           // 服务端未给长度时为 null
}

type UpdateState =
  | { state: 'none' }
  | { state: 'downloading'; version: string; progress: DownloadProgress }
  | { state: 'ready'; version: string }
  | { state: 'installing'; version: string }
  | { state: 'forced'; version: string; progress: DownloadProgress | null; error: string | null; manualUrl: string }
  | { state: 'install_failed'; version: string; error: string; manualUrl: string }

type UpdateStatus = UpdateState & {
  currentVersion: string              // 例如 "0.0.1"
  lastCheckedAt: string | null        // 最近一次成功检查清单的时间
}
```

- `none`：已是最新，或检查失败（断网、清单损坏、缺当前平台）。检查失败不单独暴露状态，界面照常显示「已是最新」。
- `downloading`：后台静默下载普通更新中。界面不打扰，最多在设置页显示进度。
- `ready`：已下载并校验签名，等下次启动或用户点「立即重启」。
- `installing`：启动时发现上次已下载的更新，正在后台线程安装（可能需要联网取更新句柄）。窗口应显示「正在完成更新」，不显示其他更新入口；这段时间网关不恢复，也不做清单检查。成功则 App 重启；失败转 `install_failed`，装不了（离线）转 `ready`，无可装内容转 `none`。
- `forced`：当前版本低于清单的 `minRequiredVersion`。`progress` 非 null 表示下载中；`error` 非 null 表示下载失败（遮罩显示「下载失败」+「重试」）；`manualUrl` 是当前平台安装包（dmg / setup.exe）直链，始终提供。
- `install_failed`：启动时安装待更新失败，以当前版本运行；设置页显示「更新安装失败」和 `manualUrl`。

### Commands

| command | 参数 | 返回 | 错误 |
| --- | --- | --- | --- |
| `update_status` | 无 | `UpdateStatus` | 无 |
| `update_check_now` | 无 | `UpdateStatus`（检查完成后的状态） | 无（失败时返回 `none`） |
| `update_restart_now` | 无 | 不返回（进程重启） | `no_pending_update` |
| `update_retry` | 无 | `UpdateStatus` | 非 `forced` 且非 `install_failed` 时为 `invalid_argument` |

- `update_restart_now`：先停网关，安装已下载的更新，然后重启；重启后按 `gatewayWanted` 恢复网关。暂时装不了（例如离线，updater 取不到更新句柄）时返回 `internal`，`message` 为「暂时无法安装更新，请稍后再试」，更新包保留。
- `UpdateStatus` 的 JSON 是 `UpdateState` 的字段与 `currentVersion`、`lastCheckedAt` 平铺在同一层（`state` 为判别字段）。
- 错误码：update 的 command 只会返回 `invalid_argument`、`upgrade_required`、`no_pending_update`、`internal`。
- `update_retry`：`forced` 时重新下载并安装；`install_failed` 时重新下载。
- 下载超时：下载过程中 60 秒没有任何进度即判失败（后台下载静默回到 `none`，`forced` 显示「下载失败」+「重试」），忙碌标志随之释放，重试可再次触发下载。
- 启动顺序：先读 pending 记录（`installing`，安装在后台线程）→ 后台线程跑首次清单检查 → 网关恢复等安装结束和首次检查结束（成功、失败或 10 秒超时）后才决定：首次检查判定强制升级则不恢复；失败或超时照常恢复。已有 `upgrade-lock.json` 或 `upgradeRequired` 时始终不恢复。
- 强制升级期间 `gateway_start` 返回 `upgrade_required`。

### Event

| event | payload | 何时发 |
| --- | --- | --- |
| `update-status` | `UpdateStatus` | 状态迁移时发；下载中的进度更新最多每 250 ms 发一次。 |

## 7. 磁盘文件（App 数据目录，不经 IPC，仅供对照）

- `config.json`、`secret`（0600）、`kclaude.env`（0600）、`kclaude.zsh` / `.bash` / `.ps1`、`kclaude.removed`（标记）。
- `gateway.pid`、`logs/gateway.log`（5 MB × 3 轮转）、`logs/requests/requests-YYYY-MM.jsonl`、`runtime/`。
- `updates/<version>.bin`（已下载的更新包）、`pending-update.json`（`{version, sha256, manualUrl, savedAt}`）、`upgrade-lock.json`（`{version, manualUrl}`：强制升级必须到达的版本，启动时据此在首次检查前就进入遮罩，运行版本达到后才清除 `upgradeRequired`）。
- 更新清单除 Tauri 的 static JSON 字段和 `minRequiredVersion` 外，还有可选的 `manualDownload: { darwin, windows }`（dmg / setup.exe 直链，强制升级下载失败时界面显示）。

## 8. 仅 debug 构建的开关

- `KIRO_GATEWAY_UPDATE_URL`：覆盖更新清单地址，只认回环地址（`127.0.0.1`、`localhost`、`::1`）且不带用户名密码，release 构建忽略。
- `KIRO_GATEWAY_SIDECAR`：覆盖网关可执行文件路径（debug 和 release 都认；指向不存在的文件时是 `start_failed`，不回退）。
- `kiro-gateway-desktop --selftest`：无窗口的验收运行，release 构建里没有这个参数。

## 变更记录

- 2026-10-23：默认端口 8000 → 6868；新增 `AppConfig.commandName` / `ConfigPatch.commandName`（默认 `cclaude`）与 `KclaudeStatus.commandName`。脚本文件名、标记段与 `foreign` 检测都按当前命令名；标记段固定为 `# >>> kiro-gateway-desktop command >>>` / `# <<< kiro-gateway-desktop command <<<`，旧版 `kclaude` 标记段视为本 App 的旧标记段，升级时就地替换。

- 2026-10-22：v1，scaffold 批次写定。相对任务描述的补充：`gateway_reveal_log`（mockup「打开完整日志」按钮需要）、`AppConfig.gatewayWanted` / `upgradeRequired`、`ConfigUpdateResult.restartRequired`、`EnvReport.kiroCliPath` / `claudePath` / `loginExpiresAt`、`KclaudeVerify.fix`（Windows 执行策略）、`request-log-appended` 跨日空推送、统一错误形状 `{ code, message }`。开机自启改由 Rust 持有（`config_update`），前端不注册 autostart 权限。
- 2026-10-23：联调批次。`EnvReport.loginCommand`；`RequestLogAppended.reset`；`gatewayWanted` 语义；`config_update` 的自启失败语义；`env_not_ready` 的 `message`；失败文案；"(未知)" 改为 "未知模型"；kclaude.env 格式与 `kclaude.removed`；磁盘文件与 debug 开关两节。
- 2026-10-23：修复批 B-update。`UpdateState` 新增 `installing`（启动时安装待更新）；写明下载 60 秒无进度超时和启动顺序。
