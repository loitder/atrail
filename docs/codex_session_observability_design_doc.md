# Codex Session Observatory 设计文档 v0.1

## 1. 背景

当前目标是从零实现一个面向 Codex 用户的 session 行为观测系统，用于查看每一次 Codex session 的行为链路、token 使用、阶段耗时、工具调用、错误与异常，并提供可视化分析能力。

这个系统不是传统 APM，也不是简单的 token 用量统计工具。它更接近“AI Coding Agent 行为追踪平台”，核心关注点是：

- 一个 session 里发生了什么；
- 每一轮用户输入、模型推理、工具调用、文件读写、命令执行之间的因果关系；
- token 和成本消耗在哪里；
- 时间耗在哪里；
- 哪些行为导致失败、重试、等待审批或长时间卡顿；
- 如何按项目、模型、任务类型、时间范围聚合分析。

## 2. 目标

### 2.1 核心目标

实现一个本地优先、可扩展到团队级的 Codex session 观测系统。

系统需要支持：

1. **Session 行为链路**
   - 展示一次 session 的完整 timeline。
   - 支持 turn、model request、stream event、tool call、tool result、approval、file edit、shell command 等节点。
   - 能看到节点之间的父子关系和耗时。

2. **Token 使用统计**
   - 按 session 统计 input token、output token、cached token、reasoning token。
   - 按 turn、model、项目、时间聚合。
   - 支持用量趋势、成本估算、异常高 token session 发现。

3. **阶段耗时统计**
   - session 总耗时。
   - 每轮 turn 耗时。
   - API 请求耗时。
   - 首 token 时间 TTFT。
   - 首工具调用时间 TTFM。
   - 工具调用耗时。
   - 用户等待审批耗时。
   - 命令执行耗时。

4. **可视化**
   - Session timeline。
   - Trace tree。
   - Token/Cost dashboard。
   - Tool usage dashboard。
   - Latency histogram。
   - Slow session/failed session 列表。
   - 项目维度、模型维度、时间维度分析。

5. **本地优先**
   - 默认读取本机 Codex session 数据。
   - 默认不上传用户 prompt、代码片段、命令输出。
   - 支持敏感字段脱敏。

6. **可扩展**
   - 后续可接 OpenTelemetry。
   - 后续可支持 Claude Code、Cursor、Gemini CLI、Aider 等其他 coding agent。

## 3. 非目标

第一阶段不做：

- 不做通用 LLM Gateway。
- 不代理 Codex API 请求。
- 不改造 Codex 客户端。
- 不实现完整 APM 系统。
- 不强依赖云端服务。
- 不默认采集源码正文、完整 prompt、完整命令输出。
- 不直接计算账单，只做成本估算。

## 4. 设计原则

### 4.1 本地优先

Codex session 数据通常包含用户 prompt、项目路径、文件名、命令、错误日志、部分代码上下文，因此系统默认应该以本地单机模式运行。

默认部署形态：

```text
Codex 本地 session 文件
        ↓
本地采集器 / 解析器
        ↓
本地 SQLite / DuckDB
        ↓
本地 Web UI
```

### 4.2 双采集入口

从零实现时，不建议只依赖一种数据源。推荐同时设计两种入口：

```text
入口 A：Codex session JSONL
入口 B：OpenTelemetry OTLP
```

原因：

- JSONL 更适合个人本地历史分析。
- OTel 更适合实时、团队级、跨机器聚合。
- JSONL 里可能有更丰富的本地上下文。
- OTel 的字段更标准，适合长期演进。

### 4.3 事件溯源

不要一开始只存聚合指标。应该先把原始行为事件标准化，再从事件派生出 trace、指标、报表。

推荐模型：

```text
Raw Event → Normalized Event → Span / Metric / Snapshot → Dashboard
```

### 4.4 可观测性数据分层

系统内部建议分为四层数据：

1. **Raw Layer**：原始 JSONL / OTel payload。
2. **Event Layer**：标准化后的 agent 行为事件。
3. **Trace Layer**：session、turn、tool call 等 span 树。
4. **Metric Layer**：token、耗时、错误、工具使用等聚合指标。

### 4.5 隐私默认安全

默认只保留：

- session id；
- 时间戳；
- 项目路径 hash；
- 模型名；
- token 数；
- 工具类型；
- 命令类型；
- 文件路径 hash 或相对路径；
- 错误类型；
- 耗时；
- 状态。

完整 prompt、代码 diff、命令输出应作为可选能力，并且需要用户显式开启。

## 5. 用户场景

### 5.1 个人用户

用户想知道：

- 今天 Codex 用了多少 token；
- 哪些 session 最贵；
- 哪个任务最慢；
- Codex 到底执行了哪些命令；
- 为什么某个 session 卡住；
- 哪些工具调用失败最多；
- 某个项目最近 Codex 使用情况如何。

### 5.2 团队管理员

管理员想知道：

- 团队整体 Codex 使用趋势；
- 哪些项目消耗最多；
- 哪些模型使用最多；
- 平均 turn latency；
- 高成本 session；
- 高失败率工具；
- 是否存在异常命令行为；
- 是否存在敏感路径或危险操作。

### 5.3 平台工程师

平台工程师想知道：

- agent 行为链路是否完整；
- OTel 指标是否正常；
- 某类任务的瓶颈在哪里；
- MCP 工具是否拖慢 session；
- Codex 版本升级前后性能变化。

## 6. 核心概念模型

### 6.1 Session

Session 是一次 Codex 对话或任务线程。

字段示例：

```text
session_id
source
project_id
project_path_hash
started_at
ended_at
status
model
codex_version
auth_mode
originator
metadata
```

### 6.2 Turn

Turn 是一次用户输入到 Codex 产出结果的完整轮次。

字段示例：

```text
turn_id
session_id
turn_index
user_message_hash
started_at
ended_at
duration_ms
status
input_tokens
output_tokens
cached_tokens
reasoning_tokens
```

### 6.3 Span

Span 是行为链路中的一个可计时节点。

常见 span 类型：

```text
session
turn
model_request
stream_response
tool_decision
tool_call
tool_result
shell_command
file_read
file_write
patch_apply
approval_wait
mcp_call
error
```

字段示例：

```text
span_id
trace_id
parent_span_id
session_id
turn_id
span_type
name
started_at
ended_at
duration_ms
status
attributes
```

### 6.4 Event

Event 是不可变行为事件，用于构建 span 和指标。

字段示例：

```text
event_id
session_id
turn_id
span_id
event_type
timestamp
attributes
raw_ref
```

### 6.5 Metric

Metric 是从 event/span 派生出来的统计数据。

典型指标：

```text
session.count
session.duration_ms
turn.duration_ms
model.request.duration_ms
model.ttft_ms
model.ttfm_ms
token.input
token.output
token.cached
token.reasoning
tool.call.count
tool.call.duration_ms
tool.call.error_count
approval.wait.duration_ms
shell.command.duration_ms
file.edit.count
error.count
```

## 7. 总体架构

```text
┌──────────────────────────────┐
│        Codex CLI / App        │
└───────────────┬──────────────┘
                │
        ┌───────┴────────┐
        │                │
        ▼                ▼
┌──────────────┐   ┌─────────────────┐
│ Session JSONL │   │ OpenTelemetry   │
│ ~/.codex/...  │   │ OTLP Exporter   │
└───────┬──────┘   └────────┬────────┘
        │                   │
        ▼                   ▼
┌─────────────────────────────────────┐
│         Collector / Ingestor         │
│  - JSONL watcher                     │
│  - OTLP receiver                     │
│  - parser                            │
│  - normalizer                        │
│  - redactor                          │
└──────────────────┬──────────────────┘
                   ▼
┌─────────────────────────────────────┐
│            Storage Layer             │
│  - SQLite / DuckDB                   │
│  - raw_events                        │
│  - sessions                          │
│  - turns                             │
│  - spans                             │
│  - metrics                           │
└──────────────────┬──────────────────┘
                   ▼
┌─────────────────────────────────────┐
│              API Server              │
│  - session query                     │
│  - trace query                       │
│  - metric query                      │
│  - search                            │
│  - export                            │
└──────────────────┬──────────────────┘
                   ▼
┌─────────────────────────────────────┐
│               Web UI                 │
│  - dashboard                         │
│  - session list                      │
│  - session detail                    │
│  - timeline                          │
│  - trace tree                        │
│  - token usage                       │
│  - latency analysis                  │
└─────────────────────────────────────┘
```

## 8. 模块设计

## 8.1 Collector

Collector 负责采集数据。

### 8.1.1 JSONL Watcher

功能：

- 监听 Codex session 目录。
- 支持历史导入。
- 支持增量 tail。
- 支持断点续读。
- 支持重复事件去重。

推荐能力：

```text
scan
watch
tail
reindex
repair
```

运行方式示例：

```bash
atrail ingest --source ~/.codex/sessions
atrail watch --source ~/.codex/sessions
```

断点记录：

```text
file_path
inode
offset
last_event_time
last_imported_at
checksum
```

### 8.1.2 OTLP Receiver

功能：

- 接收 Codex 通过 OTLP 发出的 logs、metrics、traces。
- 将 OTel span/log/metric 转成内部标准事件。
- 保留 trace_id、span_id、parent_span_id。

部署方式：

```text
Codex OTel Exporter → atrail otlp receiver → storage
```

或者：

```text
Codex OTel Exporter → OTel Collector → atrail ingest endpoint
```

### 8.1.3 Normalizer

不同来源的数据字段不一定一致，Normalizer 负责归一化。

输入：

```text
Codex JSONL event
OTel log
OTel span
OTel metric
```

输出：

```text
NormalizedEvent
NormalizedSpan
NormalizedMetric
```

核心要求：

- 同一 session 事件必须归到同一 session_id。
- 能从事件顺序推断 turn。
- 能从工具调用 start/end 事件合成 span。
- 能从 token usage 事件合成 token metric。
- 能从错误事件合成 error metric。

### 8.1.4 Redactor

Redactor 负责脱敏。

脱敏策略：

```text
none          不脱敏，仅本地使用
safe          默认策略，hash 项目路径、隐藏 prompt 正文
strict        删除 prompt、命令输出、文件路径，仅保留类型和耗时
custom        用户自定义规则
```

可脱敏字段：

- prompt；
- response；
- command；
- command output；
- file path；
- git diff；
- environment variable；
- URL；
- token；
- API key；
- email；
- IP；
- internal domain。

## 8.2 Storage Layer

第一阶段建议用 SQLite。

理由：

- 本地优先；
- 部署简单；
- 查询 session/timeline 足够；
- 方便用户备份；
- 后续可迁移 PostgreSQL。

如果主要做大规模分析，可以引入 DuckDB 作为分析引擎。

推荐组合：

```text
SQLite：事务型存储、session 明细、索引查询
DuckDB：离线分析、聚合报表、导出 parquet
```

### 8.2.1 表结构

#### sessions

```sql
CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  source TEXT NOT NULL,
  project_id TEXT,
  project_path_hash TEXT,
  started_at DATETIME,
  ended_at DATETIME,
  duration_ms INTEGER,
  status TEXT,
  model TEXT,
  codex_version TEXT,
  auth_mode TEXT,
  originator TEXT,
  raw_file_path TEXT,
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

#### turns

```sql
CREATE TABLE turns (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  turn_index INTEGER,
  started_at DATETIME,
  ended_at DATETIME,
  duration_ms INTEGER,
  status TEXT,
  user_message_hash TEXT,
  input_tokens INTEGER DEFAULT 0,
  output_tokens INTEGER DEFAULT 0,
  cached_tokens INTEGER DEFAULT 0,
  reasoning_tokens INTEGER DEFAULT 0,
  tool_call_count INTEGER DEFAULT 0,
  error_count INTEGER DEFAULT 0,
  FOREIGN KEY(session_id) REFERENCES sessions(id)
);
```

#### spans

```sql
CREATE TABLE spans (
  id TEXT PRIMARY KEY,
  trace_id TEXT NOT NULL,
  parent_span_id TEXT,
  session_id TEXT NOT NULL,
  turn_id TEXT,
  span_type TEXT NOT NULL,
  name TEXT,
  started_at DATETIME,
  ended_at DATETIME,
  duration_ms INTEGER,
  status TEXT,
  attributes_json TEXT,
  FOREIGN KEY(session_id) REFERENCES sessions(id),
  FOREIGN KEY(turn_id) REFERENCES turns(id)
);
```

#### events

```sql
CREATE TABLE events (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  turn_id TEXT,
  span_id TEXT,
  event_type TEXT NOT NULL,
  timestamp DATETIME NOT NULL,
  attributes_json TEXT,
  raw_event_json TEXT,
  raw_source TEXT,
  raw_ref TEXT,
  FOREIGN KEY(session_id) REFERENCES sessions(id)
);
```

#### token_usage

```sql
CREATE TABLE token_usage (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  turn_id TEXT,
  span_id TEXT,
  model TEXT,
  input_tokens INTEGER DEFAULT 0,
  output_tokens INTEGER DEFAULT 0,
  cached_tokens INTEGER DEFAULT 0,
  reasoning_tokens INTEGER DEFAULT 0,
  total_tokens INTEGER DEFAULT 0,
  estimated_cost_usd REAL,
  timestamp DATETIME,
  FOREIGN KEY(session_id) REFERENCES sessions(id)
);
```

#### tool_calls

```sql
CREATE TABLE tool_calls (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  turn_id TEXT,
  span_id TEXT,
  tool_type TEXT,
  tool_name TEXT,
  started_at DATETIME,
  ended_at DATETIME,
  duration_ms INTEGER,
  status TEXT,
  exit_code INTEGER,
  input_hash TEXT,
  output_hash TEXT,
  error_type TEXT,
  attributes_json TEXT,
  FOREIGN KEY(session_id) REFERENCES sessions(id)
);
```

#### artifacts

```sql
CREATE TABLE artifacts (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  turn_id TEXT,
  artifact_type TEXT,
  path_hash TEXT,
  path_display TEXT,
  operation TEXT,
  size_bytes INTEGER,
  attributes_json TEXT,
  timestamp DATETIME,
  FOREIGN KEY(session_id) REFERENCES sessions(id)
);
```

### 8.2.2 索引

```sql
CREATE INDEX idx_sessions_started_at ON sessions(started_at);
CREATE INDEX idx_sessions_project ON sessions(project_id);
CREATE INDEX idx_turns_session ON turns(session_id, turn_index);
CREATE INDEX idx_spans_session ON spans(session_id, started_at);
CREATE INDEX idx_spans_trace ON spans(trace_id, parent_span_id);
CREATE INDEX idx_events_session_time ON events(session_id, timestamp);
CREATE INDEX idx_tool_calls_session ON tool_calls(session_id, started_at);
CREATE INDEX idx_token_usage_session ON token_usage(session_id, timestamp);
```

## 8.3 API Server

第一阶段建议使用 Go 或 Node.js 实现。

如果你的目标是轻量、单二进制、方便分发，推荐 Go。

### 8.3.1 API 分类

#### Session API

```http
GET /api/sessions
GET /api/sessions/:id
GET /api/sessions/:id/timeline
GET /api/sessions/:id/trace
GET /api/sessions/:id/turns
GET /api/sessions/:id/tools
GET /api/sessions/:id/tokens
GET /api/sessions/:id/errors
```

#### Metric API

```http
GET /api/metrics/summary
GET /api/metrics/tokens
GET /api/metrics/latency
GET /api/metrics/tools
GET /api/metrics/errors
GET /api/metrics/projects
GET /api/metrics/models
```

#### Search API

```http
GET /api/search?q=...
GET /api/search/sessions?q=...
GET /api/search/tools?q=...
```

#### Admin API

```http
POST /api/admin/reindex
POST /api/admin/rescan
POST /api/admin/redact
GET /api/admin/status
```

### 8.3.2 查询示例

获取慢 session：

```sql
SELECT id, project_id, model, duration_ms, started_at
FROM sessions
WHERE duration_ms > 300000
ORDER BY duration_ms DESC
LIMIT 50;
```

获取高 token session：

```sql
SELECT session_id,
       SUM(input_tokens) AS input_tokens,
       SUM(output_tokens) AS output_tokens,
       SUM(cached_tokens) AS cached_tokens,
       SUM(reasoning_tokens) AS reasoning_tokens,
       SUM(total_tokens) AS total_tokens
FROM token_usage
GROUP BY session_id
ORDER BY total_tokens DESC
LIMIT 50;
```

获取工具耗时排行：

```sql
SELECT tool_name,
       COUNT(*) AS call_count,
       AVG(duration_ms) AS avg_duration_ms,
       MAX(duration_ms) AS max_duration_ms
FROM tool_calls
GROUP BY tool_name
ORDER BY avg_duration_ms DESC;
```

## 8.4 Web UI

### 8.4.1 页面结构

```text
/                         总览 Dashboard
/sessions                 Session 列表
/sessions/:id             Session 详情
/sessions/:id/timeline    Timeline 视图
/sessions/:id/trace       Trace Tree 视图
/sessions/:id/tokens      Token 明细
/sessions/:id/tools       Tool 调用明细
/metrics/tokens           Token 趋势
/metrics/latency          耗时分析
/metrics/tools            工具分析
/metrics/errors           错误分析
/settings                 设置
```

### 8.4.2 Dashboard

核心卡片：

```text
今日 session 数
今日 turn 数
今日 token 总量
今日估算成本
平均 session 耗时
平均 turn 耗时
工具调用数
失败工具调用数
错误 session 数
```

核心图表：

```text
Token 趋势折线图
Session 数趋势柱状图
模型使用占比
项目使用排行
工具调用排行
Latency P50/P90/P99
Top slow sessions
Top expensive sessions
Top failed tools
```

### 8.4.3 Session 列表

字段：

```text
开始时间
项目
模型
状态
耗时
token 总量
工具调用数
错误数
最后活动时间
```

筛选：

```text
时间范围
项目
模型
状态
是否失败
是否高 token
是否慢 session
工具类型
```

### 8.4.4 Session 详情

布局建议：

```text
顶部：session 基本信息
左侧：turn 列表 / timeline
右侧：详情面板
底部：token、耗时、工具调用统计
```

Session 详情需要回答四个问题：

1. 用户让 Codex 做了什么？
2. Codex 分了几步做？
3. 哪一步最耗时、最耗 token？
4. 哪一步失败或需要人工介入？

### 8.4.5 Timeline 视图

Timeline 是最重要的页面。

建议节点类型：

```text
User Message
Model Request
Model Stream
Tool Decision
Shell Command
File Read
File Write
Patch Apply
Approval Wait
MCP Tool
Error
Final Response
```

每个节点显示：

```text
时间
类型
名称
耗时
token
状态
摘要
可展开 attributes
```

视觉建议：

```text
绿色：成功
黄色：等待/审批
红色：失败
蓝色：模型调用
紫色：工具调用
灰色：文件/系统事件
```

### 8.4.6 Trace Tree 视图

Trace Tree 用来展示父子关系和并行关系。

示例：

```text
session: abc
└── turn: 1
    ├── model_request: gpt-5.5-codex
    │   ├── stream_response
    │   └── tool_decision
    ├── tool_call: shell.exec
    │   └── command: npm test
    ├── tool_call: file.read
    ├── tool_call: patch.apply
    └── model_request: final response
```

### 8.4.7 Token 视图

展示维度：

```text
按 session
按 turn
按 model
按 project
按 date
按 tool 前后阶段
```

指标：

```text
input tokens
output tokens
cached tokens
reasoning tokens
total tokens
estimated cost
cache hit ratio
```

### 8.4.8 Latency 视图

展示维度：

```text
session duration
turn duration
model request duration
TTFT
TTFM
tool duration
approval wait duration
shell command duration
```

需要支持：

```text
P50/P90/P99
平均值
最大值
慢 session 列表
异常点钻取
```

## 9. 指标口径

### 9.1 Session Duration

```text
session_duration = session.end_time - session.start_time
```

如果 session 未正常结束：

```text
session_duration = last_event_time - first_event_time
```

### 9.2 Turn Duration

```text
turn_duration = turn.end_time - turn.start_time
```

### 9.3 Model Request Duration

```text
model_request_duration = model_response_end - model_request_start
```

### 9.4 TTFT

Time To First Token。

```text
ttft = first_stream_token_time - model_request_start
```

如果没有 stream token 事件，则置为空。

### 9.5 TTFM

Time To First Meaningful Action，也可以理解为 Time To First Model Tool Decision。

```text
ttfm = first_tool_decision_time - turn_start_time
```

如果该 turn 没有工具调用，则置为空。

### 9.6 Tool Duration

```text
tool_duration = tool_result_time - tool_call_start_time
```

### 9.7 Approval Wait Duration

```text
approval_wait_duration = approval_resolved_time - approval_requested_time
```

这个指标很重要，因为它区分“Codex 慢”和“用户审批慢”。

### 9.8 Token Total

```text
total_tokens = input_tokens + output_tokens
```

也可以额外提供 billable_tokens：

```text
uncached_input_tokens = max(input_tokens - cached_tokens, 0)
billable_tokens = uncached_input_tokens + cached_tokens * cache_discount_ratio + output_tokens
```

注意：第一阶段成本估算只作为参考，不作为账单依据。

## 10. 标准事件设计

### 10.1 EventType

```text
session.started
session.ended
turn.started
turn.ended
model.request.started
model.request.ended
model.stream.started
model.stream.token
model.stream.ended
model.tool_decision
tool.call.started
tool.call.ended
tool.call.failed
shell.command.started
shell.command.ended
file.read
file.write
patch.apply.started
patch.apply.ended
approval.requested
approval.resolved
mcp.call.started
mcp.call.ended
token.usage
error
```

### 10.2 NormalizedEvent 示例

```json
{
  "event_id": "evt_01",
  "session_id": "ses_01",
  "turn_id": "turn_01",
  "event_type": "tool.call.started",
  "timestamp": "2026-05-18T10:00:00.000Z",
  "attributes": {
    "tool_type": "shell",
    "tool_name": "exec",
    "command_hash": "sha256:...",
    "command_preview": "npm test",
    "cwd_hash": "sha256:..."
  },
  "raw_source": "codex_jsonl"
}
```

### 10.3 Span 合成规则

#### session span

```text
start: session.started 或首个事件
end: session.ended 或最后事件
```

#### turn span

```text
start: turn.started 或 user message event
end: turn.ended 或 final response event
```

#### model_request span

```text
start: model.request.started
end: model.request.ended / stream.ended / error
```

#### tool_call span

```text
start: tool.call.started
end: tool.call.ended / tool.call.failed
```

如果缺少 end 事件：

```text
status = unknown
end = next sibling start time 或 session end time
```

## 11. 成本估算设计

### 11.1 价格配置

价格不要写死在代码里，使用配置文件。

```yaml
models:
  gpt-5.5-codex:
    input_per_1m: 0
    cached_input_per_1m: 0
    output_per_1m: 0
    reasoning_per_1m: 0
  gpt-5.4-mini:
    input_per_1m: 0
    cached_input_per_1m: 0
    output_per_1m: 0
    reasoning_per_1m: 0
```

用户可以手动维护。

### 11.2 成本计算

```text
uncached_input_tokens = max(input_tokens - cached_tokens, 0)

cost = uncached_input_tokens / 1_000_000 * input_price
     + cached_tokens / 1_000_000 * cached_input_price
     + output_tokens / 1_000_000 * output_price
```

### 11.3 成本可信度

每条成本数据需要有 confidence：

```text
exact       来源包含完整 token usage 和官方价格配置
estimated   token 完整，但价格由用户配置
partial     token 不完整
unknown     无法计算
```

## 12. 隐私与安全

### 12.1 本地数据风险

Codex session 数据可能包含：

- 用户 prompt；
- 项目名称；
- 文件路径；
- 源码片段；
- shell 命令；
- 命令输出；
- 内网域名；
- 环境变量；
- token 或密钥；
- 错误堆栈。

因此系统默认不得将原始数据上传到云端。

### 12.2 脱敏默认值

默认策略 safe：

```text
prompt: hash + 前 80 字摘要可选
response: 不保存全文
file path: 相对路径可选，绝对路径 hash
command: 保存 command name，参数 hash
command output: 默认不保存
error: 保存错误类型和摘要
```

### 12.3 敏感信息检测

内置规则：

```text
OpenAI key
GitHub token
AWS key
Bearer token
JWT
email
IP
URL with query
password-like env
private key block
```

### 12.4 数据保留策略

配置项：

```yaml
retention:
  raw_events_days: 7
  normalized_events_days: 90
  metrics_days: 365
  keep_redacted_only: true
```

### 12.5 权限模型

本地版本：

```text
无需登录
仅监听本地 127.0.0.1
可配置 basic auth
```

团队版本：

```text
用户登录
项目权限
session 权限
敏感字段按角色展示
审计日志
```

## 13. 技术选型

### 13.1 推荐 MVP 技术栈

```text
后端：Go
前端：React + Vite + TypeScript
数据库：SQLite
图表：ECharts 或 Recharts
Trace UI：自研树形 + 时间轴
打包：单二进制 + embedded web assets
```

### 13.2 为什么推荐 Go

- 单二进制分发方便；
- 文件监听、SQLite、HTTP Server、OTLP receiver 都成熟；
- 适合做 CLI；
- 跨平台体验好；
- 与命令行生态匹配。

### 13.3 目录结构

```text
codex-session-observatory/
  cmd/
    atrail/
      main.go
  internal/
    collector/
      jsonl_watcher.go
      otlp_receiver.go
    parser/
      codex_jsonl.go
      otel.go
    normalizer/
      event.go
      span_builder.go
    redactor/
      redactor.go
      rules.go
    storage/
      sqlite.go
      migrations/
    api/
      server.go
      sessions.go
      metrics.go
    config/
      config.go
  web/
    src/
      pages/
      components/
      charts/
      api/
  docs/
  examples/
```

### 13.4 CLI 命令设计

```bash
atrail init
atrail ingest --source ~/.codex/sessions
atrail watch --source ~/.codex/sessions
atrail serve --db ~/.atrail/atrail.db
atrail status
atrail reindex
atrail export --format json
atrail doctor
```

### 13.5 配置文件

```yaml
server:
  host: 127.0.0.1
  port: 4319

storage:
  path: ~/.atrail/atrail.db

sources:
  codex_sessions:
    enabled: true
    path: ~/.codex/sessions
  otlp:
    enabled: false
    grpc_port: 4317
    http_port: 4318

privacy:
  mode: safe
  save_prompt: false
  save_response: false
  save_command_output: false
  hash_file_paths: true

metrics:
  cost_estimation: true
  price_file: ~/.atrail/prices.yaml
```

## 14. MVP 范围

### 14.1 MVP 必做

1. 本地扫描 Codex session JSONL。
2. 增量导入 SQLite。
3. 标准化 session、turn、tool、token、event。
4. Session 列表。
5. Session timeline。
6. Token 总览。
7. 工具调用列表。
8. 慢 session 排行。
9. 基础搜索。
10. 默认脱敏。

### 14.2 MVP 不做

1. 不做团队登录。
2. 不做云端同步。
3. 不做复杂 trace flamegraph。
4. 不做多 agent 支持。
5. 不做精确账单。
6. 不做完整 prompt 语义分析。

### 14.3 MVP 页面

```text
Dashboard
Session List
Session Detail
Timeline
Token Metrics
Tool Metrics
Settings
```

## 15. 迭代路线

### Phase 0：调研与样本采集

目标：确认本地 Codex session JSONL 格式。

产出：

- 样本事件集合；
- 字段映射表；
- token usage 可用性判断；
- session/turn 推断规则。

### Phase 1：本地 JSONL 解析 MVP

目标：能导入历史 session 并展示基本页面。

能力：

- scan；
- parse；
- normalize；
- store；
- session list；
- timeline；
- token summary。

### Phase 2：实时 watch

目标：Codex 运行时 UI 能实时更新。

能力：

- 文件监听；
- 增量 tail；
- WebSocket 推送；
- live session 页面。

### Phase 3：耗时和 trace 完善

目标：形成完整行为链路。

能力：

- span builder；
- trace tree；
- tool duration；
- approval duration；
- TTFT/TTFM；
- error drilldown。

### Phase 4：OTel 接入

目标：支持 Codex OTel 数据入口。

能力：

- OTLP receiver；
- OTel span/log/metric 映射；
- trace_id/span_id 保留；
- 与 JSONL session 合并。

### Phase 5：团队版本

目标：支持多用户、多项目、远程采集。

能力：

- PostgreSQL；
- 登录；
- 项目权限；
- collector agent；
- centralized server；
- retention policy；
- alert。

## 16. 数据映射策略

### 16.1 JSONL 到内部模型

```text
JSONL line
  ↓ parse
RawEvent
  ↓ normalize
NormalizedEvent
  ↓ build
Session / Turn / Span / ToolCall / TokenUsage
```

### 16.2 OTel 到内部模型

```text
OTel Span → spans
OTel Log → events
OTel Metric → metrics/token_usage/latency aggregates
```

### 16.3 session_id 归一

优先级：

```text
1. 原始事件中的 conversation/session id
2. OTel attribute 中的 conversation id
3. 文件路径中的 session id
4. 文件 checksum + first timestamp 生成
```

### 16.4 turn_id 归一

优先级：

```text
1. 原始事件中的 turn id
2. user message 到 final response 的区间推断
3. model request 分组推断
```

## 17. 异常分析能力

### 17.1 Slow Session

规则：

```text
session_duration > P95
或者 session_duration > 用户配置阈值
```

需要展示：

- 哪个 turn 最慢；
- 哪个 tool 最慢；
- 是否等待审批；
- 是否命令执行耗时；
- 是否模型响应慢。

### 17.2 High Token Session

规则：

```text
total_tokens > P95
或者 total_tokens > 用户配置阈值
```

需要展示：

- 哪个 turn 消耗最多；
- input/output/reasoning/cached 占比；
- 是否重复读取大文件；
- 是否上下文过大。

### 17.3 Failed Session

规则：

```text
error_count > 0
或者 final status = failed
```

错误类型：

```text
model_error
tool_error
shell_exit_nonzero
approval_denied
file_permission_error
network_error
mcp_error
unknown
```

### 17.4 Risky Action

第一阶段只提示，不阻断。

规则示例：

```text
rm -rf
sudo
chmod 777
curl | sh
writing outside project
modifying env files
accessing secrets files
```

## 18. Dashboard 指标定义

### 18.1 总览指标

```text
Sessions
Turns
Total Tokens
Estimated Cost
Avg Session Duration
Avg Turn Duration
Tool Calls
Tool Error Rate
Approval Wait Time
```

### 18.2 模型维度

```text
model
session_count
total_tokens
avg_turn_duration
avg_ttft
error_rate
estimated_cost
```

### 18.3 项目维度

```text
project_id
session_count
total_tokens
tool_calls
file_edits
avg_duration
estimated_cost
```

### 18.4 工具维度

```text
tool_name
call_count
success_count
error_count
error_rate
avg_duration
p95_duration
```

## 19. 前端交互设计

### 19.1 Session Detail 页面

```text
┌──────────────────────────────────────────────┐
│ Session Header                                │
│ project / model / status / duration / tokens  │
├───────────────┬──────────────────────────────┤
│ Turn List     │ Timeline / Trace              │
│               │                              │
├───────────────┴──────────────────────────────┤
│ Token / Tool / Error Details                  │
└──────────────────────────────────────────────┘
```

### 19.2 Timeline 节点展开

折叠状态：

```text
[10:01:02] shell.exec npm test 12.3s success
```

展开状态：

```text
tool: shell.exec
status: success
duration: 12.3s
exit_code: 0
cwd: <redacted>
command: npm test
stdout: hidden by privacy policy
```

### 19.3 Trace Tree 节点

节点显示：

```text
name
span_type
duration bar
status icon
token badge
error badge
```

## 20. 导出能力

支持：

```text
JSON
CSV
Markdown Report
HTML Report
```

示例命令：

```bash
atrail export session <session_id> --format markdown
atrail export metrics --from 2026-05-01 --to 2026-05-18 --format csv
```

Markdown Report 内容：

```text
session summary
token usage
timeline
tool calls
slow steps
errors
recommendations
```

## 21. 告警能力

第一阶段可以只做本地规则提示。

规则示例：

```yaml
rules:
  - name: high_token_session
    when: total_tokens > 200000
    level: warning
  - name: slow_tool_call
    when: tool_duration_ms > 60000
    level: warning
  - name: risky_shell_command
    when: command matches "rm -rf|curl .* sh|sudo"
    level: critical
```

后续团队版可接 webhook。

## 22. 与 OTel 的关系

系统内部不应该完全绑定 OTel，但应兼容 OTel 的语义。

推荐做法：

- 内部 span/event 模型尽量接近 OTel；
- 保留 trace_id、span_id、parent_span_id；
- token 指标尽量对齐 GenAI semantic conventions；
- 支持导出 OTLP；
- 支持接收 OTLP。

这样后续既能做自己的 UI，也能接 Grafana、SigNoz、Jaeger、Tempo、Prometheus 等平台。

## 23. 关键难点

### 23.1 Codex 本地 JSONL 格式可能变化

应对：

- parser 做版本适配；
- raw_event 永久保留一段时间；
- normalizer 使用宽松 schema；
- 未识别事件进入 unknown_event 表；
- 提供 parser fixtures 测试。

### 23.2 token 数据可能不完整

应对：

- token_usage 允许 partial；
- 成本 confidence 标记；
- UI 显示“部分数据”；
- 支持用户手动重新导入。

### 23.3 session/turn 边界不好判断

应对：

- 优先使用原始 id；
- 没有 id 时使用时间和事件模式推断；
- UI 允许显示 inferred 标记。

### 23.4 隐私风险

应对：

- 默认安全脱敏；
- 本地绑定 127.0.0.1；
- 不默认上传；
- 敏感规则扫描；
- 用户显式开启全文保存。

### 23.5 多来源数据合并

应对：

- 定义 source priority；
- 使用 session_id + timestamp + event_type 去重；
- 保留 raw_ref；
- 合并时不覆盖更高可信度字段。

## 24. 测试策略

### 24.1 单元测试

```text
JSONL parser
OTel parser
normalizer
span builder
redactor
cost calculator
query layer
```

### 24.2 Fixture 测试

准备样本：

```text
simple_session.jsonl
multi_turn_session.jsonl
tool_heavy_session.jsonl
failed_session.jsonl
approval_session.jsonl
missing_token_session.jsonl
large_session.jsonl
```

### 24.3 端到端测试

```text
导入样本 session
生成 spans
生成 metrics
API 查询
UI 展示
导出报告
```

### 24.4 性能测试

目标：

```text
10k sessions 可查询
100k events 导入 < 30s
session timeline 打开 < 500ms
dashboard 查询 < 1s
```

## 25. 版本兼容策略

### 25.1 Parser Version

```text
parser_name
parser_version
codex_version_range
supported_event_types
```

### 25.2 Migration

数据库迁移使用版本号：

```text
schema_version
applied_at
checksum
```

### 25.3 Unknown Event

未识别事件保留：

```text
unknown_events
  id
  raw_event_json
  reason
  created_at
```

## 26. 推荐开发顺序

### 第一步：样本探测器

先写一个命令读取 Codex session JSONL，打印事件类型分布：

```bash
atrail inspect ~/.codex/sessions
```

输出：

```text
files: 120
sessions: 120
event types:
  user_message: 300
  assistant_message: 300
  tool_call: 800
  token_count: 300
unknown events: 12
```

### 第二步：SQLite schema + 导入器

实现：

```bash
atrail ingest ~/.codex/sessions
```

### 第三步：最小 API

实现：

```text
GET /api/sessions
GET /api/sessions/:id/timeline
GET /api/metrics/summary
```

### 第四步：最小 UI

实现：

```text
Dashboard
Session List
Session Detail Timeline
```

### 第五步：token 和耗时细化

实现：

```text
token_usage
tool_calls
span_builder
latency metrics
```

### 第六步：实时 watch

实现：

```text
fsnotify
incremental tail
websocket update
```

### 第七步：OTel

实现：

```text
OTLP receiver
OTel mapping
trace correlation
```

## 27. 项目命名建议

可选名称：

```text
atrail: Codex Session Observatory
agentlens
codexscope
tracecodex
codex-insight
agenttrail
```

我推荐：

```text
atrail
```

含义清晰，命令短，适合 CLI。

## 28. 最小可行产品验收标准

MVP 完成标准：

1. 能自动找到或配置 Codex session 目录。
2. 能导入历史 session。
3. 能展示 session 列表。
4. 能打开一个 session 看到完整 timeline。
5. 能看到每个 session 的 token 总量。
6. 能看到工具调用列表和耗时。
7. 能看到慢 session 排行。
8. 默认不保存敏感正文。
9. 能本地启动 Web UI。
10. 数据库可重新构建。

## 29. 后续高级能力

### 29.1 行为链路评分

给每个 session 生成评分：

```text
效率分
成本分
稳定性分
风险分
```

### 29.2 智能诊断

自动生成诊断：

```text
这个 session 慢主要是因为 npm test 执行了 180s。
这个 session token 高主要是因为连续读取了多个大文件。
这个 session 失败是因为 shell command exit code = 1。
```

### 29.3 Agent 行为模式分析

分析：

```text
探索型
修改型
测试型
Debug 型
文档型
重构型
```

### 29.4 多 agent 支持

抽象统一 AgentEvent：

```text
Codex
Claude Code
Gemini CLI
Aider
Cursor Agent
```

### 29.5 Plugin 系统

允许用户自定义：

```text
parser plugin
redaction plugin
metric plugin
rule plugin
export plugin
```

## 30. 总结

推荐从本地 JSONL 解析器起步，先把 Codex session 的 timeline、token、tool call、耗时做出来。系统内部模型使用 event/span/metric 三层结构，避免后续重构。隐私上默认本地、默认脱敏。等 MVP 稳定后，再接入 OpenTelemetry，把系统扩展成团队级 Codex observability 平台。

最优实现路径：

```text
JSONL Importer → SQLite → API → Timeline UI → Token/Latency Dashboard → Live Watch → OTel Receiver → Team Edition
```

这个路径风险最低，也最容易快速做出可用产品。
