# Using WSID with AI clients

`wsid mcp` is a Model Context Protocol (MCP) server that talks over stdio. AI
clients start it themselves, so you only need to tell each client where the
`wsid` binary is. `wsid mcp install` does that for you.

## Install

```sh
wsid mcp status                          # detected clients, and which have WSID
wsid mcp install --agent cursor          # one client; repeat --agent for more
wsid mcp install --all-detected          # every client found on this machine
wsid mcp install --agent zed --dry-run   # show what would change, write nothing
wsid mcp install --agent codex --print   # print the config, edit nothing
wsid mcp uninstall --agent cursor
```

Add `--json` for machine-readable output. `--accessible` gives the same plain,
linear text as the default output.

Installing twice changes nothing. Before the first change to a file, WSID
copies it to `<file>.wsid-backup`. Comments and every other setting are kept.
If a file cannot be parsed, WSID stops and leaves it alone.

If a `wsid` entry already points to a different program, WSID reports a
`conflict` and changes nothing. Pass `--force` to replace it. `--force` also
creates the config file for a client WSID could not detect.

The beta build registers itself as `wsid-beta`, so it can sit beside the
stable `wsid` entry.

## Supported clients

| ID | Client | How WSID is added |
|----|--------|-------------------|
| `claude-desktop` | Claude Desktop | Edits `claude_desktop_config.json` |
| `claude-code` | Claude Code | Runs `claude mcp add --scope user` |
| `cursor` | Cursor | Edits `~/.cursor/mcp.json` |
| `vscode` | VS Code | Runs `code --add-mcp` |
| `devin` (alias `windsurf`) | Devin for Terminal, Windsurf | Edits `mcp_config.json` |
| `zed` | Zed | Edits `settings.json` (comments kept) |
| `opencode` | OpenCode | Edits `opencode.json` or `.jsonc` |
| `cline` | Cline | Edits `cline_mcp_settings.json` |
| `codex` | Codex CLI | Edits `~/.codex/config.toml` |
| `gemini` | Gemini CLI | Edits `~/.gemini/settings.json` |

Claude Code and VS Code are configured through their own commands, so those
files are only read. To remove VS Code's entry, WSID edits `mcp.json` itself.

## Which binary gets registered

WSID registers, in this order:

1. The path given with `--binary`.
2. The Homebrew link, such as `/opt/homebrew/bin/wsid`, when installed with
   Homebrew. It survives upgrades.
3. The `wsid` found on your `PATH`, when it is the program you are running.
4. The path of the running program.

## Manual setup

For any client, the server is the command `wsid` with the argument `mcp`:

```json
{
  "mcpServers": {
    "wsid": { "command": "/path/to/wsid", "args": ["mcp"] }
  }
}
```

`wsid mcp install --agent <id> --print` prints the exact shape for a client.

## Troubleshooting

- **`not detected`:** the client's config folder was not found. Start the
  client once, or use `--force`.
- **`conflict`:** an existing `wsid` entry points somewhere else. Check it with
  `wsid mcp status`, then use `--force` to replace it.
- **Snap:** a Snap cannot edit other programs' config files. `wsid mcp install`
  prints the config to add by hand, using `/snap/bin/wsid`.
- **Flatpak:** clients outside the sandbox cannot launch a Flatpak's binary.
  Install the command line tool on your system and run the command there.
- **Windows:** after a winget install, check that the registered path is the
  link winget created and not a versioned folder.
- **Protocol versions:** WSID speaks both eras. It answers the `initialize`
  handshake of revisions 2024-11-05 through 2025-11-25. It also serves
  2026-07-28 statelessly: any request whose `params._meta` carries
  `io.modelcontextprotocol/protocolVersion` and
  `io.modelcontextprotocol/clientCapabilities` is handled without a handshake,
  and `server/discover` reports the supported versions, capabilities and
  server info. A request with an unknown modern version gets error `-32022`
  with the `supported` list.

## Client support for 2026-07-28

Checked on 2026-11-14 against public pages. Most sources are secondary, so
verify against the client's own release notes. Until a client speaks
2026-07-28, WSID serves it through `initialize`, so this does not block
anyone.

| Client | 2026-07-28 status | Source |
|--------|-------------------|--------|
| Claude Code | Stdio servers are probed for 2026-07-28 by default from v2.1.292 per its release notes (`MCP_PROTOCOL_NEGOTIATION=legacy` opts out). The MCP docs describe a staged rollout, so the two disagree. | [release notes](https://github.com/anthropics/claude-code/releases/tag/v2.1.292), [docs](https://code.claude.com/docs/en/mcp.md), as quoted by [Mixed-News](https://mixed-news.com/en/claude-code-2-1-292-stdio-mcp-protocol-2026-07-28-docs-contradict/) |
| Claude Desktop | Unverified. Anthropic only says support is rolling out across Claude products. | [announcement](https://claude.com/blog/bringing-mcp-2026-07-28-to-claude) |
| Codex CLI | Opt-in behind the `mcp_2026_07_28` feature flag, off by default. Third-party posts say v0.147.0 added opt-in support. | [PR #34747](https://github.com/openai/codex/pull/34747), [issue #33952](https://github.com/openai/codex/issues/33952), [changelog](https://developers.openai.com/codex/changelog) |
| Cursor | Not supported. A Cursor staff member said it uses 2025-11-25 and earlier, with no timeline. | [forum thread](https://forum.cursor.com/t/mcp-client-cannot-connect-to-modern-only-2026-07-28-streamable-http-servers-legacy-initialize-rejected/172536) |
| VS Code | Unverified. A third-party tracker listed 2025-11-25 as of 2026-07-31. | [Zuplo tracker](https://zuplo.com/learn/mcp/compatibility) |
| Zed | Unverified. The same tracker listed 2025-11-25. | [Zuplo tracker](https://zuplo.com/learn/mcp/compatibility) |
| Gemini CLI | Unverified. The same tracker listed 2025-06-18. | [Zuplo tracker](https://zuplo.com/learn/mcp/compatibility) |
| Devin / Windsurf, OpenCode, Cline | Unverified. No public statement found. | none |
