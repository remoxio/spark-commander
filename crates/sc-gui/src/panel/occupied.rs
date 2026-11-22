//! Calculate Occupied Space (Ctrl+L): totals counted in the background and
//! shown as they grow; measured folders then show their size in the panel.

use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::ops::space::{self, Space};
use sc_core::{grouped, human_size};

use crate::Commander;

/// What the counting thread found so far: the totals, each chosen
/// folder's size, and whether it is done.
#[derive(Default)]
struct Found {
    space: Space,
    folders: Vec<(OsString, u64)>,
    done: bool,
    error: Option<String>,
}

impl Commander {
    pub(crate) fn calculate_space(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        let panel = self.panels[side].tabs.current();
        if panel.in_archive() {
            let text = sc_core::t!("panel.space.in_archive");
            return window.push_notification(Notification::warning(text), cx);
        }
        // On `..`, the whole folder.
        let mut chosen = panel.chosen();
        if chosen.is_empty() {
            chosen = panel.entries().iter().filter(|e| !e.is_parent()).collect();
        }
        let items: Vec<(OsString, bool)> = chosen
            .into_iter()
            .map(|entry| (entry.name.clone(), entry.is_dir))
            .collect();
        let (fs, folder) = (panel.fs(), panel.folder().to_owned());
        let found = Arc::new(Mutex::new(Found::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (counting, stopping) = (found.clone(), stop.clone());
        let walked = folder.clone();
        std::thread::spawn(move || {
            let mut space = Space::default();
            for (name, is_dir) in items {
                let before = space.bytes;
                let mut seen = |space: &Space| counting.lock().unwrap().space = *space;
                let path = walked.join(&name);
                match space::measure(&*fs, &path, &mut space, &stopping, &mut seen) {
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => return,
                    Err(err) => {
                        counting.lock().unwrap().error = Some(format!("{}: {err}", path.display()));
                    }
                    Ok(()) if is_dir => {
                        let mut found = counting.lock().unwrap();
                        found.folders.push((name, space.bytes - before));
                    }
                    Ok(()) => {}
                }
            }
            let mut found = counting.lock().unwrap();
            (found.space, found.done) = (space, true);
        });

        let commander = cx.entity().downgrade();
        let view = cx.new(|cx| {
            // Shows the totals as they grow; once done, the folders get their sizes.
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    let done =
                        this.update(cx, |_, cx| cx.notify()).is_err() || found_done(&this, cx);
                    if done {
                        break;
                    }
                }
                let _ = this.update(cx, |this: &mut SpaceDialog, cx| {
                    let folders = std::mem::take(&mut this.found.lock().unwrap().folders);
                    let _ = commander.update(cx, |commander, cx| {
                        let panel = commander.panels[side].tabs.current_mut();
                        if panel.folder() == folder {
                            for (name, bytes) in folders {
                                panel.set_measured(name, bytes);
                            }
                        }
                        cx.notify();
                    });
                    cx.notify();
                });
            })
            .detach();
            SpaceDialog { found, stop }
        });
        crate::dialogs::dialog_window::show(sc_core::t!("panel.space.title"), size(px(420.), px(200.)), window, cx, move |_, _, _| view, |_, _, _| false);
    }
}

impl Commander {
    /// Alt+Shift+Enter, as Total Commander's: every folder shown gets its size
    /// in the Size column, one after another in the background, no dialog;
    /// the panel moving on stops it after the folder at hand.
    pub(crate) fn all_folder_sizes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        let panel = self.panels[side].tabs.current();
        if panel.in_archive() {
            let text = sc_core::t!("panel.space.in_archive");
            return window.push_notification(Notification::warning(text), cx);
        }
        let folders: Vec<OsString> = panel.entries().iter().filter(|e| e.is_dir && !e.is_parent()).map(|e| e.name.clone()).collect();
        self.measure_folders(side, folders, cx);
    }

    /// Total Commander's Space: marked or unmarked, the cursor staying; a
    /// folder's size then counted, as Alt+Shift+Enter counts each.
    pub(crate) fn mark_in_place(&mut self, cx: &mut Context<Self>) {
        let side = self.active;
        let panel = self.panels[side].tabs.current_mut();
        let folder = panel.current_entry().filter(|e| e.is_dir && !e.is_parent()).map(|e| e.name.clone());
        panel.mark_in_place();
        if let Some(name) = folder.filter(|_| !panel.in_archive()) {
            self.measure_folders(side, vec![name], cx);
        }
        cx.notify();
    }

    /// `folders` of `side`'s folder measured one by one in the background,
    /// each size shown once counted; going elsewhere stops it.
    fn measure_folders(&mut self, side: usize, folders: Vec<OsString>, cx: &mut Context<Self>) {
        let panel = self.panels[side].tabs.current();
        let (fs, folder) = (panel.fs(), panel.folder().to_owned());
        cx.spawn(async move |this, cx| {
            for name in folders {
                let (fs, path) = (fs.clone(), folder.join(&name));
                let measured = cx.background_executor().spawn(async move {
                    let mut space = Space::default();
                    space::measure(&*fs, &path, &mut space, &AtomicBool::new(false), &mut |_| {}).map(|()| space.bytes)
                });
                let bytes = measured.await;
                let going = this.update(cx, |this, cx| {
                    let panel = this.panels[side].tabs.current_mut();
                    if panel.folder() != folder {
                        return false;
                    }
                    if let Ok(bytes) = bytes {
                        panel.set_measured(name, bytes);
                        cx.notify();
                    }
                    true
                });
                if !going.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
    }
}

fn found_done(this: &WeakEntity<SpaceDialog>, cx: &mut AsyncApp) -> bool {
    this.read_with(cx, |this, _| this.found.lock().unwrap().done)
        .unwrap_or(true)
}

struct SpaceDialog {
    found: Arc<Mutex<Found>>,
    stop: Arc<AtomicBool>,
}

/// Closing the dialog stops the counting.
impl Drop for SpaceDialog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Render for SpaceDialog {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let found = self.found.lock().unwrap();
        let Space {
            files,
            dirs,
            bytes,
            unreadable,
        } = found.space;
        let line = |label: String, value: String| {
            h_flex()
                .gap_2()
                .child(div().w(px(110.)).child(label))
                .child(value)
        };
        v_flex()
            .w_full()
            .gap_2()
            .text_sm()
            .child(line(sc_core::t!("panel.space.files"), grouped(files)))
            .child(line(sc_core::t!("panel.space.folders"), grouped(dirs)))
            .child(line(
                sc_core::t!("panel.space.total"),
                sc_core::t!("panel.space.bytes", count = bytes, size = grouped(bytes), human = human_size(bytes)),
            ))
            .when(unreadable > 0, |el| {
                el.child(sc_core::t!("panel.space.unreadable", count = unreadable))
            })
            .children(found.error.clone())
            .child(match found.done {
                true => sc_core::t!("panel.space.done"),
                false => sc_core::t!("panel.space.counting"),
            })
            .child(
                h_flex().justify_end().child(
                    Button::new("space-close")
                        .primary()
                        .label(sc_core::t!("panel.space.close"))
                        .min_w(px(80.))
                        .on_click(|_, window, cx| crate::dialogs::dialog_window::close(window, cx)),
                ),
            )
    }
}
