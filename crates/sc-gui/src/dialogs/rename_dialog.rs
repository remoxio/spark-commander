//! The Multi-Rename Tool (Ctrl+M): masks, search and replace, case and
//! counter, with the new names previewed as they are typed. Start renames,
//! Undo takes it back.
use gpui_kit::component::scroll::ScrollableElement as _;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::ops::rename::{self, Case, Rules, Source};
use sc_core::vfs::Fs;

use crate::Commander;

/// What the buttons put into the name mask.
const FIELDS: [&str; 7] = ["[N]", "[E]", "[C]", "[P]", "[Y]-[M]-[D]", "[h][m][s]", "[N1-3]"];

/// The buttons' labels, in `FIELDS`' order.
fn field_labels() -> [String; 7] {
    [
        sc_core::t!("rename.field.name"),
        sc_core::t!("rename.field.extension"),
        sc_core::t!("rename.field.counter"),
        sc_core::t!("rename.field.folder"),
        sc_core::t!("rename.field.date"),
        sc_core::t!("rename.field.time"),
        sc_core::t!("rename.field.range"),
    ]
}

impl Commander {
    pub(crate) fn multi_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        let panel = self.panels[side].tabs.current();
        if panel.in_archive() {
            let text = sc_core::t!("rename.in_archive");
            return window.push_notification(Notification::warning(text), cx);
        }
        let chosen = panel.chosen();
        let sources: Vec<Source> = chosen
            .iter()
            .filter_map(|entry| {
                Some(Source {
                    name: entry.name.to_str()?.to_owned(),
                    is_dir: entry.is_dir,
                    modified: entry.modified,
                })
            })
            .collect();
        if sources.len() < chosen.len() {
            let text = sc_core::t!("rename.not_utf8");
            window.push_notification(Notification::warning(text), cx);
        }
        if sources.is_empty() {
            return;
        }
        let taken: HashSet<String> = panel
            .entries()
            .iter()
            .filter_map(|entry| entry.name.to_str().map(str::to_owned))
            .collect();
        let (fs, folder) = (panel.fs(), panel.folder().to_owned());
        let commander = cx.entity().downgrade();
        let build = move |_, window: &mut Window, cx: &mut App| {
        let input = |value: &str, window: &mut Window, cx: &mut App| {
            let value = value.to_owned();
            cx.new(|cx| InputState::new(window, cx).default_value(value))
        };
        let name_mask = input("[N]", window, cx);
        let ext_mask = input("[E]", window, cx);
        let search = input("", window, cx);
        let replace = input("", window, cx);
        let start = input("1", window, cx);
        let step = input("1", window, cx);
        let digits = input("1", window, cx);
        let inputs = [
            &name_mask, &ext_mask, &search, &replace, &start, &step, &digits,
        ];
        let inputs: Vec<Entity<InputState>> = inputs.into_iter().cloned().collect();
        let view = cx.new(|cx| {
            let subscriptions = inputs
                .iter()
                .map(|input| {
                    cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                        if let InputEvent::Change = event {
                            cx.notify();
                        }
                    })
                })
                .collect();
            RenameDialog {
                name_mask: name_mask.clone(),
                ext_mask,
                search,
                replace,
                start,
                step,
                digits,
                case_sensitive: false,
                first_only: false,
                with_extension: false,
                regex: false,
                case: Case::Unchanged,
                sources,
                taken,
                folder,
                fs,
                undo: None,
                status: sc_core::t!("rename.status.ready"),
                busy: false,
                commander,
                side,
                _changes: subscriptions,
            }
        });
        crate::dialogs::pack_dialog::focus_all(&name_mask, window, cx);
        view
        };
        crate::dialogs::dialog_window::show(sc_core::t!("rename.dialog_title"), size(px(980.), px(600.)), window, cx, build, |_, _, _| false);
    }
}

struct RenameDialog {
    name_mask: Entity<InputState>,
    ext_mask: Entity<InputState>,
    search: Entity<InputState>,
    replace: Entity<InputState>,
    start: Entity<InputState>,
    step: Entity<InputState>,
    digits: Entity<InputState>,
    case_sensitive: bool,
    first_only: bool,
    with_extension: bool,
    regex: bool,
    case: Case,
    sources: Vec<Source>,
    /// The folder's names, for names that exist already.
    taken: HashSet<String>,
    folder: PathBuf,
    fs: Arc<dyn Fs>,
    /// The last Start's renames, to take back.
    undo: Option<Vec<(String, String)>>,
    status: String,
    busy: bool,
    commander: WeakEntity<Commander>,
    side: usize,
    _changes: Vec<Subscription>,
}

impl RenameDialog {
    fn rules(&self, cx: &App) -> Rules {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let number = |input: &Entity<InputState>, default: i64| {
            text(input).trim().parse().unwrap_or(default)
        };
        Rules {
            name_mask: text(&self.name_mask),
            ext_mask: text(&self.ext_mask),
            search: text(&self.search),
            replace: text(&self.replace),
            case_sensitive: self.case_sensitive,
            first_only: self.first_only,
            with_extension: self.with_extension,
            regex: self.regex,
            case: self.case,
            counter_start: number(&self.start, 1),
            counter_step: number(&self.step, 1),
            counter_digits: number(&self.digits, 1).clamp(1, 12) as usize,
        }
    }

    /// The new names and their problems, or why there are none.
    fn planned(&self, cx: &App) -> Result<(Vec<String>, Vec<Option<String>>), String> {
        let new = rename::preview(&self.rules(cx), &self.sources, &self.folder)?;
        let old: Vec<String> = self.sources.iter().map(|s| s.name.clone()).collect();
        let problems = rename::problems(&old, &new, &self.taken);
        Ok((new, problems))
    }

    fn insert(&mut self, field: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = format!("{}{field}", self.name_mask.read(cx).value());
        self.name_mask
            .update(cx, |input, cx| input.set_value(text, window, cx));
        cx.notify();
    }

    /// Renames `pairs` in the background; `forward` keeps them to undo.
    fn run(&mut self, pairs: Vec<(String, String)>, forward: bool, cx: &mut Context<Self>) {
        self.busy = true;
        let (fs, folder) = (self.fs.clone(), self.folder.clone());
        cx.spawn(async move |this, cx| {
            let renamed = cx
                .background_executor()
                .spawn(async move { rename::rename_all(&*fs, &folder, &pairs) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match renamed {
                    Ok(done) => {
                        for (old, new) in &done {
                            this.taken.remove(old);
                            this.taken.insert(new.clone());
                            for source in this.sources.iter_mut().filter(|s| s.name == *old) {
                                source.name = new.clone();
                            }
                        }
                        this.status = match forward {
                            true => sc_core::t!("rename.status.renamed", count = done.len()),
                            false => sc_core::t!("rename.status.took_back", count = done.len()),
                        };
                        this.undo = forward.then_some(done);
                    }
                    Err(err) => this.status = sc_core::t!("rename.status.failed", err = err),
                }
                let side = this.side;
                let _ = this
                    .commander
                    .update(cx, |commander, cx| commander.reread(side, cx));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let Ok((new, problems)) = self.planned(cx) else {
            return;
        };
        if problems.iter().any(Option::is_some) {
            return;
        }
        let pairs = self
            .sources
            .iter()
            .map(|s| s.name.clone())
            .zip(new)
            .collect();
        self.run(pairs, true, cx);
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        if let Some(done) = self.undo.take() {
            self.run(rename::undo_pairs(&done), false, cx);
        }
    }
}

impl Commander {
    /// The side's panel reads its folder again.
    /// The folders on this computer shown, read again; marks and cursor stay.
    pub(crate) fn reread_local(&mut self, cx: &mut Context<Self>) {
        for side in &mut self.panels {
            let panel = side.tabs.current_mut();
            if panel.remote().is_none() && !panel.in_archive() {
                // shortcut: a folder that cannot be read now keeps its old list.
                let _ = panel.reload();
                side.scroll_to_cursor();
            }
        }
        self.sidebar.refresh(self.settings.show_hidden);
        cx.notify();
    }

    pub(crate) fn reread(&mut self, side: usize, cx: &mut Context<Self>) {
        let side = &mut self.panels[side];
        // shortcut: a server's folder is read on the UI thread here.
        let _ = side.tabs.current_mut().reload();
        side.scroll_to_cursor();
        cx.notify();
    }
}

impl Render for RenameDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (danger, muted, border) = (theme.danger, theme.muted_foreground, theme.border);
        let planned = self.planned(cx);
        let blocked = match &planned {
            Ok((new, problems)) => {
                problems.iter().any(Option::is_some)
                    || self.sources.iter().zip(new).all(|(s, new)| s.name == *new)
            }
            Err(_) => true,
        };
        let labeled = |label: String, input: &Entity<InputState>, width: f32| {
            v_flex()
                .gap_1()
                .child(label)
                .child(div().w(px(width)).child(Input::new(input).small()))
        };
        let flag = |id: &'static str,
                    label: String,
                    on: bool,
                    set: fn(&mut Self, bool),
                    cx: &mut Context<Self>| {
            Checkbox::new(id)
                .label(label)
                .checked(on)
                .on_click(cx.listener(move |this, on: &bool, _, cx| {
                    set(this, *on);
                    cx.notify();
                }))
        };
        let rows: Vec<AnyElement> = match &planned {
            Err(err) => vec![
                div()
                    .text_color(danger)
                    .child(err.clone())
                    .into_any_element(),
            ],
            Ok((new, problems)) => self
                .sources
                .iter()
                .zip(new.iter().zip(problems))
                .map(|(source, (new, problem))| {
                    h_flex()
                        .gap_2()
                        .child(div().w(px(420.)).truncate().child(source.name.clone()))
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .when_some(problem.as_ref(), |el, _| el.text_color(danger))
                                .when(problem.is_none() && *new == source.name, |el| {
                                    el.text_color(muted)
                                })
                                .child(match problem {
                                    Some(problem) => format!("{new}  ({problem})"),
                                    None => new.clone(),
                                }),
                        )
                        .into_any_element()
                })
                .collect(),
        };
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(
                h_flex()
                    .gap_3()
                    .items_end()
                    .child(labeled(sc_core::t!("rename.mask"), &self.name_mask, 380.))
                    .child(labeled(sc_core::t!("rename.extension"), &self.ext_mask, 120.))
                    .child(labeled(sc_core::t!("rename.counter_start"), &self.start, 80.))
                    .child(labeled(sc_core::t!("rename.step"), &self.step, 60.))
                    .child(labeled(sc_core::t!("rename.digits"), &self.digits, 60.)),
            )
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .children(FIELDS.iter().zip(field_labels()).enumerate().map(|(ix, (field, label))| {
                        Button::new(("rename-field", ix))
                            .outline()
                            .xsmall()
                            .label(label)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.insert(field, window, cx)
                            }))
                    })),
            )
            .child(
                h_flex()
                    .gap_3()
                    .items_end()
                    .child(labeled(sc_core::t!("rename.search"), &self.search, 260.))
                    .child(labeled(sc_core::t!("rename.replace"), &self.replace, 260.))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_3()
                                    .child(flag(
                                        "rename-case",
                                        sc_core::t!("rename.match_case"),
                                        self.case_sensitive,
                                        |t, on| t.case_sensitive = on,
                                        cx,
                                    ))
                                    .child(flag(
                                        "rename-first",
                                        sc_core::t!("rename.first_only"),
                                        self.first_only,
                                        |t, on| t.first_only = on,
                                        cx,
                                    )),
                            )
                            .child(
                                h_flex()
                                    .gap_3()
                                    .child(flag(
                                        "rename-ext",
                                        sc_core::t!("rename.with_extension"),
                                        self.with_extension,
                                        |t, on| t.with_extension = on,
                                        cx,
                                    ))
                                    .child(flag(
                                        "rename-regex",
                                        sc_core::t!("rename.regex"),
                                        self.regex,
                                        |t, on| t.regex = on,
                                        cx,
                                    )),
                            ),
                    ),
            )
            .child(
                h_flex().gap_2().child(sc_core::t!("rename.case")).child(
                    RadioGroup::horizontal("rename-upper")
                        .children(Case::ALL.map(Case::label))
                        .selected_index(Case::ALL.iter().position(|c| *c == self.case))
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.case = Case::ALL[*ix];
                            cx.notify();
                        })),
                ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .text_color(muted)
                            .child(div().w(px(420.)).child(sc_core::t!("rename.old_name")))
                            .child(sc_core::t!("rename.new_name")),
                    )
                    .child(
                        div()
                            .id("rename-list")
                            .h(px(280.))
                            .overflow_y_scrollbar()
                            .border_1()
                            .border_color(border)
                            .p_1()
                            .child(v_flex().gap_0p5().children(rows)),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(self.status.clone()))
                    .child(
                        Button::new("rename-start")
                            .primary()
                            .label(sc_core::t!("rename.start"))
                            .min_w(px(80.))
                            .disabled(blocked || self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                    )
                    .child(
                        Button::new("rename-undo")
                            .outline()
                            .label(sc_core::t!("rename.undo"))
                            .min_w(px(80.))
                            .disabled(self.undo.is_none() || self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.undo(cx))),
                    )
                    .child(
                        Button::new("rename-close")
                            .outline()
                            .label(sc_core::t!("rename.close"))
                            .min_w(px(80.))
                            .on_click(|_, window, cx| crate::dialogs::dialog_window::close(window, cx)),
                    ),
            )
    }
}
