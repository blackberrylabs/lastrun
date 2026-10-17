//! Lastrun, core: what the AI agents did on this machine, read back from the transcripts the
//! hosts already write. No UI, no network.
//!
//! The Blackberrylabs promises, the same for every tool: read-only (nothing written outside
//! `~/.config/lastrun/`, and only by the app's preferences; databases opened read-only and
//! immutable); no network; secrets masked the moment they are read; actions shown, never
//! run; every rule sourced. And Lastrun's own: never the text of an exchange — no prompt,
//! no reply, no command output — only the actions; approval in three states, never
//! "manual" inferred.
//!
//! `tests/promises.rs` checks that the source of this crate never mentions a write or a
//! network API, that a planted secret never comes out, and that `report` conforms to
//! `bbl-report/1`.

#![forbid(unsafe_code)]

pub mod command;
pub mod findings;
pub mod model;
pub mod read;
pub mod report;
pub mod rules;
pub mod scan;
pub mod sheets;

pub use model::{Action, Finding, Inventory, Kind, Session};
pub use scan::{scan, scan_with, Analysis, Options};
