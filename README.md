# SkillScope

SkillScope is a local analytics tool for Skill usage in AI coding agents. It scans local Codex and Claude Code JSONL transcripts, records normalized Skill invocation events in SQLite, and exposes simple CLI commands for scanning, statistics, and diagnostics.

## What it tracks

SkillScope currently detects:

- Codex explicit Skill injections from `<skill>...</skill>` transcript blocks.
- Codex implicit Skill usage when an `exec_command` reads a registered `SKILL.md` or runs a script under a Skill's `scripts/` directory.
- Claude Code Skill slash commands that match a registered local or plugin Skill.
- Claude Code assistant `Skill` tool uses.

It intentionally avoids counting plain-text Skill mentions.

## Privacy

SkillScope stores only analytics metadata needed for counting Skill usage. It does **not** store prompts, assistant messages, tool output, full shell commands, `SKILL.md` contents, slash command arguments, or Claude `Skill` tool arguments.

Stored event fields include metadata such as Skill name/path, runtime, invocation type, session/turn ids when available, source file/offset, timestamp, and confidence.

## Install / build

This repository is a single Rust crate.

```sh
cargo build
```

For a release binary:

```sh
cargo build --release
```

Then run either `target/debug/skillscope` or `target/release/skillscope`.

## Quick start

Start the local daemon:

```sh
skillscope daemon start
```

Run a scan:

```sh
skillscope scan
```

View Skill statistics:

```sh
skillscope stats
```

View invocation type statistics:

```sh
skillscope stats --group-by invocation-type
```

Check local configuration and indexing health:

```sh
skillscope doctor
```

Stop the daemon:

```sh
skillscope daemon stop
```

## Commands

### `daemon`

Runs the local HTTP backend and file watcher. `skillscope daemon` defaults to `skillscope daemon start`.

```sh
skillscope daemon start
skillscope daemon run
skillscope daemon status
skillscope daemon stop
```

Useful options:

```text
--addr <addr>                 default: 127.0.0.1:3766
--poll-interval <duration>    default: 30s
--debounce <duration>         default: 300ms
```

If you pass a non-default `--addr` to a daemon management command and leave `--service-url` at its default, SkillScope uses the matching local URL for `start`, `status`, and `stop`. Pass `--service-url` explicitly when connecting through a different local endpoint.

### `scan`

Requests one scan from the running daemon.

```sh
skillscope scan
skillscope scan --json
skillscope scan --rescan
```

`--rescan` ignores stored file cursors and re-reads transcript files. Event IDs are deterministic, so re-scanning does not duplicate existing events.

### `stats`

Queries aggregated events from the daemon.

```sh
skillscope stats
skillscope stats --since 2026-07-01
skillscope stats --group-by skill
skillscope stats --group-by invocation-type
skillscope stats --json
```

The default table includes total calls, per-runtime counts, explicit Codex/Claude slash calls, implicit Codex command calls, Claude `Skill` tool calls, and first/last seen timestamps.

### `doctor`

Prints local paths, discovered transcript counts, indexed file counts, registry diagnostics, and the latest parse error if one exists.

```sh
skillscope doctor
skillscope doctor --json
```

## Paths

Default roots:

```text
--codex-home <path>     default: ~/.codex
--claude-home <path>    default: ~/.claude
--agents-home <path>    default: ~/.agents
--db <path>             default: <data_local_dir>/skillscope/skillscope.sqlite
--service-url <url>     default: http://127.0.0.1:3766
```

The default SQLite path is platform-specific via `dirs::data_local_dir()`:

- macOS: `~/Library/Application Support/skillscope/skillscope.sqlite`
- Linux: `~/.local/share/skillscope/skillscope.sqlite`

Run `skillscope doctor` to see the exact paths used on your machine.

## Development

Common checks:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

CI runs formatting, clippy, and tests on stable Rust.

## Current scope

SkillScope is local-only. It does not currently provide a dashboard, remote sync, or cross-device aggregation.
