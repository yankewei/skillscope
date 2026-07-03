# Rust 实现设计

本文档描述 SkillScope 的 Rust 版本实现。统计口径以 [Codex Skill 调用次数统计规则](./codex-skill-invocation-analytics.md) 为准，本文只定义工程落地方式。

## 目标

第一版实现一个本地 CLI，可以：

- 扫描 Codex 本地 session JSONL。
- 识别显式 Skill 注入和隐式 Skill 命令命中。
- 将归一化调用事件写入 SQLite。
- 持久化每个日志文件的解析游标，支持增量扫描。
- 提供接近实时的 watch 模式，持续监控新增 session 文件和追加内容。
- 输出基础统计，例如按 Skill、按调用类型、按时间范围聚合。

暂不实现 dashboard、后台系统服务、Claude Code adapter、远端同步或跨设备聚合。

## CLI 形态

推荐命令：

```text
skillscope scan
skillscope watch
skillscope stats
skillscope doctor
```

### 全局参数

以下参数对所有子命令生效：

```text
--codex-home <path>     默认 ~/.codex
--agents-home <path>    默认 ~/.agents
--db <path>             默认 <data_local_dir>/skillscope/skillscope.sqlite
```

`<data_local_dir>` 由 `dirs::data_local_dir` 决定：macOS 下为 `~/Library/Application Support`，Linux 下为 `~/.local/share`。可用 `skillscope doctor` 查看本机实际路径。

### `scan`

执行一次扫描，然后退出。

默认行为：

- 扫描 `~/.codex/sessions/**/*.jsonl`。
- 从 SQLite 中读取每个文件的上次解析位置。
- 只解析新增内容。
- 将新发现的 Skill 调用写入 SQLite。

常用参数：

```text
--json                  输出本次扫描发现的事件 JSON
--rescan                忽略文件游标，重新扫描并依赖事件唯一键去重
```

### `watch`

常驻进程，接近实时监控 Codex session 目录。

启动流程：

1. 先执行一次 `scan`，补齐进程启动前已经存在的内容。
2. 使用文件监听监控 `~/.codex/sessions`。
3. 新文件创建、文件增长、文件重命名时触发增量解析。
4. 定期做一次轻量全量扫描（不重置游标），弥补文件监听丢事件。

推荐参数：

```text
--poll-interval <duration>   默认 30s，用于兜底 rescan
--debounce <duration>        默认 300ms，合并密集文件事件
```

### `stats`

从 SQLite 查询统计结果。

第一版至少支持：

```text
skillscope stats
skillscope stats --since 2026-07-01
skillscope stats --group-by skill
skillscope stats --group-by invocation-type
skillscope stats --json
```

默认输出字段：

- Skill 名称
- 调用总数
- 显式调用数
- 隐式调用数
- 首次出现时间
- 最近出现时间

### `doctor`

输出本地配置和索引健康状态：

- Codex home 是否存在。
- session 目录是否存在。
- SQLite 是否可读写。
- 已发现多少 session 文件。
- 多少文件有游标。
- 最近一次解析错误。
- Skill 注册表扫描到多少 Skill。

## Rust crate 结构

推荐先做单 crate，内部按模块拆分：

```text
src/
  main.rs
  cli.rs
  config.rs
  db.rs
  codex/
    mod.rs
    registry.rs
    scan.rs
    parser.rs
    command_detection.rs
    doctor.rs
  events.rs
  stats.rs
  watch.rs
  paths.rs
  error.rs
```

模块职责：

- `cli`：命令行参数和子命令分发。
- `config`：解析默认路径、用户参数和环境变量。
- `db`：SQLite schema、migration、读写事务。
- `codex::registry`：扫描本机 Skill 注册表。
- `codex::scan`：发现 Codex session JSONL 文件，处理文件游标，执行增量解析。
- `codex::parser`：把 JSONL 行转换为候选事件。
- `codex::command_detection`：复现 Codex 隐式 Skill 命令判定。
- `codex::doctor`：输出本地配置和索引健康状态。
- `events`：归一化事件类型和去重 key。
- `stats`：聚合查询。
- `watch`：文件监听和增量调度。
- `paths`：`~` 展开、canonicalize、home-relative 输出。

## 依赖

建议依赖：

```text
clap        CLI
serde       数据结构序列化
serde_json  JSONL 解析
rusqlite    SQLite
notify      文件监听
walkdir     文件发现
thiserror   错误类型
chrono      时间解析和格式化
shlex       shell token 化
```

先不引入 async runtime。`scan` 和 `watch` 都可以用同步代码实现，减少复杂度。后续如果要接 HTTP API 或后台服务，再评估 `tokio`。

## SQLite schema

数据库默认位置（由 `dirs::data_local_dir` 决定，平台相关）：

```text
macOS:  ~/Library/Application Support/skillscope/skillscope.sqlite
Linux:  ~/.local/share/skillscope/skillscope.sqlite
```

可用 `skillscope doctor` 查看本机实际路径。

### `schema_migrations`

```sql
CREATE TABLE schema_migrations (
  version INTEGER PRIMARY KEY,
  applied_at TEXT NOT NULL
);
```

### `parsed_files`

记录每个 JSONL 文件解析到哪里。

```sql
CREATE TABLE parsed_files (
  path TEXT PRIMARY KEY,
  canonical_path TEXT,
  file_size INTEGER NOT NULL DEFAULT 0,
  modified_at TEXT,
  byte_offset INTEGER NOT NULL DEFAULT 0,
  line_number INTEGER NOT NULL DEFAULT 0,
  partial_line TEXT NOT NULL DEFAULT '',
  session_id TEXT,
  turn_id TEXT,
  cwd TEXT,
  fingerprint TEXT,
  last_error TEXT,
  updated_at TEXT NOT NULL
);
```

字段说明：

- `path`：home-relative 或绝对路径，第一版可用绝对路径做主键。
- `byte_offset`：下次读取起点。
- `line_number`：便于报错定位。
- `partial_line`：上次读到但尚未形成完整 JSONL 行的尾部内容。
- `session_id` / `turn_id` / `cwd`：上次扫描结束时的轻量 session 上下文，用于增量扫描恢复相对路径解析和事件归属。
- `fingerprint`：用于检测文件替换，格式为 `size:{file_size}:offset:{byte_offset}`。
- `last_error`：最近一次解析错误，`doctor` 使用。

文件如果变小，说明被截断、压缩替换或轮转。处理规则：

```text
file_size < byte_offset -> byte_offset 重置为 0，partial_line 和 session 上下文清空，重新扫描，依赖事件唯一键去重。
```

### `skill_invocations`

记录归一化 Skill 调用事件。

```sql
CREATE TABLE skill_invocations (
  id TEXT PRIMARY KEY,
  runtime TEXT NOT NULL,
  source TEXT NOT NULL,
  trigger_source TEXT NOT NULL,
  invocation_type TEXT NOT NULL,
  skill_name TEXT NOT NULL,
  skill_path TEXT,
  skill_scope TEXT,
  plugin_id TEXT,
  session_id TEXT,
  turn_id TEXT,
  source_file TEXT NOT NULL,
  source_offset INTEGER NOT NULL,
  source_line INTEGER NOT NULL,
  tool_call_id TEXT,
  timestamp TEXT NOT NULL,
  confidence REAL NOT NULL,
  created_at TEXT NOT NULL
);
```

推荐索引：

```sql
CREATE INDEX idx_skill_invocations_skill_name ON skill_invocations(skill_name);
CREATE INDEX idx_skill_invocations_timestamp ON skill_invocations(timestamp);
CREATE INDEX idx_skill_invocations_session ON skill_invocations(session_id);
CREATE INDEX idx_skill_invocations_source_file ON skill_invocations(source_file);
```

事件 `id` 使用稳定去重 key，例如：

```text
codex:<source_file>:<source_offset>:<trigger_source>:<skill_path>
```

`source_offset` 比行号更适合去重，因为 JSONL 追加文件中 byte offset 稳定。

## 解析流程

### 1. 构建 Skill 注册表

扫描：

```text
~/.codex/skills/**/SKILL.md
~/.agents/skills/**/SKILL.md
~/.codex/plugins/cache/**/skills/**/SKILL.md
```

每个 Skill 至少记录：

```text
skill_name
skill_path
skill_dir
scripts_dir
scope
plugin_id
canonical_skill_path
canonical_scripts_dir
```

Skill 名称优先从 `SKILL.md` frontmatter 的 `name` 字段读取。读不到时，用父目录名。

### 2. 读取 session 文件

对每个 JSONL 文件：

1. 从 `parsed_files.byte_offset` seek。
2. 读取新增 bytes，按最后一个换行符切成完整区间和尾部半行。
3. 只解析完整行。
4. 最后一行如果没有换行，写入 `partial_line` 供诊断，但不推进到该行之后；下次从同一 offset 重读该半行。
5. 每成功处理一批完整行，写事件、新游标和当前 session 上下文。

这样可以避免进程在半行 JSON 写入时误报。

### 3. 解析 session 上下文

扫描文件时维护轻量状态：

```text
current_session_id
current_turn_id
current_cwd
```

状态来源：

- `session_meta.payload.session_id` 或 `session_meta.payload.id`
- `turn_context.payload.turn_id`
- `turn_context.payload.cwd`
- `response_item.payload.internal_chat_message_metadata_passthrough.turn_id`

如果事件自身带 turn id，优先使用事件自身的 turn id。

增量扫描从 `parsed_files` 恢复上述状态，避免只读取追加内容时丢失早先的 `cwd`、`session_id` 或 `turn_id`。`--rescan` 或文件截断重扫时清空状态并从头重建。

### 4. 显式 Skill 注入检测

对 `response_item`：

- `payload.type == "message"`
- `payload.role == "user"`
- `content[].text` 中存在 `<skill>...</skill>`

解析 `<name>` 和 `<path>`：

- `<name>` 作为候选 Skill 名称。
- `<path>` 做路径规范化，并匹配注册表。
- 匹配成功后生成 `explicit_skill_injection` 事件。

不要保存 `<skill>` 内正文。

### 5. 隐式命令检测

对 `response_item`：

- `payload.type == "function_call"`
- 第一版实现要求 `payload.name == "exec_command"`；其他 shell/unified exec 工具名需要真实 session 样本或源码依据后再加入。
- `payload.arguments` 解析出命令字符串。

命令字符串来源兼容：

- `cmd`
- `command`

用 `shlex` token 化命令，然后复现 Codex 的两类检测：

1. 读命令目标是某个注册表中的 `SKILL.md`。
2. runner 命令执行了某个 Skill 的 `scripts/` 子路径。

runner 初始集合：

```text
python
python3
bash
zsh
sh
node
deno
ruby
perl
pwsh
```

脚本扩展名初始集合：

```text
.py
.sh
.js
.ts
.rb
.pl
.ps1
```

只有能定位到单个注册表 Skill 时才生成 `implicit_skill_command` 事件。

## watch 语义

`watch` 使用 `notify` 监听 `~/.codex/sessions`。

处理规则：

- 新文件：加入扫描队列。
- 文件修改：对该文件做增量扫描。
- 文件重命名：如果新路径仍在 sessions 下，按新文件扫描。
- 删除：不删除历史事件，只在 `doctor` 中显示文件缺失。
- 监听错误：记录错误并继续，下一次 poll rescan 兜底。

为了避免密集写入时重复扫描，同一路径事件需要 debounce。推荐 300ms。

同时保留 poll rescan：

```text
每 30s 扫描一次 session 目录，发现新文件或文件大小增长就解析。
```

这样即使文件监听丢事件，也能在短时间内补上。

## 查询统计

默认统计 SQL：

```sql
SELECT
  skill_name,
  COUNT(*) AS total_count,
  SUM(CASE WHEN invocation_type = 'explicit' THEN 1 ELSE 0 END) AS explicit_count,
  SUM(CASE WHEN invocation_type = 'implicit' THEN 1 ELSE 0 END) AS implicit_count,
  MIN(timestamp) AS first_seen,
  MAX(timestamp) AS last_seen
FROM skill_invocations
GROUP BY skill_name
ORDER BY total_count DESC, skill_name ASC;
```

`--since` 通过 `timestamp` 前缀匹配过滤（支持 `YYYY-MM-DD` 或完整 ISO 时间戳）。当前未实现 `--until`。

## 隐私边界

数据库中不要保存：

- 用户 prompt 全文。
- assistant message 全文。
- tool output 全文。
- `SKILL.md` 正文。
- 完整 shell 命令。

可以保存：

- Skill 名称和路径。
- session id、turn id。
- source 文件和 offset。
- tool call id。
- 调用类型和置信度。
- 时间戳。

如果后续需要调试命令检测，可以加 `--debug-capture`，但默认关闭，并且只写入本地临时文件，不进入主数据库。

## 错误处理

解析错误不应中断整个扫描。

规则：

- 单行 JSON 解析失败：记录 `parsed_files.last_error`，跳过该行，继续后续行。
- `payload.arguments` 解析失败：忽略该候选事件。
- Skill path 无法 canonicalize：使用原始路径规范化后匹配，匹配不到就忽略。
- SQLite 写入失败：当前文件游标不能推进，避免丢事件。
- 进程中断：已提交事务保持一致，未提交批次下次重扫。

## 测试计划

单元测试：

- 显式 `<skill>` message 能解析出 Skill 名称和路径。
- 输入框历史或普通 user text 不被误判。
- `exec_command` 读取已注册 `SKILL.md` 生成隐式事件。
- `find ... -name SKILL.md` 不生成事件。
- runner 执行 `scripts/foo.py` 生成隐式事件。
- 非注册表路径不生成事件。
- 坏 JSONL 行跳过，扫描继续。
- 半行 JSONL 不解析，保存到 `partial_line`。
- 文件变小后游标重置。

集成测试：

- 用临时目录模拟 `~/.codex/sessions` 和 Skill 注册表。
- 第一次 `scan` 写入事件和游标。
- 第二次 `scan` 不重复写入。
- 向 JSONL 追加新事件后，再次 `scan` 只写新事件。
- `watch` 在新文件出现后写入事件。
- `stats` 输出正确聚合。

## 实现顺序

1. 建立 Rust CLI 和 SQLite migration。
2. 实现 Skill 注册表扫描。
3. 实现 JSONL 增量读取和游标持久化。
4. 实现显式 `<skill>` 注入检测。
5. 实现隐式命令检测。
6. 实现 `scan` 和 `stats`。
7. 加入 `watch`。
8. 补齐 `doctor` 和错误报告。

每一步都应保持命令可运行、测试可通过。`watch` 不是 `scan` 的前置条件，`scan + stats` 应先成为可用版本。
