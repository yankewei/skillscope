# AGENTS.md

Compact guidance for OpenCode sessions working in this repo. Code is the source of truth; `docs/` are design specs that diverge from implementation in places (see below).

## Build / test / lint

```sh
cargo build
cargo test --all-targets --all-features        # 13 unit tests, no integration binary
cargo test <name_substring>                     # single test, e.g. `cargo test scan_is_incremental`
cargo clippy --all-targets --all-features -- -D warnings   # CI treats warnings as errors
cargo fmt --all -- --check                      # CI checks formatting; run `cargo fmt` before commit
```

CI (`.github/workflows/ci.yml`) runs fmt → clippy → test on ubuntu-latest with stable Rust. No pre-commit hooks — failing CI locally first is the only gate. `rusqlite` uses the `bundled` feature, so no system libsqlite3 is required.

## Architecture

Single Rust crate (no workspace), 2021 edition. `Cargo.lock` is committed (binary crate). No async runtime — `scan` and `watch` are intentionally synchronous; do not add `tokio` for this CLI.

Entry flow: `main.rs` dispatches 4 subcommands (`scan`/`watch`/`stats`/`doctor`) with global flags `--codex-home`, `--claude-home`, `--agents-home`, `--db`.

- `codex/scan.rs` — incremental JSONL parser. Per-file `byte_offset` cursor lives in the `parsed_files` SQLite table.
- `codex/parser.rs` — turns JSONL lines into `SkillInvocation` events; updates lightweight session state (`session_id`/`turn_id`/`cwd`).
- `codex/command_detection.rs` — implicit Skill detection from shell commands.
- `codex/registry.rs` — scans `~/.codex/skills`, `~/.agents/skills`, `~/.codex/plugins/cache/**/skills` for `SKILL.md`.
- `claude/parser.rs` — detects Claude Code `Skill` tool uses from `~/.claude/projects/**/*.jsonl`; it does not parse prompts or persist tool args.
- `claude/scan.rs` — incremental Claude transcript scanner using the same `parsed_files.byte_offset` cursor semantics.
- `watch.rs` — `notify` file watcher; calls `scan_file` per changed path (see below).

## Non-obvious gotchas

**DB path is NOT what the docs say.** `docs/rust-implementation-design.md` says `~/.skillscope/skillscope.db`. The code (`config.rs:35`) uses `dirs::data_local_dir().join("skillscope").join("skillscope.sqlite")` — i.e. `~/Library/Application Support/skillscope/skillscope.sqlite` on macOS, `~/.local/share/skillscope/...` on Linux. Run `skillscope doctor` to print the real path. `.gitignore` covers `*.sqlite` and `*.db`.

**Docs have been reconciled with code.** `docs/rust-implementation-design.md` was aligned to the implementation: db path, module list (`scan.rs`/`doctor.rs`), global flags (`--agents-home`), removed unimplemented `scan --since` / `stats --until`. `docs/` are design specs — still trust executable code over prose for anything not verified.

**Incremental scan semantics (do not break).** `scan_file` seeks to `parsed_files.byte_offset` and reads only new bytes. The last incomplete line (no trailing `\n`) is stored in `partial_line` and `byte_offset` is NOT advanced past it — next scan re-reads from that offset. If `file_size < byte_offset` (truncation/rotation), offset resets to 0 and session state clears. Session state is persisted across incremental scans so relative script paths can resolve without re-reading earlier turns.

**Event dedup.** `SkillInvocation.id = codex:{source_file}:{source_offset}:{trigger_source}:{skill_path_or_name}` with `INSERT OR IGNORE`. `--rescan` is safe — re-scanning never duplicates events.

**Implicit detection is gated on `exec_command`.** `parser.rs` only runs command detection when `payload.name == "exec_command"`. Other shell/unified-exec tool names require real session samples before adding (see `docs/codex-skill-invocation-analytics.md`). `find ... -name SKILL.md` correctly does not count — path resolution + token filtering handle it.

**Claude Code support is intentionally simpler.** Claude Code counts only assistant `tool_use` entries with `name == "Skill"` and `input.skill` present. Do not infer calls from plain text skill mentions or try to split explicit slash vs model-selected calls unless transcript samples expose a stable source field.

**Privacy boundary.** The DB stores only skill name/path, session/turn ids, source file + offset, tool call id, timestamps, confidence. Never persist prompts, assistant messages, tool output, `SKILL.md` body, or full shell commands. If debugging command detection, write to a temp file, not the main DB.

## Watch internals

`watch.rs` builds the `SkillRegistry` once, then on each `notify` event drains the channel until quiet for `--debounce` (real coalescing), then calls `scan_file` on each changed `.jsonl` path. A poll fallback (default 30s) runs `scan_all_with_registry` to catch missed events and refreshes the registry so newly installed skills are picked up. `scan_all` rebuilds the registry internally; `scan_all_with_registry` and `scan_file` are `pub` so watch can reuse a cached registry and scan a single file by event path.

## Testing

Tests are unit tests inside `scan.rs`, `parser.rs`, `command_detection.rs`. They use `tempfile::TempDir` for fixtures and fake registries with absolute `/tmp/skills/...` paths that don't exist on disk — `canonicalize` fails and falls back to lexical normalize, which still matches. So tests do not require real skill files. There is no separate integration test binary.
