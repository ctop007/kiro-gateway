# Spec Delta

## Purpose

桌面端的请求日志页，让用户看到每次请求用了哪个模型、成功与否、耗时，以及花了多少 Kiro credits，并汇总今日和本月的 credits 用量。

## ADDED Requirements

### Requirement: 请求列表
日志页 SHALL 按时间倒序列出请求记录，每行显示：时间（本机时区）、接口（OpenAI / Anthropic）、模型、是否流式、结果（成功 / 失败 / 拒答 / 客户端断开）、耗时、credits。

#### Scenario: 有记录
- **WHEN** 网关已处理过若干请求，用户打开日志页
- **THEN** 最新的请求在最上面，每行字段齐全

#### Scenario: 没有记录
- **WHEN** 还没有任何请求
- **THEN** 显示空状态「还没有请求，在终端运行 kclaude 试试」

### Requirement: 实时更新
网关运行期间，日志页 SHALL 在新请求结束后 3 秒内显示新记录，无需手动刷新。

#### Scenario: 正在使用 kclaude
- **WHEN** 日志页打开，用户在终端用 kclaude 发了一条消息
- **THEN** 该请求结束后 3 秒内出现在列表顶部

### Requirement: 失败详情
结果为失败或拒答的记录 SHALL 可以展开查看错误摘要，并对常见错误（额度用尽 402、上下文超长、登录失效 401 / 403、拒答）给出一句处理建议。

#### Scenario: 额度用尽
- **WHEN** 用户展开一条 402 的记录
- **THEN** 显示错误摘要和「Kiro 本月额度已用完，可在 kiro-cli 中查看用量和重置时间」

### Requirement: credits 汇总
日志页顶部 SHALL 显示今日和本月的 credits 合计以及请求数，按本机时区的自然日、自然月统计；数值标注单位为 credits，MUST NOT 换算或标注为金额。

#### Scenario: 跨天
- **WHEN** 本机时间过了 0 点
- **THEN** 「今日」合计从 0 开始重新累计，「本月」合计不变（除非同时跨月）

#### Scenario: 失败请求也有 credits
- **WHEN** 一次请求失败但上游已计费 0.1 credits
- **THEN** 这 0.1 计入今日和本月合计

### Requirement: 按模型分组
日志页 SHALL 提供本月按模型分组的 credits 与请求数统计。

#### Scenario: 用了两个模型
- **WHEN** 本月用了 opus 和 haiku
- **THEN** 分组统计列出两行，各自的 credits 之和等于本月合计

### Requirement: 筛选
日志页 SHALL 支持按结果（全部 / 仅失败）和按模型筛选列表。

#### Scenario: 只看失败
- **WHEN** 用户选择「仅失败」
- **THEN** 列表只显示结果不是成功的记录，汇总数字不受筛选影响

### Requirement: 历史保留与清理
请求记录 SHALL 至少保留本月和上个月的完整数据，以保证本月统计准确；更早的记录可以被自动清理。界面 SHALL 提供「清空日志」操作，清空前需要确认。

#### Scenario: 长期使用
- **WHEN** App 连续使用半年
- **THEN** 日志页打开时间不明显变慢，本月统计依然准确

#### Scenario: 清空
- **WHEN** 用户点「清空日志」并确认
- **THEN** 列表和统计都归零，后续请求照常记录

### Requirement: 损坏的记录不影响展示
日志文件中无法解析的行（例如断电导致的半行）SHALL 被跳过，其余记录照常展示和统计。

#### Scenario: 最后一行不完整
- **WHEN** 日志文件最后一行只写了一半
- **THEN** 日志页正常显示其他记录，统计不含这半行，不报错
