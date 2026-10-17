//! Lastrun's host sheets (`hosts/*.toml`): the common schema comes from `bbl-hostrules`;
//! this module adds what reading actions needs — per location, the reader of the transcript
//! format and the host's retention; per host, how it is detected, what its transcripts
//! lack, which permission modes run without asking, and, for a host that keeps no readable
//! record of its actions, why and what exists anyway. Schema in `docs/hosts.md`. Every claim
//! names its source; the loader refuses a location without one.

use bbl_hostrules::rules::{Location, Source};
use bbl_hostrules::{Extension, HostExtension, HostRule, Rules};
use serde::{Deserialize, Serialize};

/// The readers, one per transcript format.
pub const READERS: &[&str] = &[
    "claude-jsonl",
    "codex-jsonl",
    "gemini-jsonl",
    "qwen-jsonl",
    "opencode-sqlite",
    "goose-sqlite",
    "cline-tasks",
    "cline-sessions",
    "copilot-jsonl",
    "zed-sqlite",
    "kiro-sqlite",
];

/// The fields Lastrun adds to every location: a transcript and how to read it.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RunLocation {
    /// The reader of this format (`READERS`).
    pub reader: String,
    /// Default retention in days, when the host prunes its transcripts by age.
    pub retention_days: Option<i64>,
    /// The setting that changes it (`cleanupPeriodDays`, `general.sessionRetention.maxAge`)
    /// and the file it lives in.
    pub retention_setting: Option<String>,
    pub retention_file: Option<String>,
    pub note: Option<String>,
}

impl Extension for RunLocation {
    fn validate(loc: &Location<Self>) -> Result<(), String> {
        if !READERS.contains(&loc.ext.reader.as_str()) {
            return Err(format!("unknown reader `{}`", loc.ext.reader));
        }
        if loc.ext.retention_setting.is_some() && loc.ext.retention_days.is_none() {
            return Err("a retention setting without its default `retention_days`".into());
        }
        Ok(())
    }
}

/// How a host is detected.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Detect {
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub binaries: Vec<String>,
}

/// A host that keeps no readable record of its actions: why, and what exists anyway.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NoLog {
    pub reason: String,
    /// What the person can look at instead (an export, a history file, the host's own view).
    pub exists: Option<String>,
    pub source: Source,
}

/// Permission modes under which the host runs actions without asking.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Unattended {
    /// The values as the transcript writes them (`bypassPermissions`, `danger-full-access`).
    pub modes: Vec<String>,
    pub source: Source,
}

/// The host's own command to reopen a session (`{session}` is replaced).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Resume {
    pub command: String,
    pub source: Source,
}

/// The fields Lastrun adds to every host.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RunHost {
    #[serde(default)]
    pub detect: Detect,
    pub no_action_log: Option<NoLog>,
    /// What the transcripts lack, in words (`exit code`, `approval per call`, `diff`,
    /// `time per message`), shown once per present host.
    #[serde(default)]
    pub missing: Vec<String>,
    pub unattended: Option<Unattended>,
    pub resume: Option<Resume>,
    /// The host's own directories: a write there is the host's business, not "outside the
    /// project".
    #[serde(default)]
    pub own_dirs: Vec<String>,
}

impl HostExtension for RunHost {
    fn validate<X: Extension>(rule: &HostRule<X, Self>) -> Result<(), String> {
        if rule.locations.is_empty() && rule.ext.no_action_log.is_none() {
            return Err("a host with no transcript location needs `no_action_log`".into());
        }
        if !rule.locations.is_empty() && rule.ext.no_action_log.is_some() {
            return Err("a host is readable or has no action log, not both".into());
        }
        if rule.ext.detect.paths.is_empty() && rule.ext.detect.binaries.is_empty() {
            return Err("a host needs `detect.paths` or `detect.binaries`".into());
        }
        Ok(())
    }
}

pub type Host = HostRule<RunLocation, RunHost>;
pub type Sheets = Rules<RunLocation, RunHost>;

/// The sheets shipped with Lastrun, embedded at build time.
pub const EMBEDDED: &[(&str, &str)] = &[
    ("aider.toml", include_str!("../../../hosts/aider.toml")),
    ("amp.toml", include_str!("../../../hosts/amp.toml")),
    (
        "claude-code.toml",
        include_str!("../../../hosts/claude-code.toml"),
    ),
    (
        "claude-desktop.toml",
        include_str!("../../../hosts/claude-desktop.toml"),
    ),
    ("cline.toml", include_str!("../../../hosts/cline.toml")),
    ("codex.toml", include_str!("../../../hosts/codex.toml")),
    ("copilot.toml", include_str!("../../../hosts/copilot.toml")),
    ("cursor.toml", include_str!("../../../hosts/cursor.toml")),
    ("devin.toml", include_str!("../../../hosts/devin.toml")),
    (
        "gemini-cli.toml",
        include_str!("../../../hosts/gemini-cli.toml"),
    ),
    ("goose.toml", include_str!("../../../hosts/goose.toml")),
    (
        "jetbrains-ai.toml",
        include_str!("../../../hosts/jetbrains-ai.toml"),
    ),
    (
        "kilo-code.toml",
        include_str!("../../../hosts/kilo-code.toml"),
    ),
    ("kiro.toml", include_str!("../../../hosts/kiro.toml")),
    (
        "opencode.toml",
        include_str!("../../../hosts/opencode.toml"),
    ),
    (
        "qwen-code.toml",
        include_str!("../../../hosts/qwen-code.toml"),
    ),
    (
        "roo-code.toml",
        include_str!("../../../hosts/roo-code.toml"),
    ),
    ("trae.toml", include_str!("../../../hosts/trae.toml")),
    ("warp.toml", include_str!("../../../hosts/warp.toml")),
    (
        "windsurf.toml",
        include_str!("../../../hosts/windsurf.toml"),
    ),
    ("zed.toml", include_str!("../../../hosts/zed.toml")),
];

/// Load the embedded sheets. A sheet that does not load is a bug, not a runtime condition.
pub fn embedded() -> Sheets {
    Sheets::load(EMBEDDED.iter().copied()).expect("embedded host sheets are valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_sheet_loads_and_is_sourced() {
        let s = embedded();
        assert_eq!(s.hosts.len(), 21);
        let readable = s.hosts.iter().filter(|h| !h.locations.is_empty()).count();
        assert_eq!(readable, 12, "twelve hosts with a readable action log");
        for h in &s.hosts {
            for l in &h.locations {
                assert!(
                    !l.source.quote.is_empty(),
                    "{}: a location without a quote",
                    h.id
                );
            }
            if let Some(n) = &h.ext.no_action_log {
                assert!(
                    !n.source.quote.is_empty(),
                    "{}: no_action_log without a quote",
                    h.id
                );
            }
        }
    }
}
