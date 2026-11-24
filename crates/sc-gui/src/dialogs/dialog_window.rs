//! A dialog as a window of its own, as Total Commander's: the system draws its
//! frame (KWin's decoration on KDE; on Windows the app, as Windows 11's caption), it is
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

use crate::dialogs::Role;

const CONTEXT: &str = "DialogWindow";

/// What Enter or OK does: true closes the dialog.
pub(crate) type Confirming = Rc<dyn Fn(&mut Window, &mut App) -> bool>;
/// What a button does on the window asked over.
pub(crate) type Acting = Rc<dyn Fn(&mut Window, &mut App)>;

actions!(dialog_window, [Confirm, Dismiss, WindowMenu]);

pub(crate) fn bind_keys(cx: &mut App) {
    let keys = Some(CONTEXT);
    cx.bind_keys([KeyBinding::new("enter", Confirm, keys), KeyBinding::new("escape", Dismiss, keys)]);
    // Windows: Alt+Space, the window's menu under its caption, as Windows' own.
    #[cfg(windows)]
    {
        cx.bind_keys([KeyBinding::new("alt-space", WindowMenu, None)]);
        cx.on_action(|_: &WindowMenu, cx| {
            let Some(handle) = cx.active_window() else { return };
            cx.spawn(async move |cx| {
                // Opened while Alt is down, its release would close it; the window is free once the key is over.
                while sc_core::system::theme::alt_held() {
                    cx.background_executor().timer(std::time::Duration::from_millis(15)).await;
                }
                cx.background_executor().timer(std::time::Duration::from_millis(50)).await;
                let at = cx.update_window(handle, |_, window, _| (hwnd(window), (32. * window.scale_factor()).round() as i32));
                if let Ok((Some(hwnd), below)) = at {
                    sc_core::system::theme::system_menu(hwnd, Some(below));
                }
            })
            .detach();
        });
    }
}

/// Whether dialogs open as windows of their own; Windows' in the window, as WinUI's ContentDialog.
pub(crate) fn separate() -> bool {
    !cfg!(windows)
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
    build: impl FnOnce(Option<Main>, &mut Window, &mut App) -> Entity<V> + 'static,
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
    /// The title over the content, else the content's own heading.
    heading: bool,
    content: AnyView,
    ok: Option<Ok>,
    /// The page's height last laid out, which the window takes, up to `most`.
    height: Rc<Cell<Pixels>>,
    most: Pixels,
    /// Its content's width: Windows' window is that plus its frame.
    width: Pixels,
    /// Its keys' (Enter, Escape) where nothing inside takes them.
    focus: FocusHandle,
    /// A question: on GNOME as libadwaita's alert.
    alert: bool,
    /// The window it asks over, and whether it dims it (an alert, a page with
    /// `responses`, on GNOME); its own window.
    asked: AnyWindowHandle,
    veiled: Rc<Cell<bool>>,
    own: AnyWindowHandle,
}

/// The veil over `window` on, as an alert's (black at 30%).
fn veil(window: AnyWindowHandle, cx: &mut App) {
    let veil = cx.default_global::<gpui_kit::component::Veil>();
    veil.opacity = 0.3;
    veil.windows.push(window);
}

/// The veil of an alert over `window` off (another alert may keep one).
fn unveil(window: AnyWindowHandle, cx: &mut App) {
    if cx.has_global::<gpui_kit::component::Veil>() {
        let veil = cx.global_mut::<gpui_kit::component::Veil>();
        if let Some(at) = veil.windows.iter().position(|veiled| *veiled == window) {
            veil.windows.remove(at);
        }
    }
    let _ = window.update(cx, |_, window, _| window.refresh());
}

/// GNOME's: questions as libadwaita's alert dialog.
fn adwaita() -> bool {
    sc_core::system::theme::control_style() == Some(sc_core::system::theme::ControlStyle::Adwaita)
}

/// GNOME: libadwaita's alert buttons, a row as wide as the dialog under a line,
/// flat and bold, divided by lines, in the order given.
fn alert_row(buttons: Vec<Stateful<Div>>, cx: &App) -> Div {
    let line = cx.theme().border;
    let buttons = buttons.into_iter().enumerate().map(move |(ix, button)| button.when(ix > 0, |button| button.border_l_1().border_color(line)));
    h_flex().mt_6().border_t_1().border_color(line).children(buttons)
}

/// One of `alert_row`'s: the main one's label in the accent, a dangerous one's red.
fn alert_button(id: impl Into<ElementId>, label: SharedString, color: Hsla, cx: &App) -> Stateful<Div> {
    let hover = cx.theme().foreground.opacity(0.05);
    div()
        .id(id)
        .flex_1()
        .h(px(46.))
        .flex()
        .items_center()
        .justify_center()
        .font_weight(FontWeight::BOLD)
        .text_color(color)
        .hover(move |style| style.bg(hover))
        .child(label)
}

/// `build`'s view in a dialog window over `parent`, `size` as it is about: the
/// desktop centres it at that size, and it grows or shrinks to its content.
pub(crate) fn open<V: Render>(
    title: String,
    size: Size<Pixels>,
    parent: &mut Window,
    cx: &mut App,
    build: impl FnOnce(Main, &mut Window, &mut App) -> (Entity<V>, Option<Ok>) + 'static,
) {
    open_window(title, size, true, false, parent, cx, build)
}

/// A question as `open`: on GNOME as libadwaita's alert (`alert_row`).
fn open_alert<V: Render>(
    title: String,
    size: Size<Pixels>,
    parent: &mut Window,
    cx: &mut App,
    build: impl FnOnce(Main, &mut Window, &mut App) -> (Entity<V>, Option<Ok>) + 'static,
) {
    open_window(title, size, true, true, parent, cx, build)
}

/// As `open`, the view heading itself: no title over it.
pub(crate) fn open_headed<V: Render>(
    title: String,
    size: Size<Pixels>,
    parent: &mut Window,
    cx: &mut App,
    build: impl FnOnce(Main, &mut Window, &mut App) -> (Entity<V>, Option<Ok>) + 'static,
) {
    open_window(title, size, false, false, parent, cx, build)
}

fn open_window<V: Render>(
    title: String,
    size: Size<Pixels>,
    heading: bool,
    alert: bool,
    parent: &mut Window,
    cx: &mut App,
    build: impl FnOnce(Main, &mut Window, &mut App) -> (Entity<V>, Option<Ok>) + 'static,
) {
    // GPUI draws a new window at once: after the opener's turn, so its first frame may read the opener.
    parent.defer(cx, move |parent, cx| open_now(title, size, heading, alert, parent, cx, build));
}

fn open_now<V: Render>(
    title: String,
    size: Size<Pixels>,
    heading: bool,
    alert: bool,
    parent: &mut Window,
    cx: &mut App,
    build: impl FnOnce(Main, &mut Window, &mut App) -> (Entity<V>, Option<Ok>),
) {
    let main = Main::of(parent);
    let bounds = parent.bounds();
    // GNOME: its own frame's shadow around the page, 20 px a side (GPUI Kit's), so the page keeps its size.
    let page = size.width;
    let size = match sc_core::settings::menu_button() {
        true => size + gpui_kit::size(px(40.), px(40.)),
        false => size,
    };
    // shortcut: the window opens at the guessed height and is centred at that; GPUI cannot open one hidden till it is laid out.
    let origin = bounds.center() - point(size.width / 2., size.height / 2.);
    let over = crate::Over::at(WindowBounds::Windowed(Bounds { origin, size }), parent, cx);
    let options = WindowOptions {
        kind: WindowKind::Dialog,
        is_resizable: false,
        // Windows: it minimizes the app (`native_dialog`).
        is_minimizable: cfg!(windows),
        // GNOME: its own frame, without a title bar, as libadwaita's dialogs; elsewhere the desktop's.
        window_decorations: sc_core::settings::menu_button().then_some(WindowDecorations::Client),
        // Linux: the desktop's own title bar untitled, the title a heading in the window.
        ..crate::over_main_window(if cfg!(target_os = "linux") { String::new() } else { title.clone() }, over, cx)
    };
    // Windows: its caption drawn here (`caption`), as Windows' own greys out a maximize beside minimize.
    #[cfg(windows)]
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions { title: Some(title.clone().into()), appears_transparent: true, traffic_light_position: None }),
        ..options
    };
    // GNOME: an alert dims the window it asks over, as libadwaita's (black at 30%); other dialogs do not.
    let veils = alert && adwaita();
    let asked = parent.window_handle();
    let veiled = Rc::new(Cell::new(veils));
    if veils {
        veil(asked, cx);
        parent.refresh();
    }
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
        let own = window.window_handle();
        cx.new(|cx| {
            // Closed (its window gone), the veil goes.
            let gone = veiled.clone();
            cx.on_release(move |_, cx| {
                if gone.get() {
                    unveil(asked, cx);
                }
                if cx.has_global::<crate::dialogs::Responding>() {
                    cx.global_mut::<crate::dialogs::Responding>().0.retain(|window| *window != own);
                }
            })
            .detach();
            DialogWindow { title: title.into(), heading, content: content.into(), ok, height: Rc::default(), most, width: page, focus, alert, asked, veiled: veiled.clone(), own }
        })
    });
    match opened {
        Ok((handle, _)) => {
            // In front with the keys: GNOME gives a dialog not marked modal no focus of its own.
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            let windows = cx.windows();
            let opened = &mut cx.default_global::<Opened>().0;
            opened.retain(|window| windows.contains(window));
            opened.push(handle);
        }
        Err(err) => {
            if veiled.get() {
                unveil(asked, cx);
                parent.refresh();
            }
            parent.push_notification(Notification::error(err.to_string()), cx)
        }
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
    // GNOME: as wide as libadwaita's alert.
    let width = if adwaita() { px(372.) } else { px(440.) };
    open_alert(title, size(width, px(150.)), parent, cx, move |main, _, cx| {
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
        // Its own frame's shadow around the page (GNOME's): the window that much larger.
        let pad = gpui_kit::component::window_paddings(window);
        let (frame_w, frame_h) = (pad.left + pad.right, pad.top + pad.bottom);
        let own = frame_w > px(0.);
        if wanted > px(0.) && (off(wanted + frame_h, viewport.height) || (cfg!(windows) || own) && off(self.width + frame_w, viewport.width)) {
            // Windows: centred over its owner again, after this frame (its messages come at once); elsewhere the system places it.
            #[cfg(windows)]
            if let Some(hwnd) = hwnd(window) {
                let device = |side: Pixels| (f32::from(side) * window.scale_factor()).round() as i32;
                let (width, height) = (device(self.width), device(wanted));
                cx.foreground_executor().spawn(async move { sc_core::system::theme::fit_dialog(hwnd, width, height) }).detach();
            }
            #[cfg(not(windows))]
            window.resize(size(if own { self.width + frame_w } else { viewport.width }, wanted + frame_h));
        }
        let theme = cx.theme().clone();
        let height = self.height.clone();
        // Its page answering as an alert (`responses`, GNOME's): its owner dimmed as for an alert.
        if !self.veiled.get() && cx.try_global::<crate::dialogs::Responding>().is_some_and(|marked| marked.0.contains(&self.own)) {
            self.veiled.set(true);
            veil(self.asked, cx);
            let asked = self.asked;
            cx.defer(move |cx| {
                let _ = asked.update(cx, |_, window, _| window.refresh());
            });
        }
        // GNOME: no title bar, as libadwaita's dialogs; another desktop drawing none on Wayland: an untitled one.
        #[cfg(not(windows))]
        let header = match sc_core::settings::menu_button() {
            true => None,
            false => crate::header::title_bar(SharedString::default(), window, cx),
        };
        #[cfg(windows)]
        let header = Some(caption(self.title.clone(), false, px(8.), window, cx));
        // GNOME: a question as libadwaita's alert, its title bold and centred, its buttons a row at the foot.
        let alert = self.alert && adwaita();
        // The title heads the content, as the kit's in-window dialogs.
        let heading = self.heading.then(|| match alert {
            true => div().px_6().pt_6().text_xl().font_weight(FontWeight::BOLD).text_center().child(self.title.clone()),
            false => div().px_4().pt_4().text_xl().font_weight(FontWeight::SEMIBOLD).child(self.title.clone()),
        });
        let alert_buttons = self.ok.as_ref().filter(|_| alert).and_then(|ok| Some((ok, ok.label.clone()?))).map(|(ok, label)| {
            let color = if ok.danger { theme.danger } else { theme.primary };
            let main = alert_button("dialog-ok", label.into(), color, cx).on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx)));
            let cancel = ok.cancel.then(|| {
                alert_button("dialog-cancel", t!("remote.dialog.cancel").into(), theme.foreground, cx).on_click(|_, window, _| window.remove_window())
            });
            alert_row(crate::dialogs::ordered(std::iter::once((Role::Main, main)).chain(cancel.map(|cancel| (Role::Dismiss, cancel)))), cx)
        });
        let buttons = self.ok.as_ref().filter(|_| !alert).and_then(|ok| Some((ok, ok.label.clone()?))).map(|(ok, label)| {
            let button = Button::new("dialog-ok").label(label).min_w(px(80.));
            let button = if ok.danger { button.danger() } else { button.primary() };
            let cancel = ok.cancel.then(|| {
                Button::new("dialog-cancel")
                    .outline()
                    .label(t!("remote.dialog.cancel"))
                    .min_w(px(80.))
                    .on_click(|_, window, _| window.remove_window())
            });
            let ok = button.on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx)));
            crate::dialogs::responses(cx, std::iter::once((Role::Main, ok)).chain(cancel.map(|cancel| (Role::Dismiss, cancel))))
        });
        let page = v_flex()
            .relative()
            .w_full()
            .children(header)
            .children(heading)
            .map(|page| match alert {
                true => page.child(v_flex().px_6().pt_2().text_center().child(self.content.clone())).children(alert_buttons),
                false => page.child(v_flex().p_4().child(self.content.clone()).children(buttons)),
            })
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
        // Its own frame (GNOME's) without a title bar: the page round as the window.
        let (top_left, top_right) = gpui_kit::component::top_corners(window, cx);
        let (bottom_left, bottom_right) = gpui_kit::component::bottom_corners(window, cx);
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .rounded_tl(top_left)
            .rounded_tr(top_right)
            .rounded_bl(bottom_left)
            .rounded_br(bottom_right)
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.confirm(window, cx)))
            .on_action(|_: &Dismiss, window, _| window.remove_window())
            .child(page)
    }
}

/// Windows: the caption as Windows 11 draws it, a dialog's without maximize (as
/// KWin's): the app's icon, the title, minimize (maximize) and close in 46 px cells;
/// Windows moves the window by it and takes the buttons' clicks.
/// `lead`: the room before the icon, 8 px at the window's edge.
pub(crate) fn caption(title: SharedString, maximize: bool, lead: Pixels, window: &Window, cx: &App) -> Div {
    use gpui_kit::component::{AlertIcon, NativeControls};
    let theme = cx.theme();
    let frame = cx.try_global::<NativeControls>().and_then(|native| native.dialog_frame);
    let state = usize::from(!window.is_window_active());
    let (height, fill, text) = frame.map_or((px(32.), theme.background, theme.foreground), |frame| (frame.height, frame.fill[state], frame.text[state]));
    let shade = hsla(0., 0., if theme.mode.is_dark() { 1. } else { 0. }, if theme.mode.is_dark() { 0.06 } else { 0.04 });
    // Segoe MDL2 Assets' ChromeMinimize, ChromeMaximize, ChromeRestore and ChromeClose, Windows' own.
    let glyph = |code: &'static str| div().font_family("Segoe MDL2 Assets").text_size(px(10.)).child(code);
    let cell = |id: &'static str, area: WindowControlArea| div().id(id).w(px(46.)).h_full().flex().items_center().justify_center().window_control_area(area);
    // Siblings, not inside the drag area: Windows takes the first area under the pointer.
    let drag = h_flex().id("caption").flex_1().min_w_0().h_full().pl(lead).gap(px(8.)).window_control_area(WindowControlArea::Drag);
    // The window's menu: right-clicked, or its icon clicked, as Windows' own caption.
    #[cfg(windows)]
    let drag = drag.on_mouse_down(MouseButton::Right, |_, window, cx| {
        cx.stop_propagation();
        system_menu(window, false, cx);
    });
    let icon = cx.try_global::<AlertIcon>().map(|icon| {
        let icon = div().id("caption-icon").flex_none().child(img(icon.0.clone()).size(px(16.)));
        #[cfg(windows)]
        let icon = icon.on_mouse_down(MouseButton::Left, |_, window, cx| {
            cx.stop_propagation();
            system_menu(window, true, cx);
        });
        icon
    });
    // Windows' own close under the pointer: white on e81123.
    let close = cell("caption-close", WindowControlArea::Close).hover(|style| style.bg(rgb(0xe81123)).text_color(white()));
    h_flex()
        .h(height)
        .flex_none()
        .bg(fill)
        .text_color(text)
        .child(drag.children(icon).child(div().text_size(px(12.)).truncate().child(title)))
        .child(cell("caption-minimize", WindowControlArea::Min).hover(move |style| style.bg(shade)).child(glyph("\u{E921}")))
        .when(maximize, |bar| {
            let glyph = glyph(if window.is_maximized() { "\u{E923}" } else { "\u{E922}" });
            bar.child(cell("caption-maximize", WindowControlArea::Max).hover(move |style| style.bg(shade)).child(glyph))
        })
        .child(close.child(glyph("\u{E8BB}")))
}

/// Windows: the window's menu, at the pointer or under the caption's left end (as
/// Alt+Space), once this event is over: its own loop would run inside GPUI's.
#[cfg(windows)]
fn system_menu(window: &Window, below_caption: bool, cx: &mut App) {
    if let Some(hwnd) = hwnd(window) {
        let below = below_caption.then(|| (32. * window.scale_factor()).round() as i32);
        cx.spawn(async move |_| sc_core::system::theme::system_menu(hwnd, below)).detach();
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
    /// Losing work (Don't Save): outlined, on GNOME red as libadwaita's destructive.
    Discard,
}

/// `text` (under a small `note`) and a button for each choice, in a dialog
/// window over `parent`; a button closes it, then runs on `parent`. The
/// choices as Windows orders them, the last one leaving things as they were
/// (Cancel).
pub(crate) fn choose(title: String, note: Option<String>, text: String, choices: Vec<Choice>, width: Pixels, parent: &mut Window, cx: &mut App) {
    open_alert(title, size(width, px(200.)), parent, cx, move |main, _, cx| (cx.new(|_| Choose { note, text, choices: choices.into(), main }), None));
}

struct Choose {
    note: Option<String>,
    text: String,
    choices: Rc<[Choice]>,
    main: Main,
}

impl Render for Choose {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let main = self.main;
        let primary = self.choices.iter().position(|choice| matches!(choice.style, Style::Primary));
        let last = self.choices.len().saturating_sub(1);
        let buttons = self.choices.iter().enumerate().map(|(ix, choice)| {
            let run = choice.run.clone();
            let button = Button::new(("choice", ix)).label(choice.label.clone()).on_click(move |_, window, cx| {
                window.remove_window();
                let run = run.clone();
                main.run(cx, move |window, cx| run(window, cx));
            });
            let role = match ix {
                _ if Some(ix) == primary => Role::Main,
                _ if ix == last => Role::Dismiss,
                _ => Role::Other,
            };
            let button = match choice.style {
                Style::Primary => button.primary(),
                Style::Danger => button.danger(),
                Style::Outline | Style::Discard => button.outline(),
            };
            (role, button)
        });
        // GNOME: libadwaita's alert, the buttons a row out to the dialog's edges.
        if adwaita() {
            let theme = cx.theme();
            let row = self.choices.iter().enumerate().map(|(ix, choice)| {
                let run = choice.run.clone();
                let color = match choice.style {
                    Style::Primary => theme.primary,
                    Style::Danger | Style::Discard => theme.danger,
                    Style::Outline => theme.foreground,
                };
                let button = alert_button(("choice", ix), choice.label.clone().into(), color, cx).on_click(move |_, window, cx| {
                    window.remove_window();
                    let run = run.clone();
                    main.run(cx, move |window, cx| run(window, cx));
                });
                let role = match ix {
                    _ if Some(ix) == primary => Role::Main,
                    _ if ix == last => Role::Dismiss,
                    _ => Role::Other,
                };
                (role, button)
            });
            // libadwaita's order: Cancel first, the main one last.
            let mut row: Vec<_> = row.collect();
            row.sort_by_key(|(role, _)| match role {
                Role::Dismiss => 0,
                Role::Main => 2,
                _ => 1,
            });
            let row = alert_row(row.into_iter().map(|(_, button)| button).collect(), cx).mx(px(-24.));
            return v_flex()
                .gap_3()
                .text_sm()
                .children(self.note.clone().map(|note| div().text_xs().child(note)))
                .child(div().whitespace_normal().child(self.text.clone()))
                .child(row);
        }
        v_flex()
            .gap_3()
            .text_sm()
            .children(self.note.clone().map(|note| div().text_xs().child(note)))
            .child(div().whitespace_normal().child(self.text.clone()))
            .child(crate::dialogs::footer(cx).flex_wrap().children(crate::dialogs::ordered(buttons)))
    }
}
