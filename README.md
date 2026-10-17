# Lastrun

**What your AI agents actually did on this machine — commands, files, packages — session by
session, read back from the hosts' own transcripts.**

After a session with an AI coding agent, nobody has the timeline. An agent ran
`git reset --hard` at startup, a sub-agent deleted a folder, a script written yesterday ran
today, a key printed by a command now sits in a transcript — and the person finds out later,
from the damage. Yet most hosts already write every tool call to disk: Claude Code keeps the
command, its result and the patch of each edit; Codex keeps its rollouts; OpenCode, Goose and
Zed keep SQLite databases. Scattered, in formats each host calls internal, pruned after 30
days by some, and unreadable by eye.

Lastrun reads those transcripts after the fact — no hook to install, nothing to have set up
before — and turns them into **actions**: the command (secrets masked), the file and what
happened to it, the tool of an MCP server, the approval and how it ended. **Never the text of
an exchange**: no prompt, no reply, no command output.

## What it finds

| Finding | What it is | Severity |
|---|---|---|
| `destructive` | A destructive command that ran and was not denied: `rm -r` on the home directory, the root, the working directory or a path outside the project; `git reset --hard`; `git clean -f`; `git push --force`; `DROP TABLE`; `find … -delete`; `mkfs`; `dd of=/dev/…`; a one-line script that deletes | act now |
| `secret-in-transcript` | A credential in clear inside a transcript — printed by a command, or read from a file — masked | act now |
| `outside-project` | Files written or deleted outside the session's project (temporary directories and the host's own folders aside) | review |
| `unattended-destructive` | A session run without asking (`bypassPermissions`, `danger-full-access`, `yolo`) with a destructive command or a write outside the project | review |
| `pipe-to-shell` | `curl … \| sh`, `bash <(curl …)` | review |
| `deferred-script` | A script written by an agent in one session and run in a later one — what it did is not in any transcript, only its name | review |
| `package-install` | Packages installed by an agent (npm, pnpm, yarn, pip, uv, cargo, apt, brew, go) | good to know |
| `pruning-soon` | Sessions the host will delete within 7 days (Claude Code, Gemini CLI: 30 days by default) | good to know |
| `denied` | Calls the host recorded as refused | information |
| `partial-record` | What a readable host's transcripts lack: exit code, approval per call, diff, time | information |
| `no-action-log` | A host present on the machine that keeps no readable record of its actions | information |

Every command rule names its source — the git or package manager documentation, or the
incident it caused (`rules/*.toml`). Approval is shown in three states and no more:
**denied** (the only state most hosts record for certain), **allowed** (by whom only when the
host says so), **not recorded**.

## Hosts

Twelve hosts whose transcripts record their actions, each read from its upstream source code
or documentation (`hosts/*.toml`): Claude Code (and the Code and Cowork tabs of Claude
Desktop), Codex CLI, Gemini CLI, Qwen Code, OpenCode, Kilo Code, Goose, Cline, Roo Code,
GitHub Copilot CLI, Zed, Kiro CLI. Nine more are recognised and shown as keeping no readable
record — with what exists anyway: Aider, Amp, Cursor, Claude Desktop chats, Devin, JetBrains
AI Assistant, Trae, Warp, Windsurf.

## Use

```sh
lastrun                       # the findings, most serious first
lastrun sessions              # the sessions of the last 30 days, newest first
lastrun actions 3f9a          # what one session did, in order (id or its first characters)
lastrun actions 3f9a --all    # with the reads, searches and listings
lastrun deleted               # every file deleted, by a tool or by a command
lastrun packages              # every package installed by an agent
lastrun denied                # every call recorded as refused
lastrun hosts                 # present or not, readable or not, what each lacks
lastrun --since 90            # a wider window (older transcripts may be gone)
lastrun --host codex          # one host
lastrun report                # bbl-report/1 for the Blackberrylabs control tower
lastrun --json …              # for scripts
lastrun-app                   # the desktop app
```

## Promises

- **Read-only.** Nothing is written outside `~/.config/lastrun/` (the app's preferences).
  Databases are opened read-only and immutable: not even a `-wal` file is created.
- **No network.** Links are shown, never followed.
- **Never the text of an exchange.** No prompt, no reply, no command output, in the CLI, the
  app or the report — checked by a test that plants them and looks for them.
- **Secrets masked** the moment they are read, in commands and in transcripts.
- **A state, not a judgment.** "Ran `git reset --hard origin/main`, completed" — never "the
  agent wanted to", never "dangerous" without the rule that says so.
- **Not a surveillance tool.** It reads the machine of the person who runs it, tells no one,
  and does not say who typed a prompt.

## Part of Blackberrylabs

Lastrun is a module of the [Blackberrylabs](https://github.com/blackberrylabs) family —
read-only tools that show what AI coding agents do to your machine. Want only Lastrun?
It is complete on its own. Want several? The
[control tower](https://github.com/blackberrylabs/blackberrylabs) gathers what every
module finds in one list, most serious first: `lastrun report` is what it reads,
`lastrun-app --select` is how it opens Lastrun at a finding.

## Build

```sh
cargo build --release          # target/release/lastrun, lastrun-app
cargo test --workspace
packaging/build.sh             # .deb, .rpm, AppImage into dist/
```

A Rust workspace: `crates/core` (host sheets, one action reader per transcript format, command
rules, findings, the report — no UI, no network), `crates/cli`, `crates/app` (iced, on
[`bbl-ui`](https://github.com/blackberrylabs/bbl-ui)). Host sheets, path resolution and secret
masking come from [`bbl-hostrules`](https://github.com/blackberrylabs/bbl-hostrules); the
report from [`bbl-report`](https://github.com/blackberrylabs/bbl-report).
`#![forbid(unsafe_code)]`. The design brief, plan and journal are kept in French at the root.

## Status

In development — command line first, the app after its mock-up is validated.

## License

MIT — Alexandre Do.
