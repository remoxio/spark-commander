//! Change Attributes: the permission bits of the chosen entries, each on,
//! off or mixed; only the bits clicked change. Runs in the queue.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::ops::attributes::{self, BITS, Change};
use sc_core::ops::Queue;
use sc_core::vfs::Fs;

use crate::Commander;

impl Commander {
    pub(crate) fn change_attributes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        if panel.in_archive() {
            let text = sc_core::t!("attributes.in_archive");
            return window.push_notification(Notification::warning(text), cx);
        }
        let chosen = panel.chosen();
        if chosen.is_empty() {
            return;
        }
        let modes: Vec<u32> = chosen
            .iter()
            .filter_map(|entry| attributes::mode_of(&entry.attributes))
            .collect();
        let bits = (modes.len() == chosen.len()).then(|| attributes::common_bits(&modes));
        let time = match chosen.as_slice() {
            [one] => one.modified,
            _ => None,
        }
        .unwrap_or_else(SystemTime::now);
        let title = match chosen.as_slice() {
            [one] => sc_core::t!("attributes.dialog_title_one", name = one.display_name()),
            many => sc_core::t!("attributes.dialog_title_many", count = many.len()),
        };
        let any_dir = chosen.iter().any(|entry| entry.is_dir);
        let paths: Vec<PathBuf> = chosen
            .iter()
            .map(|entry| panel.folder().join(&entry.name))
            .collect();
        let fs = panel.fs();
        let queue = self.transfers.queue.clone();
        let time_text = sc_core::local_time(time, "%Y-%m-%d %H:%M:%S");
        let build = move |_, window: &mut Window, cx: &mut App| {
        let time = cx.new(|cx| InputState::new(window, cx).default_value(time_text));
        let octal = cx.new(|cx| InputState::new(window, cx));
        let view = cx.new(|cx| {
            let octal_changes =
                cx.subscribe(&octal, |this: &mut AttributesDialog, input, event, cx| {
                    if let InputEvent::Change = event {
                        let text = input.read(cx).value().to_string();
                        if let Ok(mode) = u32::from_str_radix(text.trim(), 8)
                            && mode <= 0o777
                        {
                            this.bits = Some(BITS.map(|(bit, _)| Some(mode & bit != 0)));
                            cx.notify();
                        }
                    }
                });
            AttributesDialog {
                initial: bits,
                bits,
                time_on: false,
                time,
                octal,
                recurse: false,
                any_dir,
                fs,
                paths,
                queue,
                _octal: octal_changes,
            }
        });
        view.update(cx, |this, cx| this.show_octal(window, cx));
        view
        };
        let ok = |view: &Entity<AttributesDialog>, window: &mut Window, cx: &mut App| view.update(cx, |this, cx| this.finish(window, cx));
        crate::dialogs::dialog_window::show(title, size(px(460.), px(420.)), window, cx, build, ok);
    }
}

struct AttributesDialog {
    /// As the chosen entries have them; none where the file system has no bits.
    initial: Option<[Option<bool>; 9]>,
    bits: Option<[Option<bool>; 9]>,
    time_on: bool,
    time: Entity<InputState>,
    octal: Entity<InputState>,
    recurse: bool,
    any_dir: bool,
    fs: Arc<dyn Fs>,
    paths: Vec<PathBuf>,
    queue: Queue,
    _octal: Subscription,
}

impl AttributesDialog {
    /// On, off, and back to mixed where it started mixed.
    fn toggle(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(bits), Some(initial)) = (self.bits.as_mut(), self.initial) else {
            return;
        };
        bits[ix] = match (bits[ix], initial[ix]) {
            (None, _) => Some(true),
            (Some(true), _) => Some(false),
            (Some(false), None) => None,
            (Some(false), Some(_)) => Some(true),
        };
        self.show_octal(window, cx);
        cx.notify();
    }

    /// The bits as octal, unless some are mixed.
    fn show_octal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.bits.and_then(|bits| {
            BITS.iter().zip(bits).try_fold(0, |mode, ((bit, _), on)| {
                Some(mode | if on? { *bit } else { 0 })
            })
        });
        let text = mode.map(|mode| format!("{mode:03o}")).unwrap_or_default();
        self.octal
            .update(cx, |input, cx| input.set_value(text, window, cx));
    }

    fn change(&self, cx: &App) -> Result<Change, String> {
        let mut change = Change {
            recurse: self.recurse,
            ..Change::default()
        };
        if let (Some(bits), Some(initial)) = (self.bits, self.initial) {
            for (ix, (bit, _)) in BITS.iter().enumerate() {
                match bits[ix] {
                    on if on == initial[ix] => {}
                    Some(true) => change.set |= bit,
                    Some(false) => change.clear |= bit,
                    None => {}
                }
            }
        }
        if self.time_on {
            let text = self.time.read(cx).value().to_string();
            let time = attributes::parse_time(&text)
                .ok_or_else(|| sc_core::t!("attributes.bad_time", text = text))?;
            change.modified = Some(time);
        }
        Ok(change)
    }

    /// Queues the change; false keeps the dialog open after a wrong date.
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.change(cx) {
            Ok(change) => {
                let task = attributes::task(self.fs.clone(), self.paths.clone(), change);
                self.queue.push_task(task);
                true
            }
            Err(err) => {
                window.push_notification(Notification::error(err), cx);
                false
            }
        }
    }
}

impl Render for AttributesDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let grid = self.bits.map(|bits| {
            let cell = |ix: usize, cx: &mut Context<Self>| {
                let (label, primary) = match bits[ix] {
                    Some(true) => ("✓", true),
                    Some(false) => ("", false),
                    None => ("–", false),
                };
                Button::new(("bit", ix))
                    .small()
                    .w(px(36.))
                    .label(label)
                    .when(primary, |button| button.primary())
                    .when(!primary, |button| button.outline())
                    .on_click(cx.listener(move |this, _, window, cx| this.toggle(ix, window, cx)))
            };
            let heading = |text: String| div().w(px(56.)).child(text);
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .gap_1()
                        .child(div().w(px(70.)))
                        .child(heading(sc_core::t!("attributes.read")))
                        .child(heading(sc_core::t!("attributes.write")))
                        .child(heading(sc_core::t!("attributes.execute"))),
                )
                .children(
                    [sc_core::t!("attributes.owner"), sc_core::t!("attributes.group"), sc_core::t!("attributes.others")]
                        .into_iter()
                        .enumerate()
                        .map(|(row, who)| {
                            h_flex()
                                .gap_1()
                                .child(div().w(px(70.)).child(who))
                                .children(
                                    (0..3)
                                        .map(|col| div().w(px(56.)).child(cell(row * 3 + col, cx))),
                                )
                        }),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().w(px(70.)).child(sc_core::t!("attributes.octal")))
                        .child(div().w(px(90.)).child(Input::new(&self.octal).small())),
                )
        });
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .children(grid)
            .when(self.bits.is_none(), |el| {
                el.child(sc_core::t!("attributes.no_bits"))
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Checkbox::new("attr-time")
                            .label(sc_core::t!("attributes.change_time"))
                            .checked(self.time_on)
                            .on_click(cx.listener(|this, on: &bool, _, cx| {
                                this.time_on = *on;
                                cx.notify();
                            })),
                    )
                    .child(div().w(px(170.)).child(Input::new(&self.time).small()))
                    .child(
                        Button::new("attr-now")
                            .small()
                            .outline()
                            .label(sc_core::t!("attributes.now"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                let now =
                                    sc_core::local_time(SystemTime::now(), "%Y-%m-%d %H:%M:%S");
                                this.time
                                    .update(cx, |input, cx| input.set_value(now, window, cx));
                                this.time_on = true;
                                cx.notify();
                            })),
                    ),
            )
            .when(self.any_dir, |el| {
                el.child(
                    Checkbox::new("attr-recurse")
                        .label(sc_core::t!("attributes.recurse"))
                        .checked(self.recurse)
                        .on_click(cx.listener(|this, on: &bool, _, cx| {
                            this.recurse = *on;
                            cx.notify();
                        })),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("attr-ok")
                            .primary()
                            .label(sc_core::t!("attributes.ok"))
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.finish(window, cx) {
                                    crate::dialogs::dialog_window::close(window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("attr-cancel")
                            .outline()
                            .label(sc_core::t!("attributes.cancel"))
                            .min_w(px(80.))
                            .on_click(|_, window, cx| crate::dialogs::dialog_window::close(window, cx)),
                    ),
            )
    }
}
