//! F5 and F6: their dialog, the queue's progress strip, the sidebar's
//! Operations, and the questions a running job asks.
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{MAIN_SEPARATOR, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress as Bar;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, IndexPath, Sizable, StyledExt, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::Panel;
use sc_core::menu::Command;
use sc_core::ops::{Answer, JobView, Kind, Operation, Overwrite, Question, Queue, Status, Target, Task as Job};
use sc_core::vfs::{Fs, Local, Meta};

use crate::Commander;
use crate::dialogs::Role;
use crate::panel::drag_drop::DragLabel;
use gpui_kit::component::menu::ContextMenuExt;
use sc_core::sidebar::SidebarTab;

const OVERWRITE: [Overwrite; 5] = [Overwrite::Ask, Overwrite::All, Overwrite::Skip, Overwrite::Older, Overwrite::Rename];

fn overwrite_label(overwrite: Overwrite) -> String {
    match overwrite {
        Overwrite::Ask => sc_core::t!("transfer.overwrite.ask"),
        Overwrite::All => sc_core::t!("transfer.overwrite.all"),
        Overwrite::Skip => sc_core::t!("transfer.overwrite.skip"),
        Overwrite::Older => sc_core::t!("transfer.overwrite.older"),
        Overwrite::Rename => sc_core::t!("transfer.overwrite.rename"),
    }
}

/// The select choice `kind` is in messages.
fn kind_id(kind: Kind) -> &'static str {
    match kind {
        Kind::Copy => "copy",
        Kind::Move => "move",
    }
}

/// The F5 or F6 dialog's title, and its button's.
fn verb(kind: Kind) -> String {
    match kind {
        Kind::Copy => sc_core::t!("transfer.title.copy"),
        Kind::Move => sc_core::t!("transfer.title.move"),
    }
}

/// The queue, and what the main window last made of it.
#[derive(Default)]
pub struct Transfers {
    pub queue: Queue,
    version: u64,
    /// Finished jobs whose panels were read again.
    seen: HashSet<u64>,
    /// The job whose question is on screen.
    asked: Option<u64>,
    /// The newest job's id seen.
    newest: u64,
    /// The first job queued since, and when: still going a while later, it opens the sidebar's Operations.
    pending: Option<(u64, Instant)>,
}

/// Where files go: a panel's folder, or a folder shown in it.
pub(crate) struct Place {
    pub fs: Arc<dyn Fs>,
    pub folder: PathBuf,
    /// A server's address, for the dialog.
    pub server: Option<String>,
    pub in_archive: bool,
    /// Inside an archive: it and its folder, or why 7-Zip cannot change it.
    pub archive: Option<Result<(PathBuf, PathBuf), String>>,
}

impl Place {
    pub fn of(panel: &Panel) -> Self {
        Place {
            fs: panel.fs(),
            folder: panel.folder().to_owned(),
            server: panel.remote().map(|remote| remote.url()),
            in_archive: panel.in_archive(),
            archive: panel.in_archive().then(|| panel.archive_place().map_err(|err| err.to_string())),
        }
    }
}

/// What goes: a panel's entries, or files from another program.
#[derive(Clone)]
pub(crate) struct Batch {
    pub from: Arc<dyn Fs>,
    /// Their folder, the dialog's text being relative to it.
    pub folder: PathBuf,
    pub sources: Vec<PathBuf>,
    /// The one's name, shown name and whether a folder; none for several.
    pub one: Option<(OsString, String, bool)>,
    /// Inside an archive: the panel takes `names` out first.
    pub archived: Option<(Panel, Vec<OsString>)>,
    /// The side whose marks are dropped once queued.
    pub unmark: Option<usize>,
    pub summary: Summary,
}

/// What goes, as the dialog shows it: how many files and folders, the files'
/// size, the one's time.
#[derive(Clone, Default)]
pub(crate) struct Summary {
    pub files: usize,
    pub folders: usize,
    pub bytes: u64,
    pub modified: Option<SystemTime>,
}

impl Summary {
    fn of<'a>(entries: impl IntoIterator<Item = (bool, u64, Option<SystemTime>)>) -> Self {
        let mut summary = Summary::default();
        for (is_dir, size, modified) in entries {
            match is_dir {
                true => summary.folders += 1,
                false => {
                    summary.files += 1;
                    summary.bytes += size;
                }
            }
            summary.modified = modified;
        }
        summary
    }
}

impl Batch {
    /// A panel's chosen entries; with `row`, what a drag from there takes.
    pub fn of(panel: &Panel, side: usize, row: Option<usize>) -> Option<Self> {
        let entries = match row {
            Some(row) => panel.dragged(row),
            None => panel.chosen(),
        };
        let folder = panel.folder().to_owned();
        let one = match entries.as_slice() {
            [] => return None,
            [one] => Some((one.name.clone(), one.display_name(), one.is_dir)),
            _ => None,
        };
        let names: Vec<OsString> = entries.iter().map(|e| e.name.clone()).collect();
        let summary = Summary::of(entries.iter().map(|e| (e.is_dir, e.size, e.modified)));
        Some(Batch {
            from: match panel.in_archive() {
                true => Arc::new(Local),
                false => panel.fs(),
            },
            sources: names.iter().map(|name| folder.join(name)).collect(),
            archived: panel.in_archive().then(|| (panel.clone(), names)),
            unmark: Some(side),
            folder,
            one,
            summary,
        })
    }

    /// Files of this computer, from another program.
    pub fn paths(paths: Vec<PathBuf>) -> Option<Self> {
        let folder = paths.first()?.parent()?.to_owned();
        let one = match paths.as_slice() {
            [one] => {
                let name = one.file_name()?.to_owned();
                Some((name.clone(), name.display().to_string(), one.is_dir()))
            }
            _ => None,
        };
        let summary = Summary::of(paths.iter().map(|path| {
            let meta = std::fs::metadata(path).ok();
            (path.is_dir(), meta.as_ref().map_or(0, |meta| meta.len()), meta.and_then(|meta| meta.modified().ok()))
        }));
        Some(Batch {
            from: Arc::new(Local),
            folder,
            sources: paths,
            one,
            archived: None,
            unmark: None,
            summary,
        })
    }
}

impl Commander {
    /// The chosen entries into the other panel's folder, by way of the queue.
    pub(crate) fn transfer(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        let Some(batch) = Batch::of(self.panels[side].tabs.current(), side, None) else {
            return;
        };
        let to = Place::of(self.panels[1 - side].tabs.current());
        self.open_transfer(kind, batch, to, window, cx);
    }

    /// F5's or F6's dialog for `batch` into `to`, then the queue.
    pub(crate) fn open_transfer(
        &mut self,
        kind: Kind,
        batch: Batch,
        to: Place,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &to.archive {
            Some(Ok((archive, dir))) => return self.pack_into(kind, batch, archive.clone(), dir.clone(), window, cx),
            Some(Err(why)) => return window.push_notification(Notification::warning(why.clone()), cx),
            None => {}
        }
        // Moved out: 7-Zip must take them out of the archive after.
        if let (Some((panel, _)), Kind::Move) = (&batch.archived, kind)
            && let Err(err) = panel.archive_place()
        {
            return window.push_notification(Notification::warning(err.to_string()), cx);
        }
        let target = match &batch.one {
            Some((name, _, false)) => to.folder.join(name).display().to_string(),
            _ => folder_text(&to.folder),
        };
        let command = match kind {
            Kind::Copy => Command::Copy,
            Kind::Move => Command::Move,
        };
        let [key, _] = self.settings.keys.keys_of(command);
        let shown = Shown::new(kind, &batch, to.server.as_deref(), &key);
        let commander = cx.entity().downgrade();
        let Place { fs: to, server, folder, .. } = to;
        open_dialog(
            shown,
            target,
            window,
            cx,
            move |text, overwrite, window, cx| {
                let _ = commander.update(cx, |this, cx| {
                    // The server's own address typed: its path.
                    let text = server
                        .as_deref()
                        .and_then(|url| text.strip_prefix(url))
                        .unwrap_or(&text);
                    // A server's path from its folder: from the local one, Windows put its drive before it (C:\home\…).
                    let base = if server.is_some() { &folder } else { &batch.folder };
                    let target = Target::parse(text, base, batch.sources.len(), &*to);
                    this.enqueue(kind, batch.clone(), to.clone(), target, overwrite, window, cx);
                });
            },
        );
    }

    /// F5 or F6 into an archive: asked, then 7-Zip packs them into its
    /// folder `dir` as a queued task; another archive's are taken out first.
    fn pack_into(&mut self, kind: Kind, batch: Batch, archive: PathBuf, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if batch.from.id() != "local" {
            let text = sc_core::t!("transfer.archive.local_only");
            return window.push_notification(Notification::warning(text), cx);
        }
        let name = archive.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let into = match dir.as_os_str().is_empty() {
            true => name.clone(),
            false => format!("{name}{MAIN_SEPARATOR}{}", dir.display()),
        };
        let (id, count) = (kind_id(kind), batch.sources.len());
        let (title, text) = match &batch.one {
            Some((_, shown, _)) => (
                sc_core::t!("transfer.into_archive.title_one", kind = id, name = shown, archive = name),
                sc_core::t!("transfer.into_archive.question_one", kind = id, name = shown, into = into),
            ),
            None => (
                sc_core::t!("transfer.into_archive.title_many", kind = id, count = count, archive = name),
                sc_core::t!("transfer.into_archive.question_many", kind = id, count = count, into = into),
            ),
        };
        let verb = verb(kind);
        let commander = cx.entity().downgrade();
        crate::dialogs::dialog_window::question(verb.clone(), text, verb, false, window, cx, move |window, cx| {
            {
                let (batch, archive, dir, title) = (batch.clone(), archive.clone(), dir.clone(), title.clone());
                let _ = commander.update(cx, |this, cx| {
                    let queue = this.transfers.queue.clone();
                    let pack = move |sources: Vec<PathBuf>| {
                        queue.push_task(Job {
                            title,
                            run: Box::new(move |_| {
                                sc_core::archive::add(&archive, &sources, &dir)?;
                                if kind == Kind::Move {
                                    sources.iter().try_for_each(|source| sc_core::vfs::remove_all(&Local, source))?;
                                }
                                Ok(String::new())
                            }),
                        });
                    };
                    match batch.archived {
                        None => pack(batch.sources),
                        Some((panel, names)) => {
                            let taken = cx.background_executor().spawn(async move { panel.extract(&names) });
                            cx.spawn_in(window, async move |_, cx| match taken.await {
                                Ok(sources) => pack(sources),
                                Err(err) => {
                                    let note = Notification::error(sc_core::t!("transfer.extract_failed", err = err)).autohide(false);
                                    let _ = cx.update(|window, cx| window.push_notification(note, cx));
                                }
                            })
                            .detach();
                        }
                    }
                    if let Some(side) = batch.unmark {
                        this.panels[side].tabs.current_mut().mark_all(false);
                    }
                    cx.notify();
                });
            }
        });
    }

    /// Into the queue; an archive's are taken out first.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn enqueue(
        &mut self,
        kind: Kind,
        batch: Batch,
        to: Arc<dyn Fs>,
        target: Target,
        overwrite: Overwrite,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let queue = self.transfers.queue.clone();
        let from = batch.from.clone();
        // Moved out of an archive: what was moved goes from it, by 7-Zip, once the move is done.
        let leave = match (&batch.archived, kind) {
            (Some((panel, names)), Kind::Move) => panel.archive_place().ok().map(|(archive, dir)| {
                let inside: Vec<PathBuf> = names.iter().map(|name| dir.join(name)).collect();
                let name = archive.file_name().unwrap_or_default().to_string_lossy().into_owned();
                let queue = queue.clone();
                move |taken: Vec<PathBuf>| {
                    queue.push_task(Job {
                        title: sc_core::t!("transfer.remove_moved", name = name),
                        run: Box::new(move |_| {
                            // Moved whole is gone from where it was taken out to; skipped, failed, cancelled or half-moved is still there.
                            let gone: Vec<PathBuf> = inside.into_iter().zip(taken).filter(|(_, out)| std::fs::symlink_metadata(out).is_err()).map(|(inside, _)| inside).collect();
                            if !gone.is_empty() {
                                sc_core::archive::delete(&archive, &gone)?;
                            }
                            Ok(String::new())
                        }),
                    });
                }
            }),
            _ => None,
        };
        let push = move |sources: Vec<PathBuf>| {
            // An item a job: each cancelled apart.
            queue.push_each(Operation {
                kind,
                from,
                sources: sources.clone(),
                to,
                target,
                overwrite,
            });
            if let Some(leave) = leave {
                leave(sources);
            }
        };
        match batch.archived {
            None => drop(push(batch.sources)),
            Some((panel, names)) => {
                cx.spawn_in(window, async move |_, cx| {
                    let taken = cx
                        .background_executor()
                        .spawn(async move { panel.extract(&names) })
                        .await;
                    match taken {
                        Ok(sources) => drop(push(sources)),
                        Err(err) => {
                            let text = sc_core::t!("transfer.extract_failed", err = err);
                            let note = Notification::error(text).autohide(false);
                            let _ = cx.update(|window, cx| window.push_notification(note, cx));
                        }
                    }
                })
                .detach();
            }
        }
        // Queued: the marks have done their work.
        if let Some(side) = batch.unmark {
            self.panels[side].tabs.current_mut().mark_all(false);
        }
        cx.notify();
    }

    /// Checks the queue several times a second while the window lives.
    pub(crate) fn watch_queue(window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let watched = this.update_in(cx, |this, window, cx| {
                    this.queue_changed(window, cx);
                    this.show_slow_jobs(window, cx);
                    // After the finished were taken in (their notes, the panels read again);
                    // failed and cancelled ones stay until cleared.
                    let minutes = u64::from(this.settings.queue.clear_after_minutes);
                    this.transfers.queue.forget_done(Duration::from_secs(minutes * 60));
                });
                if watched.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Jobs still going 2 seconds after they were queued: the sidebar's Operations
    /// shows their progress. A quick one (a server's file opened) leaves it as it is.
    fn show_slow_jobs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((first, _)) = self.transfers.pending.filter(|(_, at)| at.elapsed() >= Duration::from_secs(2)) else {
            return;
        };
        self.transfers.pending = None;
        if self.transfers.queue.jobs().iter().any(|job| job.id >= first && !job.status.finished()) {
            self.show_operations(window, cx);
        }
    }

    fn queue_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The question's dialog gone unanswered (closed in place of another): asked again.
        let lost = self.transfers.asked.is_some() && !window.has_active_dialog(cx) && !crate::dialogs::dialog_window::any_open(cx);
        let transfers = &mut self.transfers;
        let version = transfers.queue.version();
        if version == transfers.version && !lost {
            return;
        }
        if lost {
            transfers.asked = None;
        }
        transfers.version = version;
        let jobs = transfers.queue.jobs();
        let newest = jobs.iter().map(|job| job.id).max().unwrap_or(0);
        if newest > transfers.newest {
            transfers.pending.get_or_insert((transfers.newest + 1, Instant::now()));
            transfers.newest = newest;
        }
        let finished: Vec<u64> = jobs
            .iter()
            .filter(|job| job.status.finished())
            .map(|job| job.id)
            .collect();
        let newly: Vec<&JobView> = jobs
            .iter()
            .filter(|job| job.status.finished() && !transfers.seen.contains(&job.id))
            .collect();
        transfers.seen = finished.into_iter().collect();
        // What a task found, or why it failed.
        for job in &newly {
            let note = match (&job.status, &job.note) {
                (Status::Failed, Some(note)) => {
                    let text = format!("{}: {note}", job.title);
                    Notification::error(text).autohide(false)
                }
                (_, Some(note)) => Notification::success(format!("{}: {note}", job.title)),
                _ => continue,
            };
            window.push_notification(note, cx);
        }
        let newly = !newly.is_empty();
        if newly {
            for ix in 0..self.panels.len() {
                // shortcut: a panel whose folder is gone keeps its list until it moves.
                let remote = self.panels[ix].tabs.current().remote().is_some();
                self.navigate_on(ix, remote, Panel::reload, window, cx);
            }
        }
        if self.transfers.asked.is_some_and(|id| {
            !jobs
                .iter()
                .any(|job| job.id == id && job.question.is_some())
        }) {
            self.transfers.asked = None;
        }
        if self.transfers.asked.is_none()
            && let Some(job) = jobs.iter().find(|job| job.question.is_some())
        {
            self.transfers.asked = Some(job.id);
            ask(&self.transfers.queue, job, window, cx);
        }
        cx.notify();
    }
}

fn folder_text(folder: &std::path::Path) -> String {
    let mut text = folder.display().to_string();
    if !text.ends_with(MAIN_SEPARATOR) {
        text.push(MAIN_SEPARATOR);
    }
    text
}

type Done = Rc<dyn Fn(String, Overwrite, &mut Window, &mut App)>;

/// The F5 or F6 dialog's words and what it shows of what goes.
struct Shown {
    kind: Kind,
    /// "Copy 3 items".
    heading: String,
    /// The one's name and whether a folder; none for several.
    one: Option<(String, bool)>,
    /// The one's name, or the first few names.
    name: String,
    /// Its size and time, or how many files and folders.
    detail: String,
    destination: String,
    /// The target is this computer's: its folder may be browsed for.
    local: bool,
    /// The one file's name the target ends in, kept in a folder browsed for.
    file_name: Option<OsString>,
    /// The command's key, which pressed again here does it.
    key: Option<Keystroke>,
}

impl Shown {
    fn new(kind: Kind, batch: &Batch, server: Option<&str>, key: &str) -> Self {
        let (summary, count) = (&batch.summary, batch.sources.len());
        let when = summary.modified.map(sc_core::dates::date_time);
        let detail = match &batch.one {
            Some((_, _, true)) => [Some(sc_core::t!("properties.kind.folder")), when].into_iter().flatten().collect::<Vec<_>>().join(" · "),
            Some(_) => [Some(sc_core::human_size(summary.bytes)), when].into_iter().flatten().collect::<Vec<_>>().join(" · "),
            None => {
                let files = (summary.files > 0).then(|| sc_core::t!("transfer.count.files", count = summary.files));
                let folders = (summary.folders > 0).then(|| sc_core::t!("transfer.count.folders", count = summary.folders));
                let counts = [files, folders].into_iter().flatten().collect::<Vec<_>>().join(", ");
                // The files' size alone would leave the folders' out.
                match summary.folders {
                    0 => format!("{counts} · {}", sc_core::human_size(summary.bytes)),
                    _ => counts,
                }
            }
        };
        let names: Vec<String> = batch.sources.iter().filter_map(|path| path.file_name()).map(|name| name.display().to_string()).collect();
        let name = match &batch.one {
            Some((_, shown, _)) => shown.clone(),
            None if names.len() > 3 => format!("{}, …", names[..3].join(", ")),
            None => names.join(", "),
        };
        // One stroke: a chord's second key would come to the dialog alone.
        let stroke = key.split_whitespace().count().eq(&1).then(|| Keystroke::parse(key).ok()).flatten();
        Shown {
            kind,
            heading: sc_core::t!("transfer.heading", kind = kind_id(kind), count = count),
            one: batch.one.as_ref().map(|(_, shown, folder)| (shown.clone(), *folder)),
            name,
            detail,
            destination: match server {
                Some(url) => sc_core::t!("transfer.destination_server", server = url),
                None => sc_core::t!("transfer.destination"),
            },
            local: server.is_none(),
            file_name: batch.one.as_ref().filter(|(_, _, folder)| !folder).map(|(name, _, _)| name.clone()),
            key: stroke,
        }
    }
}

/// What goes, where to, and what to do with files there already.
fn open_dialog(shown: Shown, target: String, window: &mut Window, cx: &mut App, done: impl Fn(String, Overwrite, &mut Window, &mut App) + 'static) {
    let title = verb(shown.kind);
    // Linux and macOS: a window of its own (macOS's sheet); its answer goes to the main window.
    if crate::dialogs::dialog_window::separate() {
        return crate::dialogs::dialog_window::open_headed(title, size(px(560.), px(360.)), window, cx, move |main, window, cx| {
            (cx.new(|cx| TransferDialog::new(shown, target, Rc::new(done), Some(main), window, cx)), None)
        });
    }
    let view = cx.new(|cx| TransferDialog::new(shown, target, Rc::new(done), None, window, cx));
    let input = view.read(cx).input.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let ok = view.clone();
        // Headed by the view.
        dialog.width(px(560.)).overlay_closable(false).child(view.clone()).on_ok(move |_, window, cx| {
            ok.update(cx, |this, cx| this.finish(window, cx));
            true
        })
    });
    input.update(cx, |input, cx| {
        input.focus(window, cx);
        input.select_all(window, cx);
    });
}

struct TransferDialog {
    shown: Shown,
    input: Entity<InputState>,
    overwrite: Overwrite,
    choices: Entity<SelectState<Vec<String>>>,
    done: Done,
    /// In a window of its own: the main window, which takes the answer.
    main: Option<crate::dialogs::dialog_window::Main>,
    _subscriptions: Vec<Subscription>,
}

impl TransferDialog {
    fn new(shown: Shown, target: String, done: Done, main: Option<crate::dialogs::dialog_window::Main>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(target));
        let labels: Vec<String> = OVERWRITE.map(overwrite_label).to_vec();
        let choices = cx.new(|cx| SelectState::new(labels.clone(), Some(IndexPath::new(0)), window, cx));
        let mut subscriptions = vec![cx.subscribe_in(&choices, window, move |this: &mut Self, _, event: &SelectEvent<Vec<String>>, _, cx| {
            let SelectEvent::Confirm(Some(label)) = event else { return };
            if let Some(at) = labels.iter().position(|shown| shown == label) {
                this.overwrite = OVERWRITE[at];
                cx.notify();
            }
        })];
        // A window of its own: Enter in the field is its OK.
        if main.is_some() {
            input.update(cx, |input, cx| {
                input.focus(window, cx);
                input.select_all(window, cx);
            });
            subscriptions.push(cx.subscribe_in(&input, window, |this: &mut Self, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.finish(window, cx);
                    this.close(window, cx);
                }
            }));
        }
        TransferDialog { shown, input, overwrite: Overwrite::Ask, choices, done, main, _subscriptions: subscriptions }
    }

    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.input.read(cx).value().to_string();
        match self.main {
            Some(main) => {
                let (done, overwrite) = (self.done.clone(), self.overwrite);
                main.run(cx, move |window, cx| done(target, overwrite, window, cx));
            }
            None => (self.done)(target, self.overwrite, window, cx),
        }
    }

    fn close(&self, window: &mut Window, cx: &mut App) {
        crate::dialogs::dialog_window::close(window, cx);
    }

    /// Another folder for the target, the one file's name kept.
    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: None });
        let (input, name) = (self.input.clone(), self.shown.file_name.clone());
        cx.spawn_in(window, async move |_, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(folder) = paths.into_iter().next() else { return };
            let text = match name {
                Some(name) => folder.join(name).display().to_string(),
                None => folder_text(&folder),
            };
            let _ = cx.update(|window, cx| input.update(cx, |input, cx| input.set_value(text, window, cx)));
        })
        .detach();
    }
}

impl Render for TransferDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, tile) = (theme.muted_foreground, theme.border, theme.muted);
        let shown = &self.shown;
        let command = match shown.kind {
            Kind::Copy => sc_core::menu::Command::Copy,
            Kind::Move => sc_core::menu::Command::Move,
        };
        let heading = h_flex()
            .gap_2()
            .children(crate::menus::toolbar_icon(command).map(|icon| Icon::new(icon).size_5().text_color(muted)))
            .child(div().text_xl().font_semibold().child(shown.heading.clone()));
        // The one's icon as the panel shows it; several, a stack.
        let icon = match &shown.one {
            Some((name, folder)) => {
                let icon = sc_core::icons::material().icon(name, *folder, !theme.mode.is_dark());
                img(format!("material/{icon}")).size(px(28.)).into_any_element()
            }
            None => Icon::new(gpui_kit::assets::IconName::Files).size(px(24.)).text_color(muted).into_any_element(),
        };
        // Framed, as WinUI's cards: it fills the width and needs no line under it.
        let card = h_flex()
            .gap_3()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .child(div().flex_none().size(px(48.)).rounded(px(10.)).bg(tile).flex().items_center().justify_center().child(icon))
            .child(
                v_flex()
                    .min_w_0()
                    .gap_0p5()
                    .child(div().text_base().font_medium().truncate().child(shown.name.clone()))
                    .child(div().text_color(muted).truncate().child(shown.detail.clone())),
            );
        let label = |text: String| div().text_color(muted).child(text);
        let browse = Button::new("transfer-browse")
            .outline()
            .icon(Icon::new(gpui_kit::assets::IconName::FolderOpen))
            .tooltip(sc_core::t!("transfer.browse"))
            .disabled(!shown.local)
            .on_click(cx.listener(|this, _, window, cx| this.browse(window, cx)));
        let destination = v_flex()
            .gap_1p5()
            .child(label(shown.destination.clone()))
            .child(h_flex().gap_2().child(div().flex_1().min_w_0().child(Input::new(&self.input))).child(browse));
        let exists = v_flex().gap_1p5().child(label(sc_core::t!("transfer.if_exists"))).child(Select::new(&self.choices));
        v_flex()
            .w_full()
            .gap_4()
            .text_sm()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                // The command's key, pressed again, does it.
                let Some(key) = &this.shown.key else { return };
                // Held from opening it, it repeats: not an answer.
                if !event.is_held && event.keystroke.key == key.key && event.keystroke.modifiers == key.modifiers {
                    cx.stop_propagation();
                    this.finish(window, cx);
                    this.close(window, cx);
                }
            }))
            .child(heading)
            .child(card)
            .child(destination)
            .child(exists)
            .child(
                crate::dialogs::footer(cx)
                    .children(crate::dialogs::ok_cancel(
                        Button::new("transfer-ok")
                            .primary()
                            .label(verb(shown.kind))
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.finish(window, cx);
                                this.close(window, cx);
                            })),
                        Button::new("transfer-cancel")
                            .outline()
                            .label(sc_core::t!("transfer.cancel"))
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                    )),
            )
    }
}

/// A running job's question; it waits until a button answers.
fn ask(queue: &Queue, job: &JobView, window: &mut Window, cx: &mut App) {
    let Some(question) = job.question.clone() else {
        return;
    };
    let answer = |answer: Answer| {
        let label = match answer {
            Answer::Overwrite => sc_core::t!("transfer.answer.overwrite"),
            Answer::OverwriteAll => sc_core::t!("transfer.answer.overwrite_all"),
            Answer::Skip => sc_core::t!("transfer.answer.skip"),
            Answer::SkipAll => sc_core::t!("transfer.answer.skip_all"),
            Answer::Rename => sc_core::t!("transfer.answer.rename"),
            Answer::Retry => sc_core::t!("transfer.answer.retry"),
            Answer::Elevate => sc_core::t!("transfer.answer.elevate"),
            Answer::Cancel => sc_core::t!("transfer.answer.cancel"),
        };
        (answer, label)
    };
    let (title, text, answers) = match &question {
        Question::Exists {
            target,
            source,
            existing,
        } => (
            sc_core::t!("transfer.exists.title"),
            format!(
                "{}\n\n{}",
                target.display(),
                sc_core::t!("transfer.exists.text", existing = describe(existing), new = describe(source))
            ),
            [Answer::Overwrite, Answer::OverwriteAll, Answer::Skip, Answer::SkipAll, Answer::Rename, Answer::Cancel].map(answer).to_vec(),
        ),
        // Permission denied: an administrator may, as Total Commander offers.
        Question::Failed { path, error, elevatable: true } => (
            sc_core::t!("transfer.error.title"),
            format!("{}\n\n{error}\n\n{}", path.display(), sc_core::t!("transfer.error.elevate_hint")),
            [Answer::Elevate, Answer::Retry, Answer::Skip, Answer::SkipAll, Answer::Cancel].map(answer).to_vec(),
        ),
        Question::Failed { path, error, .. } => (
            sc_core::t!("transfer.error.title"),
            format!("{}\n\n{error}", path.display()),
            [Answer::Retry, Answer::Skip, Answer::SkipAll, Answer::Cancel].map(answer).to_vec(),
        ),
    };
    let (queue, id) = (queue.clone(), job.id);
    let job_title = job.title.clone();
    if crate::dialogs::dialog_window::separate() {
        use crate::dialogs::dialog_window::{Choice, Style};
        let choices = answers.into_iter().map(|(answer, label)| {
            let queue = queue.clone();
            let style = match answer {
                Answer::Overwrite | Answer::Retry => Style::Primary,
                Answer::Cancel => Style::Danger,
                _ => Style::Outline,
            };
            Choice { label, style, run: Rc::new(move |_, _| queue.answer(id, answer)) }
        });
        return crate::dialogs::dialog_window::choose(title, Some(job_title), text, choices.collect(), px(560.), window, cx);
    }
    window.open_dialog(cx, move |dialog, _, cx| {
        let buttons = answers.iter().map(|(answer, label)| {
            let (queue, answer) = (queue.clone(), *answer);
            let button = Button::new(label.clone())
                .label(label.clone())
                .on_click(move |_, window, cx| {
                    queue.answer(id, answer);
                    crate::dialogs::dialog_window::close(window, cx);
                });
            match answer {
                Answer::Overwrite | Answer::Retry => (Role::Main, button.primary()),
                Answer::Cancel => (Role::Dismiss, button.with_variant(ButtonVariant::Danger)),
                _ => (Role::Other, button.outline()),
            }
        });
        dialog
            .title(title.clone())
            .width(px(560.))
            .keyboard(false)
            .close_button(false)
            .overlay_closable(false)
            .child(
                v_flex()
                    .gap_3()
                    .text_sm()
                    .child(div().text_xs().child(job_title.clone()))
                    .child(div().whitespace_normal().child(text.clone()))
                    .child(crate::dialogs::footer(cx).flex_wrap().children(crate::dialogs::ordered(buttons))),
            )
    });
}

fn describe(meta: &Meta) -> String {
    let time = meta
        .modified
        .map(sc_core::dates::date_time_seconds);
    sc_core::t!(
        "transfer.exists.describe",
        count = meta.size,
        size = sc_core::grouped(meta.size),
        time = time.unwrap_or_default()
    )
}

/// A waiting job dragged in the Operations list, to wait elsewhere.
#[derive(Clone, Copy)]
struct JobDrag(u64);

/// How far a job is, 0 to 1, by its bytes, else its files; none while counting.
fn done_part(progress: &sc_core::ops::Progress) -> Option<f32> {
    let part = match (progress.total_bytes, progress.total_files) {
        _ if progress.scanning => return None,
        (0, 0) => return None,
        (0, files) => progress.files as f32 / files as f32,
        (bytes, _) => progress.bytes as f32 / bytes as f32,
    };
    Some(part.clamp(0., 1.))
}

/// `1:05:00`, `4:10`, `12 s`.
fn duration_text(seconds: u64) -> String {
    match seconds {
        0..60 => sc_core::t!("operations.duration.seconds", seconds = seconds),
        60..3600 => format!("{}:{:02}", seconds / 60, seconds % 60),
        _ => format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60),
    }
}

/// A running job's files, bytes, speed and time left, as far as known.
fn running_text(job: &JobView) -> String {
    let p = &job.progress;
    let mut parts = Vec::new();
    if p.total_files > 1 {
        parts.push(sc_core::t!("operations.job.files", files = p.files, count = p.total_files));
    }
    if p.total_bytes > 0 {
        parts.push(sc_core::t!("operations.job.bytes", done = sc_core::human_size(p.bytes), total = sc_core::human_size(p.total_bytes)));
    }
    let seconds = job.started.map_or(0., |at| at.elapsed().as_secs_f64());
    let speed = (seconds > 1.).then(|| (p.bytes - p.resumed) as f64 / seconds).filter(|speed| *speed > 0.);
    if let Some(speed) = speed {
        parts.push(sc_core::t!("transfer.speed", size = sc_core::human_size(speed as u64)));
        if p.total_bytes > p.bytes {
            parts.push(sc_core::t!("operations.job.left", time = duration_text(((p.total_bytes - p.bytes) as f64 / speed) as u64)));
        }
    }
    match parts.is_empty() {
        true => sc_core::t!("operations.job.running"),
        false => parts.join(" · "),
    }
}

impl Commander {
    /// Commands › Operation Queue: the sidebar's Operations, shown.
    pub(crate) fn show_operations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar.tab != Some(SidebarTab::Operations) {
            self.toggle_sidebar(SidebarTab::Operations, window, cx);
        }
    }

    /// The Operations header's summary while jobs run or wait (or failed),
    /// and how far the running ones are, 0 to 1.
    pub(crate) fn operations_summary(&self) -> Option<(String, Option<f32>)> {
        let jobs = self.transfers.queue.jobs();
        let count = |status: Status| jobs.iter().filter(|job| job.status == status).count();
        let (running, waiting, failed) = (count(Status::Running), count(Status::Waiting), count(Status::Failed));
        let parts: Vec<f32> = jobs.iter().filter(|job| job.status == Status::Running).filter_map(|job| done_part(&job.progress)).collect();
        let done = (!parts.is_empty()).then(|| parts.iter().sum::<f32>() / parts.len() as f32);
        let mut text = Vec::new();
        match done {
            Some(done) => text.push(format!("{:.0}%", done * 100.)),
            None if running > 0 => text.push(sc_core::t!("operations.summary.running", count = running)),
            None => {}
        }
        if waiting > 0 {
            text.push(sc_core::t!("operations.summary.waiting", count = waiting));
        }
        if failed > 0 {
            text.push(sc_core::t!("operations.summary.failed", count = failed));
        }
        (!text.is_empty()).then(|| (text.join(" · "), done))
    }

    /// The jobs: running and waiting ones in the queue's order (the waiting
    /// dragged into another), then the finished ones, the newest first.
    pub(crate) fn render_operations(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let jobs = self.transfers.queue.jobs();
        let waiting: Rc<Vec<u64>> = Rc::new(jobs.iter().filter(|job| job.status == Status::Waiting).map(|job| job.id).collect());
        let (unfinished, finished): (Vec<&JobView>, Vec<&JobView>) = jobs.iter().partition(|job| !job.status.finished());
        let rows: Vec<AnyElement> = unfinished.into_iter().chain(finished.iter().rev().copied()).map(|job| self.render_job(job, waiting.clone(), cx).into_any_element()).collect();
        let theme = cx.theme();
        let queue = self.transfers.queue.clone();
        let list = v_flex().id("operations").flex_1().min_h_0().overflow_y_scroll().px_1().gap_0p5().children(rows);
        let list = match jobs.is_empty() {
            true => list.child(div().p_2().text_color(theme.muted_foreground).child(sc_core::t!("operations.empty"))),
            false => list,
        };
        let settings = Button::new("operations-settings")
            .ghost()
            .small()
            .icon(Icon::new(gpui_kit::assets::IconName::Settings))
            .tooltip(sc_core::t!("operations.settings_tooltip"))
            .on_click(cx.listener(|this, _, window, cx| this.open_settings_at("Operations", window, cx)));
        let clear = (!finished.is_empty()).then(|| Button::new("operations-clear").ghost().small().label(sc_core::t!("operations.clear")).on_click(move |_, _, _| queue.clear_finished()));
        v_flex().size_full().child(list).child(h_flex().px_1().pt_1().child(settings).child(div().flex_1()).children(clear))
    }

    fn render_job(&self, job: &JobView, waiting: Rc<Vec<u64>>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let (id, queue) = (job.id, self.transfers.queue.clone());
        let p = &job.progress;
        let (detail, color) = match &job.status {
            Status::Waiting => (sc_core::t!("operations.job.waiting"), theme.muted_foreground),
            Status::Running if job.question.is_some() => (sc_core::t!("operations.job.answer"), theme.warning),
            Status::Running if p.scanning => (sc_core::t!("operations.job.counting", count = p.total_files), theme.muted_foreground),
            Status::Running => (running_text(job), theme.muted_foreground),
            Status::Done if p.skipped > 0 => (sc_core::t!("operations.job.done_skipped", count = p.skipped), theme.muted_foreground),
            Status::Done => (job.note.clone().unwrap_or_else(|| sc_core::t!("operations.job.done")), theme.muted_foreground),
            Status::Cancelled => (sc_core::t!("operations.job.cancelled"), theme.muted_foreground),
            Status::Failed => (job.note.clone().unwrap_or_else(|| sc_core::t!("operations.job.failed")), theme.danger),
        };
        let percent = done_part(p).map(|done| done * 100.);
        let running = job.status == Status::Running;
        let is_waiting = job.status == Status::Waiting;
        let cancel = (!job.status.finished()).then(|| {
            let queue = queue.clone();
            Button::new(("job-cancel", id)).ghost().xsmall().icon(IconName::Close).tooltip(sc_core::t!("operations.job.cancel_tooltip")).on_click(move |_, _, _| queue.cancel(id))
        });
        let title = job.title.clone();
        let target = theme.drop_target;
        let row = v_flex()
            .id(("job", id))
            .px_1p5()
            .py_1()
            .gap_1()
            .rounded(theme.radius)
            .hover(|style| style.bg(theme.foreground.opacity(0.06)))
            .when(job.status.finished(), |el| el.opacity(0.75))
            .child(
                h_flex()
                    .gap_1()
                    .child(div().flex_1().min_w_0().truncate().child(title.clone()))
                    .children(cancel),
            )
            .when(running, |el| el.child(Bar::new(("job-bar", id)).value(percent.unwrap_or(0.))))
            .child(div().text_xs().text_color(color).whitespace_normal().child(detail));
        // Waiting ones go where they are dropped, or a place up or down.
        let dropped = self.transfers.queue.clone();
        let row = row.when(is_waiting, move |row| {
            let text: SharedString = title.into();
            row.on_drag(JobDrag(id), move |_, grab, _, cx| cx.new(|_| DragLabel { text: text.clone(), grab }))
                .drag_over::<JobDrag>(move |style, _, _, _| style.bg(target))
                .on_drop(move |JobDrag(moved): &JobDrag, _, _| {
                    let at = |job: u64| waiting.iter().position(|&id| id == job);
                    if let (Some(from), Some(to)) = (at(*moved), at(id)) {
                        dropped.reorder(*moved, to as isize - from as isize);
                    }
                })
        });
        let step = move |by: isize, label: String| {
            let queue = queue.clone();
            gpui_kit::component::menu::PopupMenuItem::new(label).on_click(move |_, _, _| queue.reorder(id, by))
        };
        let (menu_queue, going) = (self.transfers.queue.clone(), !job.status.finished());
        row.context_menu(move |menu, _, _| {
            let queue = menu_queue.clone();
            let menu = match is_waiting {
                true => menu.item(step(-1, sc_core::t!("operations.menu.move_up"))).item(step(1, sc_core::t!("operations.menu.move_down"))).separator(),
                false => menu,
            };
            match going {
                true => menu.item(gpui_kit::component::menu::PopupMenuItem::new(sc_core::t!("operations.menu.cancel")).on_click(move |_, _, _| queue.cancel(id))),
                false => menu.item(gpui_kit::component::menu::PopupMenuItem::new(sc_core::t!("operations.menu.clear")).on_click(move |_, _, _| queue.clear_finished())),
            }
        })
    }
}
