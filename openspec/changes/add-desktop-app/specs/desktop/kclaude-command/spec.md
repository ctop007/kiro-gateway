# Spec Delta

## Purpose

桌面端在用户的终端里装上 `kclaude` 命令：它和 `claude` 用法相同，但会通过本机的 KiroGateway 使用 Kiro 模型，而原来的 `claude` 和它的配置完全不受影响。

## ADDED Requirements

### Requirement: 启动时自动安装
App 启动时 SHALL 检查 `kclaude` 是否已安装，未安装就自动安装，并在界面上显示结果；安装后新打开的终端 MUST 能直接使用 `kclaude`。

#### Scenario: 首次打开 App
- **WHEN** 用户第一次打开 App
- **THEN** `kclaude` 被安装，界面显示「已安装，新开一个终端即可使用」

#### Scenario: 已安装且是最新
- **WHEN** `kclaude` 已由本 App 安装且内容是最新的
- **THEN** 不做任何写入

### Requirement: 不影响现有 claude
安装 `kclaude` MUST NOT 修改 `claude` 命令本身、Claude Code 的配置文件（`~/.claude/` 下的文件）、以及全局的 `ANTHROPIC_*` 环境变量；在终端直接运行 `claude` 的行为 SHALL 与安装前一致。

#### Scenario: 安装后运行 claude
- **WHEN** `kclaude` 安装完成后，用户在新终端运行 `claude`
- **THEN** `claude` 使用用户原来的账号和配置，不连本机网关

#### Scenario: 运行 kclaude
- **WHEN** 网关运行中，用户运行 `kclaude`
- **THEN** Claude Code 连接本机网关，使用 App 保存的访问密钥，opus / sonnet / haiku 映射到 App 设置的 Kiro 模型

### Requirement: macOS 安装方式
macOS 上，桌面端 SHALL 在用户的 zsh 启动文件中追加一段带开始和结束标记的内容来提供 `kclaude`；用户的默认 shell 是 bash 时 MUST 写入 bash 启动文件。写入前 SHALL 备份原文件。

#### Scenario: 已有手动安装的 kclaude
- **WHEN** `~/.zshrc` 里已有用户自己加的 kclaude（例如 source `scripts/kclaude.zsh`）
- **THEN** App 不重复添加，界面显示「检测到已有 kclaude，未改动」，并提供「改由 App 管理」的按钮

#### Scenario: 备份
- **WHEN** App 写入 `~/.zshrc`
- **THEN** 写入前生成带时间戳的备份文件；备份失败时不写入，并提示失败原因

### Requirement: Windows 安装方式
Windows 上，桌面端 SHALL 在当前用户的 PowerShell profile 中追加一段带标记的 `kclaude` 函数（profile 不存在时创建）；写入前 SHALL 备份已有 profile。

#### Scenario: Windows PowerShell 与 PowerShell 7 都存在
- **WHEN** 用户同时装有 Windows PowerShell 5 和 PowerShell 7
- **THEN** 两者的当前用户 profile 都能使用 `kclaude`

#### Scenario: 执行策略禁止运行脚本
- **WHEN** PowerShell 执行策略不允许加载 profile
- **THEN** 界面提示执行策略问题，并给出可复制的修复命令，不擅自修改执行策略

### Requirement: 网关未运行时 kclaude 给出明确提示
运行 `kclaude` 时如果网关未运行，`kclaude` SHALL 直接退出并提示「KiroGateway 未运行，请在 Kiro Gateway App 里打开开关」，MUST NOT 自行在后台启动网关。

#### Scenario: 开关关闭时运行 kclaude
- **WHEN** 网关已停止，用户运行 `kclaude`
- **THEN** 终端显示上述提示，命令以非零状态退出，不启动 Claude Code

### Requirement: 密钥与端口变化后无需重装
`kclaude` SHALL 在每次运行时从 App 保存的配置读取端口、访问密钥和模型映射；用户在 App 里改了端口或模型后，已打开的终端里运行 `kclaude` MUST 使用新值。

#### Scenario: 修改端口
- **WHEN** 用户在 App 里把端口从 8000 改到 8100 并重启网关
- **THEN** 在原来已打开的终端里运行 `kclaude` 连接到 8100

### Requirement: 卸载 kclaude
界面 SHALL 提供「移除 kclaude」操作，删除 App 写入的那一段（按标记定位），保留文件中其他内容不变。

#### Scenario: 移除
- **WHEN** 用户点「移除 kclaude」
- **THEN** 启动文件中只有带标记的那一段被删除，其余内容逐字节不变，新终端中 `kclaude` 不再可用

### Requirement: 模型映射可设置
界面 SHALL 允许设置 opus、sonnet、haiku 三档各自对应的 Kiro 模型，默认值与 `scripts/kclaude.zsh` 一致；可选模型列表 MUST 来自网关的 `/v1/models`。

#### Scenario: 网关未运行时设置模型
- **WHEN** 网关未运行，用户打开模型设置
- **THEN** 显示当前已保存的映射，模型下拉不可用并提示「启动网关后可选择模型」
