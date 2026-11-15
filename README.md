# WSID

**What Should I Do?** An offline-first task manager for people and their local
AI agents. It tells you what to work on next, so you spend less time deciding and
more time doing.

> WSID has not had a public release yet. Install instructions marked `FIXME:`
> are placeholders until the first release is published.

## What it is

WSID fills the gap between a text file of todos and a full project tracker such
as Jira. It is built for one person, or a small group on the same machine,
working alongside AI coding agents:

- **Local and offline.** Everything lives in a single SQLite file on your
  machine. There is no server and no account.
- **Shared with your agents.** An MCP server lets Claude, Cursor, and other
  tools read the same task list, pick up work, leave notes, and hand work back
  for review.
- **Opinionated about what is next.** Instead of leaving you to sort a backlog,
  the Focus view ranks actionable tasks with a scoring algorithm.
- **Accessible.** The terminal and desktop apps are designed for keyboard use
  and screen readers.

## How it works

Each task has an importance (1 to 5), an optional deadline, a time estimate, an
assignee, tags, a project, and notes. Tasks can block each other, and WSID keeps
those links acyclic.

**Focus view.** Tasks that are blocked are hidden. The rest are scored from how
important they are and how urgent they are. Urgency comes from *effective
slack*: the time left until the effective deadline minus the time the task is
estimated to take. Less slack means higher urgency, and overdue tasks escalate
quickly. A task without a deadline of its own inherits one from an ancestor
task. You can change the ranking with a preset:

| Preset | Favors |
|--------|--------|
| Balanced | Importance and urgency together |
| Quick Wins | Short tasks |
| Eat the Frog | The hardest, most important task first |
| Unblock Others | Tasks that other people or agents are waiting on |

**Eisenhower matrix.** Tasks are also sorted into four quadrants (urgent and
important, important, urgent, neither) using the same slack calculation.

**Collaboration.** Users can be humans or agents. In notes and descriptions,
`@user` mentions create a blocking task for that user, `@takenote:<user>` leaves
a non-blocking note for them, and `#AB1234` links another task by its short ID.
A task can go through a review step, so an agent can finish work and wait for a
human to approve it.

## Ways to use it

| Form | Command | Best for |
|------|---------|----------|
| Terminal UI (TUI) | `wsid` | Daily use from the keyboard: Focus, Kanban, Eisenhower, Database, Data Entry, and Settings views |
| Command line (CLI) | `wsid add`, `wsid next`, `wsid show`, `wsid search`, and more | Scripts and quick edits. Use `--json` for machine-readable output or `--accessible` for linear, screen-reader-friendly text |
| MCP server | `wsid mcp` | Letting AI agents read and update your tasks over stdio |
| Desktop app (GUI) | `wsid-gui` | A windowed interface built with Tauri |

All four share one database, so a task added in the terminal shows up in the
desktop app and in your agent's tools.

### Where your data lives

The default database is `wsid.db` in your OS data folder:

- macOS: `~/Library/Application Support/wsid`
- Windows: `%LOCALAPPDATA%\wsid`
- Linux: `$XDG_DATA_HOME/wsid` (or `~/.local/share/wsid`)

To use a different database, pass `--db <path>`, set `WSID_DB`, or use `--local`
to keep a `.wsid.db` file in the current project folder.

## Install

`FIXME:` Replace each placeholder with a tested command once the first release
is published.

| Platform | Package | Command |
|----------|---------|---------|
| macOS and Linux | Homebrew (CLI) | `FIXME: brew install WhatShouldIDoToday/tap/wsid` |
| macOS | Homebrew (desktop app) | `FIXME: brew install --cask WhatShouldIDoToday/tap/wsid-gui` |
| Windows | winget | `FIXME: winget install WhatShouldIDoToday.WSID` |
| Linux | Snap | `FIXME: sudo snap install wsid` |
| Linux | `.deb`, `.rpm`, AppImage, `.flatpak` | `FIXME:` download from [GitHub Releases](https://github.com/WhatShouldIDoToday/WSID/releases) |
| macOS, Windows | Installer | `FIXME:` download the `.dmg` or `.exe` from [GitHub Releases](https://github.com/WhatShouldIDoToday/WSID/releases) |

`FIXME:` Beta builds from the `dev` branch are planned, with separate packages
and a separate data folder from stable. Document the beta install commands once
they exist.

### Connect an AI agent

Run `wsid mcp status` to see which AI clients WSID finds, then
`wsid mcp install --agent <id>` or `wsid mcp install --all-detected`. See
[`docs/MCP.md`](docs/MCP.md) for the supported clients and troubleshooting.
To add WSID by hand instead:

```json
{
  "mcpServers": {
    "wsid": {
      "command": "/absolute/path/to/wsid",
      "args": ["mcp"]
    }
  }
}
```

Use the full path to `wsid`. Desktop apps on macOS usually do not see your shell
`PATH`.

## Build from source

[`mise`](https://mise.jdx.dev) installs the pinned tools (Rust, Bun, and
others):

```sh
mise install
mise run build      # frontend bundle and Rust workspace, release mode
mise run run        # run the terminal app
mise run run:gui    # run the desktop app
mise run bundle     # build desktop installers with the Tauri CLI
mise run pc         # format, lint, license, and test checks
```

On Linux, building the desktop app needs the WebKitGTK development packages
described in the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

## Repository layout

- `crates/wsid-core`: storage, scoring, search, and configuration.
- `crates/wsid-mcp`: the MCP server library.
- `crates/wsid-cli`: the `wsid` binary (CLI, TUI, and `wsid mcp`).
- `crates/wsid-gui`: the Tauri desktop app, with a Svelte frontend in `ui/`.

## Contributing

Work happens on feature branches that are squash-merged into `dev`. `dev` is
merged into `main` for stable releases. Pull request titles use
[Conventional Commits](https://www.conventionalcommits.org/) (for example
`feat(cli): add export command`), because release notes and version numbers are
generated from them. Agents and contributors should also read
[`AGENTS.md`](AGENTS.md).

## License

Apache-2.0. See [`LICENSE`](LICENSE). Third-party license notices are available
inside the app (`wsid about --full`).
