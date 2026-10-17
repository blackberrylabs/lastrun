//! What Lastrun finds: the actions the agents took, the sessions they belong to, and the
//! findings. An action is never the text of an exchange — no prompt, no reply, no command
//! output — only what was done: a command (secrets masked), a file and what happened to it,
//! a tool of an MCP server, with its approval and how it ended.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Serialize;

/// The kinds of finding. One slug each, one place on the four common severities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// A destructive command that ran and was not denied.
    Destructive,
    /// A secret in clear inside a transcript: the transcript became a secret store.
    SecretInTranscript,
    /// A file written or deleted outside the session's project.
    OutsideProject,
    /// A session run without asking, with a destructive or out-of-project action in it.
    UnattendedDestructive,
    /// Code downloaded and piped straight into a shell.
    PipeToShell,
    /// A script written by an agent in one session, run in a later one.
    DeferredScript,
    /// Packages installed by an agent.
    PackageInstall,
    /// Sessions the host will prune within seven days.
    PruningSoon,
    /// Calls denied by the person or by a rule.
    Denied,
    /// A readable host whose transcripts lack something (exit code, approval, diff, time).
    PartialRecord,
    /// A host present on the machine that keeps no readable record of its actions.
    NoActionLog,
}

impl Kind {
    pub const ALL: [Kind; 11] = [
        Kind::Destructive,
        Kind::SecretInTranscript,
        Kind::OutsideProject,
        Kind::UnattendedDestructive,
        Kind::PipeToShell,
        Kind::DeferredScript,
        Kind::PackageInstall,
        Kind::PruningSoon,
        Kind::Denied,
        Kind::PartialRecord,
        Kind::NoActionLog,
    ];

    /// The slug, as it appears in `--json` and in the tower's report.
    pub fn slug(self) -> &'static str {
        match self {
            Kind::Destructive => "destructive",
            Kind::SecretInTranscript => "secret-in-transcript",
            Kind::OutsideProject => "outside-project",
            Kind::UnattendedDestructive => "unattended-destructive",
            Kind::PipeToShell => "pipe-to-shell",
            Kind::DeferredScript => "deferred-script",
            Kind::PackageInstall => "package-install",
            Kind::PruningSoon => "pruning-soon",
            Kind::Denied => "denied",
            Kind::PartialRecord => "partial-record",
            Kind::NoActionLog => "no-action-log",
        }
    }

    /// Where the kind sits on the four common severities (written in the CHANGELOG).
    pub fn severity(self) -> bbl_report::Severity {
        use bbl_report::Severity::*;
        match self {
            Kind::Destructive | Kind::SecretInTranscript => ActNow,
            Kind::OutsideProject
            | Kind::UnattendedDestructive
            | Kind::PipeToShell
            | Kind::DeferredScript => Review,
            Kind::PackageInstall | Kind::PruningSoon => GoodToKnow,
            Kind::Denied | Kind::PartialRecord | Kind::NoActionLog => Info,
        }
    }

    /// Label as the CLI prints it.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Destructive => "DESTRUCTIVE",
            Kind::SecretInTranscript => "SECRET",
            Kind::OutsideProject => "OUTSIDE",
            Kind::UnattendedDestructive => "UNATTENDED",
            Kind::PipeToShell => "PIPE TO SHELL",
            Kind::DeferredScript => "DEFERRED",
            Kind::PackageInstall => "PACKAGES",
            Kind::PruningSoon => "PRUNING SOON",
            Kind::Denied => "DENIED",
            Kind::PartialRecord => "PARTIAL",
            Kind::NoActionLog => "NO LOG",
        }
    }
}

/// What an action is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActKind {
    /// A shell command.
    Shell,
    /// A file created, modified, deleted or read through a tool.
    File,
    /// A tool of an MCP server.
    Mcp,
    /// A web fetch or search through a tool of the host.
    Network,
    /// A sub-agent started.
    Agent,
    Other,
}

/// What happened to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileOp {
    Create,
    Modify,
    Delete,
    Read,
}

/// The approval of one call, in three states and no more (decision R4): denied is the only
/// state most hosts record for certain; allowed never says by whom unless the host does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Decision {
    Denied,
    Allowed,
    NotRecorded,
}

#[derive(Debug, Clone, Serialize)]
pub struct Approval {
    pub decision: Decision,
    /// Who or what decided, when the host writes it (`config`, `user`, `rule`).
    pub source: Option<String>,
    /// The permission mode or approval policy in force (`default`, `bypassPermissions`,
    /// `danger-full-access`, `yolo`).
    pub mode: Option<String>,
}

impl Approval {
    pub fn not_recorded() -> Self {
        Approval {
            decision: Decision::NotRecorded,
            source: None,
            mode: None,
        }
    }
}

/// How the call ended, as far as the transcript says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Completed,
    Failed,
    Denied,
    Cancelled,
    Interrupted,
    /// Started, no result written (still running, crashed, or the host keeps none).
    Unknown,
}

/// One action of an agent, projected on the common schema.
#[derive(Debug, Clone, Serialize)]
pub struct Action {
    pub host: String,
    pub session: String,
    pub call_id: Option<String>,
    /// The tool name as the host wrote it (`Bash`, `run_shell_command`, `developer__shell`).
    pub tool: String,
    pub kind: ActKind,
    /// For a shell command: the command, secrets masked.
    pub command: Option<String>,
    /// For a file: the path (absolute when the host or the session's directory says so).
    pub path: Option<PathBuf>,
    pub op: Option<FileOp>,
    /// Inferred from a command (`rm …`) rather than written by the host as a file action.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub inferred: bool,
    /// For an MCP tool: the server.
    pub server: Option<String>,
    /// Lines added and removed, when the host writes the diff.
    pub diff: Option<(u32, u32)>,
    pub when: Option<DateTime<Utc>>,
    pub cwd: Option<PathBuf>,
    pub approval: Approval,
    pub status: Status,
    pub exit_code: Option<i64>,
    pub model: Option<String>,
    /// Run by a sub-agent.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub subagent: bool,
    /// The transcript the action was read from, and the line (JSONL) when there is one.
    pub source: PathBuf,
    pub line: Option<u32>,
}

impl Action {
    pub fn ran(&self) -> bool {
        !matches!(
            self.status,
            Status::Denied | Status::Failed | Status::Cancelled
        ) && self.approval.decision != Decision::Denied
    }

    pub fn touches_file(&self) -> bool {
        matches!(
            self.op,
            Some(FileOp::Create | FileOp::Modify | FileOp::Delete)
        )
    }
}

/// One session of one host.
#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub host: String,
    pub host_name: String,
    pub id: String,
    /// The working directory the session started in.
    pub project: Option<PathBuf>,
    pub started: Option<DateTime<Utc>>,
    pub ended: Option<DateTime<Utc>>,
    /// The permission modes seen during the session.
    pub modes: Vec<String>,
    pub actions: usize,
    pub commands: usize,
    pub files_touched: usize,
    pub denied: usize,
    pub subagent: bool,
    pub transcript: PathBuf,
    /// When the host will prune the transcript, if it prunes.
    pub prune_at: Option<DateTime<Utc>>,
    /// The host's command to reopen the session.
    pub resume: Option<String>,
}

/// A host as Lastrun sees it.
#[derive(Debug, Clone, Serialize)]
pub struct HostStatus {
    pub id: String,
    pub name: String,
    pub present: bool,
    /// Why it counts as present, or where it was looked for.
    pub presence: String,
    /// Reads its actions (`true`), or keeps no readable record (`false`).
    pub readable: bool,
    pub transcripts: Vec<PathBuf>,
    pub sessions: usize,
    pub actions: usize,
    /// What its transcripts lack (`exit code`, `approval per call`, `diff`, `time per message`).
    pub missing: Vec<String>,
    pub retention_days: Option<i64>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub kind: Kind,
    /// Stable across runs for the same thing.
    pub id: String,
    pub title: String,
    pub subject: String,
    pub detail: String,
    pub host: Option<String>,
    pub session: Option<String>,
    pub path: Option<PathBuf>,
    pub line: Option<u32>,
    /// The commands or paths behind the finding, masked.
    pub evidence: Vec<String>,
    /// What the person could do — shown, never run.
    pub hint: Option<String>,
    /// A command of the host itself, shown to copy.
    pub command: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Inventory {
    pub home: PathBuf,
    /// First day of the window, local date.
    pub since: String,
    pub hosts: Vec<HostStatus>,
    pub sessions: Vec<Session>,
    pub actions: Vec<Action>,
    pub findings: Vec<Finding>,
    pub errors: Vec<String>,
    /// How many characters of a session id tell it apart from every other (`short`).
    #[serde(skip)]
    pub short_len: usize,
}

/// The shortest length, from 8, at which every id is told apart from the others: OpenCode's
/// ids begin with their time, so sessions opened a minute apart share a long prefix.
pub fn short_len<'a>(ids: impl Iterator<Item = &'a str>) -> usize {
    let mut v: Vec<&str> = ids.collect();
    v.sort_unstable();
    v.dedup();
    v.windows(2)
        .map(|w| {
            w[0].chars()
                .zip(w[1].chars())
                .take_while(|(a, b)| a == b)
                .count()
                + 1
        })
        .fold(8, usize::max)
}

impl Inventory {
    /// A session id as the CLI, the app and the findings show it; `lastrun actions` takes it.
    pub fn short<'a>(&self, id: &'a str) -> &'a str {
        match id.char_indices().nth(self.short_len.max(8)) {
            Some((i, _)) => &id[..i],
            None => id,
        }
    }

    pub fn session(&self, host: &str, id: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.host == host && s.id == id)
    }

    pub fn commands(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| a.kind == ActKind::Shell)
            .count()
    }

    pub fn deleted(&self) -> impl Iterator<Item = &Action> {
        self.actions
            .iter()
            .filter(|a| a.op == Some(FileOp::Delete) && a.ran())
    }

    pub fn denied(&self) -> impl Iterator<Item = &Action> {
        self.actions
            .iter()
            .filter(|a| a.approval.decision == Decision::Denied || a.status == Status::Denied)
    }
}
