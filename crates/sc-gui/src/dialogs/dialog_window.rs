//! A dialog as a window of its own, as Total Commander's: the system draws its
//! frame (KWin's decoration on KDE, Windows' caption with the app's icon), it is
//! centred over the window it was opened from, which takes no input while it
//! is open, and minimized with it; on macOS a sheet of that window. As tall as
//! its content; Enter is its OK, Escape closes it.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::t;

const CONTEXT: &str = "DialogWindow";

/// What Enter or OK does: true closes the dialog.
pub(crate) type Confirming = Rc<dyn Fn(&mut Window, &mut App) -> bool>;
/// What a button does on the window asked over.
pub(crate) type Acting = Rc<dyn Fn(&mut Window, &mut App)>;

actions!(dialog_window, [Confirm, Dismiss]);

pub(crate) fn bind_keys(cx: &mut App) {
    let keys = Some(CONTEXT);
    cx.bind_keys([KeyBinding::new("enter", Confirm, keys), KeyBinding::new("escape", Dismiss, keys)]);
}

/// Whether dialogs open as windows of their own; else in the window, as before.
pub(crate) fn separate() -> bool {
    true
}

/// Windows: the window's handle, for what GPUI does not do.
#[cfg(windows)]
fn hwnd(window: &Window) -> Option<isize> {
    match raw_window_handle::HasWindowHandle::window_handle(window).ok()?.as_raw() {
        raw_window_handle::RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

/// The dialog windows opened (some may be closed since).
#[derive(Default)]
struct Opened(Vec<AnyWindowHandle>);

impl Global for Opened {}

fn is_dialog(window: AnyWindowHandle, cx: &App) -> bool {
    cx.try_global::<Opened>().is_some_and(|opened| opened.0.contains(&window))
}

/// Whether a dialog window is open.
pub(crate) fn any_open(cx: &App) -> bool {
    let windows = cx.windows();
    cx.try_global::<Opened>().is_some_and(|opened| opened.0.iter().any(|window| windows.contains(window)))
}

/// Closes the dialog `window` shows: the window itself when it is a dialog window.
pub(crate) fn close(window: &mut Window, cx: &mut App) {
    match is_dialog(window.window_handle(), cx) {
        true => window.remove_window(),
        false => window.close_dialog(cx),
    }
}

/// The window a dialog was opened from, which takes its answer.
#[derive(Clone, Copy)]
pub(crate) struct Main(AnyWindowHandle);

impl Main {
    pub(crate) fn of(window: &Window) -> Self {
        Main(window.window_handle())
    }

    /// `f` on that window, once the dialog's turn is over.
    pub(crate) fn run(&self, cx: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
        let main = self.0;
        cx.defer(move |cx| {
            let _ = main.update(cx, |_, window, cx| f(window, cx));
        });
    }
}

/// What Enter does, on the dialog window, true to close it; with a label, an OK
/// button under the content too (Cancel beside it).
pub(crate) struct Ok {
    pub label: Option<String>,
    pub cancel: bool,
    pub danger: bool,
    pub run: Confirming,
}

impl Ok {
    /// Enter alone: the content has its own buttons.
    pub(crate) fn enter(run: impl Fn(&mut Window, &mut App) -> bool + 'static) -> Self {
        Ok { label: None, cancel: false, danger: false, run: Rc::new(run) }
    }
}

/// `build`'s view as a dialog: in a window of its own here (`size` about its
/// size), else in `window`, `size`'s width; Enter runs `ok` (true: it closes).
/// The view has its own buttons; it answers with `answer`, closes with `close`.
pub(crate) fn show<V: Render>(
    title: String,
    size: Size<Pixels>,
    window: &mut Window,
    cx: &mut App,
    build: impl FnOnce(Option<Main>, &mut Window, &mut App) -> Entity<V>,
    ok: impl Fn(&Entity<V>, &mut Window, &mut App) -> bool + 'static,
) {
    if separate() {
        return open(title, size, window, cx, move |main, window, cx| {
            let view = build(Some(main), window, cx);
            let shown = view.clone();
            (view, Some(Ok::enter(move |window, cx| ok(&shown, window, cx))))
        });
    }
    let view = build(None, window, cx);
    let ok = Rc::new(ok);
    window.open_dialog(cx, move |dialog, _, _| {
        let (shown, ok) = (view.clone(), ok.clone());
        dialog
            .title(title.clone())
            .width(size.width)
            .overlay_closable(false)
            .child(view.clone())
            .on_ok(move |_, window, cx| ok(&shown, window, cx))
    });
}

/// `f` on the window the dialog was opened from: `main`'s, once its turn is
/// over; else at once on `window`, which it is in.
pub(crate) fn answer(main: Option<Main>, window: &mut Window, cx: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
    match main {
        Some(main) => main.run(cx, f),
        None => f(window, cx),
    }
}

struct DialogWindow {
    title: SharedString,
    content: AnyView,
    ok: Option<Ok>,
    /// The page's height last laid out, which the window takes, up to `most`.
    height: Rc<Cell<Pixels>>,
    most: Pixels,
    /// Its content's width: Windows' window is that plus its frame.
    width: Pixels,
    /// Its keys' (Enter, Escape) where nothing inside takes them.
    focus: FocusHandle,
}

/// `build`'s view in a dialog window over `parent`, `size` as it is about: the
/// desktop centres it at that size, and it grows or shrinks to its content.
pub(crate) fn open<V: Render>(
    title: String,
    size: Size<Pixels>,
    parent: &mut Window,
    cx: &mut App,
    build: impl FnOnce(Main, &mut Window, &mut App) -> (Entity<V>, Option<Ok>),
) {
    let main = Main::of(parent);
    let bounds = parent.bounds();
    // shortcut: the window opens at the guessed height and is centred at that; GPUI cannot open one hidden till it is laid out.
    let origin = bounds.center() - point(size.width / 2., size.height / 2.);
    let over = crate::Over::at(WindowBounds::Windowed(Bounds { origin, size }), parent, cx);
    let options = WindowOptions {
        kind: WindowKind::Dialog,
        is_resizable: false,
        // Windows: it minimizes the app (`native_dialog`).
        is_minimizable: cfg!(windows),
        ..crate::over_main_window(title.clone(), over, cx)
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        #[cfg(windows)]
        if let Some(hwnd) = hwnd(window) {
            sc_core::system::theme::native_dialog(hwnd);
            // The caption as the main window's.
            sc_core::system::theme::set_caption_color(hwnd, if cx.theme().mode.is_dark() { 0x202020 } else { 0xffffff });
        }
        let (content, ok) = build(main, window, cx);
        let (most, focus) = (bounds.size.height * 0.9, cx.focus_handle());
        // Unless a field has it already; one focused once shown takes it after.
        if window.focused(cx).is_none() {
            window.focus(&focus, cx);
        }
        cx.new(|_| DialogWindow { title: title.into(), content: content.into(), ok, height: Rc::default(), most, width: size.width, focus })
    });
    match opened {
        Ok((handle, _)) => {
            let windows = cx.windows();
            let opened = &mut cx.default_global::<Opened>().0;
            opened.retain(|window| windows.contains(window));
            opened.push(handle);
        }
        Err(err) => parent.push_notification(Notification::error(err.to_string()), cx),
    }
}

/// A question with an OK and a Cancel button, in a window of its own here, else
/// in `window`; `ok` on `window` once it closed.
pub(crate) fn question(
    title: String,
    text: String,
    ok_label: String,
    danger: bool,
    window: &mut Window,
    cx: &mut App,
    ok: impl Fn(&mut Window, &mut App) + 'static,
) {
    if separate() {
        return confirm(title, text, ok_label, danger, window, cx, ok);
    }
    let ok = Rc::new(ok);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let ok = ok.clone();
        alert
            .title(title.clone())
            .description(text.clone())
            .ok_text(ok_label.clone())
            .when(danger, |alert| alert.ok_variant(gpui_kit::component::button::ButtonVariant::Danger))
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                // Closed first: what OK does may open another dialog, or move the focus.
                window.close_dialog(cx);
                ok(window, cx);
                false
            })
    });
}

/// A question with an OK and a Cancel button; `ok` on the window it was asked over.
fn confirm(
    title: String,
    text: String,
    ok_label: String,
    danger: bool,
    parent: &mut Window,
    cx: &mut App,
    ok: impl Fn(&mut Window, &mut App) + 'static,
) {
    let ok = Rc::new(ok);
    open(title, size(px(440.), px(150.)), parent, cx, move |main, _, cx| {
        let text = cx.new(|_| Text(text));
        let run = move |_: &mut Window, cx: &mut App| {
            let ok = ok.clone();
            main.run(cx, move |window, cx| ok(window, cx));
            true
        };
        (text, Some(Ok { label: Some(ok_label), cancel: true, danger, run: Rc::new(run) }))
    });
}

struct Text(String);

impl Render for Text {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().text_sm().whitespace_normal().child(self.0.clone())
    }
}

impl DialogWindow {
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self.ok.as_ref().map(|ok| ok.run.clone()) else {
            return cx.propagate();
        };
        if run(window, cx) {
            window.remove_window();
        }
    }
}

impl Render for DialogWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let wanted = self.height.get().min(self.most);
        let off = |a: Pixels, b: Pixels| (a - b).abs() > px(1.);
        if wanted > px(0.) && (off(wanted, viewport.height) || cfg!(windows) && off(self.width, viewport.width)) {
            // Windows: centred over its owner again, after this frame (its messages come at once); elsewhere the system places it.
            #[cfg(windows)]
            if let Some(hwnd) = hwnd(window) {
                let device = |side: Pixels| (f32::from(side) * window.scale_factor()).round() as i32;
                let (width, height) = (device(self.width), device(wanted));
                cx.foreground_executor().spawn(async move { sc_core::system::theme::fit_dialog(hwnd, width, height) }).detach();
            }
            #[cfg(not(windows))]
            window.resize(size(viewport.width, wanted));
        }
        let theme = cx.theme().clone();
        let height = self.height.clone();
        // GNOME's on Wayland: the app draws the frame.
        let header = crate::header::title_bar(self.title.clone(), window, cx);
        // macOS: a sheet has no title bar; its title heads it, as an alert's.
        let heading = cfg!(target_os = "macos").then(|| div().px_4().pt_4().text_sm().font_weight(FontWeight::BOLD).child(self.title.clone()));
        let buttons = self.ok.as_ref().and_then(|ok| Some((ok, ok.label.clone()?))).map(|(ok, label)| {
            let button = Button::new("dialog-ok").label(label).min_w(px(80.));
            let button = if ok.danger { button.danger() } else { button.primary() };
            h_flex()
                .pt_4()
                .gap_2()
                .justify_end()
                .child(button.on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))))
                .when(ok.cancel, |el| {
                    el.child(
                        Button::new("dialog-cancel")
                            .outline()
                            .label(t!("remote.dialog.cancel"))
                            .min_w(px(80.))
                            .on_click(|_, window, _| window.remove_window()),
                    )
                })
        });
        let page = v_flex()
            .relative()
            .w_full()
            .children(header)
            .children(heading)
            .child(v_flex().p_4().child(self.content.clone()).children(buttons))
            .child(
                canvas(
                    move |bounds, window, _| {
                        // Drawn again (and resized) next frame; a refresh while drawing is lost.
                        if height.replace(bounds.size.height) != bounds.size.height {
                            window.request_animation_frame();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            );
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.confirm(window, cx)))
            .on_action(|_: &Dismiss, window, _| window.remove_window())
            .child(page)
    }
}

/// A button of `choose`'s: how it looks, what it does on the window asked over.
pub(crate) struct Choice {
    pub label: String,
    pub style: Style,
    pub run: Acting,
}

#[derive(Clone, Copy)]
pub(crate) enum Style {
    Primary,
    Danger,
    Outline,
}

/// `text` (under a small `note`) and a button for each choice, in a dialog
/// window over `parent`; a button closes it, then runs on `parent`.
pub(crate) fn choose(title: String, note: Option<String>, text: String, choices: Vec<Choice>, width: Pixels, parent: &mut Window, cx: &mut App) {
    open(title, size(width, px(200.)), parent, cx, move |main, _, cx| (cx.new(|_| Choose { note, text, choices: choices.into(), main }), None));
}

struct Choose {
    note: Option<String>,
    text: String,
    choices: Rc<[Choice]>,
    main: Main,
}

impl Render for Choose {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let main = self.main;
        let buttons = self.choices.iter().enumerate().map(|(ix, choice)| {
            let run = choice.run.clone();
            let button = Button::new(("choice", ix)).label(choice.label.clone()).on_click(move |_, window, cx| {
                window.remove_window();
                let run = run.clone();
                main.run(cx, move |window, cx| run(window, cx));
            });
            match choice.style {
                Style::Primary => button.primary(),
                Style::Danger => button.danger(),
                Style::Outline => button.outline(),
            }
        });
        v_flex()
            .gap_3()
            .text_sm()
            .children(self.note.clone().map(|note| div().text_xs().child(note)))
            .child(div().whitespace_normal().child(self.text.clone()))
            .child(h_flex().gap_2().flex_wrap().justify_end().children(buttons))
    }
}
