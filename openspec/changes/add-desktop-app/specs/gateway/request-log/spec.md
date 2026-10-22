# Spec Delta

## Purpose

网关为每个模型请求写一条结构化记录（时间、接口、模型、状态、耗时、credits），供桌面端或其他工具读取统计，不含对话内容。

## ADDED Requirements

### Requirement: 请求日志默认关闭、通过配置开启
网关 SHALL 只在配置项 `REQUEST_LOG_DIR` 设为非空目录路径时写请求日志；未设置时 MUST 不创建任何文件，行为与现在完全一致。

#### Scenario: 未配置时不写文件
- **WHEN** `REQUEST_LOG_DIR` 未设置，客户端发起一次 `/v1/messages` 请求
- **THEN** 网关不创建任何请求日志文件，响应与现在一致

#### Scenario: 配置后写入指定目录
- **WHEN** `REQUEST_LOG_DIR` 设为 `/tmp/x` 且目录不存在，客户端发起一次请求
- **THEN** 网关创建目录，并在当月文件末尾追加一行记录

### Requirement: 按月分文件
记录 SHALL 按请求开始时间所在的本机自然月写入 `requests-YYYY-MM.jsonl`；网关 MUST 每次写入时打开文件、追加、关闭，不长期持有文件句柄，以便其他程序可以安全地删除旧月份文件。

#### Scenario: 跨月运行
- **WHEN** 网关从 10 月 31 日一直运行到 11 月 1 日
- **THEN** 10 月 31 日开始的请求写入 `requests-YYYY-10.jsonl`，11 月 1 日开始的写入 `requests-YYYY-11.jsonl`

#### Scenario: 文件在运行中被删除
- **WHEN** 网关运行中，当月文件被外部删除，随后又有请求结束
- **THEN** 网关重新创建该文件并写入新记录，不报错

### Requirement: 每个模型请求结束时追加一行 JSON
网关 SHALL 对每个 `POST /v1/chat/completions` 与 `POST /v1/messages` 请求，在响应结束（非流式返回或流式最后一个事件发出、出错、客户端断开）后追加恰好一行 UTF-8 JSON。其他接口（`/v1/models`、`/health`、`/v1/messages/count_tokens`）MUST 不记录。

#### Scenario: 非流式成功
- **WHEN** 一次非流式 `/v1/messages` 请求成功返回
- **THEN** 文件新增一行，`status` 为 200，`stream` 为 false

#### Scenario: 流式成功
- **WHEN** 一次流式 `/v1/chat/completions` 请求完整结束
- **THEN** 文件新增一行，`status` 为 200，`stream` 为 true，`duration_ms` 覆盖到流结束为止

#### Scenario: 不记录的接口
- **WHEN** 客户端请求 `/v1/models` 或 `/health`
- **THEN** 文件不新增记录

### Requirement: 记录字段
每条记录 SHALL 包含：`ts`（请求开始时间，带时区的 ISO 8601）、`id`（与 `[Credit]` 日志中 `req=` 相同的响应 ID，拿不到时为 null）、`api`（`openai` 或 `anthropic`）、`model`（客户端请求的模型名）、`stream`、`status`（返回给客户端的 HTTP 状态码）、`outcome`（`ok` / `error` / `refused` / `client_disconnected`）、`duration_ms`、`credits`（数字，未产生计费时为 0）、`error`（失败时的错误摘要，最长 200 字符，成功时为 null）。

#### Scenario: credits 与计费日志一致
- **WHEN** 一次请求触发两次上游调用，计费事件合计 0.42 credits
- **THEN** 该请求的记录 `credits` 为 0.42，与同一 `id` 的 `[Credit]` 日志行一致

#### Scenario: 拒答
- **WHEN** Kiro 返回 `CONTENT_FILTERED`，网关向客户端返回拒答
- **THEN** 记录的 `outcome` 为 `refused`

### Requirement: 失败请求同样记录
网关 SHALL 为失败的请求写记录，包括上游 4xx / 5xx、网络错误、请求校验失败（422）、流式中途出错和客户端中途断开。

#### Scenario: 上游返回 402
- **WHEN** Kiro 返回 402 额度用尽，网关把错误返回给客户端
- **THEN** 记录 `status` 为 402，`outcome` 为 `error`，`error` 含错误摘要

#### Scenario: 流式中途出错
- **WHEN** 流式响应已发出部分内容后上游连接中断
- **THEN** 记录 `outcome` 为 `error`，`error` 非空，`credits` 为出错前已累计的值

#### Scenario: 客户端断开
- **WHEN** 流式响应进行中客户端关闭连接
- **THEN** 记录 `outcome` 为 `client_disconnected`

### Requirement: 四条路径行为一致
OpenAI 与 Anthropic 两套接口、流式与非流式两条路径 SHALL 产生字段与语义相同的记录。

#### Scenario: 同一请求在四条路径下
- **WHEN** 分别以 OpenAI 流式、OpenAI 非流式、Anthropic 流式、Anthropic 非流式发起请求，且上游返回相同的计费事件
- **THEN** 四条记录除 `api`、`stream`、`id`、`ts`、`duration_ms` 外字段相同

### Requirement: 不记录敏感内容
记录 MUST NOT 包含消息内容、系统提示、工具参数、模型输出、请求头、access token、refresh token 或 `PROXY_API_KEY`。

#### Scenario: 错误摘要里带有请求内容
- **WHEN** 上游错误信息中回显了部分用户消息
- **THEN** 记录的 `error` 字段被截断到 200 字符以内，且记录中不出现任何其他请求内容字段

### Requirement: 写日志失败不影响请求
写请求日志失败（磁盘满、无权限、路径是目录）时，网关 SHALL 照常返回响应，只在应用日志中记录一次警告。

#### Scenario: 文件无写权限
- **WHEN** `REQUEST_LOG_DIR` 指向一个没有写权限的目录，客户端发起请求
- **THEN** 客户端收到正常响应，应用日志出现写入失败的警告

### Requirement: 并发请求的记录不交错
多个请求同时结束时，每条记录 SHALL 是完整独立的一行，不会被另一条记录截断或混写。

#### Scenario: 并发 20 个请求
- **WHEN** 20 个请求并发完成
- **THEN** 文件新增 20 行，每行都能独立解析为 JSON
