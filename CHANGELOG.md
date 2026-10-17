# Changelog

## [Unreleased] — 0.1.0

First version: what the AI agents did on a machine, read back from the hosts' transcripts —
read-only, no network, never the text of an exchange.

### Findings and their place on the four common severities

- **Act now**: `destructive` (a destructive command that ran and was not denied),
  `secret-in-transcript` (a credential in clear in a transcript).
- **Review**: `outside-project`, `unattended-destructive`, `pipe-to-shell`, `deferred-script`.
- **Good to know**: `package-install`, `pruning-soon`.
- **Information**: `denied`, `partial-record`, `no-action-log`.

### Read

- 21 host sheets (`hosts/*.toml`, schema `docs/hosts.md`): 12 with a readable action log, 9
  shown as keeping none, each with its source.
- 11 action readers: Claude Code JSONL (sub-agents under `subagents/`, `toolUseResult`,
  `permissionDecision`, `permissionMode`, the refusal text), Codex rollouts (`.jsonl` and
  `.zst`, `function_call`, apply_patch envelopes, `patch_apply_end`, the paginated
  `command_execution` with `declined`), Gemini CLI (`toolCalls` and `$patch`), Qwen Code
  (`functionCall`, `toolCallResult`, approval mode), OpenCode and Kilo (SQLite, V1 `part` and
  V2 `session_message`, one call counted once), Goose (`toolRequest` / `toolResponse`), Cline
  and Roo task files, Cline's SDK sessions, Copilot CLI events (with `permission.completed`),
  Zed threads (JSON or zstd), Kiro (the predecessor's shape, marked observed).
- Command rules (`rules/*.toml`): 11 destructive, 2 pipe-to-shell, 9 package managers, each
  sourced. `rm` targets become deleted files.

### App

- `lastrun-app`: the family's screen — a headline, four cards (sessions, commands, files
  deleted, packages), the findings in the middle; on the left four tabs, **Sessions**,
  **Deleted**, **Packages**, **Refused**; on the right the finding or the session picked,
  its actions in order (filtered by kind, searched in commands and paths only), and the diff
  of an edit, read again from the transcript on demand, secrets masked (Claude Code, Codex,
  OpenCode, Kilo, Gemini CLI, Qwen Code). A deleted file says whether it exists again.
  English and French, light and dark. `--select <finding id>` (the tower), `--session <id>`,
  `--tab`, `--since`, `--home`.

### Summary figures for the tower

`sessions_30d`, `commands_30d`, `files_deleted_30d`, `findings`.

### Masking

- A credential of a known shape (a provider's prefix: `sk-ant-`, `ghp_`, `AKIA`, a JWT…) keeps
  its first 6 and last 4 characters, as everywhere in the family.
- A value under a secret-looking name (`DB_PASSWORD=`, `apiKey:`, `Bearer`) shows none of its
  characters: `DB_PASSWORD=••••••••`. The name must end with KEY, TOKEN, SECRET, PASSWORD,
  PASSWD, PWD or CREDENTIAL(S), and not be a name that only ends like one (a key that indexes
  — `cacheKey`, `primaryKey` —, a public key, a paging token, a bare `key`); identifiers,
  dotted names, paths, code and references are not values. JSON is read unescaped, value beside its key, so an escaped line break never glues
  two values together.
