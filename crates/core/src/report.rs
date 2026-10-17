//! The `bbl-report/1` view of an inventory, for the Blackberrylabs control tower. Nothing
//! here is new: only what the CLI and the app already show — commands masked, no text of
//! any exchange.

use bbl_report::{
    Action, ActionKind, Evidence, Finding, Module, Open, Report, Subject, SubjectKind, Summary,
    Tone,
};

use crate::model::{Inventory, Kind};

/// Binary name of the app, as packaged.
pub const APP: &str = "lastrun-app";
/// Binary name of the command line, as packaged.
pub const CLI: &str = "lastrun";

pub fn module() -> Module {
    Module {
        id: "lastrun".into(),
        name: "Lastrun".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        verb: "replay".into(),
        tagline: "What your AI agents actually did on this machine — commands, files, packages — session by session".into(),
        app: Some(APP.into()),
        cli: Some(CLI.into()),
        home: "https://github.com/blackberrylabs/lastrun".into(),
    }
}

/// Build the report. `duration_ms` is the time the scan took.
pub fn build(inv: &Inventory, duration_ms: u64) -> Report {
    let mut report = Report::new(module());
    report.duration_ms = duration_ms;
    let n = inv.findings.len();
    let deleted = inv.deleted().count();
    let worst = inv
        .findings
        .iter()
        .map(|f| f.kind.severity().number())
        .min();
    report.summary = vec![
        Summary {
            key: "sessions_30d".into(),
            label: "Sessions".into(),
            value: inv.sessions.len().to_string(),
            tone: Tone::Neutral,
        },
        Summary {
            key: "commands_30d".into(),
            label: "Commands run".into(),
            value: inv.commands().to_string(),
            tone: Tone::Neutral,
        },
        Summary {
            key: "files_deleted_30d".into(),
            label: "Files deleted".into(),
            value: deleted.to_string(),
            tone: if deleted == 0 {
                Tone::Good
            } else {
                Tone::Amber
            },
        },
        Summary {
            key: "findings".into(),
            label: "Findings".into(),
            value: n.to_string(),
            tone: match worst {
                None => Tone::Good,
                Some(1) => Tone::Alert,
                Some(2) => Tone::Amber,
                _ => Tone::Neutral,
            },
        },
    ];
    for f in &inv.findings {
        let mut actions = Vec::new();
        if let Some(h) = &f.hint {
            actions.push(Action {
                kind: ActionKind::Hint,
                label: h.clone(),
                target: None,
            });
        }
        if let Some(c) = &f.command {
            actions.push(Action {
                kind: ActionKind::Command,
                label: "Reopen the session in its host".into(),
                target: Some(c.clone()),
            });
        }
        if let Some(p) = &f.path {
            actions.push(Action {
                kind: ActionKind::OpenFile,
                label: "Open file".into(),
                target: Some(p.display().to_string()),
            });
        }
        if let Some(u) = &f.url {
            actions.push(Action {
                kind: ActionKind::OpenUrl,
                label: "Source of the rule".into(),
                target: Some(u.clone()),
            });
        }
        let subject = match f.kind {
            Kind::NoActionLog | Kind::PartialRecord | Kind::PruningSoon => SubjectKind::Host,
            Kind::SecretInTranscript | Kind::DeferredScript => SubjectKind::File,
            _ => SubjectKind::Other,
        };
        let mut evidence: Vec<Evidence> = f
            .evidence
            .iter()
            .map(|e| Evidence {
                label: e.clone(),
                path: None,
                line: None,
            })
            .collect();
        if let Some(p) = &f.path {
            evidence.push(Evidence {
                label: crate::scan::display_home(p, &inv.home),
                path: Some(p.display().to_string()),
                line: f.line,
            });
        }
        report.findings.push(Finding {
            id: format!("{}/{}", f.kind.slug(), f.id),
            severity: f.kind.severity(),
            kind: f.kind.slug().into(),
            title: f.title.clone(),
            subject: Subject {
                kind: subject,
                label: f.subject.clone(),
            },
            detail: f.detail.clone(),
            evidence,
            actions,
            open: Some(Open {
                app: APP.into(),
                args: vec!["--select".into(), f.id.clone()],
            }),
        });
    }
    report.notes.push(format!(
        "{} session(s), {} action(s) since {} from {} host(s) present; read-only, no text of any exchange",
        inv.sessions.len(),
        inv.actions.iter().filter(|a| !a.inferred).count(),
        inv.since,
        inv.hosts.iter().filter(|h| h.present).count()
    ));
    report
}
