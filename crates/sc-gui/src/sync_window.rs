//! Commands > Synchronize Dirs in a window of its own, as Total Commander's:
//! the left and right panels' folders compared, each difference's action
//! changed by a click on it, then done as one job of the queue.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::ops::sync_dirs::{self, Action, Item, Options, Root, State};
use sc_core::{grouped, human_size, t};

use crate::Commander;

const CONTEXT: &str = "SyncDirs";
const ROW_HEIGHT: f32 = 22.;

actions!(sync_dirs, [StopOrClose]);

pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", StopOrClose, Some(CONTEXT))]);
}

impl Commander {
    /// The left panel's folder against the right one's.
    pub(crate) fn synchronize_dirs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let roots = self.panels.each_ref().map(|side| {
            let panel = side.tabs.current();
            let folder = panel.found_label().is_none() && !panel.in_archive();
            folder.then(|| Root::new(panel.fs(), panel.remote().cloned(), panel.folder().to_owned()))
        });
        let [Some(left), Some(right)] = roots else {
            let text = t!("sync.error.no_folders");
            return window.push_notification(Notification::warning(text), cx);
        };
        let main = window.bounds();
        let size = size(px(900.), px(680.)).min(&main.size);
        let origin = main.origin + point((main.size.width - size.width) / 2., (main.size.height - size.height) / 2.);
        let over = crate::Over::at(WindowBounds::Windowed(Bounds { origin, size }), window, cx);
        let options = crate::over_main_window(t!("sync.title"), over, cx);
        let commander = cx.entity().downgrade();
        let main = window.window_handle();
        let opened = gpui_kit::open_window(options, cx, |window, cx| {
            crate::menus::lend(window, main, cx);
            cx.new(|cx| SyncDirs::new(commander, left, right, window, cx))
        });
        if let Err(err) = opened {
            window.push_notification(Notification::error(err.to_string()), cx);
        }
    }
}

/// Which items are listed, by what is done with them.
#[derive(Clone, Copy)]
struct Show {
    to_right: bool,
    to_left: bool,
    equal: bool,
    other: bool,
}

impl Show {
    fn lists(self, item: &Item) -> bool {
        match item.action {
            Action::ToRight | Action::DeleteRight => self.to_right,
            Action::ToLeft | Action::DeleteLeft => self.to_left,
            Action::Nothing if item.state == State::Equal => self.equal,
            Action::Nothing => self.other,
        }
    }
}

struct SyncDirs {
    commander: WeakEntity<Commander>,
    left: Root,
    right: Root,
    options: Options,
    mask: Entity<InputState>,
    items: Vec<Item>,
    show: Show,
    /// The listed items' places in `items`.
    shown: Vec<usize>,
    /// While comparing: its stop, and the folder it is in.
    comparing: Option<(Arc<AtomicBool>, Arc<Mutex<PathBuf>>)>,
    status: String,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    _mask_events: Subscription,
}

impl SyncDirs {
    fn new(commander: WeakEntity<Commander>, left: Root, right: Root, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mask = cx.new(|cx| InputState::new(window, cx).default_value("*").placeholder("*.txt;*.md"));
        let events = cx.subscribe_in(&mask, window, |this, _, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.compare(window, cx);
            }
        });
        let mut this = Self {
            commander,
            left,
            right,
            options: Options::default(),
            mask,
            items: Vec::new(),
            show: Show { to_right: true, to_left: true, equal: false, other: true },
            shown: Vec::new(),
            comparing: None,
            status: String::new(),
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            _mask_events: events,
        };
        this.compare(window, cx);
        window.focus(&this.focus, cx);
        this
    }

    fn compare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.comparing.is_some() {
            return;
        }
        let (stop, at) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(PathBuf::new())));
        self.comparing = Some((stop.clone(), at.clone()));
        self.items.clear();
        self.shown.clear();
        let (left, right, options) = (self.left.clone(), self.right.clone(), self.options);
        let mask = self.mask.read(cx).value().to_string();
        let compared = cx.background_executor().spawn(async move {
            sync_dirs::compare(&left, &right, options, &mask, &stop, &mut |folder| *at.lock().unwrap() = folder.to_owned())
        });
        cx.spawn_in(window, async move |this, cx| {
            let mut compared = std::pin::pin!(compared);
            // The folder it is in, shown meanwhile.
            let found = loop {
                let tick = cx.background_executor().timer(Duration::from_millis(200));
                match futures::future::select(compared, std::pin::pin!(tick)).await {
                    futures::future::Either::Left((found, _)) => break found,
                    futures::future::Either::Right((_, pending)) => {
                        compared = pending;
                        if this.update(cx, |this, cx| this.show_folder(cx)).is_err() {
                            return;
                        }
                    }
                }
            };
            let _ = this.update(cx, |this, cx| this.compared(found, cx));
        })
        .detach();
        cx.notify();
    }

    fn show_folder(&mut self, cx: &mut Context<Self>) {
        if let Some((_, at)) = &self.comparing {
            let folder = at.lock().unwrap().display().to_string();
            self.status = t!("sync.status.comparing", folder = if folder.is_empty() { "..." } else { &folder });
            cx.notify();
        }
    }

    fn compared(&mut self, found: std::io::Result<Vec<Item>>, cx: &mut Context<Self>) {
        self.comparing = None;
        match found {
            Ok(items) => self.items = items,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => self.status = t!("sync.status.stopped"),
            Err(err) => self.status = t!("sync.error.compare", err = err),
        }
        self.list(cx);
    }

    /// The items `show` wants, and the line under them.
    fn list(&mut self, cx: &mut Context<Self>) {
        self.shown = (0..self.items.len()).filter(|&ix| self.show.lists(&self.items[ix])).collect();
        if self.comparing.is_none() && !self.items.is_empty() {
            let count = |actions: &[Action]| self.items.iter().filter(|item| actions.contains(&item.action)).count();
            let equal = self.items.iter().filter(|item| item.state == State::Equal).count();
            self.status = t!(
                "sync.status.counts",
                right = count(&[Action::ToRight]),
                left = count(&[Action::ToLeft]),
                delete = count(&[Action::DeleteLeft, Action::DeleteRight]),
                equal = equal,
                alone = count(&[Action::Nothing]) - equal,
            );
        } else if self.comparing.is_none() && self.status.is_empty() {
            self.status = t!("sync.status.both_empty");
        }
        cx.notify();
    }

    /// A click on an item's action: the next it may have.
    fn cycle(&mut self, ix: usize, cx: &mut Context<Self>) {
        let item = &mut self.items[ix];
        let choices = item.choices();
        let at = choices.iter().position(|action| *action == item.action);
        item.action = choices[at.map_or(0, |at| (at + 1) % choices.len())];
        // It stays listed until the list is made again.
        let shown = std::mem::take(&mut self.shown);
        self.list(cx);
        self.shown = shown;
    }

    fn stop_or_close(&mut self, window: &mut Window) {
        match &self.comparing {
            Some((stop, _)) => stop.store(true, Ordering::Relaxed),
            None => window.remove_window(),
        }
    }

    /// Asked first: what goes where, how much.
    fn synchronize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let todo: Vec<Item> = self.items.iter().filter(|item| item.action != Action::Nothing).cloned().collect();
        if todo.is_empty() {
            return;
        }
        let bytes = |action: Action, side: fn(&Item) -> Option<&sync_dirs::Side>| -> (usize, u64) {
            let some = todo.iter().filter(|item| item.action == action);
            // A folder's own size is not what is in it.
            (some.clone().count(), some.filter_map(side).filter(|side| !side.is_dir).map(|side| side.size).sum())
        };
        let (right, right_bytes) = bytes(Action::ToRight, |item| item.left.as_ref());
        let (left, left_bytes) = bytes(Action::ToLeft, |item| item.right.as_ref());
        let (gone_left, _) = bytes(Action::DeleteLeft, |item| item.left.as_ref());
        let (gone_right, _) = bytes(Action::DeleteRight, |item| item.right.as_ref());
        let mut lines = Vec::new();
        if right > 0 {
            lines.push(t!("sync.confirm.copy_right", count = right, size = human_size(right_bytes)));
        }
        if left > 0 {
            lines.push(t!("sync.confirm.copy_left", count = left, size = human_size(left_bytes)));
        }
        if gone_left > 0 {
            lines.push(t!("sync.confirm.delete_left", count = gone_left));
        }
        if gone_right > 0 {
            lines.push(t!("sync.confirm.delete_right", count = gone_right));
        }
        let trash = if gone_left + gone_right > 0 { "yes" } else { "no" };
        let text = t!("sync.confirm.text", actions = lines.join(", "), trash = trash);
        let (commander, roots) = (self.commander.clone(), (self.left.clone(), self.right.clone()));
        crate::dialogs::dialog_window::question(t!("sync.confirm.title"), text, t!("sync.confirm.ok"), false, window, cx, move |window, cx| {
            let task = sync_dirs::synchronize(roots.0.clone(), roots.1.clone(), todo.clone());
            let _ = commander.update(cx, |this, _| this.transfers.queue.push_task(task));
            // The queue under the panels shows how it goes.
            window.remove_window();
        });
    }

    fn render_row(&self, row: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let ix = self.shown[row];
        let item = &self.items[ix];
        let date = crate::date_fits(cx);
        let size = |side: &Option<sync_dirs::Side>| match side {
            Some(side) if side.is_dir => "<DIR>".to_owned(),
            Some(side) => grouped(side.size),
            None => String::new(),
        };
        let time = |side: &Option<sync_dirs::Side>| side.as_ref().and_then(|side| side.modified).map(sc_core::dates::date_time).unwrap_or_default();
        let (mark, color, end) = match item.action {
            Action::ToRight => ("→", theme.primary, false),
            Action::ToLeft => ("←", theme.primary, false),
            Action::DeleteLeft => ("✕", theme.danger, false),
            Action::DeleteRight => ("✕", theme.danger, true),
            Action::Nothing if item.state == State::Equal => ("=", theme.muted_foreground, false),
            Action::Nothing => ("≠", theme.warning, false),
        };
        let action = div()
            .id(("action", ix))
            .w(px(56.))
            .flex_none()
            .flex()
            .px_2()
            .when(end, |el| el.justify_end())
            .when(!end, |el| el.justify_center())
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|style| style.bg(theme.list_hover))
            .text_color(color)
            .child(mark)
            .on_click(cx.listener(move |this, _, _, cx| this.cycle(ix, cx)));
        let muted = theme.muted_foreground;
        h_flex()
            .id(("item", ix))
            .w_full()
            .h(px(ROW_HEIGHT))
            .px_2()
            .gap_3()
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis_start().child(item.path.display().to_string()))
            .child(div().w(px(90.)).flex_none().text_right().child(size(&item.left)))
            .child(div().w(date).flex_none().whitespace_nowrap().text_color(muted).child(time(&item.left)))
            .child(action)
            .child(div().w(date).flex_none().whitespace_nowrap().text_color(muted).child(time(&item.right)))
            .child(div().w(px(90.)).flex_none().text_right().child(size(&item.right)))
    }
}

impl Render for SyncDirs {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = crate::header::window_bar(t!("sync.title"), window, cx);
        let theme = cx.theme().clone();
        let comparing = self.comparing.is_some();
        let option = |id: &'static str, label: String, on: bool, set: fn(&mut Options, bool)| {
            Checkbox::new(id).label(label).checked(on).disabled(comparing).on_click(cx.listener(move |this, checked: &bool, _, cx| {
                set(&mut this.options, *checked);
                cx.notify();
            }))
        };
        let filter = |id: &'static str, label: String, on: bool, set: fn(&mut Show, bool)| {
            Checkbox::new(id).label(label).checked(on).on_click(cx.listener(move |this, checked: &bool, _, cx| {
                set(&mut this.show, *checked);
                this.list(cx);
            }))
        };
        let place = |label: String, root: &Root| {
            h_flex()
                .gap_2()
                .min_w_0()
                .child(div().w(px(44.)).flex_none().text_color(theme.muted_foreground).child(label))
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis_start().child(sync_dirs::place(root)))
        };
        let date = crate::date_fits(cx);
        let column = |label: String, width: Pixels, right: bool| div().w(width).flex_none().when(right, |el| el.text_right()).child(label);
        let list = uniform_list("sync-items", self.shown.len(), cx.processor(|this, range: std::ops::Range<usize>, _, cx| range.map(|row| this.render_row(row, cx)).collect()))
            .track_scroll(&self.scroll)
            .flex_1();
        let todo = self.items.iter().any(|item| item.action != Action::Nothing);
        v_flex()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .text_sm()
            .children(header)
            .on_action(cx.listener(|this, _: &StopOrClose, window, _| this.stop_or_close(window)))
            .child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .child(place(t!("sync.left"), &self.left))
                    .child(place(t!("sync.right"), &self.right))
                    .child(
                        h_flex()
                            .gap_4()
                            .child(option("subfolders", t!("sync.option.subfolders"), self.options.subfolders, |options, on| options.subfolders = on))
                            .child(option("content", t!("sync.option.by_content"), self.options.by_content, |options, on| options.by_content = on))
                            .child(option("ignore-date", t!("sync.option.ignore_date"), self.options.ignore_date, |options, on| options.ignore_date = on))
                            .child(option("asymmetric", t!("sync.option.asymmetric"), self.options.asymmetric, |options, on| options.asymmetric = on)),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(t!("sync.mask.label"))
                            .child(div().w(px(220.)).child(Input::new(&self.mask).small()))
                            .child(div().flex_1())
                            .child(match comparing {
                                true => Button::new("stop").small().label(t!("sync.stop")).on_click(cx.listener(|this, _, window, _| this.stop_or_close(window))),
                                false => Button::new("compare").small().primary().label(t!("sync.compare")).on_click(cx.listener(|this, _, window, cx| this.compare(window, cx))),
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_4()
                            .child(div().text_color(theme.muted_foreground).child(t!("sync.show.label")))
                            .child(filter("show-right", t!("sync.show.to_right"), self.show.to_right, |show, on| show.to_right = on))
                            .child(filter("show-left", t!("sync.show.to_left"), self.show.to_left, |show, on| show.to_left = on))
                            .child(filter("show-equal", t!("sync.show.equal"), self.show.equal, |show, on| show.equal = on))
                            .child(filter("show-other", t!("sync.show.other"), self.show.other, |show, on| show.other = on)),
                    ),
            )
            .child(
                h_flex()
                    .px_2()
                    .mx_3()
                    .h(px(ROW_HEIGHT))
                    .gap_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_color(theme.muted_foreground)
                    .child(div().flex_1().child(t!("sync.column.name")))
                    .child(column(t!("sync.column.size"), px(90.), true))
                    .child(column(t!("sync.column.date"), date, false))
                    .child(column(String::new(), px(56.), false))
                    .child(column(t!("sync.column.date"), date, false))
                    .child(column(t!("sync.column.size"), px(90.), true)),
            )
            .child(v_flex().flex_1().min_h_0().mx_3().child(list))
            .child(
                h_flex()
                    .p_3()
                    .gap_2()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(div().flex_1().min_w_0().truncate().text_color(theme.muted_foreground).child(self.status.clone()))
                    .children(crate::dialogs::ok_cancel(
                        Button::new("synchronize")
                            .small()
                            .primary()
                            .label(t!("sync.synchronize"))
                            .disabled(comparing || !todo)
                            .on_click(cx.listener(|this, _, window, cx| this.synchronize(window, cx))),
                        Button::new("close").small().outline().label(t!("sync.close")).on_click(|_, window, _| window.remove_window()),
                    )),
            )
    }
}
