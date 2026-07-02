# Handoff: SkillScope Skill Invocation Analytics

## Context

The user wants to build a system that monitors how often AI agents invoke Skills, so the team can analyze Skill usage and quality.

The current workspace directory is empty except for this handoff document. No implementation, PRD, ADR, issue, commit, or diff has been created yet.

## Current Direction

The design shifted from runtime-hook-first to session-log-first.

Recommended product direction:

- Start by analyzing historical session logs.
- Parse logs into normalized Skill invocation events.
- Use runtime hooks later as an accuracy enhancement, not as a first-version dependency.

Reasoning:

- Runtime hooks are not consistently available across all agent runtimes.
- Session logs allow immediate analysis of historical data.
- A normalized event schema keeps future hook, wrapper, and log-parser sources compatible.

## Claude Code Session Findings

Claude Code stores useful session data mainly under:

```text
~/.claude/projects/**/*.jsonl
```

On the inspected machine, `~/.claude/transcripts/*.jsonl` did not contain observed Skill markers.

Claude Code has two relevant markers for Skill-related activity.

### 1. Assistant Tool Use: High Confidence Skill Invocation

Automatic Skill invocation appears as an assistant `tool_use` block:

```json
{
  "type": "assistant",
  "message": {
    "content": [
      {
        "type": "tool_use",
        "name": "Skill",
        "id": "toolu_...",
        "input": {
          "skill": "think",
          "args": "..."
        }
      }
    ]
  }
}
```

Detection rule:

```text
entry.type === "assistant"
contentBlock.type === "tool_use"
contentBlock.name === "Skill"
contentBlock.input.skill exists
```

This should be treated as `confidence = 1.0`.

The `tool_use.id` can be linked to a later `user` message containing a `tool_result` with the same `tool_use_id`. That result can be used to infer success/failure via `toolUseResult.success` or `tool_result.is_error`.

Observed local scan summary:

```text
~/.claude/projects files scanned: 200
assistant Skill tool_use markers: 30
tool_use results linked: 30/30
success: 29
failure: 1
```

### 2. User Command Tags: Slash-Style Entry Point

User-entered slash commands appear inside `user.message.content` as XML-like command tags:

```xml
<command-name>think</command-name>
<command-message>think</command-message>
<command-args>...</command-args>
```

Observed local scan summary:

```text
command tag markers: 87
```

Important: `command-name` does not necessarily mean Skill. It may be a built-in Claude Code command, a plugin slash command, or a Skill exposed via slash command.

Examples seen in command tags included both likely Skills and non-Skill commands:

```text
handoff
grill-me
writing-great-skills
plugin
model
reload-plugins
permissions
compact
exit
```

Therefore, command-tag parsing must classify the command against registries before counting it as a Skill.

## Command vs Skill Classification

Do not classify a `command-name` by name shape alone. Build registries and classify explicitly.

Suggested registry sources:

```text
Skill registry:
~/.claude/skills/*/SKILL.md
~/.claude/plugins/cache/**/skills/*/SKILL.md
~/.claude/plugins/marketplaces/**/skills/*/SKILL.md

Slash command registry:
~/.claude/commands/*.md
~/.claude/plugins/cache/**/commands/*.md
~/.claude/plugins/marketplaces/**/commands/*.md
```

Also maintain a built-in Claude Code command denylist or separate built-in-command classification, including at least:

```text
plugin
model
permissions
reload-plugins
reload-skills
compact
exit
add-dir
help
clear
```

Recommended classification order:

1. `assistant tool_use` with `name === "Skill"` is always a Skill invocation.
2. `command-name` matching an installed Skill canonical name is `skill_slash_command`.
3. `command-name` matching an installed Skill short name, with no same-name command conflict, is `skill_slash_command` with slightly lower confidence.
4. `command-name` matching known built-in commands or command registry is slash command, not Skill.
5. `command-name` matching both Skill and command is `ambiguous`; report separately.
6. Unknown names should not be counted as Skill by default.

## Suggested Event Shape

For high-confidence assistant Skill calls:

```json
{
  "event_type": "skill_invocation",
  "runtime": "claude_code",
  "source": "claude_code_project_jsonl",
  "trigger_source": "assistant_tool_use",
  "skill_name": "think",
  "tool_use_id": "toolu_...",
  "timestamp": "...",
  "session_id": "...",
  "outcome": "completed",
  "confidence": 1.0
}
```

For command tags:

```json
{
  "event_type": "command_invocation",
  "runtime": "claude_code",
  "source": "claude_code_project_jsonl",
  "command_name": "handoff",
  "classification": "skill_slash_command",
  "matched_skill": "handoff",
  "confidence": 0.95
}
```

Only `classification === "skill_slash_command"` should enter Skill usage statistics.

## Existing Reference Scripts Found

There are useful local reference scripts under the user's Claude configuration. Paths are redacted to home-relative form.

```text
~/.claude/plugins/marketplaces/claude-plugins-official/plugins/session-report/skills/session-report/analyze-sessions.mjs
~/.claude/plugins/cache/rightcapital/rc-shared/1.0.4/skills/rc-improving-skills-from-feedback/parse_session.py
```

Relevant findings from these scripts:

- `analyze-sessions.mjs` already scans `~/.claude/projects/**/*.jsonl` and reports `skillInvocations`.
- It counts `assistant` content blocks where `tool_use.name === "Skill"` and `input.skill` exists.
- It also treats `<command-name>` / `<command-message>` tags as possible slash-command activity.
- It deduplicates resumed sessions by `uuid` and assistant API calls by `requestId`.
- It notes subagent transcripts under `<project>/<sessionId>/subagents/*.jsonl`.

## Open Design Decisions

- Whether the first implementation should be a CLI-only analyzer or also persist normalized events.
- Whether to use JSONL-only output first or immediately add SQLite.
- How to resolve same-name conflicts between plugin commands and Skills.
- Whether to report built-in commands separately for general agent behavior analysis.
- Whether to parse only `~/.claude/projects` first or include Codex/Cursor adapters in version one.

## Recommended Next Step

Implement Phase 1 as a local CLI analyzer in the current empty workspace:

1. Create a small ESM Node.js project.
2. Add a Claude Code parser for `~/.claude/projects/**/*.jsonl`.
3. Extract high-confidence `assistant tool_use Skill` invocations first.
4. Link `tool_use.id` to `tool_result.tool_use_id` for outcome.
5. Build a simple Skill/command registry classifier for command tags.
6. Output summary by Skill: count, success, failure, unknown, first seen, last seen.
7. Add JSON output mode so later dashboard or reports can consume it.

## Suggested Skills

- `think`: Use if the next agent needs to finalize product scope or schema before implementation.
- `design`: Use only if building a dashboard UI; not needed for CLI-first work.
- `codebase-design`: Use if the parser/registry/event schema starts becoming a reusable module.
- `tdd`: Use if implementing the parser test-first with JSONL fixtures.
- `check`: Use after implementation to review parser correctness, privacy boundaries, and edge cases.

## Privacy Notes

- Do not dump raw session content into reports by default.
- Store only metadata needed for analytics: timestamps, session IDs, project labels, Skill names, outcome, and confidence.
- Redact or hash user prompts unless the user explicitly requests semantic analysis.
- Avoid storing full `command-args` because they may contain sensitive content.
- Avoid printing full local filesystem paths in shared reports; prefer home-relative or hashed project identifiers.

## No Existing Artifacts To Reference

No PRD, ADR, issue, implementation diff, or commit exists yet for this work in the current workspace.
