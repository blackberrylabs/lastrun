//! Lastrun desktop app (iced, on bbl-ui): the KPI cards, the list of findings, the detail
//! of the selected one, the footer. Reads the same core as the CLI; writes only its
//! preferences (`~/.config/lastrun/settings.json`).
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::{
    button, column, container, horizontal_space, mouse_area, row, scrollable, text, Space,
};
use iced::{Alignment, Background, Border, Element, Length, Task, Theme};

use bbl_ui::palette::{alert, good, muted};
use bbl_ui::widgets::{card, inner, kpi, small_button, swatch};
use bbl_ui::{Lang, Prefs, Strings};
use lastrun_core::{scan, Finding, Inventory};

const EN: &str = include_str!("../../../locales/en.json");
const FR: &str = include_str!("../../../locales/fr.json");

fn main() -> iced::Result {
    iced::application(App::title, App::update, App::view)
        .theme(App::theme)
        .window_size((1280.0, 820.0))
        .run_with(App::new)
}

#[derive(Debug, Clone)]
enum Message {
    Loaded(Arc<Inventory>),
    Rescan,
    SetLang(Lang),
    ToggleTheme,
    Select(String),
    OpenFile(PathBuf),
    Copy(String),
}

struct App {
    prefs: Prefs<()>,
    lang: Lang,
    strings: Strings,
    dark: bool,
    home: PathBuf,
    inventory: Option<Arc<Inventory>>,
    loading: bool,
    selected: Option<String>,
}

impl App {
    fn new() -> (App, Task<Message>) {
        let prefs: Prefs<()> = Prefs::load("lastrun");
        let lang = prefs.initial_lang();
        let dark = prefs.initial_dark();
        let strings = Strings::load(EN, FR, lang);
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        // `--select <finding id>` (the Blackberrylabs tower) preselects a finding.
        let selected = requested_selection();
        let task = load(home.clone());
        (
            App {
                prefs,
                lang,
                strings,
                dark,
                home,
                inventory: None,
                loading: true,
                selected,
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
            Message::Loaded(inv) => {
                self.inventory = Some(inv);
                self.loading = false;
            }
            Message::Rescan => {
                self.loading = true;
                return load(self.home.clone());
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
            Message::Select(id) => self.selected = Some(id),
            Message::OpenFile(p) => {
                let _ = std::process::Command::new("xdg-open").arg(&p).spawn();
            }
            Message::Copy(s) => return iced::clipboard::write(s),
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let s = &self.strings;
        let inv = self.inventory.as_deref();
        let n = |v: Option<usize>| v.map(|x| x.to_string()).unwrap_or_else(|| "—".into());
        let files = inv.map(|i| i.sessions.len());
        let findings = inv.map(|i| i.findings.len());
        let dot = findings.map(|f| {
            if f == 0 {
                good(self.dark)
            } else {
                alert(self.dark)
            }
        });
        let header = row![
            kpi(s.t("kpi.files", &[]), n(files), "", None),
            kpi(s.t("kpi.findings", &[]), n(findings), "", dot),
            container(if self.loading {
                Element::from(
                    button(text(s.t("header.scanning", &[])).size(12)).style(button::secondary),
                )
            } else {
                Element::from(
                    button(text(s.t("header.rescan", &[])).size(12)).on_press(Message::Rescan),
                )
            })
            .center_y(Length::Fill),
        ]
        .spacing(12)
        .height(Length::Fixed(96.0));

        let body: Element<'_, Message> = match inv {
            Some(i) => row![self.view_list(i), self.view_detail(i)]
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
        let status = match inv {
            Some(i) => s.t(
                "status.ready",
                &[
                    ("files", &i.sessions.len().to_string()),
                    ("findings", &i.findings.len().to_string()),
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
        column![header, body, footer].spacing(12).padding(14).into()
    }

    fn view_list<'a>(&'a self, inv: &'a Inventory) -> Element<'a, Message> {
        let s = &self.strings;
        let mut rows = column![].spacing(2);
        if inv.findings.is_empty() {
            rows = rows.push(text(s.t("list.empty", &[])).size(13).style(text::secondary));
        }
        for f in &inv.findings {
            let selected = self.selected.as_deref() == Some(f.id.as_str());
            let color = match f.kind.severity() {
                bbl_report::Severity::ActNow | bbl_report::Severity::Review => alert(self.dark),
                bbl_report::Severity::GoodToKnow => good(self.dark),
                bbl_report::Severity::Info => muted(),
            };
            let line = row![
                swatch(color, 9),
                text(f.title.clone()).size(13),
                horizontal_space(),
                text("›").size(13).style(text::secondary)
            ]
            .spacing(10)
            .align_y(Alignment::Center);
            let cell =
                container(line)
                    .padding([6, 8])
                    .width(Length::Fill)
                    .style(move |theme: &Theme| {
                        let p = theme.extended_palette();
                        container::Style {
                            background: if selected {
                                Some(Background::Color(p.primary.weak.color))
                            } else {
                                None
                            },
                            border: Border {
                                radius: 6.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }
                    });
            rows = rows.push(mouse_area(cell).on_press(Message::Select(f.id.clone())));
        }
        card(
            column![
                text(s.t("list.title", &[])).size(15),
                text(s.t("list.subtitle", &[]))
                    .size(12)
                    .style(text::secondary),
                scrollable(rows.padding(iced::Padding {
                    right: 14.0,
                    ..Default::default()
                }))
                .height(Length::Fill),
            ]
            .spacing(8)
            .into(),
        )
        .width(Length::FillPortion(5))
        .height(Length::Fill)
        .into()
    }

    fn view_detail<'a>(&'a self, inv: &'a Inventory) -> Element<'a, Message> {
        let s = &self.strings;
        let content: Element<'_, Message> = match self
            .selected
            .as_deref()
            .and_then(|id| inv.findings.iter().find(|f| f.id == id))
        {
            Some(f) => self.view_finding(f),
            None => text(s.t("detail.note", &[]))
                .size(13)
                .style(text::secondary)
                .into(),
        };
        card(scrollable(content).height(Length::Fill).into())
            .width(Length::FillPortion(4))
            .height(Length::Fill)
            .into()
    }

    fn view_finding<'a>(&'a self, f: &'a Finding) -> Element<'a, Message> {
        let s = &self.strings;
        let mut col = column![
            text(f.title.clone()).size(16),
            text(f.subject.clone()).size(12).style(text::secondary),
            Space::with_height(6),
            text(f.detail.clone()).size(13)
        ]
        .spacing(4);
        if let Some(p) = &f.path {
            col = col.push(inner(
                column![
                    text(p.display().to_string())
                        .size(12)
                        .font(iced::Font::MONOSPACE),
                    row![
                        small_button(s.t("detail.open", &[]), Message::OpenFile(p.clone())),
                        small_button(
                            s.t("detail.copy", &[]),
                            Message::Copy(p.display().to_string())
                        ),
                    ]
                    .spacing(6),
                ]
                .spacing(4)
                .into(),
            ));
        }
        if let Some(h) = &f.hint {
            col = col.push(Space::with_height(8));
            col = col.push(
                text(s.t("detail.hint", &[]).to_uppercase())
                    .size(11)
                    .style(text::secondary),
            );
            col = col.push(text(format!("→ {h}")).size(13));
        }
        col = col.push(Space::with_height(12));
        col = col.push(
            text(s.t("detail.note", &[]))
                .size(12)
                .style(text::secondary),
        );
        col.into()
    }
}

/// The `--select` argument, if any.
fn requested_selection() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--select" {
            return args.next();
        }
        if let Some(v) = a.strip_prefix("--select=") {
            return Some(v.to_string());
        }
    }
    None
}

/// The scan on a thread, so the window stays responsive.
fn load(home: PathBuf) -> Task<Message> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let mut env = bbl_hostrules::env::Env::current();
        env.home = home;
        let _ = tx.send(Arc::new(scan(&env)));
    });
    Task::perform(async move { rx.await.unwrap_or_default() }, Message::Loaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_have_the_same_keys() {
        bbl_ui::i18n::same_keys(EN, FR).unwrap();
    }
}
