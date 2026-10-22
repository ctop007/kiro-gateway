# Spec Delta

## Purpose

桌面端自动检查本机是否具备运行 kclaude 的条件（kiro-cli 已安装且已登录、Claude Code 已安装），并对每个没通过的项目给出用户能照着做的修复提示。

## ADDED Requirements

### Requirement: 检测项与触发时机
桌面端 SHALL 在 App 启动时、用户点「重新检测」时、以及每次打开网关开关前，依次检测：kiro-cli 已安装、kiro-cli 已登录、登录有效、Claude Code 已安装。每项结果为「通过」「未通过」之一，并展示在主界面上。

#### Scenario: 全部通过
- **WHEN** 本机装有 kiro-cli 且已登录、装有 Claude Code，用户打开 App
- **THEN** 四项都显示通过，网关开关可用

#### Scenario: 用户修复后重新检测
- **WHEN** 某项未通过，用户按提示在终端完成操作后点「重新检测」
- **THEN** 该项变为通过，无需重启 App

### Requirement: kiro-cli 安装检测
桌面端 SHALL 在 PATH 及 kiro-cli 的默认安装位置中查找 kiro-cli；找不到时提示用户先安装 kiro-cli，并提供打开 kiro-cli 官方安装页面的按钮。

#### Scenario: GUI 启动时 PATH 不全
- **WHEN** macOS 上从 Finder 启动 App（不继承终端的 PATH），kiro-cli 装在 `~/.local/bin`
- **THEN** 检测仍然通过

#### Scenario: 未安装
- **WHEN** 本机没有 kiro-cli
- **THEN** 该项未通过，提示「未找到 kiro-cli，请先安装」并提供安装页按钮；后续两项登录检测显示为跳过

### Requirement: 登录检测
桌面端 SHALL 读取 kiro-cli 的登录数据库，确认其中存在登录凭证；数据库不存在或没有凭证时，提示用户在终端运行 `kiro-cli login`，并提供一键复制该命令的按钮。读取 MUST 只读，不修改数据库。

#### Scenario: 已安装未登录
- **WHEN** kiro-cli 已安装，但数据库里没有登录凭证
- **THEN** 该项未通过，显示「请在终端运行 kiro-cli login」和复制按钮

#### Scenario: 数据库被 kiro-cli 占用
- **WHEN** kiro-cli 正在运行并持有数据库
- **THEN** 检测仍能读出结果，不报错、不阻塞 kiro-cli

### Requirement: 登录有效性检测
登录凭证已过期且无法刷新时，桌面端 SHALL 判定为未通过并提示重新登录；凭证已过期但带有可刷新的凭证时 MUST 判定为通过（网关会自动刷新）。

#### Scenario: access token 过期但可刷新
- **WHEN** 凭证中的过期时间已过，但存在 refresh token
- **THEN** 该项通过

#### Scenario: 网关刷新失败
- **WHEN** 网关启动后刷新 token 返回 401 / 403
- **THEN** 网关状态显示「启动失败」，原因写明登录已失效，提示运行 `kiro-cli login`

### Requirement: Claude Code 安装检测
桌面端 SHALL 检测 `claude` 命令是否存在；不存在时提示安装 Claude Code 并提供官方安装说明链接。此项未通过时网关仍可启动（其他客户端可以用），但界面 MUST 说明 `kclaude` 暂时无法使用。

#### Scenario: 未安装 Claude Code
- **WHEN** 本机没有 `claude`
- **THEN** 该项未通过并给出安装链接，网关开关仍可用，kclaude 区域显示「需先安装 Claude Code」

### Requirement: kiro-cli 相关项未通过时禁止启动
kiro-cli 安装、登录、登录有效三项中任一项未通过时，网关开关 SHALL 不可打开，并在开关旁说明原因。

#### Scenario: 未登录时点开关
- **WHEN** 登录检测未通过，用户尝试打开开关
- **THEN** 开关不可操作，旁边显示「请先登录 kiro-cli」

### Requirement: 检测结果不泄露凭证
检测结果、界面文字和日志 MUST NOT 显示 token、refresh token 或 client secret 的任何部分；最多显示登录方式和过期时间。

#### Scenario: 展示登录信息
- **WHEN** 登录检测通过
- **THEN** 界面最多显示登录方式（如 AWS SSO）和过期时间，不显示任何凭证字符
