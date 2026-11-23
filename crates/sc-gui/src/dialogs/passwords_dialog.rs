//! The saved passwords' master password (sc_core's secrets): set or changed
//! in Settings › Passwords, typed to unlock them on another computer.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::*;
use sc_core::remote::secrets;
use sc_core::t;

/// What the master password is asked for.
#[derive(Clone, Copy, PartialEq)]
pub enum Ask {
    /// To open the saved passwords here.
    Unlock,
    /// A new one, set or changed.
    Set,
    /// One to set before the first password is saved.
    First,
    /// Typed again to show a saved password.
    Reveal,
}

type Done = Rc<dyn Fn(&mut Window, &mut App)>;

/// The note that the saved passwords are locked, gone once they are open.
struct Locked;

/// Soon after the start: saved passwords locked here (the settings came from
/// another computer) are offered to unlock.
pub fn init(cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_secs(2)).await;
        // The keyring may wait for its wallet to open.
        if cx.background_spawn(async { secrets::locked() }).await {
            cx.update(offer_unlock);
        }
    })
    .detach();
}

fn offer_unlock(cx: &mut App) {
    let note = Notification::info(t!("passwords.locked_note")).id::<Locked>().autohide(false).action(|_, _, _| {
        Button::new("unlock-passwords").primary().label(t!("passwords.unlock_button")).on_click(|_, window, cx| ask(Ask::Unlock, window, cx, |_, _| {}))
    });
    crate::notify_main(note, cx);
}

/// Keeps `password` for `key`, in the background (the keyring may wait); the
/// first one asks for a master password before, cancelled, nothing is kept.
pub fn keep(key: String, password: String, window: &mut Window, cx: &mut App) {
    let save = move |cx: &mut App| {
        let (key, password) = (key.clone(), password.clone());
        // shortcut: a keyring that cannot keep it means asking again next time.
        cx.background_spawn(async move { secrets::remember(&key, &password) }).detach();
    };
    match secrets::has_master() {
        true => save(cx),
        false => ask(Ask::First, window, cx, move |_, cx| save(cx)),
    }
}

/// Asks for the master password (a new one twice) and unlocks or sets it in
/// the background; `done` hears once that has worked.
pub fn ask(what: Ask, window: &mut Window, cx: &mut App, done: impl Fn(&mut Window, &mut App) + 'static) {
    let done: Done = Rc::new(done);
    let build = move |main, window: &mut Window, cx: &mut App| {
        let mut masked = |cx: &mut App| cx.new(|cx| InputState::new(window, cx).masked(true));
        let first = masked(cx);
        let again = matches!(what, Ask::Set | Ask::First).then(|| masked(cx));
        crate::dialogs::pack_dialog::focus_all(&first, window, cx);
        cx.new(|_| MasterDialog { what, first, again, problem: None, working: false, done, main })
    };
    let title = match what {
        Ask::Unlock => t!("passwords.title.unlock"),
        Ask::Set | Ask::First => t!("passwords.title.set"),
        Ask::Reveal => t!("passwords.title.reveal"),
    };
    let height = if matches!(what, Ask::Set | Ask::First) { 300. } else { 230. };
    // It closes itself once the password has worked.
    let ok = |view: &Entity<MasterDialog>, window: &mut Window, cx: &mut App| {
        view.update(cx, |this, cx| this.finish(window, cx));
        false
    };
    crate::dialogs::dialog_window::show(title, size(px(480.), px(height)), window, cx, build, ok);
}

struct MasterDialog {
    what: Ask,
    first: Entity<InputState>,
    /// A new one's repeat.
    again: Option<Entity<InputState>>,
    problem: Option<String>,
    working: bool,
    done: Done,
    main: Option<crate::dialogs::dialog_window::Main>,
}

impl MasterDialog {
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.working {
            return;
        }
        let master = self.first.read(cx).value().to_string();
        let again = self.again.as_ref().map(|again| again.read(cx).value().to_string());
        self.problem = match () {
            _ if master.is_empty() => Some(t!("passwords.problem.empty")),
            _ if again.is_some_and(|again| again != master) => Some(t!("passwords.problem.differ")),
            _ => None,
        };
        if self.problem.is_some() {
            return cx.notify();
        }
        self.working = true;
        cx.notify();
        let what = self.what;
        // Slow on purpose (PBKDF2), and the keyring may wait.
        let work = cx.background_spawn(async move {
            match what {
                Ask::Unlock | Ask::Reveal => secrets::unlock(&master),
                Ask::Set | Ask::First => secrets::set_master(&master),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let worked = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.working = false;
                match worked {
                    Ok(()) => {
                        crate::dialogs::dialog_window::close(window, cx);
                        let text = match this.what {
                            Ask::Unlock => Some(t!("passwords.done.unlocked")),
                            Ask::Set | Ask::First => Some(t!("passwords.done.set")),
                            Ask::Reveal => None,
                        };
                        let done = this.done.clone();
                        crate::dialogs::dialog_window::answer(this.main, window, cx, move |window, cx| {
                            window.remove_notification::<Locked>(cx);
                            if let Some(text) = text {
                                window.push_notification(Notification::success(text), cx);
                            }
                            done(window, cx);
                        });
                    }
                    Err(err) => {
                        let text = err.to_string();
                        let mut chars = text.trim_end_matches('.').chars();
                        this.problem = chars.next().map(|first| format!("{}{}.", first.to_uppercase(), chars.as_str()));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
}

impl Render for MasterDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let about = match self.what {
            Ask::Unlock => t!("passwords.about.unlock"),
            Ask::Set => t!("passwords.about.set"),
            Ask::First => t!("passwords.about.first"),
            Ask::Reveal => t!("passwords.about.reveal"),
        };
        let field = |label: String, input: &Entity<InputState>| h_flex().gap_3().child(div().w(px(130.)).flex_none().child(label)).child(div().flex_1().child(Input::new(input)));
        let theme = cx.theme();
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(div().whitespace_normal().text_color(theme.muted_foreground).child(about))
            .child(field(t!("passwords.field.master"), &self.first))
            .children(self.again.as_ref().map(|again| field(t!("passwords.field.again"), again)))
            .children(self.problem.clone().map(|problem| div().whitespace_normal().text_color(theme.danger).child(problem)))
            .child(
                crate::dialogs::footer(cx)
                    .child(
                        Button::new("master-ok")
                            .primary()
                            .label(if self.working { t!("passwords.working") } else { t!("passwords.ok") })
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| this.finish(window, cx))),
                    )
                    .child(Button::new("master-cancel").outline().label(t!("passwords.cancel")).min_w(px(80.)).on_click(|_, window, cx| crate::dialogs::dialog_window::close(window, cx))),
            )
    }
}
