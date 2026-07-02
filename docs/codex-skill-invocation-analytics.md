# Codex Skill 调用次数统计规则

本文档定义 SkillScope 在 Codex 本地 session 日志中统计 Skill 调用次数的规则。目标是尽量贴近 Codex 源码中的 Skill 触发逻辑，同时避免把输入框草稿、普通文本提及、泛搜索命令误算成真正调用。

## 数据来源

Codex 本地 session 主要记录在：

```text
~/.codex/sessions/YYYY/MM/DD/*.jsonl
```

每一行是一个 JSON 事件。和 Skill 统计相关的事件主要有：

- `session_meta`：session 元信息，包含 session id、cwd、客户端版本等。
- `turn_context`：turn 元信息，包含 turn id、cwd、model、sandbox 等。
- `response_item`：模型可见或模型产生的上下文项，其中包括用户消息、工具调用、工具结果等。
- `event_msg`：UI 或运行时事件，通常用于补充状态，不作为 Skill 调用的主要依据。

不要使用 `~/.codex/history.jsonl` 统计 Skill 调用。它是输入历史和召回历史，不等价于已经提交给 AI 的上下文。

## 官方触发路径

Codex 源码中 Skill 调用有两类触发路径。

### 1. 显式 Skill 调用

显式调用来自用户提交的 Skill mention 或结构化 Skill 输入。提交后，Codex 会在 turn 构建阶段解析 `UserInput::Skill` 或文本 mention，读取对应 `SKILL.md`，然后把 Skill 正文注入到模型上下文。

本地 session JSONL 中，这种调用表现为一个 `response_item`：

```json
{
  "type": "response_item",
  "payload": {
    "type": "message",
    "role": "user",
    "content": [
      {
        "type": "input_text",
        "text": "<skill>\n<name>diagnose</name>\n<path>/.../diagnose/SKILL.md</path>\n...\n</skill>"
      }
    ]
  }
}
```

统计规则：

- `payload.type === "message"`
- `payload.role === "user"`
- 任一 `content[].text` 包含 `<skill>`、`<name>...</name>`、`<path>...</path>` 和 `</skill>`
- `<path>` 能匹配到已安装 Skill 注册表中的 `SKILL.md`

满足以上条件时，计为一次显式 Skill 调用：

```text
trigger_source = explicit_skill_injection
invocation_type = explicit
confidence = 1.0
```

### 2. 隐式 Skill 调用

隐式调用来自 agent 执行命令时命中某个 Skill 的资源。Codex 源码会在 shell 或 unified exec 工具执行前调用隐式检测逻辑，判断命令是否属于某个 Skill。

本地 session JSONL 不会直接保存 Codex 的 analytics fact，因此离线分析需要复现源码中的判定逻辑。

隐式调用有两个主要信号：

1. 命令读取已注册 Skill 的 `SKILL.md`。
2. 命令运行已注册 Skill 的 `scripts/` 目录下脚本。

对应 session 事件通常是：

```json
{
  "type": "response_item",
  "payload": {
    "type": "function_call",
    "name": "exec_command",
    "arguments": "{\"cmd\":\"sed -n '1,220p' /.../diagnose/SKILL.md\"}",
    "call_id": "call_xxx"
  }
}
```

统计规则：

- `payload.type === "function_call"`
- 工具名是 shell 或 exec 类工具，例如 `exec_command`
- `payload.arguments` 能解析出命令字符串
- 命令经 shell token 化后，满足下列任一条件：
  - 读命令目标路径是某个已注册 Skill 的 `SKILL.md`
  - 脚本运行命令目标路径位于某个已注册 Skill 的 `scripts/` 目录下

满足以上条件时，计为一次隐式 Skill 调用：

```text
trigger_source = implicit_skill_command
invocation_type = implicit
confidence = 0.9
```

如果命令只是泛搜索，例如 `find ... -name SKILL.md` 或 `rg SKILL.md`，但无法解析为单个已注册 Skill，不计数。

当前 Rust 第一版实现只接受已在本地 session 样本中验证过的 `payload.name === "exec_command"`。如果后续确认 Codex rollout 中还有其他 shell 或 unified exec 工具名，应先补样本和测试，再扩展允许列表。

## 计数口径

Codex Skill 调用次数定义为：

```text
显式 <skill> 注入次数 + 隐式 Skill 命令命中次数
```

默认按事件计数：

- 同一个 session 中显式注入一次 `diagnose`，计 `diagnose +1`。
- 同一个 session 中再次显式注入 `diagnose`，再计 `diagnose +1`。
- 隐式读取 `diagnose/SKILL.md`，计 `diagnose +1`。
- 隐式运行 `diagnose/scripts/foo.py`，计 `diagnose +1`。

后续可以增加按 turn 去重或按 Skill 去重的报表视图，但原始事件层不做去重。

## 不计数的情况

以下情况不算 Skill 调用：

- 用户只在输入框中 mention 了 Skill，但提交前删除。
- 用户打开 Skill mention popup，但没有提交。
- 用户普通文本里提到 Skill 名称，但没有形成有效 Skill mention。
- 助手普通文本说“我会使用 diagnose”，但没有发生显式注入或隐式命令命中。
- 命令泛搜索 `*/SKILL.md`，无法定位到单个已注册 Skill。
- 读取非注册表里的 `SKILL.md`。
- 读取 README、普通文档、代码文件。
- `~/.codex/history.jsonl` 中出现 Skill mention 文本。

关键原则：只有已经进入 session rollout，并且能证明 Codex 给模型注入了 Skill 内容，或运行时命中了 Skill 资源的事件，才进入调用统计。

## 输入框草稿边界

输入框中的 mention 是 TUI/composer 状态。只有用户真正提交后，Codex 才会生成 `UserInput::Skill`，进入 core turn 处理，然后注入 `<skill>...</skill>` 并写入 session JSONL。

因此：

```text
输入框中临时选择 Skill -> 删除 -> 未提交
```

不会落到 `~/.codex/sessions/*.jsonl`，也不计为 Skill 调用。

如果提交过的文本被写入 `~/.codex/history.jsonl`，也只代表输入历史，不代表模型上下文。SkillScope 不从该文件统计调用次数。

## Skill 注册表

分析前先构建本机 Skill 注册表，至少扫描：

```text
~/.codex/skills/**/SKILL.md
~/.agents/skills/**/SKILL.md
~/.codex/plugins/cache/**/skills/**/SKILL.md
```

注册表字段建议包括：

- `skill_name`
- `skill_path`
- `skill_dir`
- `scripts_dir`
- `scope`，例如 user、repo、system、plugin
- `plugin_id`，如果来自插件

路径匹配时应做路径规范化：

- 展开 `~`
- 处理相对路径和当前 turn 的 `cwd`
- 尽可能 canonicalize 已存在路径
- 比较时兼容符号链接解析后的路径

## 事件输出

建议输出归一化事件：

```json
{
  "event_type": "skill_invocation",
  "runtime": "codex",
  "source": "codex_session_jsonl",
  "trigger_source": "explicit_skill_injection",
  "invocation_type": "explicit",
  "skill_name": "diagnose",
  "skill_path": "~/.agents/skills/diagnose/SKILL.md",
  "timestamp": "2026-07-02T12:30:11.929Z",
  "session_id": "019f...",
  "turn_id": "019f...",
  "confidence": 1.0
}
```

隐式事件示例：

```json
{
  "event_type": "skill_invocation",
  "runtime": "codex",
  "source": "codex_session_jsonl",
  "trigger_source": "implicit_skill_command",
  "invocation_type": "implicit",
  "skill_name": "diagnose",
  "skill_path": "~/.agents/skills/diagnose/SKILL.md",
  "timestamp": "2026-07-02T12:30:11.929Z",
  "session_id": "019f...",
  "turn_id": "019f...",
  "tool_call_id": "call_xxx",
  "confidence": 0.9
}
```

不要输出原始 prompt、assistant message、完整 tool output 或完整 `SKILL.md` 内容。

## 源码依据

以下源码位置用于校准本规则：

- 显式 Skill mention 收集和注入：`codex-rs/core-skills/src/injection.rs`
- `<skill>...</skill>` 注入格式：`codex-rs/core-skills/src/skill_instructions.rs`
- 隐式命令检测：`codex-rs/core-skills/src/invocation_utils.rs`
- shell 和 unified exec 执行前的隐式检测调用：`codex-rs/core/src/tools/handlers/shell/shell_command.rs`、`codex-rs/core/src/tools/handlers/unified_exec/exec_command.rs`
- Skill invocation analytics fact：`codex-rs/analytics/src/facts.rs`、`codex-rs/analytics/src/client.rs`
- session rollout JSONL 格式：`codex-rs/protocol/src/protocol.rs`
- rollout 持久化策略：`codex-rs/rollout/src/policy.rs`

结论：Codex 源码中有官方 Skill invocation analytics，但本地 session JSONL 不直接持久化 analytics fact。离线统计本地日志时，应使用本规则从 session rollout 中复原显式和隐式 Skill 调用。
