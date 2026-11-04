# WSID — Agent & Contributor Guide

## Project Purpose

WSID ("What Should I Do?") is an offline-first, cross-platform desktop and
terminal task manager written in Rust with first-class screen-reader
accessibility and local AI agent collaboration via the Model Context Protocol
(MCP). It ranks actionable tasks using an Eisenhower + effective-slack scoring
engine, tracks blocking DAG relationships, and automates multi-user review and
comment-mention workflows (`@user`, `@takenote:<user>`, `#AB1234`) backed by a
single-file SQLite WAL database.

## Workspace Structure

- `crates/wsid-core`: Core library containing the SQLite WAL storage engine,
  migrations, `AA0000` short ID generator, DAG cycle detection, comment parser,
  `in_review` subtask lifecycle, hybrid FTS5 trigram + `nucleo-matcher` fuzzy
  search, Focus/Eisenhower scoring, configuration, and third-party license
  bundles.
- `crates/wsid-mcp`: Stdio JSON-RPC 2.0 MCP server library exposing WSID task,
  focus queue, search, note, relation, and attachment tools to AI coding agents.
- `crates/wsid-cli`: Unified `wsid` terminal binary providing CLI subcommands
  (including `--accessible` linear screen-reader output and `--json`), the
  interactive Ratatui TUI (`wsid`), and the stdio MCP server (`wsid mcp`).
- `crates/wsid-gui`: Tauri v2 desktop application (`wsid-gui`) paired with a
  Vite + Svelte 5 + TypeScript frontend in `crates/wsid-gui/ui`.

## UI & Accessibility Guidelines

All changes to the Tauri desktop GUI (`crates/wsid-gui`) or Ratatui terminal UI
(`crates/wsid-cli/src/tui`) must follow the design, progressive-disclosure, and
accessibility rules in [`docs/AGENT_UI_GUIDELINES.md`](docs/AGENT_UI_GUIDELINES.md).

## Tooling & Tasks (`mise.toml`)

Tool versions (`rust`, `bun`, `cargo-about`, `cargo-deny`, `cargo-nextest`,
the Tauri CLI, `rumdl`, `taplo`, `typos`) are pinned, and project workflows are
managed, in [`mise.toml`](mise.toml):

- `mise run build` / `mise run build:debug`: Build the frontend bundle and Rust
  workspace (`release` or `debug`).
- `mise run run` / `mise run run:debug`: Build and run the default `wsid`
  CLI/TUI binary.
- `mise run run:gui` / `mise run run:gui:debug`: Build and run the `wsid-gui`
  desktop application.
- `mise run fmt` / `mise run fmt:check`: Format (or check formatting of) Rust,
  Svelte/TypeScript (`bun fmt`), TOML (`taplo`), and Markdown (`rumdl`).
- `mise run lint`: Run `cargo clippy` and `svelte-check`.
- `mise run licenses` / `mise run licenses:check`: Generate or verify
  third-party license attributions via `cargo-about`.
- `mise run test`: Run the workspace test suite (`cargo test --workspace`).
- `mise run test:ci`: Run the frontend tests and the workspace tests with
  `cargo nextest`, exactly as CI does.
- `mise run bundle`: Build the desktop installers with the Tauri CLI. It builds
  the frontend first, because Tauri does not.
- `mise run pc`: Run all pre-commit checks (`fmt:check`, `lint`,
  `licenses:check`, and `test`).
- `mise run hooks:install`: Install the git pre-commit hook (`mise generate
  git-pre-commit`).

## Git, Pull Requests & CI

- Branch from `dev` and open pull requests into `dev`. Squash-merge feature
  PRs. `dev` is merged into `main` with a merge commit, and a Release Please PR
  on `main` publishes stable releases.
- PR titles must be [Conventional Commits](https://www.conventionalcommits.org/)
  (`feat(cli): ...`, `fix(core): ...`, `ci: ...`). Release notes and version
  bumps are generated from them.
- Run `mise run pc` before pushing. CI (`.github/workflows/ci.yml`) runs the
  same checks plus `cargo deny`, `typos`, and tests on Linux, macOS, and
  Windows. The `ci-gate` job summarizes them and is the check to require.
- Add the `package-check` label to a PR to build unsigned installers for every
  platform. PRs into `main` always build them.
- Never edit `Cargo.lock` versions or release versions by hand. Release Please
  bumps `Cargo.toml`, `tauri.conf.json`, and `ui/package.json` together.
- Do not commit signing keys, certificates, or tokens. Secrets live in the
  `release` and `beta` GitHub environments.
