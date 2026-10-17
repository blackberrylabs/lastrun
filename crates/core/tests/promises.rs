//! The promises every Blackberrylabs tool keeps, and Lastrun's own, checked on this crate.

use std::path::{Path, PathBuf};

use bbl_hostrules::env::{Env, Os};
use lastrun_core::{scan_with, Options};

mod common;
use common as fixture_home;

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn the_core_never_writes_and_never_reaches_the_network() {
    let mut files = Vec::new();
    rs_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(files.len() > 10);
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let code = text.split("#[cfg(test)]").next().unwrap_or("");
        for forbidden in [
            "fs::write",
            "File::create",
            "fs::remove",
            "fs::rename",
            "OpenOptions",
            "create_dir",
            "std::net",
            "TcpStream",
            "reqwest",
            "ureq",
            "Command::new",
            "SQLITE_OPEN_READ_WRITE",
            "SQLITE_OPEN_CREATE",
        ] {
            assert!(
                !code.contains(forbidden),
                "{}: mentions {forbidden}",
                f.display()
            );
        }
    }
}

fn scanned() -> (tempfile::TempDir, lastrun_core::Inventory) {
    let tmp = tempfile::tempdir().unwrap();
    fixture_home::fixture(tmp.path());
    let env = Env::fixture(tmp.path(), Os::Linux);
    let (inv, _) = scan_with(
        &env,
        &lastrun_core::sheets::embedded(),
        &lastrun_core::rules::Rules::embedded(),
        &Options::default(),
    );
    (tmp, inv)
}

#[test]
fn the_report_conforms() {
    let (_tmp, inv) = scanned();
    let report = lastrun_core::report::build(&inv, 1);
    let json = report.to_json();
    let back = bbl_report::conformance::check(&json).expect("the report conforms to bbl-report/1");
    assert_eq!(back.module.id, "lastrun");
    assert_eq!(back.findings.len(), inv.findings.len());
    assert_eq!(
        back.summary
            .iter()
            .map(|s| s.key.as_str())
            .collect::<Vec<_>>(),
        [
            "sessions_30d",
            "commands_30d",
            "files_deleted_30d",
            "findings"
        ]
    );
    for f in &back.findings {
        assert_eq!(f.open.as_ref().unwrap().app, "lastrun-app");
    }
}

#[test]
fn no_text_of_an_exchange_and_no_secret_ever_comes_out() {
    let (_tmp, inv) = scanned();
    let inventory = serde_json::to_string(&inv).unwrap();
    let report = lastrun_core::report::build(&inv, 1).to_json();
    let key = fixture_home::fake_key();
    for out in [&inventory, &report] {
        assert!(!out.contains(&key), "the planted key came out");
        for c in fixture_home::CANARIES {
            assert!(!out.contains(c), "{c} came out");
        }
    }
    // the secret is found, masked
    let f = inv
        .findings
        .iter()
        .find(|f| f.kind == lastrun_core::Kind::SecretInTranscript)
        .unwrap();
    assert!(
        f.evidence.iter().all(|e| e.contains('…')),
        "{:?}",
        f.evidence
    );
}

#[test]
fn a_scan_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    fixture_home::fixture(tmp.path());
    let before = walk(tmp.path());
    let env = Env::fixture(tmp.path(), Os::Linux);
    let _ = scan_with(
        &env,
        &lastrun_core::sheets::embedded(),
        &lastrun_core::rules::Rules::embedded(),
        &Options::default(),
    );
    assert_eq!(
        walk(tmp.path()),
        before,
        "a file was created, removed or modified"
    );
}

fn walk(dir: &Path) -> Vec<(PathBuf, u64, Option<std::time::SystemTime>)> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                let md = std::fs::metadata(&p).ok();
                out.push((
                    p,
                    md.as_ref().map(|m| m.len()).unwrap_or(0),
                    md.and_then(|m| m.modified().ok()),
                ));
            }
        }
    }
    out.sort();
    out
}
