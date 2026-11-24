//! Files › Download from Link: an HTTP(S) or FTP(S) link's file into the
//! active panel's folder, this computer's only.

use crate::dialogs::Role;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt, v_flex};
use gpui_kit::*;
use sc_core::ops::download;

use crate::Commander;

type Done = Rc<dyn Fn(String, Option<String>, &mut Window, &mut App)>;

impl Commander {
    /// Asks for a link, then downloads it into the active panel's folder.
    pub(crate) fn download_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        if panel.remote().is_some() || panel.in_archive() {
            let text = sc_core::t!("download.local_only");
            return window.push_notification(Notification::warning(text), cx);
        }
        let dir = panel.folder().to_owned();
        let this = cx.entity().downgrade();
        let into = dir.clone();
        open(dir, window, cx, move |link, name, window, cx| {
            let _ = this.update(cx, |this, cx| this.start_download(link, name, into.clone(), window, cx));
        });
    }

    fn start_download(&mut self, link: String, name: Option<String>, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        // A server's password, or its unknown key, is asked in a dialog.
        let (ask, _) = self.dialog_ask(window, cx);
        self.transfers.queue.run_now(download::download(link.trim().to_owned(), dir, name, ask));
        cx.notify();
    }
}

/// The link (the clipboard's, if it holds one) and the name to save as;
/// `done` gets them once they will do.
fn open(dir: PathBuf, window: &mut Window, cx: &mut App, done: impl Fn(String, Option<String>, &mut Window, &mut App) + 'static) {
    let clipboard = cx.read_from_clipboard().and_then(|item| item.text()).filter(|text| download::is_link(text));
    let done: Done = Rc::new(done);
    let build = move |main, window: &mut Window, cx: &mut App| {
    let link = cx.new(|cx| InputState::new(window, cx).placeholder(sc_core::t!("download.link_placeholder")).default_value(clipboard.unwrap_or_default()));
    let name = cx.new(|cx| InputState::new(window, cx).placeholder(sc_core::t!("download.name_placeholder")));
    let view = cx.new(|cx| {
        // A changed name is asked about again.
        let changed = |cx: &mut Context<DownloadDialog>, input: &Entity<InputState>| {
            cx.subscribe(input, |this: &mut DownloadDialog, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    (this.problem, this.replace) = (None, None);
                    cx.notify();
                }
            })
        };
        DownloadDialog {
            _changes: [changed(cx, &link), changed(cx, &name)],
            link: link.clone(),
            name,
            dir,
            problem: None,
            replace: None,
            done,
            main,
        }
    });
    crate::dialogs::pack_dialog::focus_all(&link, window, cx);
    view
    };
    // It closes itself once the link will do.
    let ok = |view: &Entity<DownloadDialog>, window: &mut Window, cx: &mut App| {
        view.update(cx, |this, cx| this.finish(window, cx));
        false
    };
    crate::dialogs::dialog_window::show(sc_core::t!("download.dialog_title"), size(px(560.), px(220.)), window, cx, build, ok);
}

struct DownloadDialog {
    _changes: [Subscription; 2],
    link: Entity<InputState>,
    name: Entity<InputState>,
    dir: PathBuf,
    problem: Option<String>,
    /// The name the user was told is taken: OK again replaces it.
    replace: Option<String>,
    done: Done,
    main: Option<crate::dialogs::dialog_window::Main>,
}

impl DownloadDialog {
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let link = self.link.read(cx).value().trim().to_string();
        let typed = self.name.read(cx).value().trim().to_string();
        let name = download::safe_name(&typed);
        self.problem = match () {
            _ if link.is_empty() => Some(sc_core::t!("download.problem.empty")),
            _ if !download::is_link(&link) => Some(sc_core::t!("download.problem.not_link")),
            _ if !typed.is_empty() && name.as_deref() != Some(typed.as_str()) => Some(sc_core::t!("download.problem.bad_name")),
            _ if name.as_ref().is_some_and(|name| self.dir.join(name).exists() && self.replace.as_ref() != Some(name)) => {
                self.replace = name.clone();
                Some(sc_core::t!("download.problem.exists", name = typed))
            }
            _ => None,
        };
        if self.problem.is_some() {
            return cx.notify();
        }
        let done = self.done.clone();
        crate::dialogs::dialog_window::close(window, cx);
        crate::dialogs::dialog_window::answer(self.main, window, cx, move |window, cx| done(link, name, window, cx));
    }
}

impl Render for DownloadDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let field = |label: String, input: &Entity<InputState>| v_flex().gap_1().child(label).child(Input::new(input));
        let ok = match self.replace.is_some() && self.problem.is_some() {
            true => sc_core::t!("download.replace"),
            false => sc_core::t!("download.download"),
        };
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(field(sc_core::t!("download.link"), &self.link))
            .child(field(sc_core::t!("download.save_as"), &self.name))
            .child(div().whitespace_normal().text_color(theme.muted_foreground).child(sc_core::t!("download.into", folder = self.dir.display())))
            .children(self.problem.clone().map(|problem| div().whitespace_normal().text_color(theme.danger).child(problem)))
            .child(
                crate::dialogs::responses(cx, [
                    (Role::Main, Button::new("download-ok").primary().label(ok).min_w(px(80.)).on_click(cx.listener(|this, _, window, cx| this.finish(window, cx)))),
                    (Role::Dismiss, Button::new("download-cancel").outline().label(sc_core::t!("download.cancel")).min_w(px(80.)).on_click(|_, window, cx| crate::dialogs::dialog_window::close(window, cx))),
                ]),
            )
    }
}
