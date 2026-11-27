//! Files by mouse and clipboard: dragged between panels, dropped in from
//! other programs, dragged out to them; Ctrl+C, Ctrl+X and Ctrl+V.

use std::path::Path;

use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt};
use gpui_kit::*;
use sc_core::ops::{Kind, Overwrite, Target};

use crate::Commander;
use crate::transfer::{Batch, Place};

/// A string entry's metadata saying the files are cut: as the vendored
/// gpui-pre-linux reads and writes it for other programs.
const CUT: &str = "cut";

/// A drag from a panel's row; what it takes is the panel's at the drop.
#[derive(Clone, Copy)]
pub(crate) struct Dragged {
    pub side: usize,
    pub row: usize,
}

/// What follows the pointer, just below and right of it: GPUI places it
/// where the row was grabbed, `grab` from its corner.
pub(crate) struct DragLabel {
    pub text: SharedString,
    pub grab: Point<Pixels>,
}

impl Render for DragLabel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let label = div()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .text_sm()
            .shadow_md()
            .child(self.text.clone());
        div()
            .pl(self.grab.x + px(14.))
            .pt(self.grab.y + px(14.))
            .child(label)
    }
}

/// Files copied or cut with Ctrl+C or Ctrl+X, and the text put on the
/// clipboard for them, which tells whether they are still what it holds.
pub(crate) struct FileClip {
    batch: Batch,
    cut: bool,
    text: String,
}

impl Commander {
    pub(crate) fn drag_label(&self, dragged: Dragged) -> SharedString {
        let panel = self.panels[dragged.side].tabs.current();
        match panel.dragged(dragged.row).as_slice() {
            [one] => one.display_name().into(),
            many => sc_core::t!("panel.drag.items", count = many.len()).into(),
        }
    }

    /// For other programs, when the drag leaves the window: this computer's files only.
    pub(crate) fn drag_payload(&self, dragged: Dragged) -> Option<ExternalDragPayload> {
        let panel = self.panels[dragged.side].tabs.current();
        if panel.remote().is_some() || panel.in_archive() {
            return None;
        }
        let entries = panel.dragged(dragged.row);
        let paths = entries.iter().map(|e| (panel.folder().join(&e.name), e.is_dir));
        let paths = FileDragPaths::new(paths);
        (!entries.is_empty()).then_some(ExternalDragPayload::Files(paths))
    }

    /// Where a drop onto `side` goes: into the folder at `row`, else the panel's.
    fn drop_place(&self, side: usize, row: Option<usize>) -> Place {
        let panel = self.panels[side].tabs.current();
        let mut place = Place::of(panel);
        let folder = row.and_then(|row| panel.entries().get(row)).filter(|e| e.is_dir);
        if let Some(entry) = folder {
            place.folder = match entry.is_parent() {
                true => place.folder.parent().map(Path::to_owned).unwrap_or(place.folder),
                false => place.folder.join(&entry.name),
            };
        }
        place
    }

    /// From a panel; Shift moves, as Total Commander's.
    pub(crate) fn drop_files(
        &mut self,
        dragged: Dragged,
        side: usize,
        row: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = self.panels[dragged.side].tabs.current();
        let onto = row.and_then(|row| source.entries().get(row).map(|e| e.name.clone()));
        let dropped_on_itself = side == dragged.side
            && match row {
                None => true,
                Some(_) => source.dragged(dragged.row).iter().any(|e| Some(&e.name) == onto.as_ref()),
            };
        if dropped_on_itself {
            return;
        }
        let Some(batch) = Batch::of(source, dragged.side, Some(dragged.row)) else {
            return;
        };
        let to = self.drop_place(side, row);
        self.open_transfer(kind(window), batch, to, window, cx);
    }

    /// From another program.
    pub(crate) fn drop_paths(
        &mut self,
        paths: &ExternalPaths,
        side: usize,
        row: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(batch) = Batch::paths(paths.paths().to_vec()) {
            let to = self.drop_place(side, row);
            self.open_transfer(kind(window), batch, to, window, cx);
        }
    }

    /// Ctrl+C and Ctrl+X: the chosen files, for Ctrl+V here or elsewhere.
    pub(crate) fn copy_files(&mut self, cut: bool, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        let Some(mut batch) = Batch::of(self.panels[side].tabs.current(), side, None) else {
            return;
        };
        if cut && batch.archived.is_some() {
            let text = sc_core::t!("panel.clipboard.no_cut_in_archive");
            return window.push_notification(Notification::warning(text), cx);
        }
        batch.unmark = None;
        let text = sc_core::system::clipboard::text_of(&batch.sources);
        // This computer's files go as a file list too, which file managers paste.
        let local = batch.archived.is_none() && batch.from.id() == "local";
        let item = match local {
            true => ClipboardItem {
                entries: vec![
                    ClipboardEntry::ExternalPaths(ExternalPaths(batch.sources.clone().into())),
                    ClipboardEntry::String(ClipboardString {
                        text: text.clone(),
                        metadata: cut.then(|| CUT.to_owned()),
                    }),
                ],
            },
            false => ClipboardItem::new_string(text.clone()),
        };
        cx.write_to_clipboard(item);
        // GPUI writes no file list on Windows.
        #[cfg(windows)]
        if local && let Err(error) = sc_core::system::clipboard::add_files(&batch.sources, cut) {
            eprintln!("file list to the clipboard: {error}");
        }
        let count = batch.sources.len();
        self.file_clip = Some(FileClip { batch, cut, text });
        let note = match cut {
            true => sc_core::t!("panel.clipboard.cut", count = count),
            false => sc_core::t!("panel.clipboard.copied", count = count),
        };
        window.push_notification(Notification::info(note), cx);
    }

    /// Ctrl+V: the files copied or cut here, or another program's, into this folder.
    pub(crate) fn paste_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let to = Place::of(self.panels[self.active].tabs.current());
        if to.in_archive {
            let text = sc_core::t!("panel.clipboard.no_paste_in_archive");
            return window.push_notification(Notification::warning(text), cx);
        }
        let item = cx.read_from_clipboard();
        let held = item.as_ref().and_then(|item| item.text());
        let ours = self.file_clip.as_ref().filter(|clip| held.as_ref() == Some(&clip.text));
        let (batch, cut) = match ours {
            Some(clip) => (clip.batch.clone(), clip.cut),
            // Another program's: this computer's files, copied or cut.
            None => match item.as_ref().and_then(other_files) {
                Some((batch, cut)) => (batch, cut),
                #[cfg(windows)]
                None => return self.paste_virtual_files(to, window, cx),
                #[cfg(not(windows))]
                None => {
                    let text = sc_core::t!("panel.clipboard.no_files");
                    return window.push_notification(Notification::warning(text), cx);
                }
            },
        };
        let here = batch.archived.is_none()
            && batch.from.id() == to.fs.id()
            && batch.sources.iter().any(|source| source.parent() == Some(&to.folder));
        if here {
            let text = sc_core::t!("panel.clipboard.same_folder");
            return window.push_notification(Notification::warning(text), cx);
        }
        // A cut is pasted once, as file managers do.
        if cut {
            self.file_clip = None;
            cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
        }
        let kind = match cut {
            true => Kind::Move,
            false => Kind::Copy,
        };
        let target = Target::Into(to.folder);
        self.enqueue(kind, batch, to.fs, target, Overwrite::Ask, window, cx);
    }
}

impl Commander {
    /// Windows: files offered with no path (an Outlook attachment), saved
    /// aside on a thread of their own, as the source may be slow, then moved in.
    #[cfg(windows)]
    fn paste_virtual_files(&mut self, to: Place, window: &mut Window, cx: &mut Context<Self>) {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_nanos());
        let dir = std::env::temp_dir().join(format!("sparkcommander-paste-{stamp}"));
        let saving = std::thread::spawn(move || sc_core::system::clipboard::save_virtual_files(&dir));
        let saved = cx.background_executor().spawn(async move { saving.join().unwrap_or_else(|_| Ok(Vec::new())) });
        cx.spawn_in(window, async move |this, cx| {
            let saved = saved.await;
            let _ = this.update_in(cx, |this, window, cx| match saved.map(Batch::paths) {
                Ok(Some(batch)) => this.enqueue(Kind::Move, batch, to.fs, Target::Into(to.folder), Overwrite::Ask, window, cx),
                Ok(None) => window.push_notification(Notification::warning(sc_core::t!("panel.clipboard.no_files")), cx),
                Err(err) => window.push_notification(Notification::error(err.to_string()), cx),
            });
        })
        .detach();
    }
}

fn kind(window: &Window) -> Kind {
    match window.modifiers().shift {
        true => Kind::Move,
        false => Kind::Copy,
    }
}

/// A file list, cut or copied, or text naming files.
fn other_files(item: &ClipboardItem) -> Option<(Batch, bool)> {
    let listed = item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::ExternalPaths(paths) => Some(paths.paths().to_vec()),
        _ => None,
    });
    let cut = item.entries().iter().any(|entry| {
        matches!(entry, ClipboardEntry::String(text) if text.metadata.as_deref() == Some(CUT))
    });
    match listed {
        Some(paths) => Some((Batch::paths(paths)?, cut)),
        None => Some((Batch::paths(sc_core::system::clipboard::paths_in(&item.text()?)?)?, false)),
    }
}
