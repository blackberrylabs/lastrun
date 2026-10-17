//! Lastrun desktop app (iced, on bbl-ui): on the left the sessions, the files deleted, the
//! packages installed and the refusals, in four tabs; in the middle the findings, most
//! serious first; on the right the finding or the session picked, its actions in order and
//! the diff of an edit on demand. Reads the same core as the CLI; never the text of an
//! exchange; writes only its preferences (`~/.config/lastrun/settings.json`).
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Datelike, Local, Utc};
use iced::widget::{
    button, checkbox, column, container, horizontal_space, mouse_area, row, scrollable, text,
    text_input, Space,
};
use iced::{Alignment, Background, Border, Color, Element, Length, Task, Theme};

use bbl_report::Severity;
use bbl_ui::palette::{alert, amber, good, muted};
use bbl_ui::widgets::{card, inner, kpi, pill, small_button, swatch};
use bbl_ui::{Lang, Prefs, Strings};
use lastrun_core::diff;
use lastrun_core::findings::{describe, outside_project};
use lastrun_core::model::{ActKind, Action, Decision, FileOp, Finding, Inventory, Kind, Status};
use lastrun_core::scan::display_home;
use lastrun_core::sheets::Sheets;
use lastrun_core::Analysis;

const EN: &str = include_str!("../../../locales/en.json");
const FR: &str = include_str!("../../../locales/fr.json");

/// Above this many actions, a session's chronology asks for a filter.
const MAX_ROWS: usize = 1500;

fn main() -> iced::Result {
    iced::application(App::title, App::update, App::view)
        .theme(App::theme)
        .window_size((1280.0, 820.0))
        .run_with(App::new)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Sessions,
    Deleted,
    Packages,
    Refused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActFilter {
    All,
    Commands,
    Files,
    Packages,
    Mcp,
    Web,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Sel {
    Nothing,
    Finding(String),
    /// A session, and one of its actions (an index into `Inventory::actions`).
    Session {
        host: String,
        id: String,
        action: Option<usize>,
    },
}

/// What one scan gives the app: the inventory, what the commands meant, and what the view
/// needs that the view must not compute (it would stat files at every frame).
#[derive(Debug, Default)]
struct Data {
    inv: Inventory,
    an: Analysis,
    sheets: Option<Sheets>,
    /// Deleted paths that exist again today.
    exists: HashSet<PathBuf>,
    /// Actions that wrote or deleted outside their session's project.
    outside: HashSet<usize>,
    /// A delete inferred from a command → that command, masked.
    by_command: HashMap<usize, String>,
    /// Destructive findings per (host, session).
    destructive: HashMap<(String, String), usize>,
}

#[derive(Debug, Clone)]
enum Message {
    Loaded(Arc<Data>, u64),
    Rescan,
    SetDays(i64),
    SetLang(Lang),
    ToggleTheme,
    SelectFinding(String),
    SelectSession(String, String, Option<usize>),
    SelectAction(usize),
    FindingsHost(Option<String>),
    LeftHost(Option<String>),
    ToggleInfo(bool),
    ToggleSubagents(bool),
    ToggleOutsideOnly(bool),
    SetTab(Tab),
    SetActFilter(ActFilter),
    ActSearch(String),
    ToggleFilesRead(bool),
    ShowDiff(usize),
    OpenFile(String),
    OpenUrl(String),
    Copy(String),
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct Extra {
    /// The window, in days (decision R5: 30 by default).
    #[serde(default)]
    since_days: Option<i64>,
}

struct App {
    prefs: Prefs<Extra>,
    lang: Lang,
    strings: Strings,
    dark: bool,
    home: PathBuf,
    days: i64,
    data: Option<Arc<Data>>,
    loading: bool,
    elapsed_ms: u64,
    sel: Sel,
    findings_host: Option<String>,
    left_host: Option<String>,
    show_info: bool,
    show_subagents: bool,
    outside_only: bool,
    tab: Tab,
    act_filter: ActFilter,
    act_search: String,
    files_read: bool,
    /// The diff asked for: the action, and what was read (None: not kept by the host).
    diff: Option<(usize, Option<Vec<diff::Line>>)>,
    /// `--session <id or its first characters>`, opened once the scan is done.
    pending_session: Option<String>,
}

impl App {
    fn new() -> (App, Task<Message>) {
        let prefs: Prefs<Extra> = Prefs::load("lastrun");
        let lang = prefs.initial_lang();
        let dark = prefs.initial_dark();
        let strings = Strings::load(EN, FR, lang);
        let args = parse_args();
        let home = args.home.unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/"))
        });
        let days = args.since.or(prefs.extra.since_days).unwrap_or(30).max(1);
        let task = load(home.clone(), days);
        (
            App {
                prefs,
                lang,
                strings,
                dark,
                home,
                days,
                data: None,
                loading: true,
                elapsed_ms: 0,
                sel: args.select.map(Sel::Finding).unwrap_or(Sel::Nothing),
                findings_host: None,
                left_host: None,
                show_info: false,
                show_subagents: true,
                outside_only: false,
                tab: args.tab,
                act_filter: ActFilter::All,
                act_search: String::new(),
                files_read: false,
                diff: None,
                pending_session: args.session,
            },
            task,
        )
    }

    fn title(&self) -> String {
        self.strings.t("app.title", &[])
    }

    fn theme(&self) -> Theme {
        if self.dark {
            Theme::Dark
        } else {
            Theme::Light
        }
    }

    fn remember(&mut self) {
        self.prefs.remember(self.lang, self.dark);
        self.prefs.save();
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Loaded(data, ms) => {
                if let Sel::Finding(id) = &self.sel {
                    if let Some(f) = data.inv.findings.iter().find(|f| &f.id == id) {
                        if f.kind.severity() == Severity::Info {
                            self.show_info = true;
                        }
                    }
                }
                if let Some(p) = self.pending_session.take() {
                    let hits: Vec<_> = data
                        .inv
                        .sessions
                        .iter()
                        .filter(|x| x.id.starts_with(&p))
                        .collect();
                    if let [x] = hits.as_slice() {
                        self.sel = Sel::Session {
                            host: x.host.clone(),
                            id: x.id.clone(),
                            action: None,
                        };
                    }
                }
                self.data = Some(data);
                self.elapsed_ms = ms;
                self.loading = false;
                self.diff = None;
            }
            Message::Rescan => {
                self.loading = true;
                return load(self.home.clone(), self.days);
            }
            Message::SetDays(d) => {
                self.days = d.max(1);
                self.prefs.extra.since_days = Some(self.days);
                self.prefs.save();
                self.loading = true;
                return load(self.home.clone(), self.days);
            }
            Message::SetLang(l) => {
                self.lang = l;
                self.strings = Strings::load(EN, FR, l);
                self.remember();
            }
            Message::ToggleTheme => {
                self.dark = !self.dark;
                self.remember();
            }
            Message::SelectFinding(id) => {
                self.sel = Sel::Finding(id);
                self.diff = None;
            }
            Message::SelectSession(host, id, action) => {
                let same = matches!(&self.sel, Sel::Session { host: h, id: i, .. } if *h == host && *i == id);
                if !same {
                    self.act_filter = ActFilter::All;
                    self.act_search.clear();
                }
                // an action picked from the left that the "files read" switch would hide
                if let (Some(i), Some(d)) = (action, &self.data) {
                    if d.inv
                        .actions
                        .get(i)
                        .is_some_and(|a| a.op == Some(FileOp::Read))
                    {
                        self.files_read = true;
                    }
                }
                self.sel = Sel::Session { host, id, action };
                self.diff = None;
            }
            Message::SelectAction(i) => {
                if let Sel::Session { action, .. } = &mut self.sel {
                    *action = Some(i);
                }
                self.diff = None;
            }
            Message::FindingsHost(h) => self.findings_host = h,
            Message::LeftHost(h) => self.left_host = h,
            Message::ToggleInfo(b) => self.show_info = b,
            Message::ToggleSubagents(b) => self.show_subagents = b,
            Message::ToggleOutsideOnly(b) => self.outside_only = b,
            Message::SetTab(t) => self.tab = t,
            Message::SetActFilter(f) => self.act_filter = f,
            Message::ActSearch(s) => self.act_search = s,
            Message::ToggleFilesRead(b) => self.files_read = b,
            Message::ShowDiff(i) => {
                if let Some(d) = &self.data {
                    if let Some(a) = d.inv.actions.get(i) {
                        // read on demand, from the transcript, read-only
                        self.diff = Some((i, diff::read(a)));
                    }
                }
            }
            Message::OpenFile(p) | Message::OpenUrl(p) => {
                // the one thing the app launches: the system opener, on a path or a URL —
                // never a command of ours, never a command of the agents
                let target = p
                    .rsplit_once(':')
                    .filter(|(_, l)| {
                        l.chars().all(|c| c.is_ascii_digit()) && !p.starts_with("http")
                    })
                    .map(|(a, _)| a.to_string())
                    .unwrap_or(p);
                let _ = std::process::Command::new("xdg-open").arg(&target).spawn();
            }
            Message::Copy(s) => return iced::clipboard::write(s),
        }
        Task::none()
    }

    // ---- helpers ---------------------------------------------------------------------

    fn sev_color(&self, s: Severity) -> Color {
        match s {
            Severity::ActNow => alert(self.dark),
            Severity::Review => amber(self.dark),
            Severity::GoodToKnow => good(self.dark),
            Severity::Info => muted(),
        }
    }

    fn sel_bg(&self) -> Color {
        if self.dark {
            Color::from_rgb8(0x22, 0x33, 0x55)
        } else {
            Color::from_rgb8(0xe8, 0xf0, 0xfe)
        }
    }

    /// `Oct 17 17:44` / `17 oct. 17:44`, local time.
    fn when(&self, t: Option<DateTime<Utc>>) -> String {
        match t {
            Some(t) => {
                let l = t.with_timezone(&Local);
                format!("{} {}", self.day(t), l.format("%H:%M"))
            }
            None => self.strings.t("time.none", &[]),
        }
    }

    fn day(&self, t: DateTime<Utc>) -> String {
        let l = t.with_timezone(&Local);
        let m = (l.month0()) as usize;
        match self.lang {
            Lang::Fr => format!("{} {}", l.day(), MONTHS_FR[m]),
            _ => format!("{} {}", MONTHS_EN[m], l.day()),
        }
    }

    fn num(&self, n: usize) -> String {
        let s = n.to_string();
        let sep = match self.lang {
            Lang::Fr => '\u{202f}',
            _ => ',',
        };
        // groups of three from the right
        let mut out = String::new();
        let mut k = 0;
        for c in s.chars().rev() {
            if k == 3 {
                out.push(sep);
                k = 0;
            }
            out.push(c);
            k += 1;
        }
        out.chars().rev().collect()
    }

    fn plural(&self, n: usize, key: &str) -> String {
        if n == 1 {
            self.strings.t(&format!("{key}_one"), &[])
        } else {
            self.strings.t(key, &[("n", &self.num(n))])
        }
    }

    /// The approval in three states (decision R4), with the source and mode the host wrote.
    fn approval(&self, a: &lastrun_core::model::Approval) -> String {
        let s = &self.strings;
        let mut out = s.t(
            match a.decision {
                Decision::Denied => "approval.denied",
                Decision::Allowed => "approval.allowed",
                Decision::NotRecorded => "approval.not_recorded",
            },
            &[],
        );
        let mut extra = Vec::new();
        if let Some(src) = &a.source {
            extra.push(src.clone());
        }
        if let Some(m) = &a.mode {
            extra.push(s.t("approval.mode", &[("mode", m)]));
        }
        if !extra.is_empty() {
            out.push_str(&format!(" ({})", extra.join(", ")));
        }
        out
    }

    fn host_name<'a>(&self, inv: &'a Inventory, id: &'a str) -> &'a str {
        inv.hosts
            .iter()
            .find(|h| h.id == id)
            .map(|h| h.name.as_str())
            .unwrap_or(id)
    }

    fn visible_findings<'a>(&self, inv: &'a Inventory) -> Vec<&'a Finding> {
        inv.findings
            .iter()
            .filter(|f| self.show_info || f.kind.severity() != Severity::Info)
            .filter(|f| {
                self.findings_host
                    .as_ref()
                    .is_none_or(|h| f.host.as_ref() == Some(h))
            })
            .collect()
    }

    fn counts(inv: &Inventory) -> [usize; 4] {
        let mut c = [0usize; 4];
        for f in &inv.findings {
            c[(f.kind.severity().number() - 1) as usize] += 1;
        }
        c
    }

    fn packages_count(d: &Data) -> (usize, BTreeMap<String, usize>, usize) {
        let mut names: BTreeSet<(String, String, String)> = BTreeSet::new();
        let mut sessions: BTreeSet<(String, String)> = BTreeSet::new();
        for (i, inst) in &d.an.installs {
            let a = &d.inv.actions[*i];
            if !a.ran() {
                continue;
            }
            sessions.insert((a.host.clone(), a.session.clone()));
            for p in &inst.packages {
                names.insert((
                    format!("{}/{}", a.host, a.session),
                    inst.manager.clone(),
                    lastrun_core::rules::package_key(&inst.manager, p),
                ));
            }
        }
        let mut by_manager: BTreeMap<String, usize> = BTreeMap::new();
        for (_, m, _) in &names {
            *by_manager.entry(m.clone()).or_default() += 1;
        }
        (names.len(), by_manager, sessions.len())
    }

    // ---- view ------------------------------------------------------------------------

    fn view(&self) -> Element<'_, Message> {
        let s = &self.strings;
        let data = self.data.as_deref();
        let body: Element<'_, Message> = match data {
            Some(d) => row![self.view_left(d), self.view_list(d), self.view_detail(d)]
                .spacing(12)
                .height(Length::Fill)
                .into(),
            None => container(
                text(s.t("header.scanning", &[]))
                    .size(16)
                    .style(text::secondary),
            )
            .center(Length::Fill)
            .into(),
        };
        let status = match data {
            Some(d) => s.t(
                "status.ready",
                &[
                    (
                        "hosts",
                        &d.inv.hosts.iter().filter(|h| h.present).count().to_string(),
                    ),
                    (
                        "readable",
                        &d.inv
                            .hosts
                            .iter()
                            .filter(|h| h.present && h.readable)
                            .count()
                            .to_string(),
                    ),
                    ("sessions", &self.num(d.inv.sessions.len())),
                ],
            ),
            None => s.t("header.scanning", &[]),
        };
        let theme_label = if self.dark {
            s.t("theme.light", &[])
        } else {
            s.t("theme.dark", &[])
        };
        let footer = bbl_ui::widgets::footer(
            status,
            self.lang,
            Message::SetLang,
            theme_label,
            Message::ToggleTheme,
        );
        let mut col = column![self.view_header(data)].spacing(12).padding(14);
        if let Some(d) = data {
            col = col.push(self.view_kpis(d));
        }
        col.push(body).push(footer).into()
    }

    fn view_header(&self, data: Option<&Data>) -> Element<'_, Message> {
        let s = &self.strings;
        let sentence: Element<'_, Message> = match data {
            None => text(s.t("headline.scanning", &[])).size(19).into(),
            Some(d) => {
                let inv = &d.inv;
                let readable = inv.hosts.iter().filter(|h| h.present && h.readable).count();
                let [act, review, _, _] = Self::counts(inv);
                let sessions = self.plural(inv.sessions.len(), "headline.sessions");
                let commands = self.plural(inv.commands(), "headline.commands");
                let deleted = self.plural(inv.deleted().count(), "headline.deleted");
                let days = self.days.to_string();
                let base = [
                    ("sessions", sessions.as_str()),
                    ("commands", commands.as_str()),
                    ("deleted", deleted.as_str()),
                    ("days", days.as_str()),
                ];
                if readable == 0 {
                    text(s.t("headline.none", &[])).size(19).into()
                } else if act == 0 && review == 0 {
                    text(s.t("headline.clear", &base)).size(19).into()
                } else if act == 0 {
                    let r = review.to_string();
                    let mut a = base.to_vec();
                    a.push(("review", r.as_str()));
                    text(s.t("headline.review", &a)).size(19).into()
                } else {
                    let r = review.to_string();
                    let marker = self.plural(act, "headline.things");
                    let mut a = base.to_vec();
                    a.push(("act", marker.as_str()));
                    a.push(("review", r.as_str()));
                    let full = s.t("headline.act", &a);
                    match full.find(&marker) {
                        Some(i) => {
                            let (x, rest) = full.split_at(i);
                            let (y, z) = rest.split_at(marker.len());
                            row![
                                text(x.to_string()).size(19),
                                text(y.to_string()).size(19).color(alert(self.dark)),
                                text(z.to_string()).size(19)
                            ]
                            .into()
                        }
                        None => text(full).size(19).into(),
                    }
                }
            }
        };
        let when = if self.loading {
            s.t("header.scanning", &[])
        } else {
            s.t(
                "header.read",
                &[("s", &format!("{:.1}", self.elapsed_ms as f64 / 1000.0))],
            )
        };
        let rescan: Element<'_, Message> = if self.loading {
            button(text(s.t("header.scanning", &[])).size(12))
                .style(button::secondary)
                .into()
        } else {
            button(text(s.t("header.rescan", &[])).size(12))
                .style(button::primary)
                .on_press(Message::Rescan)
                .into()
        };
        let mut r = row![sentence, horizontal_space()]
            .spacing(10)
            .align_y(Alignment::Center);
        for d in [30i64, 90] {
            r = r.push(chip(
                s.t("header.days", &[("n", &d.to_string())]),
                self.days == d,
                Message::SetDays(d),
            ));
        }
        r.push(text(when).size(12).style(text::secondary))
            .push(rescan)
            .into()
    }

    fn view_kpis(&self, d: &Data) -> Element<'_, Message> {
        let s = &self.strings;
        let inv = &d.inv;
        // sessions, per host, and the sub-agents among them
        let mut per_host: BTreeMap<&str, usize> = BTreeMap::new();
        for x in &inv.sessions {
            *per_host.entry(x.host_name.as_str()).or_default() += 1;
        }
        let mut hosts: Vec<(&str, usize)> = per_host.into_iter().collect();
        hosts.sort_by_key(|h| std::cmp::Reverse(h.1));
        let hosts_line = hosts
            .iter()
            .map(|(h, n)| format!("{} {n}", short_host(h)))
            .collect::<Vec<_>>()
            .join(" · ");
        let subs = inv.sessions.iter().filter(|x| x.subagent).count();
        let sessions_sub = if inv.sessions.is_empty() {
            s.t("kpi.sessions.none", &[])
        } else if subs > 0 {
            s.t(
                "kpi.sessions.sub_agents",
                &[("hosts", &hosts_line), ("n", &subs.to_string())],
            )
        } else {
            hosts_line
        };
        // commands, those that failed, the hosts that write no exit code
        let failed = inv
            .actions
            .iter()
            .filter(|a| a.kind == ActKind::Shell && a.status == Status::Failed)
            .count();
        let no_exit: Vec<String> = inv
            .hosts
            .iter()
            .filter(|h| {
                h.present && h.sessions > 0 && h.missing.iter().any(|m| m.starts_with("exit code"))
            })
            .map(|h| short_host(&h.name))
            .collect();
        let mut commands_sub = s.t("kpi.commands.failed", &[("n", &self.num(failed))]);
        if !no_exit.is_empty() {
            commands_sub.push_str(" · ");
            commands_sub.push_str(&s.t("kpi.commands.no_exit", &[("hosts", &no_exit.join(", "))]));
        }
        // deletes, by a tool or by a command, and outside their project
        let deleted: Vec<&Action> = inv.deleted().collect();
        let by_cmd = deleted.iter().filter(|a| a.inferred).count();
        let outside = inv
            .actions
            .iter()
            .enumerate()
            .filter(|(i, a)| a.op == Some(FileOp::Delete) && a.ran() && d.outside.contains(i))
            .count();
        // packages
        let (npk, by_manager, pk_sessions) = Self::packages_count(d);
        let managers = by_manager
            .iter()
            .map(|(m, n)| format!("{m} {n}"))
            .collect::<Vec<_>>()
            .join(" · ");
        row![
            kpi(
                s.t("kpi.sessions", &[]),
                self.num(inv.sessions.len()),
                sessions_sub,
                None
            ),
            kpi(
                s.t("kpi.commands", &[]),
                self.num(inv.commands()),
                commands_sub,
                None
            ),
            kpi(
                s.t("kpi.deleted", &[]),
                self.num(deleted.len()),
                s.t(
                    "kpi.deleted.sub",
                    &[
                        ("tool", &self.num(deleted.len() - by_cmd)),
                        ("command", &self.num(by_cmd)),
                        ("outside", &self.num(outside)),
                    ]
                ),
                (outside > 0).then(|| amber(self.dark))
            ),
            kpi(
                s.t("kpi.packages", &[]),
                self.num(npk),
                if npk == 0 {
                    s.t("kpi.packages.none", &[])
                } else {
                    s.t(
                        "kpi.packages.sub",
                        &[("managers", &managers), ("n", &pk_sessions.to_string())],
                    )
                },
                None
            ),
        ]
        .spacing(12)
        .into()
    }

    // ---- left column: four tabs ------------------------------------------------------

    fn view_left(&self, d: &Data) -> Element<'_, Message> {
        let s = &self.strings;
        let inv = &d.inv;
        let (npk, _, _) = Self::packages_count(d);
        let counts = [
            (Tab::Sessions, "tab.sessions", inv.sessions.len()),
            (Tab::Deleted, "tab.deleted", inv.deleted().count()),
            (Tab::Packages, "tab.packages", npk),
            (Tab::Refused, "tab.refused", inv.denied().count()),
        ];
        let mut tabs = row![].spacing(4);
        for (t, key, n) in counts {
            let b = button(text(s.t(key, &[("n", &self.num(n))])).size(12))
                .padding([4, 10])
                .on_press(Message::SetTab(t));
            tabs = tabs.push(if self.tab == t {
                b.style(button::primary)
            } else {
                b.style(button::text)
            });
        }
        let content: Element<'_, Message> = match self.tab {
            Tab::Sessions => self.view_sessions(d),
            Tab::Deleted => self.view_deleted(d),
            Tab::Packages => self.view_packages(d),
            Tab::Refused => self.view_refused(d),
        };
        card(
            column![
                tabs,
                scrollable(container(content).padding(iced::Padding {
                    right: 12.0,
                    ..Default::default()
                }))
                .height(Length::Fill)
            ]
            .spacing(8)
            .into(),
        )
        .width(Length::FillPortion(5))
        .height(Length::Fill)
        .into()
    }

    /// `All hosts` and one chip per host with sessions, for the left column.
    fn left_chips(&self, inv: &Inventory) -> Element<'_, Message> {
        let s = &self.strings;
        let mut hosts: Vec<(String, String)> = inv
            .sessions
            .iter()
            .map(|x| (x.host.clone(), x.host_name.clone()))
            .collect();
        hosts.sort();
        hosts.dedup();
        let mut chips = vec![chip(
            s.t("list.all_hosts", &[]),
            self.left_host.is_none(),
            Message::LeftHost(None),
        )];
        for (id, name) in hosts {
            let on = self.left_host.as_deref() == Some(id.as_str());
            chips.push(chip(short_host(&name), on, Message::LeftHost(Some(id))));
        }
        chip_lines(chips, 6)
    }

    fn left_row<'a>(
        &self,
        when: String,
        main: Element<'a, Message>,
        right: Option<Element<'a, Message>>,
        selected: bool,
        msg: Message,
    ) -> Element<'a, Message> {
        let mut r = row![
            text(when)
                .size(11)
                .style(text::secondary)
                .width(Length::Fixed(84.0)),
            container(main).width(Length::Fill)
        ]
        .spacing(8)
        .align_y(Alignment::Start);
        if let Some(x) = right {
            r = r.push(x);
        }
        let bg = selected.then(|| Background::Color(self.sel_bg()));
        mouse_area(
            container(r)
                .padding([5, 8])
                .width(Length::Fill)
                .style(move |_| container::Style {
                    background: bg,
                    border: Border {
                        radius: 6.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        )
        .on_press(msg)
        .into()
    }

    fn group_head<'a>(&self, label: String) -> Element<'a, Message> {
        container(text(label).size(11).style(text::secondary))
            .padding(iced::Padding {
                top: 8.0,
                bottom: 2.0,
                left: 4.0,
                ..Default::default()
            })
            .into()
    }

    fn view_sessions(&self, d: &Data) -> Element<'_, Message> {
        let s = &self.strings;
        let inv = &d.inv;
        let mut col = column![
            text(s.t("sessions.sub", &[]))
                .size(11)
                .style(text::secondary),
            self.left_chips(inv),
            checkbox(s.t("sessions.subagents", &[]), self.show_subagents)
                .on_toggle(Message::ToggleSubagents)
                .size(14)
                .text_size(11)
        ]
        .spacing(4);
        let soon = Utc::now() + chrono::Duration::days(7);
        let mut n = 0;
        for x in &inv.sessions {
            if self.left_host.as_ref().is_some_and(|h| *h != x.host)
                || (!self.show_subagents && x.subagent)
            {
                continue;
            }
            n += 1;
            let unattended: Vec<String> = d
                .sheets
                .as_ref()
                .and_then(|sh| sh.host(&x.host))
                .and_then(|h| h.ext.unattended.as_ref())
                .map(|u| u.modes.clone())
                .unwrap_or_default();
            let project = x
                .project
                .as_deref()
                .map(|p| display_home(p, &inv.home))
                .unwrap_or_else(|| s.t("sessions.no_project", &[]));
            // line 1: the project, the host, the id; line 2: the counts; line 3: the marks
            let title = row![
                text(project).size(13),
                text(short_host(&x.host_name))
                    .size(11)
                    .style(text::secondary),
                text(inv.short(&x.id).to_string())
                    .size(11)
                    .font(iced::Font::MONOSPACE)
                    .style(text::secondary),
            ]
            .spacing(6)
            .align_y(Alignment::Center);
            let mut meta = vec![s.t(
                "sessions.meta",
                &[
                    ("actions", &self.num(x.actions)),
                    ("commands", &self.num(x.commands)),
                    ("files", &self.num(x.files_touched)),
                ],
            )];
            if x.denied > 0 {
                meta.push(s.t("sessions.refused", &[("n", &x.denied.to_string())]));
            }
            let mut marks: Vec<Element<'_, Message>> = Vec::new();
            if let Some(nd) = d.destructive.get(&(x.host.clone(), x.id.clone())) {
                marks.push(pill(
                    s.t("sessions.destructive", &[("n", &nd.to_string())]),
                    alert(self.dark),
                ));
            }
            for m in &x.modes {
                let c = if unattended.contains(m) {
                    amber(self.dark)
                } else {
                    muted()
                };
                marks.push(pill(m.clone(), c));
            }
            if x.subagent {
                marks.push(pill(s.t("sessions.subagent", &[]), muted()));
            }
            if let Some(p) = x.prune_at {
                marks.push(pill(
                    s.t("sessions.pruned", &[("date", &self.day(p))]),
                    if p <= soon { alert(self.dark) } else { muted() },
                ));
            }
            let mut main = column![
                title,
                text(meta.join(" · ")).size(11).style(text::secondary)
            ]
            .spacing(2);
            if !marks.is_empty() {
                main = main.push(chip_lines(marks, 4));
            }
            let right: Option<Element<'_, Message>> = None;
            let selected = matches!(&self.sel, Sel::Session { host, id, .. } if *host == x.host && *id == x.id);
            col = col.push(self.left_row(
                self.when(x.ended.or(x.started)),
                main.into(),
                right,
                selected,
                Message::SelectSession(x.host.clone(), x.id.clone(), None),
            ));
        }
        if n == 0 {
            col = col.push(
                text(s.t("sessions.none", &[]))
                    .size(12)
                    .style(text::secondary),
            );
        }
        col.into()
    }

    fn view_deleted(&self, d: &Data) -> Element<'_, Message> {
        let s = &self.strings;
        let inv = &d.inv;
        let mut col = column![
            text(s.t("deleted.sub", &[]))
                .size(11)
                .style(text::secondary),
            self.left_chips(inv),
            checkbox(s.t("deleted.outside_only", &[]), self.outside_only)
                .on_toggle(Message::ToggleOutsideOnly)
                .size(14)
                .text_size(11)
        ]
        .spacing(4);
        let mut rows: Vec<(usize, &Action)> = inv
            .actions
            .iter()
            .enumerate()
            .filter(|(_, a)| a.op == Some(FileOp::Delete) && a.ran())
            .filter(|(_, a)| self.left_host.as_ref().is_none_or(|h| *h == a.host))
            .filter(|(i, _)| !self.outside_only || d.outside.contains(i))
            .collect();
        rows.reverse(); // newest first
        let mut last: Option<String> = None;
        for (i, a) in &rows {
            let x = inv.session(&a.host, &a.session);
            let group = format!(
                "{} — {} · {} {}",
                a.when.map(|t| self.day(t)).unwrap_or_default(),
                x.and_then(|x| x.project.as_deref())
                    .map(|p| display_home(p, &inv.home))
                    .unwrap_or_default(),
                short_host(self.host_name(inv, &a.host)),
                inv.short(&a.session)
            );
            if last.as_ref() != Some(&group) {
                col = col.push(self.group_head(group.clone()));
                last = Some(group);
            }
            let path = a
                .path
                .as_deref()
                .map(|p| display_home(p, &inv.home))
                .unwrap_or_else(|| "?".into());
            let mut head = row![text(path).size(12).font(iced::Font::MONOSPACE)]
                .spacing(6)
                .align_y(Alignment::Center);
            if d.outside.contains(i) {
                head = head.push(pill(s.t("deleted.outside", &[]), amber(self.dark)));
            }
            let how = match d.by_command.get(i) {
                Some(c) => s.t("deleted.by_command", &[("command", &shorten(c, 70))]),
                None if a.inferred => s.t("deleted.by_a_command", &[]),
                None => s.t("deleted.by_tool", &[("tool", &a.tool)]),
            };
            let right = a
                .path
                .as_ref()
                .filter(|p| d.exists.contains(*p))
                .map(|_| pill(s.t("deleted.exists_again", &[]), muted()));
            let selected = matches!(&self.sel, Sel::Session { action: Some(k), .. } if k == i);
            col = col.push(
                self.left_row(
                    a.when
                        .map(|t| t.with_timezone(&Local).format("%H:%M").to_string())
                        .unwrap_or_default(),
                    column![head, text(how).size(11).style(text::secondary)]
                        .spacing(2)
                        .into(),
                    right,
                    selected,
                    Message::SelectSession(a.host.clone(), a.session.clone(), Some(*i)),
                ),
            );
        }
        if rows.is_empty() {
            col = col.push(
                text(s.t("deleted.none", &[]))
                    .size(12)
                    .style(text::secondary),
            );
        } else {
            col = col.push(
                container(
                    text(s.t("deleted.folder_note", &[]))
                        .size(11)
                        .style(text::secondary),
                )
                .padding(iced::Padding {
                    top: 8.0,
                    ..Default::default()
                }),
            );
        }
        col.into()
    }

    fn view_packages(&self, d: &Data) -> Element<'_, Message> {
        let s = &self.strings;
        let inv = &d.inv;
        let mut col = column![
            text(s.t("packages.sub", &[]))
                .size(11)
                .style(text::secondary),
            self.left_chips(inv)
        ]
        .spacing(4);
        let mut rows: Vec<(usize, &lastrun_core::rules::Install)> =
            d.an.installs
                .iter()
                .filter(|(i, _)| inv.actions[*i].ran())
                .filter(|(i, _)| {
                    self.left_host
                        .as_ref()
                        .is_none_or(|h| *h == inv.actions[*i].host)
                })
                .map(|(i, x)| (*i, x))
                .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.0));
        for (i, inst) in &rows {
            let a = &inv.actions[*i];
            let what = if inst.packages.is_empty() {
                s.t("packages.project", &[])
            } else {
                inst.packages.join(", ")
            };
            let project = inv
                .session(&a.host, &a.session)
                .and_then(|x| x.project.as_deref())
                .map(|p| display_home(p, &inv.home))
                .unwrap_or_default();
            let selected = matches!(&self.sel, Sel::Session { action: Some(k), .. } if k == i);
            col = col.push(
                self.left_row(
                    self.when(a.when),
                    column![
                        row![
                            pill(inst.manager.clone(), muted()),
                            text(what).size(12).width(Length::Fill)
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                        text(format!(
                            "{} · {} · {project}",
                            short_host(self.host_name(inv, &a.host)),
                            inv.short(&a.session)
                        ))
                        .size(11)
                        .style(text::secondary)
                    ]
                    .spacing(2)
                    .into(),
                    None,
                    selected,
                    Message::SelectSession(a.host.clone(), a.session.clone(), Some(*i)),
                ),
            );
        }
        if rows.is_empty() {
            col = col.push(
                text(s.t("packages.none", &[]))
                    .size(12)
                    .style(text::secondary),
            );
        }
        col.into()
    }

    fn view_refused(&self, d: &Data) -> Element<'_, Message> {
        let s = &self.strings;
        let inv = &d.inv;
        let mut col = column![
            text(s.t("refused.sub", &[]))
                .size(11)
                .style(text::secondary),
            self.left_chips(inv)
        ]
        .spacing(4);
        let mut rows: Vec<(usize, &Action)> = inv
            .actions
            .iter()
            .enumerate()
            .filter(|(_, a)| a.approval.decision == Decision::Denied || a.status == Status::Denied)
            .filter(|(_, a)| self.left_host.as_ref().is_none_or(|h| *h == a.host))
            .collect();
        rows.reverse();
        for (i, a) in &rows {
            let selected = matches!(&self.sel, Sel::Session { action: Some(k), .. } if k == i);
            col = col.push(
                self.left_row(
                    self.when(a.when),
                    column![
                        text(shorten(&describe(a, &inv.home), 90))
                            .size(12)
                            .font(iced::Font::MONOSPACE),
                        text(format!(
                            "{} · {} · {}",
                            short_host(self.host_name(inv, &a.host)),
                            inv.short(&a.session),
                            self.approval(&a.approval)
                        ))
                        .size(11)
                        .style(text::secondary)
                    ]
                    .spacing(2)
                    .into(),
                    None,
                    selected,
                    Message::SelectSession(a.host.clone(), a.session.clone(), Some(*i)),
                ),
            );
        }
        if rows.is_empty() {
            col = col.push(
                text(s.t("refused.none", &[]))
                    .size(12)
                    .style(text::secondary),
            );
        }
        // the hosts present whose transcripts write no approval per call
        let blind: Vec<String> = inv
            .hosts
            .iter()
            .filter(|h| h.present && h.sessions > 0)
            .filter(|h| h.missing.iter().any(|m| m.starts_with("approval")))
            .map(|h| short_host(&h.name))
            .collect();
        if !blind.is_empty() {
            col = col.push(
                container(
                    text(s.t("refused.blind", &[("hosts", &blind.join(", "))]))
                        .size(11)
                        .style(text::secondary),
                )
                .padding(iced::Padding {
                    top: 8.0,
                    ..Default::default()
                }),
            );
        }
        col.into()
    }

    // ---- middle: the findings --------------------------------------------------------

    fn view_list(&self, d: &Data) -> Element<'_, Message> {
        let s = &self.strings;
        let inv = &d.inv;
        let visible = self.visible_findings(inv);
        let mut hosts: Vec<&str> = inv
            .findings
            .iter()
            .filter_map(|f| f.host.as_deref())
            .collect();
        hosts.sort();
        hosts.dedup();
        let mut chips = vec![chip(
            s.t("list.all_hosts", &[]),
            self.findings_host.is_none(),
            Message::FindingsHost(None),
        )];
        for id in hosts {
            chips.push(chip(
                short_host(self.host_name(inv, id)),
                self.findings_host.as_deref() == Some(id),
                Message::FindingsHost(Some(id.to_string())),
            ));
        }
        let controls = checkbox(s.t("list.show_info", &[]), self.show_info)
            .on_toggle(Message::ToggleInfo)
            .size(14)
            .text_size(11);
        let mut list = column![].spacing(2);
        if visible.is_empty() {
            list = list.push(text(s.t("list.none", &[])).size(12).style(text::secondary));
        }
        let mut last: Option<Severity> = None;
        for f in &visible {
            let sev = f.kind.severity();
            if last != Some(sev) {
                let n = visible.iter().filter(|x| x.kind.severity() == sev).count();
                list = list.push(self.group_head(format!(
                    "{} — {n}",
                    s.t(&format!("sev.{}", sev.number()), &[]).to_uppercase()
                )));
                last = Some(sev);
            }
            let selected = self.sel == Sel::Finding(f.id.clone());
            let who = f
                .host
                .as_deref()
                .map(|h| short_host(self.host_name(inv, h)))
                .unwrap_or_default();
            let r = row![
                swatch(self.sev_color(sev), 8),
                text(who)
                    .size(11)
                    .style(text::secondary)
                    .width(Length::Fixed(76.0)),
                text(plain(&f.title)).size(13).width(Length::Fill),
                text("›").size(13).style(text::secondary),
            ]
            .spacing(10)
            .align_y(Alignment::Center);
            let bg = selected.then(|| Background::Color(self.sel_bg()));
            let item = container(r)
                .padding([6, 8])
                .width(Length::Fill)
                .style(move |_| container::Style {
                    background: bg,
                    border: Border {
                        radius: 6.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
            list = list.push(mouse_area(item).on_press(Message::SelectFinding(f.id.clone())));
        }
        let hidden = inv
            .findings
            .iter()
            .filter(|f| f.kind.severity() == Severity::Info)
            .count();
        if !self.show_info && hidden > 0 {
            list = list.push(
                container(
                    text(s.t("list.hidden", &[("n", &hidden.to_string())]))
                        .size(11)
                        .style(text::secondary),
                )
                .padding(iced::Padding {
                    top: 8.0,
                    ..Default::default()
                }),
            );
        }
        card(
            column![
                text(s.t("list.title", &[])).size(15),
                text(s.t("list.sub", &[])).size(11).style(text::secondary),
                chip_lines(chips, 5),
                controls,
                scrollable(list).height(Length::Fill)
            ]
            .spacing(6)
            .into(),
        )
        .width(Length::FillPortion(4))
        .height(Length::Fill)
        .into()
    }

    // ---- right: a finding, a session, or what Lastrun shows ---------------------------

    fn view_detail(&self, d: &Data) -> Element<'_, Message> {
        let content: Element<'_, Message> = match &self.sel {
            Sel::Finding(id) => match d.inv.findings.iter().find(|f| &f.id == id) {
                Some(f) => self.view_finding(d, f),
                None => self.view_about(),
            },
            Sel::Session { host, id, action } => match d.inv.session(host, id) {
                Some(x) => self.view_session(d, x, *action),
                None => self.view_about(),
            },
            Sel::Nothing => self.view_about(),
        };
        card(
            scrollable(container(content).padding(iced::Padding {
                right: 14.0,
                ..Default::default()
            }))
            .height(Length::Fill)
            .into(),
        )
        .width(Length::FillPortion(4))
        .height(Length::Fill)
        .into()
    }

    fn heading<'a>(&self, key: &str) -> Element<'a, Message> {
        text(self.strings.t(key, &[]).to_uppercase())
            .size(11)
            .style(text::secondary)
            .into()
    }

    fn view_finding(&self, d: &Data, f: &Finding) -> Element<'_, Message> {
        let s = &self.strings;
        let inv = &d.inv;
        let sev = f.kind.severity();
        let mut head = row![
            pill(f.kind.label().to_string(), self.sev_color(sev)),
            text(s.t(&format!("sev.{}", sev.number()), &[]))
                .size(12)
                .style(text::secondary)
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        if let Some(h) = &f.host {
            head = head.push(text("·").size(12).style(text::secondary));
            head = head.push(
                text(self.host_name(inv, h).to_string())
                    .size(12)
                    .style(text::secondary),
            );
        }
        let mut col = column![head, text(plain(&f.title)).size(16)].spacing(4);
        col = col.push(Space::with_height(8));
        col = col.push(self.heading("detail.means"));
        col = col.push(text(s.t(&format!("kind.{}", f.kind.slug()), &[])).size(13));
        col = col.push(Space::with_height(8));
        col = col.push(self.heading("detail.where"));
        let mut where_col = column![
            text(plain(&f.subject)).size(12).font(iced::Font::MONOSPACE),
            text(plain(&f.detail)).size(12)
        ]
        .spacing(4);
        for e in &f.evidence {
            where_col = where_col.push(
                text(e.clone())
                    .size(11)
                    .font(iced::Font::MONOSPACE)
                    .style(text::secondary),
            );
        }
        let mut buttons = row![].spacing(6);
        if let Some(p) = &f.path {
            let shown = match f.line {
                Some(l) => format!("{}:{l}", display_home(p, &inv.home)),
                None => display_home(p, &inv.home),
            };
            let target = p.display().to_string();
            where_col = where_col.push(
                text(shown)
                    .size(11)
                    .font(iced::Font::MONOSPACE)
                    .style(text::secondary),
            );
            buttons = buttons
                .push(small_button(
                    s.t("detail.open_file", &[]),
                    Message::OpenFile(target.clone()),
                ))
                .push(small_button(
                    s.t("detail.copy_path", &[]),
                    Message::Copy(target),
                ));
        }
        if let (Some(h), Some(id)) = (&f.host, &f.session) {
            if inv.session(h, id).is_some() {
                buttons = buttons.push(small_button(
                    s.t("detail.show_session", &[]),
                    Message::SelectSession(h.clone(), id.clone(), None),
                ));
            }
        }
        where_col = where_col.push(buttons);
        col = col.push(inner(where_col.into()));
        if f.hint.is_some() || f.command.is_some() {
            col = col.push(Space::with_height(8));
            col = col.push(self.heading("detail.do"));
        }
        if let Some(h) = &f.hint {
            col = col.push(
                row![
                    text("→").size(13).style(text::secondary),
                    text(plain(h)).size(13)
                ]
                .spacing(8),
            );
        }
        if let Some(c) = &f.command {
            col = col.push(
                row![
                    text("→").size(13).style(text::secondary),
                    column![
                        text(s.t("detail.reopen", &[])).size(13),
                        text(c.clone()).size(12).font(iced::Font::MONOSPACE),
                        small_button(s.t("detail.copy_command", &[]), Message::Copy(c.clone()))
                    ]
                    .spacing(3)
                ]
                .spacing(8),
            );
        }
        if let Some(u) = &f.url {
            col = col.push(Space::with_height(8));
            col = col.push(self.heading("detail.source"));
            col = col.push(
                column![
                    text(shorten(u, 60)).size(11).font(iced::Font::MONOSPACE),
                    row![
                        small_button(s.t("detail.open_link", &[]), Message::OpenUrl(u.clone())),
                        small_button(s.t("detail.copy_link", &[]), Message::Copy(u.clone()))
                    ]
                    .spacing(6),
                ]
                .spacing(3),
            );
        }
        col = col.push(Space::with_height(16));
        col = col.push(
            text(s.t("detail.promise", &[]))
                .size(11)
                .style(text::secondary),
        );
        col.into()
    }

    fn act_matches(&self, d: &Data, i: usize, a: &Action) -> bool {
        if !self.files_read && a.op == Some(FileOp::Read) {
            return false;
        }
        let kind_ok = match self.act_filter {
            ActFilter::All => true,
            ActFilter::Commands => a.kind == ActKind::Shell,
            ActFilter::Files => a.kind == ActKind::File,
            ActFilter::Packages => d.an.installs.iter().any(|(k, _)| *k == i),
            ActFilter::Mcp => a.kind == ActKind::Mcp,
            ActFilter::Web => a.kind == ActKind::Network,
        };
        if !kind_ok {
            return false;
        }
        let q = self.act_search.trim().to_lowercase();
        // commands and paths only: nothing else of the transcript is searched
        q.is_empty()
            || a.command
                .as_deref()
                .is_some_and(|c| c.to_lowercase().contains(&q))
            || a.path
                .as_deref()
                .is_some_and(|p| p.to_string_lossy().to_lowercase().contains(&q))
    }

    fn kind_word(&self, a: &Action) -> String {
        let key = match (a.kind, a.op) {
            (ActKind::Shell, _) => "act.run",
            (ActKind::File, Some(FileOp::Create)) => "act.create",
            (ActKind::File, Some(FileOp::Modify)) => "act.edit",
            (ActKind::File, Some(FileOp::Delete)) => "act.delete",
            (ActKind::File, _) => "act.read",
            (ActKind::Mcp, _) => "act.mcp",
            (ActKind::Network, _) => "act.web",
            (ActKind::Agent, _) => "act.agent",
            (ActKind::Other, _) => "act.tool",
        };
        self.strings.t(key, &[])
    }

    fn status_text(&self, a: &Action) -> (String, Color) {
        let s = &self.strings;
        match (a.status, a.exit_code) {
            (_, Some(0)) => ("exit 0".into(), good(self.dark)),
            (_, Some(c)) => (format!("exit {c}"), alert(self.dark)),
            (Status::Completed, None) => (s.t("status.ok", &[]), good(self.dark)),
            (Status::Failed, None) => (s.t("status.failed", &[]), alert(self.dark)),
            (Status::Denied, None) => (s.t("status.denied", &[]), alert(self.dark)),
            (Status::Cancelled, None) => (s.t("status.cancelled", &[]), muted()),
            (Status::Interrupted, None) => (s.t("status.interrupted", &[]), muted()),
            (Status::Unknown, None) => ("?".into(), muted()),
        }
    }

    fn what<'a>(&self, d: &Data, a: &Action) -> Element<'a, Message> {
        let inv = &d.inv;
        match a.kind {
            ActKind::File => {
                let p = a
                    .path
                    .as_deref()
                    .map(|p| match a.cwd.as_deref() {
                        Some(c) if p.starts_with(c) && c != Path::new("/") => {
                            p.strip_prefix(c).unwrap_or(p).display().to_string()
                        }
                        _ => display_home(p, &inv.home),
                    })
                    .unwrap_or_else(|| "?".into());
                let mut r = row![text(shorten(&p, 44)).size(12).font(iced::Font::MONOSPACE)]
                    .spacing(6)
                    .align_y(Alignment::Center);
                if let Some((x, y)) = a.diff {
                    r = r.push(text(format!("+{x} −{y}")).size(11).style(text::secondary));
                }
                if a.inferred {
                    r = r.push(
                        text(self.strings.t("act.by_command_above", &[]))
                            .size(11)
                            .style(text::secondary),
                    );
                }
                r.into()
            }
            ActKind::Mcp => row![
                text(a.tool.clone()).size(12).font(iced::Font::MONOSPACE),
                text(self.strings.t(
                    "act.on_server",
                    &[("server", a.server.as_deref().unwrap_or("?"))]
                ))
                .size(11)
                .style(text::secondary)
            ]
            .spacing(6)
            .into(),
            ActKind::Shell => text(shorten(a.command.as_deref().unwrap_or(&a.tool), 48))
                .size(12)
                .font(iced::Font::MONOSPACE)
                .into(),
            _ => text(a.tool.clone())
                .size(12)
                .font(iced::Font::MONOSPACE)
                .into(),
        }
    }

    fn view_session(
        &self,
        d: &Data,
        x: &lastrun_core::Session,
        picked: Option<usize>,
    ) -> Element<'_, Message> {
        let s = &self.strings;
        let inv = &d.inv;
        let mut head = format!("{} · {}", x.host_name, x.id);
        if x.subagent {
            head.push_str(&format!(" · {}", s.t("sessions.subagent", &[])));
        }
        let project = x
            .project
            .as_deref()
            .map(|p| display_home(p, &inv.home))
            .unwrap_or_else(|| s.t("sessions.no_project", &[]));
        let span = match (x.started, x.ended) {
            (Some(a), Some(b)) if self.day(a) != self.day(b) => {
                format!("{} → {}", self.day(a), self.day(b))
            }
            (Some(a), _) | (None, Some(a)) => self.day(a),
            _ => String::new(),
        };
        let idx: Vec<usize> = inv
            .actions
            .iter()
            .enumerate()
            .filter(|(_, a)| a.host == x.host && a.session == x.id)
            .map(|(i, _)| i)
            .collect();
        let models: BTreeSet<&str> = idx
            .iter()
            .filter_map(|&i| inv.actions[i].model.as_deref())
            .collect();
        let mut meta = s.t(
            "session.meta",
            &[
                ("actions", &self.num(x.actions)),
                ("commands", &self.num(x.commands)),
                ("files", &self.num(x.files_touched)),
                ("refused", &x.denied.to_string()),
            ],
        );
        if !models.is_empty() {
            meta.push_str(" · ");
            meta.push_str(&models.into_iter().collect::<Vec<_>>().join(", "));
        }
        let mut col = column![
            text(head.to_uppercase()).size(11).style(text::secondary),
            text(format!("{project} — {span}")).size(16),
            text(meta).size(11).style(text::secondary),
        ]
        .spacing(3);
        if let Some(r) = &x.resume {
            col = col.push(
                row![
                    text(s.t("session.reopen", &[]))
                        .size(11)
                        .style(text::secondary),
                    text(shorten(r, 40)).size(11).font(iced::Font::MONOSPACE),
                    small_button(s.t("detail.copy_command", &[]), Message::Copy(r.clone()))
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        }
        // filters: by kind, a search over commands and paths, the files read
        let mut chips = row![].spacing(6);
        for (f, key) in [
            (ActFilter::All, "filter.all"),
            (ActFilter::Commands, "filter.commands"),
            (ActFilter::Files, "filter.files"),
            (ActFilter::Packages, "filter.packages"),
            (ActFilter::Mcp, "filter.mcp"),
            (ActFilter::Web, "filter.web"),
        ] {
            chips = chips.push(chip(
                s.t(key, &[]),
                self.act_filter == f,
                Message::SetActFilter(f),
            ));
        }
        col = col.push(Space::with_height(4)).push(chips).push(
            row![
                text_input(&s.t("session.search", &[]), &self.act_search)
                    .on_input(Message::ActSearch)
                    .size(11)
                    .padding(4)
                    .width(Length::Fill),
                checkbox(s.t("session.files_read", &[]), self.files_read)
                    .on_toggle(Message::ToggleFilesRead)
                    .size(14)
                    .text_size(11)
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
        // the chronology
        let shown: Vec<usize> = idx
            .iter()
            .copied()
            .filter(|&i| self.act_matches(d, i, &inv.actions[i]))
            .collect();
        let mut list = column![].spacing(1);
        let mut last_day: Option<String> = None;
        for &i in shown.iter().take(MAX_ROWS) {
            let a = &inv.actions[i];
            if let Some(t) = a.when {
                let day = self.day(t);
                if last_day.as_ref() != Some(&day) {
                    list = list.push(self.group_head(day.clone()));
                    last_day = Some(day);
                }
            }
            let (st, st_color) = self.status_text(a);
            let mut r = row![
                text(
                    a.when
                        .map(|t| t.with_timezone(&Local).format("%H:%M").to_string())
                        .unwrap_or_default()
                )
                .size(11)
                .style(text::secondary)
                .width(Length::Fixed(36.0)),
                text(self.kind_word(a)).size(11).width(Length::Fixed(44.0)),
                text(st).size(11).color(st_color).width(Length::Fixed(52.0)),
                container(self.what(d, a)).width(Length::Fill),
            ]
            .spacing(6)
            .align_y(Alignment::Center);
            if a.approval.decision == Decision::Denied && a.status != Status::Denied {
                r = r.push(pill(s.t("status.denied", &[]), alert(self.dark)));
            }
            if a.subagent && !x.subagent {
                r = r.push(
                    text(s.t("sessions.subagent", &[]))
                        .size(10)
                        .style(text::secondary),
                );
            }
            let bg = (picked == Some(i)).then(|| Background::Color(self.sel_bg()));
            list = list.push(
                mouse_area(
                    container(r)
                        .padding([3, 6])
                        .width(Length::Fill)
                        .style(move |_| container::Style {
                            background: bg,
                            border: Border {
                                radius: 5.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                )
                .on_press(Message::SelectAction(i)),
            );
        }
        if shown.is_empty() {
            list = list.push(
                text(s.t("session.none", &[]))
                    .size(12)
                    .style(text::secondary),
            );
        } else if shown.len() > MAX_ROWS {
            list = list.push(
                text(s.t("session.more", &[("n", &self.num(shown.len() - MAX_ROWS))]))
                    .size(11)
                    .style(text::secondary),
            );
        }
        col = col.push(list);
        // the action picked
        if let Some(i) = picked {
            if let Some(a) = inv
                .actions
                .get(i)
                .filter(|a| a.host == x.host && a.session == x.id)
            {
                col = col.push(Space::with_height(8));
                col = col.push(self.view_action(d, i, a));
            }
        }
        col = col.push(Space::with_height(12));
        col = col.push(
            text(s.t("session.promise", &[]))
                .size(11)
                .style(text::secondary),
        );
        col.into()
    }

    fn view_action(&self, d: &Data, i: usize, a: &Action) -> Element<'_, Message> {
        let s = &self.strings;
        let inv = &d.inv;
        let mut col = column![text(
            format!("{} · {}", s.t("action.selected", &[]), self.kind_word(a)).to_uppercase()
        )
        .size(11)
        .style(text::secondary)]
        .spacing(4);
        let main = match (&a.command, &a.path) {
            (Some(c), _) => c.clone(),
            (None, Some(p)) => display_home(p, &inv.home),
            _ => a.tool.clone(),
        };
        col = col.push(text(main).size(12).font(iced::Font::MONOSPACE));
        let mut facts = vec![
            match d.by_command.get(&i) {
                Some(c) => s.t("deleted.by_command", &[("command", &shorten(c, 60))]),
                None => s.t("deleted.by_tool", &[("tool", &a.tool)]),
            },
            self.approval(&a.approval),
            self.status_text(a).0,
        ];
        if a.op == Some(FileOp::Delete) {
            if let Some(p) = &a.path {
                facts.push(if d.exists.contains(p) {
                    s.t("action.exists_again", &[])
                } else {
                    s.t("action.gone", &[])
                });
            }
        }
        if d.outside.contains(&i) {
            facts.push(s.t("deleted.outside", &[]));
        }
        if let Some(m) = &a.model {
            facts.push(m.clone());
        }
        col = col.push(text(facts.join(" · ")).size(11).style(text::secondary));
        let mut buttons = row![].spacing(6);
        if let Some(p) = &a.path {
            let folder = p
                .parent()
                .map(|f| f.display().to_string())
                .unwrap_or_else(|| p.display().to_string());
            buttons = buttons
                .push(small_button(
                    s.t("action.open_folder", &[]),
                    Message::OpenFile(folder),
                ))
                .push(small_button(
                    s.t("detail.copy_path", &[]),
                    Message::Copy(p.display().to_string()),
                ));
        }
        if let Some(c) = &a.command {
            buttons = buttons.push(small_button(
                s.t("action.copy_command", &[]),
                Message::Copy(c.clone()),
            ));
        }
        buttons = buttons.push(small_button(
            s.t("action.open_transcript", &[]),
            Message::OpenFile(match a.line {
                Some(l) => format!("{}:{l}", a.source.display()),
                None => a.source.display().to_string(),
            }),
        ));
        let editable = a.kind == ActKind::File
            && matches!(a.op, Some(FileOp::Modify | FileOp::Create))
            && !a.inferred;
        if editable {
            buttons = buttons.push(small_button(
                s.t("action.show_diff", &[]),
                Message::ShowDiff(i),
            ));
        }
        col = col.push(buttons);
        if let Some((k, lines)) = &self.diff {
            if *k == i {
                col = col.push(self.view_diff(lines.as_deref()));
            }
        }
        inner(col.into())
    }

    fn view_diff(&self, lines: Option<&[diff::Line]>) -> Element<'_, Message> {
        let s = &self.strings;
        let Some(lines) = lines else {
            return text(s.t("diff.not_kept", &[]))
                .size(11)
                .style(text::secondary)
                .into();
        };
        let mut col = column![].spacing(0);
        for l in lines {
            let (prefix, color) = match l.mark {
                '+' => ("+ ", Some(good(self.dark))),
                '-' => ("- ", Some(alert(self.dark))),
                '@' => ("", Some(muted())),
                _ => ("  ", None),
            };
            let t = text(format!("{prefix}{}", l.text))
                .size(11)
                .font(iced::Font::MONOSPACE);
            col = col.push(match color {
                Some(c) => t.color(c),
                None => t,
            });
        }
        container(
            column![
                text(s.t("diff.title", &[])).size(11).style(text::secondary),
                scrollable(col).height(Length::Shrink)
            ]
            .spacing(4),
        )
        .padding([6, 0])
        .into()
    }

    fn view_about(&self) -> Element<'_, Message> {
        let s = &self.strings;
        column![
            text(s.t("about.title", &[])).size(16),
            Space::with_height(8),
            text(s.t("about.body", &[])).size(13),
            Space::with_height(8),
            text(s.t("about.never", &[]))
                .size(12)
                .style(text::secondary),
            Space::with_height(8),
            text(s.t("about.pick", &[])).size(12).style(text::secondary),
        ]
        .spacing(4)
        .into()
    }
}

const MONTHS_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTHS_FR: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
    "déc.",
];

/// Chips in lines of `per` (iced 0.13 has no wrapping row).
fn chip_lines<'a>(chips: Vec<Element<'a, Message>>, per: usize) -> Element<'a, Message> {
    let mut col = column![].spacing(4);
    let mut line = row![].spacing(6).align_y(Alignment::Center);
    for (i, c) in chips.into_iter().enumerate() {
        if i > 0 && i % per == 0 {
            col = col.push(line);
            line = row![].spacing(6).align_y(Alignment::Center);
        }
        line = line.push(c);
    }
    col.push(line).into()
}

fn chip<'a>(label: String, on: bool, msg: Message) -> Element<'a, Message> {
    let b = button(text(label).size(11)).padding([2, 8]).on_press(msg);
    if on {
        b.style(button::primary).into()
    } else {
        b.style(button::secondary).into()
    }
}

struct Args {
    select: Option<String>,
    session: Option<String>,
    home: Option<PathBuf>,
    since: Option<i64>,
    tab: Tab,
}

/// `--select <finding id>` (the tower), `--session <id>` (as `lastrun sessions` shows it),
/// `--home <dir>` (a fixture), `--since <days>`, `--tab sessions|deleted|packages|refused`.
fn parse_args() -> Args {
    let mut out = Args {
        select: None,
        session: None,
        home: None,
        since: None,
        tab: Tab::Sessions,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--select" {
            out.select = args.next();
        } else if let Some(v) = a.strip_prefix("--select=") {
            out.select = Some(v.to_string());
        } else if a == "--session" {
            out.session = args.next();
        } else if a == "--home" {
            out.home = args.next().map(PathBuf::from);
        } else if a == "--since" {
            out.since = args.next().and_then(|v| v.parse().ok());
        } else if a == "--tab" {
            out.tab = match args.next().as_deref() {
                Some("deleted") => Tab::Deleted,
                Some("packages") => Tab::Packages,
                Some("refused") => Tab::Refused,
                _ => Tab::Sessions,
            };
        }
    }
    out
}

/// The scan on a thread, so the window stays responsive; what the view needs from the disk
/// (which deleted files exist again) is looked up here, once.
fn load(home: PathBuf, days: i64) -> Task<Message> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut env = bbl_hostrules::env::Env::current();
        if env.home != home {
            env = bbl_hostrules::env::Env::fixture(&home, env.os);
        }
        let opt = lastrun_core::Options {
            since_days: days,
            ..Default::default()
        };
        let sheets = lastrun_core::sheets::embedded();
        let (inv, an) =
            lastrun_core::scan_with(&env, &sheets, &lastrun_core::rules::Rules::embedded(), &opt);
        let data = prepare(inv, an, sheets);
        let _ = tx.send((Arc::new(data), started.elapsed().as_millis() as u64));
    });
    Task::perform(
        async move { rx.await.unwrap_or_else(|_| (Arc::new(Data::default()), 0)) },
        |(d, ms)| Message::Loaded(d, ms),
    )
}

fn prepare(inv: Inventory, an: Analysis, sheets: Sheets) -> Data {
    let mut d = Data {
        inv,
        an,
        ..Default::default()
    };
    for (i, a) in d.inv.actions.iter().enumerate() {
        if outside_project(&d.inv, &sheets, a) {
            d.outside.insert(i);
        }
        if a.op == Some(FileOp::Delete) && a.ran() {
            if let Some(p) = &a.path {
                if p.exists() {
                    d.exists.insert(p.clone());
                }
            }
            if a.inferred {
                // the command the delete was read from: same call, a shell action
                if let Some(c) = d.inv.actions.iter().find(|c| {
                    c.kind == ActKind::Shell
                        && c.host == a.host
                        && c.session == a.session
                        && c.call_id == a.call_id
                        && c.line == a.line
                        && c.when == a.when
                }) {
                    if let Some(cmd) = &c.command {
                        d.by_command.insert(i, cmd.clone());
                    }
                }
            }
        }
    }
    for f in &d.inv.findings {
        if f.kind == Kind::Destructive {
            if let (Some(h), Some(s)) = (&f.host, &f.session) {
                *d.destructive.entry((h.clone(), s.clone())).or_default() += 1;
            }
        }
    }
    d.sheets = Some(sheets);
    d
}

/// Titles and details carry Markdown backticks for the CLI; the app shows them plain.
fn plain(s: &str) -> String {
    s.replace('`', "")
}

fn short_host(name: &str) -> String {
    name.split(" (")
        .next()
        .unwrap_or(name)
        .replace(" CLI", "")
        .replace("GitHub ", "")
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_have_the_same_keys() {
        bbl_ui::i18n::same_keys(EN, FR).unwrap();
    }

    #[test]
    fn every_kind_and_severity_has_its_strings() {
        for lang in [Lang::En, Lang::Fr] {
            let s = Strings::load(EN, FR, lang);
            for k in Kind::ALL {
                assert_ne!(
                    s.t(&format!("kind.{}", k.slug()), &[]),
                    format!("kind.{}", k.slug()),
                    "missing kind.{}",
                    k.slug()
                );
            }
            for sev in Severity::ALL {
                assert_ne!(
                    s.t(&format!("sev.{}", sev.number()), &[]),
                    format!("sev.{}", sev.number())
                );
            }
        }
    }

    #[test]
    fn helpers() {
        assert_eq!(plain("a `b` c"), "a b c");
        assert_eq!(short_host("Codex CLI"), "Codex");
        assert_eq!(short_host("GitHub Copilot CLI"), "Copilot");
        assert_eq!(shorten("abcdef", 4), "abc…");
    }
}
