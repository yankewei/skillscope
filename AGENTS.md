# AGENTS.md

Compact guidance for OpenCode sessions working in this repo. Code is the source of truth; `docs/` are design specs that diverge from implementation in places (see below).

## Build / test / lint

```sh
cargo build
cargo test --all-targets --all-features        # unit tests, no integration binary
cargo test <name_substring>                     # single test, e.g. `cargo test scan_is_incremental`
cargo clippy --all-targets --all-features -- -D warnings   # CI treats warnings as errors
cargo fmt --all -- --check                      # CI checks formatting; run `cargo fmt` before commit
```

CI (`.github/workflows/ci.yml`) runs fmt → clippy → test on ubuntu-latest with stable Rust. No pre-commit hooks — failing CI locally first is the only gate. `rusqlite` uses the `bundled` feature, so no system libsqlite3 is required.

## Architecture

Single Rust crate (no workspace), 2021 edition. `Cargo.lock` is committed (binary crate). The app is now daemon-first: `skillscope daemon` runs the local axum/tokio backend, owns scanning/query execution, and the CLI-facing commands call that daemon rather than falling back to direct DB access. `serve` is retained as an alias for now.

Entry flow: `main.rs` dispatches `daemon` plus client commands (`scan`/`stats`/`doctor`). `daemon start/status/stop` manages the local daemon process/control plane. `watch` is retained as a compatibility command that tells users to run `daemon`; continuous watching belongs to the daemon. Global flags include `--codex-home`, `--claude-home`, `--agents-home`, `--db`, and `--service-url`.

- `codex/scan.rs` — incremental JSONL parser. Per-file `byte_offset` cursor lives in the `parsed_files` SQLite table.
- `codex/parser.rs` — turns JSONL lines into `SkillInvocation` events; updates lightweight session state (`session_id`/`turn_id`/`cwd`).
- `codex/command_detection.rs` — implicit Skill detection from shell commands.
- `codex/registry.rs` — scans `~/.codex/skills`, `~/.agents/skills`, `~/.codex/plugins/cache/**/skills`, `~/.claude/skills`, `~/.claude/plugins/cache/**/skills` for `SKILL.md`. Plugin skills also read `plugin_name` from `.claude-plugin/plugin.json`.
- `claude/parser.rs` — detects Claude Code `Skill` tool uses from `~/.claude/projects/**/*.jsonl`; it does not parse prompts or persist tool args.
- `claude/scan.rs` — incremental Claude transcript scanner using the same `parsed_files.byte_offset` cursor semantics.
- `server.rs` — axum HTTP backend (`/health`, `/scan`, `/stats/skills`, `/stats/invocation-types`, `/doctor`, `/shutdown`) and daemon lifecycle.
- `client.rs` — small HTTP client used by CLI commands; it does not open SQLite or parse transcripts.
- `watch.rs` — `notify` file watcher used by the service; calls `scan_file` per changed path (see below).

## Non-obvious gotchas

**DB path is NOT what the docs say.** `docs/rust-implementation-design.md` says `~/.skillscope/skillscope.db`. The code (`config.rs:35`) uses `dirs::data_local_dir().join("skillscope").join("skillscope.sqlite")` — i.e. `~/Library/Application Support/skillscope/skillscope.sqlite` on macOS, `~/.local/share/skillscope/...` on Linux. Run `skillscope doctor` to print the real path. `.gitignore` covers `*.sqlite` and `*.db`.

**Docs have been reconciled with code.** `docs/rust-implementation-design.md` was aligned to the implementation: db path, module list (`scan.rs`/`doctor.rs`), global flags (`--agents-home`), removed unimplemented `scan --since` / `stats --until`. `docs/` are design specs — still trust executable code over prose for anything not verified.

**Incremental scan semantics (do not break).** `scan_file` seeks to `parsed_files.byte_offset` and reads only new bytes. The last incomplete line (no trailing `\n`) is stored in `partial_line` and `byte_offset` is NOT advanced past it — next scan re-reads from that offset. If `file_size < byte_offset` (truncation/rotation), offset resets to 0 and session state clears. Session state is persisted across incremental scans so relative script paths can resolve without re-reading earlier turns.

**Event dedup.** `SkillInvocation.id = {runtime}:{source_file}:{source_offset}:{trigger_source}:{skill_path_or_name}[:{tool_call_id}]` with `INSERT OR IGNORE`. The optional tool call id prevents same-line tool-use collisions. `--rescan` is safe — re-scanning never duplicates events.

**Implicit detection is gated on `exec_command`.** `parser.rs` only runs command detection when `payload.name == "exec_command"`. Other shell/unified-exec tool names require real session samples before adding (see `docs/codex-skill-invocation-analytics.md`). `find ... -name SKILL.md` correctly does not count — path resolution + token filtering handle it.

**Claude Code support is intentionally simpler.** Claude Code counts explicit skill slash commands from user `<command-name>` transcript entries, and model-selected skills from assistant `tool_use` entries with `name == "Skill"` and `input.skill` present. Slash commands are matched against the `SkillRegistry` whitelist: a plain `/foo` must match a registered `agent`/`claude_user` skill named `foo`; a namespaced `/plugin:skill` must match a `claude_plugin` skill whose `plugin_name` is `plugin` and `name` is `skill`. Unregistered slash commands (custom prompts, built-ins, MCP commands) are dropped. `claude::parse_line` receives `&SkillRegistry`. Do not infer calls from plain text skill mentions or persist slash args/tool args.

**Privacy boundary.** The DB stores only skill name/path, session/turn ids, source file + offset, tool call id, timestamps, confidence. Never persist prompts, assistant messages, tool output, `SKILL.md` body, or full shell commands. If debugging command detection, write to a temp file, not the main DB.

## Watch internals

`skillscope daemon` starts the backend and a watcher. `watch.rs` builds the `SkillRegistry` once, then on each `notify` event drains the channel until quiet for `--debounce` (real coalescing), refreshes the registry first if skill/plugin paths changed, then calls the correct Codex or Claude `scan_file` for changed `.jsonl` paths. A poll fallback (default 30s) runs `scan_all_with_registry` to catch missed events.

## Testing

Tests are unit tests inside `scan.rs`, `parser.rs`, `command_detection.rs`. They use `tempfile::TempDir` for fixtures and fake registries with absolute `/tmp/skills/...` paths that don't exist on disk — `canonicalize` fails and falls back to lexical normalize, which still matches. So tests do not require real skill files. There is no separate integration test binary.
