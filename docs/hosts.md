# Host sheets

One TOML file per host in `hosts/`, embedded in the binary. The common fields come from
[`bbl-hostrules`](https://github.com/blackberrylabs/bbl-hostrules) (`id`, `name`, `vendor`,
`kind`, `status`, `references`, `hints`, `[[locations]]` with `scope`, `format`, `path`,
`[locations.source]`); Lastrun adds:

## Per host

| Field | Meaning |
|---|---|
| `[detect]` `paths`, `binaries` | How the host is found on the machine. |
| `missing` | What its transcripts lack, in words (`exit code`, `approval per call`, `diff`, `time per call`); shown once as `partial-record` when the host is present. |
| `own_dirs` | The host's own folders: a write there is not "outside the project". |
| `[unattended]` `modes`, `source` | The permission modes, as the transcript writes them, under which the host acts without asking (`bypassPermissions`, `danger-full-access`, `yolo`). |
| `[resume]` `command`, `source` | The host's command to reopen a session; `{session}` is replaced. |
| `[no_action_log]` `reason`, `exists`, `source` | For a host that keeps no readable record of its actions: why, and what exists anyway. A host has locations or this, never both. |

## Per location (a transcript)

| Field | Meaning |
|---|---|
| `reader` | `claude-jsonl`, `codex-jsonl`, `gemini-jsonl`, `qwen-jsonl`, `opencode-sqlite`, `goose-sqlite`, `cline-tasks`, `cline-sessions`, `copilot-jsonl`, `zed-sqlite`, `kiro-sqlite`. |
| `retention_days` | The host's default retention, when it deletes transcripts by age. |
| `retention_setting`, `retention_file` | The setting that changes it and where it lives; read to compute the pruning date (`general.sessionRetention.enabled: false` turns it off). |
| `note` | The fields the reader uses, and anything a reader of the sheet should know. |

Every location and every `no_action_log` has a `source` with `kind` (`documentation`,
`source-code`, `observed`), `url`, `read_on`, a verbatim `quote` and an optional `note`. The
loader refuses a sheet without one.

## Command rules

`rules/destructive.toml` and `rules/pipe-to-shell.toml`: `[[rule]]` with `id`, `label`, a
`regex` matched on one simple command (after `sudo`, `env`, assignments and `bash -c` are
unwrapped) — or a `special` evaluated in code when the danger depends on the paths
(`rm-recursive`, `find-delete`) — and a `source`. `rules/packages.toml`: `[[manager]]` with a
`regex` whose `args` group holds the package names.

## SQLite

Databases are opened with `mode=ro&immutable=1`: SQLite writes nothing, not even a `-wal` or
`-shm` file. The price is that changes the host has not yet checkpointed are not seen; a
session still running may show fewer actions than it has.
