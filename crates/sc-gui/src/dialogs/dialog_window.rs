//! A dialog as a window of its own (Linux): the desktop draws its frame (KWin's
//! decoration on KDE) and keeps it centred over the main window, which takes
//! no input while it is open. As tall as its content; Escape closes it.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt, v_flex};
use gpui_kit::*;

/// Whether dialogs open as windows of their own: on Linux, where the desktop frames them.
pub(crate) fn separate() -> bool {
    cfg!(any(target_os = "linux", target_os = "freebsd"))
}

struct DialogWindow {
    title: SharedString,
    content: AnyView,
    /// The page's height last laid out, which the window takes.
    height: Rc<Cell<Pixels>>,
}

/// `build`'s view in a dialog window over `parent`, `size` as it is about: the
/// desktop centres it at that size, and it grows or shrinks to its content.
pub(crate) fn open<V: Render>(
    title: String,
    size: Size<Pixels>,
    parent: &mut Window,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) {
    let main = parent.bounds();
    // shortcut: the window opens at the guessed height and is centred at that; GPUI cannot open one hidden till it is laid out.
    let origin = main.center() - point(size.width / 2., size.height / 2.);
    let over = crate::Over::at(WindowBounds::Windowed(Bounds { origin, size }), parent, cx);
    let options = WindowOptions {
        kind: WindowKind::Dialog,
        is_resizable: false,
        is_minimizable: false,
        ..crate::over_main_window(title.clone(), over, cx)
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        let content = build(window, cx);
        cx.new(|_| DialogWindow { title: title.into(), content: content.into(), height: Rc::default() })
    });
    if let Err(err) = opened {
        parent.push_notification(Notification::error(err.to_string()), cx);
    }
}

impl Render for DialogWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let wanted = self.height.get();
        if wanted > px(0.) && (wanted - viewport.height).abs() > px(1.) {
            window.resize(size(viewport.width, wanted));
        }
        let theme = cx.theme().clone();
        let height = self.height.clone();
        // GNOME's on Wayland: the app draws the frame.
        let header = crate::header::title_bar(self.title.clone(), window, cx);
        let page = v_flex()
            .relative()
            .w_full()
            .children(header)
            .child(div().p_4().child(self.content.clone()))
            .child(
                canvas(
                    move |bounds, window, _| {
                        if height.replace(bounds.size.height) != bounds.size.height {
                            window.refresh();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            );
        div()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .capture_key_down(|event, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    window.remove_window();
                }
            })
            .child(page)
    }
}
