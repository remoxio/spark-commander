//! Pack (Alt+F5), Unpack Specific Files (Alt+F9) and Test Archives
//! (Alt+Shift+F9): their dialogs; the work runs in the queue.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::{Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::ops::Queue;
use sc_core::archive::packing::{self, Format, Pack, Unpack};
use sc_core::{Panel, archive, command};

use crate::Commander;

const LEVELS: [u8; 5] = [0, 1, 5, 7, 9];

fn level_label(level: u8) -> String {
    match level {
        0 => sc_core::t!("pack.level.store"),
        1 => sc_core::t!("pack.level.fastest"),
        5 => sc_core::t!("pack.level.normal"),
        7 => sc_core::t!("pack.level.maximum"),
        _ => sc_core::t!("pack.level.ultra"),
    }
}

impl Commander {
    /// The other panel's folder, where packing and unpacking go; this one's
    /// if that one is on a server or in an archive.
    pub(crate) fn local_target(&self) -> PathBuf {
        let other = self.panels[1 - self.active].tabs.current();
        let this = self.panels[self.active].tabs.current();
        match other.remote().is_none() && !other.in_archive() {
            true => other.folder().to_owned(),
            false => this.folder().to_owned(),
        }
    }

    /// The chosen entries on this computer, outside archives; else why not,
    /// `what` naming the work in its message (packing, splitting, …).
    pub(crate) fn local_chosen(&self, what: &str) -> Result<(&Panel, Vec<OsString>), String> {
        let panel = self.panels[self.active].tabs.current();
        if panel.remote().is_some() || panel.in_archive() {
            return Err(sc_core::t!("pack.local_only", what = what));
        }
        let names: Vec<OsString> = panel.chosen().iter().map(|e| e.name.clone()).collect();
        Ok((panel, names))
    }

    pub(crate) fn pack(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (panel, names) = match self.local_chosen("packing") {
            Ok(found) => found,
            Err(err) => return window.push_notification(Notification::warning(err), cx),
        };
        if names.is_empty() {
            return;
        }
        let folder = panel.folder().to_owned();
        let base = match names.as_slice() {
            [one] => one.to_string_lossy().into_owned(),
            _ => folder
                .file_name()
                .map_or("archive".into(), |name| name.to_string_lossy().into_owned()),
        };
        let format = Format::Zip;
        let target = self.local_target().join(format.rename(&base));
        let label = sc_core::t!("pack.label", count = names.len());
        let queue = self.transfers.queue.clone();
        let build = move |_, window: &mut Window, cx: &mut App| {
        let target =
            cx.new(|cx| InputState::new(window, cx).default_value(target.display().to_string()));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let volume = cx.new(|cx| InputState::new(window, cx).default_value("100"));
        let view = cx.new(|_| PackDialog {
            label,
            target: target.clone(),
            format,
            level: 5,
            move_files: false,
            separate: false,
            split: false,
            volume,
            encrypt: false,
            password,
            folder,
            names,
            queue,
        });
        focus_all(&target, window, cx);
        view
        };
        open(sc_core::t!("pack.dialog_title"), 520., window, cx, build, |this, window, cx| this.finish(window, cx));
    }

    pub(crate) fn unpack(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (panel, names) = match self.local_chosen("unpacking") {
            Ok(found) => found,
            Err(err) => return window.push_notification(Notification::warning(err), cx),
        };
        let archives: Vec<PathBuf> = names
            .iter()
            .filter(|name| archive::is_archive(name))
            .map(|name| panel.folder().join(name))
            .collect();
        if archives.is_empty() {
            let text = sc_core::t!("pack.unpack.choose");
            return window.push_notification(Notification::warning(text), cx);
        }
        let label = sc_core::t!("pack.unpack.label", count = archives.len());
        let target = self.local_target().display().to_string();
        let queue = self.transfers.queue.clone();
        let base = panel.folder().to_owned();
        let build = move |_, window: &mut Window, cx: &mut App| {
        let target = cx.new(|cx| InputState::new(window, cx).default_value(target));
        let mask = cx.new(|cx| InputState::new(window, cx).default_value("*"));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let view = cx.new(|_| UnpackDialog {
            label,
            target: target.clone(),
            mask,
            with_paths: true,
            overwrite: false,
            separate: archives.len() > 1,
            password,
            archives,
            base,
            queue,
        });
        focus_all(&target, window, cx);
        view
        };
        open(sc_core::t!("pack.unpack.dialog_title"), 420., window, cx, build, |this, window, cx| this.finish(window, cx));
    }

    pub(crate) fn test_archives(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (panel, names) = match self.local_chosen("testing") {
            Ok(found) => found,
            Err(err) => return window.push_notification(Notification::warning(err), cx),
        };
        let archives: Vec<PathBuf> = names
            .iter()
            .filter(|name| archive::is_archive(name))
            .map(|name| panel.folder().join(name))
            .collect();
        if archives.is_empty() {
            let text = sc_core::t!("pack.test.choose");
            return window.push_notification(Notification::warning(text), cx);
        }
        // shortcut: no password to give; an encrypted archive reports that it needs one.
        self.transfers
            .queue
            .push_task(packing::test(archives, None));
    }
}

/// A dialog of `build`'s view, about `height` tall; OK runs `finish`, which keeps it open by returning false.
pub(crate) fn open<V: Render>(
    title: String,
    height: f32,
    window: &mut Window,
    cx: &mut App,
    build: impl FnOnce(Option<crate::dialogs::dialog_window::Main>, &mut Window, &mut App) -> Entity<V> + 'static,
    finish: fn(&mut V, &mut Window, &mut Context<V>) -> bool,
) {
    let ok = move |view: &Entity<V>, window: &mut Window, cx: &mut App| view.update(cx, |this, cx| finish(this, window, cx));
    crate::dialogs::dialog_window::show(title, size(px(600.), px(height)), window, cx, build, ok);
}

/// After the menu that opened the dialog has given the focus back to the panel.
pub(crate) fn focus_all(input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    let input = input.clone();
    window.defer(cx, move |window, cx| {
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
    });
}

pub(crate) fn buttons<V: 'static>(
    id: &'static str,
    cx: &mut Context<V>,
    finish: fn(&mut V, &mut Window, &mut Context<V>) -> bool,
) -> impl IntoElement {
    h_flex()
        .gap_2()
        .justify_end()
        .child(
            Button::new((id, 0usize))
                .primary()
                .label(sc_core::t!("pack.ok"))
                .min_w(px(80.))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if finish(this, window, cx) {
                        crate::dialogs::dialog_window::close(window, cx);
                    }
                })),
        )
        .child(
            Button::new((id, 1usize))
                .outline()
                .label(sc_core::t!("pack.cancel"))
                .min_w(px(80.))
                .on_click(|_, window, cx| crate::dialogs::dialog_window::close(window, cx)),
        )
}

pub(crate) fn check<V: 'static>(
    id: &'static str,
    label: String,
    on: bool,
    cx: &mut Context<V>,
    set: fn(&mut V, bool),
) -> Checkbox {
    Checkbox::new(id)
        .label(label)
        .checked(on)
        .on_click(cx.listener(move |this, on: &bool, _, cx| {
            set(this, *on);
            cx.notify();
        }))
}

struct PackDialog {
    label: String,
    target: Entity<InputState>,
    format: Format,
    level: u8,
    move_files: bool,
    separate: bool,
    split: bool,
    volume: Entity<InputState>,
    encrypt: bool,
    password: Entity<InputState>,
    folder: PathBuf,
    names: Vec<OsString>,
    queue: Queue,
}

impl PackDialog {
    /// The target's name follows the format.
    fn set_format(&mut self, format: Format, window: &mut Window, cx: &mut Context<Self>) {
        self.format = format;
        let text = self.target.read(cx).value().to_string();
        let path = Path::new(&text);
        let renamed = match path.file_name() {
            Some(name) => path.with_file_name(format.rename(&name.to_string_lossy())),
            None => path.to_owned(),
        };
        let renamed = renamed.display().to_string();
        self.target
            .update(cx, |input, cx| input.set_value(renamed, window, cx));
        cx.notify();
    }

    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let text = self.target.read(cx).value().to_string();
        let archive = command::resolve(text.trim(), &self.folder);
        let password = self.password.read(cx).value().to_string();
        let encrypt = self.encrypt && self.format.encrypts();
        let fail = |err: String, window: &mut Window, cx: &mut Context<Self>| {
            window.push_notification(Notification::error(err), cx);
            false
        };
        if encrypt && password.is_empty() {
            return fail(sc_core::t!("pack.password_missing"), window, cx);
        }
        let volume_mb = match self.split && !self.format.compressed_tar() {
            false => None,
            true => match self.volume.read(cx).value().trim().parse::<u64>() {
                Ok(mb) if mb > 0 => Some(mb),
                _ => return fail(sc_core::t!("pack.volume_invalid"), window, cx),
            },
        };
        self.queue.push_task(packing::pack(Pack {
            archive,
            format: self.format,
            level: self.level,
            folder: self.folder.clone(),
            names: self.names.clone(),
            password: encrypt.then_some(password),
            volume_mb,
            move_files: self.move_files,
            separate: self.separate,
        }));
        true
    }
}

impl Render for PackDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let format = Format::ALL.iter().position(|f| *f == self.format);
        let level = LEVELS.iter().position(|level| *level == self.level);
        let tar = self.format.compressed_tar();
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(
                v_flex()
                    .gap_1()
                    .child(self.label.clone())
                    .child(Input::new(&self.target)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().w(px(100.)).child(sc_core::t!("pack.format")))
                    .child(
                        RadioGroup::horizontal("pack-format")
                            .children(Format::ALL.map(Format::label))
                            .selected_index(format)
                            .on_click(cx.listener(|this, ix: &usize, window, cx| {
                                this.set_format(Format::ALL[*ix], window, cx)
                            })),
                    ),
            )
            .when(self.format != Format::Tar, |el| {
                el.child(
                    h_flex()
                        .gap_2()
                        .child(div().w(px(100.)).flex_none().child(sc_core::t!("pack.compression")))
                        .child(
                            RadioGroup::horizontal("pack-level")
                                .children(LEVELS.map(level_label))
                                .selected_index(level)
                                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                    this.level = LEVELS[*ix];
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(check(
                "pack-move",
                sc_core::t!("pack.move_files"),
                self.move_files,
                cx,
                |this, on| this.move_files = on,
            ))
            .child(check(
                "pack-separate",
                sc_core::t!("pack.separate"),
                self.separate,
                cx,
                |this, on| this.separate = on,
            ))
            .when(!tar, |el| {
                el.child(
                    h_flex()
                        .gap_2()
                        .child(check(
                            "pack-split",
                            sc_core::t!("pack.split"),
                            self.split,
                            cx,
                            |this, on| this.split = on,
                        ))
                        .child(div().w(px(80.)).child(Input::new(&self.volume).small()))
                        .child(sc_core::t!("pack.split_unit")),
                )
            })
            .when(self.format.encrypts(), |el| {
                el.child(
                    h_flex()
                        .gap_2()
                        .child(check(
                            "pack-encrypt",
                            sc_core::t!("pack.encrypt"),
                            self.encrypt,
                            cx,
                            |this, on| this.encrypt = on,
                        ))
                        .child(div().flex_1().child(Input::new(&self.password).small())),
                )
            })
            .child(buttons("pack", cx, PackDialog::finish))
    }
}

struct UnpackDialog {
    label: String,
    target: Entity<InputState>,
    mask: Entity<InputState>,
    with_paths: bool,
    overwrite: bool,
    separate: bool,
    password: Entity<InputState>,
    archives: Vec<PathBuf>,
    /// Relative targets start here.
    base: PathBuf,
    queue: Queue,
}

impl UnpackDialog {
    fn finish(&mut self, _: &mut Window, cx: &mut Context<Self>) -> bool {
        let text = self.target.read(cx).value().to_string();
        let password = self.password.read(cx).value().to_string();
        self.queue.push_task(packing::unpack(Unpack {
            archives: self.archives.clone(),
            target: command::resolve(text.trim(), &self.base),
            mask: self.mask.read(cx).value().to_string(),
            with_paths: self.with_paths,
            overwrite: self.overwrite,
            separate: self.separate,
            password: (!password.is_empty()).then_some(password),
        }));
        true
    }
}

impl Render for UnpackDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(
                v_flex()
                    .gap_1()
                    .child(self.label.clone())
                    .child(Input::new(&self.target)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().w(px(110.)).child(sc_core::t!("pack.unpack.mask")))
                    .child(div().flex_1().child(Input::new(&self.mask).small())),
            )
            .child(check(
                "unpack-paths",
                sc_core::t!("pack.unpack.with_paths"),
                self.with_paths,
                cx,
                |this, on| this.with_paths = on,
            ))
            .child(check(
                "unpack-overwrite",
                sc_core::t!("pack.unpack.overwrite"),
                self.overwrite,
                cx,
                |this, on| this.overwrite = on,
            ))
            .child(check(
                "unpack-separate",
                sc_core::t!("pack.unpack.separate"),
                self.separate,
                cx,
                |this, on| this.separate = on,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().w(px(110.)).child(sc_core::t!("pack.unpack.password")))
                    .child(div().flex_1().child(Input::new(&self.password).small())),
            )
            .child(buttons("unpack", cx, UnpackDialog::finish))
    }
}
