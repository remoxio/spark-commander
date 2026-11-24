//! The window's own title bar, GNOME's header bar: where the window draws its
//! frame (GNOME on Wayland always draws none for it; the ☰ button asks for
//! it), the title, the ☰ button holding the menus, and the window's buttons;
//! on macOS the toolbar's place in a taller title bar.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu};
use gpui_kit::component::{ActiveTheme, GlobalState, Icon, Sizable, TitleBar, WindowButtonPictures, WindowCornerRadius, h_flex, top_corners};
use gpui_kit::*;

use crate::Commander;
use sc_core::menu::Command;

impl Commander {
    /// Asks for our own title bar where the menus go behind the ☰ button.
    pub(crate) fn request_frame(&self, window: &mut Window) {
        let own = sc_core::settings::menu_button();
        window.request_decorations(if own { WindowDecorations::Client } else { WindowDecorations::Server });
        // GPUI keeps telling the window manager of our frame's shadow margin otherwise.
        if !own {
            window.set_client_inset(px(0.));
        }
    }

    /// Whether the menus are behind the ☰ button, not in a bar.
    pub(crate) fn menus_behind_button(&self, window: &Window) -> bool {
        sc_core::settings::menu_button() && own_frame(window)
    }

    /// The title bar, where the window draws its own frame; the toolbar taken
    /// into it, on its left, as GNOME's apps have their buttons.
    pub(crate) fn render_header(&self, toolbar: &mut Option<AnyElement>, window: &Window, cx: &mut App) -> Option<AnyElement> {
        if cfg!(target_os = "macos") {
            return Some(self.render_mac_header(toolbar, window, cx));
        }
        // Windows: drawn here, as WinUI's, so a dialog's backdrop dims it with the window.
        if cfg!(windows) {
            let toggle = (self.settings.sidebar_button == sc_core::settings::SidebarButton::TitleBar).then(|| self.windows_sidebar_toggle(cx));
            // Beside ☰ the icon as far from its lines as from the title.
            let lead = if toggle.is_some() { px(0.) } else { px(8.) };
            let caption = crate::dialogs::dialog_window::caption("Spark Commander".into(), true, lead, window, cx).flex_1().bg(self.colors.bar);
            return Some(h_flex().bg(self.colors.bar).children(toggle).child(caption).into_any_element());
        }
        // No title beside the toolbar, as GNOME's Files has none beside its buttons; without one, the title.
        let bar = title_bar(if toolbar.is_some() { "" } else { "Spark Commander" }, window, cx)?;
        let focus = self.focus.clone();
        let menu = Button::new("app-menu")
            .ghost()
            .small()
            .icon(Icon::new(Lucide::Menu))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
                // The menu bar's menus, as they are now; their commands go to the panels.
                let menus = GlobalState::global(cx).app_menus().to_vec();
                menus.into_iter().fold(menu.action_context(focus.clone()).mnemonics(true), |menu, top| {
                    let items = top.items;
                    menu.submenu(top.name, window, cx, move |menu, window, cx| {
                        menu.with_menu_items(items.clone(), window, cx)
                    })
                })
            });
        // A press on the button must not start moving the window.
        let menu = div().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(menu);
        let bar = bar
            .on_close_window(|_, _, cx| cx.quit())
            .child(
                h_flex()
                    .w_full()
                    .px_1()
                    .children(toolbar.take().map(|bar| h_flex().flex_1().min_w_0().child(bar)))
                    .children(self.menus_behind_button(window).then_some(menu)),
            );
        Some(bar.into_any_element())
    }

    /// macOS: the toolbar in the title bar, as Finder's: beside the sidebar,
    /// which holds the window's buttons and the sidebar's; hidden, the bar has them.
    fn render_mac_header(&self, toolbar: &mut Option<AnyElement>, window: &Window, cx: &mut App) -> AnyElement {
        let title = toolbar.is_none().then(|| {
            let title = div().font_weight(FontWeight::BOLD).text_color(cx.theme().muted_foreground).child("Spark Commander");
            div().absolute().inset_0().flex().items_center().justify_center().child(title)
        });
        let sidebar = self.sidebar.drawn();
        // Full screen hides the window's buttons.
        let lights = if window.is_fullscreen() { 8. } else { MAC_LIGHTS_END };
        let bar = TitleBar::new()
            .h(MAC_BAR)
            // The capsules 10 px under the window's top, as the window's buttons' middle, 6 above the panels.
            .pt(px(4.))
            .pl(px(if sidebar { 8. } else { lights }))
            .bg(self.sidebar.chrome(self.colors.window))
            .border_color(cx.theme().border)
            .children(title)
            // Together at the left: the bar spreads its children apart.
            .child(h_flex().flex_1().min_w_0().gap_2().children((!sidebar).then(|| mac_capsule(vec![self.sidebar_toggle()], cx))).children(toolbar.take()));
        // Right-clicked anywhere on it, as Finder's.
        match self.main_commander(cx) {
            Some(commander) => div().id("title-bar-area").child(bar).context_menu(crate::toolbar_menu(commander)).into_any_element(),
            None => bar.into_any_element(),
        }
    }

    /// The main window's commander, for its title bar's menus.
    fn main_commander(&self, cx: &App) -> Option<WeakEntity<Commander>> {
        cx.try_global::<crate::MainWindow>().map(|main| main.1.clone())
    }

    /// Windows: WinUI's navigation button before the app's icon; shows or hides the sidebar.
    fn windows_sidebar_toggle(&self, cx: &App) -> AnyElement {
        let focus = self.focus.clone();
        let shade = cx.theme().foreground.opacity(0.06);
        div()
            .id("caption-sidebar")
            .w(px(34.))
            .h(px(32.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .hover(move |style| style.bg(shade))
            // Segoe MDL2 Assets' GlobalNavButton.
            .child(div().font_family("Segoe MDL2 Assets").text_size(px(14.)).child("\u{E700}"))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                window.focus(&focus, cx);
                window.dispatch_action(Box::new(crate::menus::Run(Command::ToggleSidebar)), cx);
            })
            .into_any_element()
    }

    /// macOS: shows or hides the sidebar, first in the toolbar.
    pub(crate) fn sidebar_toggle(&self) -> AnyElement {
        let command = Command::ToggleSidebar;
        let [key, _] = self.settings.keys.keys_of(command);
        let name = sc_core::menu::name(command);
        let tip = match key.is_empty() {
            true => name.to_owned(),
            false => format!("{name} ({})", crate::dialogs::keys_dialog::shown(&key)),
        };
        let focus = self.focus.clone();
        let button = Button::new("sidebar-toggle").ghost().with_size(MAC_BUTTON).icon(Icon::new(Lucide::PanelLeft)).tooltip(tip).on_click(move |_, window, cx| {
            window.focus(&focus, cx);
            window.dispatch_action(Box::new(crate::menus::Run(command)), cx);
        });
        // A press on it must not start moving the window.
        div().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(button).into_any_element()
    }
}

/// macOS: the title bar's height with the toolbar in it, and where the window's
/// buttons sit (AppKit's are 14 px high), as in Finder, Notes and Xcode: inside
/// the sidebar, which is inset 8 px from the window's edges; then where they end.
pub(crate) const MAC_BAR: Pixels = px(46.);
pub(crate) const MAC_LIGHTS: Point<Pixels> = Point { x: px(19.), y: px(18.) };
pub(crate) const MAC_LIGHTS_END: f32 = 96.5;
/// macOS: the toolbar's buttons, their icons 18 px, as large as Notes' and Finder's.
pub(crate) const MAC_BUTTON: gpui_kit::component::Size = gpui_kit::component::Size::Size(px(24.));

/// macOS: a toolbar group's capsule, as AppKit's: fully round, a shade
/// lighter than the bar, a fine rim.
pub(crate) fn mac_capsule(buttons: Vec<AnyElement>, cx: &App) -> Div {
    let (fill, rim) = match cx.theme().mode.is_dark() {
        true => (hsla(0., 0., 1., 0.06), hsla(0., 0., 1., 0.14)),
        false => (hsla(0., 0., 1., 0.7), hsla(0., 0., 0., 0.1)),
    };
    h_flex().flex_none().h(px(30.)).px(px(2.)).rounded_full().bg(fill).border_1().border_color(rim).children(buttons)
}

/// A window's own title bar: Windows' caption as the main window's, else `title_bar`'s.
pub(crate) fn window_bar(title: impl Into<SharedString>, window: &Window, cx: &mut App) -> Option<AnyElement> {
    if cfg!(windows) {
        return Some(crate::dialogs::dialog_window::caption(title.into(), true, px(8.), window, cx).into_any_element());
    }
    title_bar(title, window, cx).map(IntoElement::into_any_element)
}

/// The title bar of a window that draws its own frame: any on GNOME on
/// Wayland, which draws none, and the main one for its ☰ button.
pub(crate) fn title_bar(title: impl Into<SharedString>, window: &Window, cx: &mut App) -> Option<TitleBar> {
    if !own_frame(window) {
        return None;
    }
    // The window buttons as GTK draws them, from the desktop's icon theme.
    if !cx.has_global::<WindowButtonPictures>()
        && let Some(pictures) = sc_core::system::theme::window_buttons()
    {
        let pictures = pictures.clone().map(|path| path.to_string_lossy().into_owned().into());
        cx.set_global(WindowButtonPictures(pictures));
    }
    let title = div().min_w_0().truncate().font_weight(FontWeight::BOLD).child(title.into());
    let title = div().absolute().inset_0().px(px(140.)).flex().items_center().justify_center().child(title);
    let mut bar = TitleBar::new().child(title);
    // As tall as GTK's, its top corners the window's.
    if let Some(header) = sc_core::system::theme::header_bar() {
        let (top_left, top_right) = top_corners(window, cx);
        let color = header.color(cx.theme().mode.is_dark(), window.is_window_active());
        bar = bar.h(px(header.height)).rounded_tl(top_left).rounded_tr(top_right).bg(rgb(color));
    }
    Some(bar)
}

/// The window's top corners as the GTK theme rounds them, before a window opens.
pub(crate) fn round_corners(cx: &mut App) {
    if let Some(header) = sc_core::system::theme::header_bar() {
        cx.set_global(WindowCornerRadius(px(header.radius)));
    }
}

fn own_frame(window: &Window) -> bool {
    matches!(window.window_decorations(), Decorations::Client { .. })
}
