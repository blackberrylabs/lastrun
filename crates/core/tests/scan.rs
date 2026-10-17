//! A fake home with one host of each format and one finding of each kind.
//! `LASTRUN_FIXTURE_OUT=/some/dir cargo test -p lastrun-core --test scan write_fixture`
//! writes it on disk, to look at the CLI and the app by hand.

mod common;

use std::path::{Path, PathBuf};

use bbl_hostrules::env::{Env, Os};
use chrono::Utc;
use common::fixture;
use lastrun_core::model::{ActKind, Decision, FileOp, Kind, Status};
use lastrun_core::{scan_with, Options};

fn env(home: &Path) -> Env {
    Env::fixture(home, Os::Linux)
}

#[test]
fn one_finding_of_each_kind() {
    let tmp = tempfile::tempdir().unwrap();
    fixture(tmp.path());
    let home: PathBuf = tmp.path().to_path_buf();
    let (inv, _) = scan_with(
        &env(&home),
        &lastrun_core::sheets::embedded(),
        &lastrun_core::rules::Rules::embedded(),
        &Options::default(),
    );
    let kinds: Vec<Kind> = inv.findings.iter().map(|f| f.kind).collect();
    for k in Kind::ALL {
        assert!(
            kinds.contains(&k),
            "no {k:?} in {kinds:?}\n{:#?}",
            inv.findings
        );
    }
    // the sessions, newest first
    let ids: Vec<&str> = inv.sessions.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids,
        ["oc-s3", "cx-s2", "gm-s4", "cc-s1", "cc-s0"],
        "{ids:?}"
    );
    // the refusal is denied, the script is created, the deletes are there
    let edit = inv
        .actions
        .iter()
        .find(|a| a.call_id.as_deref() == Some("t5"))
        .unwrap();
    assert_eq!(
        (edit.approval.decision, edit.status),
        (Decision::Denied, Status::Denied)
    );
    let deleted: Vec<String> = inv
        .deleted()
        .map(|a| a.path.as_ref().unwrap().display().to_string())
        .collect();
    assert!(
        deleted.iter().any(|d| d.ends_with("proj/old.txt")),
        "{deleted:?}"
    );
    assert!(
        deleted.iter().any(|d| d.ends_with("proj/build")),
        "{deleted:?}"
    );
    assert!(inv
        .actions
        .iter()
        .any(|a| a.kind == ActKind::File && a.op == Some(FileOp::Create)));
    // destructive only for the reset, not for `rm -rf build`
    let d: Vec<&str> = inv
        .findings
        .iter()
        .filter(|f| f.kind == Kind::Destructive)
        .map(|f| f.title.as_str())
        .collect();
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(d[0].contains("git reset --hard"));
    // the session in bypassPermissions is the unattended one
    let u = inv
        .findings
        .iter()
        .find(|f| f.kind == Kind::UnattendedDestructive)
        .unwrap();
    assert_eq!(u.session.as_deref(), Some("cc-s1"));
    // pruning: the 25-day-old Claude session, with the setting to raise
    let p = inv
        .findings
        .iter()
        .find(|f| f.kind == Kind::PruningSoon)
        .unwrap();
    assert!(p.hint.as_deref().unwrap().contains("cleanupPeriodDays"));
    // packages: left-pad
    let pk = inv
        .findings
        .iter()
        .find(|f| f.kind == Kind::PackageInstall)
        .unwrap();
    assert!(pk.evidence.iter().any(|e| e.contains("left-pad")));
    // deferred: written by Claude, run by Codex
    let df = inv
        .findings
        .iter()
        .find(|f| f.kind == Kind::DeferredScript)
        .unwrap();
    assert!(
        df.detail.contains("Claude Code") && df.detail.contains("Codex CLI"),
        "{}",
        df.detail
    );
    // outside: ~/.bashrc
    let o = inv
        .findings
        .iter()
        .find(|f| f.kind == Kind::OutsideProject)
        .unwrap();
    assert!(
        o.evidence.iter().any(|e| e.contains(".bashrc")),
        "{:?}",
        o.evidence
    );
    // Cursor present, no log; the hosts not on this machine are not findings
    assert!(inv
        .findings
        .iter()
        .any(|f| f.kind == Kind::NoActionLog && f.host.as_deref() == Some("cursor")));
    assert!(!inv
        .findings
        .iter()
        .any(|f| f.host.as_deref() == Some("warp")));
}

#[test]
fn the_window_leaves_older_actions_out() {
    let tmp = tempfile::tempdir().unwrap();
    fixture(tmp.path());
    let opt = Options {
        now: Utc::now(),
        since_days: 7,
    };
    let (inv, _) = scan_with(
        &env(tmp.path()),
        &lastrun_core::sheets::embedded(),
        &lastrun_core::rules::Rules::embedded(),
        &opt,
    );
    assert!(
        !inv.sessions.iter().any(|s| s.id == "cc-s0"),
        "a 25-day-old session in a 7-day window"
    );
}

#[test]
fn write_fixture() {
    if let Ok(dir) = std::env::var("LASTRUN_FIXTURE_OUT") {
        let p = PathBuf::from(dir);
        std::fs::create_dir_all(&p).unwrap();
        fixture(&p);
    }
}
