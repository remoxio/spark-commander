//! The dialogs over the main window.

pub mod about;
pub mod activity_dialog;
pub mod associations_dialog;
pub mod attributes_dialog;
pub mod bookmarks_dialog;
pub mod connections_dialog;
pub mod dialog_window;
pub mod download_dialog;
pub mod find_dialog;
pub mod keys_dialog;
pub mod name_dialog;
pub mod pack_dialog;
pub mod passwords_dialog;
pub mod properties_dialog;
pub mod rename_dialog;
pub mod settings_dialog;
pub mod tools_dialog;

use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme, NativeControls, WindowClose, WindowCornerRadius, h_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{AnyElement, AnyWindowHandle, App, Div, FontWeight, Global, IntoElement, ParentElement, Stateful, Styled, canvas, div, px};
use sc_core::system::theme::{ButtonOrder, button_order};

/// A dialog button's part, for the desktop's order of them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// What Enter does: OK, Save, Start.
    Main,
    Other,
    /// What leaves it as it was: Cancel, Close, No.
    Dismiss,
    Apply,
}

/// A dialog's buttons, written as Windows orders them, in this desktop's
/// order: KDE's Apply before Cancel; macOS's and GTK's Apply first, the others
/// the other way round, Cancel, the main one last.
pub(crate) fn ordered<T>(buttons: impl IntoIterator<Item = (Role, T)>) -> Vec<T> {
    arrange(button_order(), buttons.into_iter().collect())
}

fn arrange<T>(order: ButtonOrder, mut buttons: Vec<(Role, T)>) -> Vec<T> {
    let at = |buttons: &[(Role, T)], role| buttons.iter().position(|(is, _)| *is == role);
    match order {
        ButtonOrder::Windows => {}
        ButtonOrder::Kde => {
            if let (Some(apply), Some(dismiss)) = (at(&buttons, Role::Apply), at(&buttons, Role::Dismiss))
                && apply > dismiss
            {
                let apply = buttons.remove(apply);
                buttons.insert(dismiss, apply);
            }
        }
        ButtonOrder::MainLast => {
            buttons.reverse();
            buttons.sort_by_key(|(role, _)| match role {
                Role::Apply => 0,
                Role::Other => 1,
                Role::Dismiss => 2,
                Role::Main => 3,
            });
        }
    }
    buttons.into_iter().map(|(_, button)| button).collect()
}

/// OK and Cancel in the desktop's order.
pub(crate) fn ok_cancel<T>(ok: T, cancel: T) -> Vec<T> {
    ordered([(Role::Main, ok), (Role::Dismiss, cancel)])
}

/// A dialog's answers: Cancel, the main action (OK, Copy, Save), or Close alone.
/// On GNOME as libadwaita's alert: a row out to the dialog's edges under a line,
/// equal parts, flat and bold, Cancel first and the main one last, its label in the
/// accent, a dangerous one's red; the dialog then dims the window it was opened
/// over, as an alert (`Responding`). Elsewhere the footer, in this desktop's order.
pub(crate) fn responses(cx: &App, buttons: impl IntoIterator<Item = (Role, Button)>) -> AnyElement {
    let mut buttons: Vec<(Role, Button)> = buttons.into_iter().collect();
    if sc_core::system::theme::control_style() != Some(sc_core::system::theme::ControlStyle::Adwaita) {
        return footer(cx).children(ordered(buttons)).into_any_element();
    }
    buttons.sort_by_key(|(role, _)| match role {
        Role::Dismiss => 0,
        Role::Main => 2,
        _ => 1,
    });
    let theme = cx.theme();
    let (line, hover) = (theme.border, theme.foreground.opacity(0.05));
    let corner = cx.try_global::<WindowCornerRadius>().map_or(px(0.), |radius| radius.0);
    let last = buttons.len().saturating_sub(1);
    let row = buttons.into_iter().enumerate().map(|(ix, (_, button))| {
        let color = match button.variant() {
            ButtonVariant::Primary => theme.primary,
            ButtonVariant::Danger => theme.danger,
            _ => theme.foreground,
        };
        let look = ButtonCustomVariant::new(cx).color(theme.transparent).foreground(color).hover(hover).active(hover.opacity(0.1));
        button
            .with_variant(ButtonVariant::Custom(look))
            .flex_1()
            .h(px(46.))
            .rounded(px(0.))
            .font_weight(FontWeight::BOLD)
            .when(ix == 0, |button| button.rounded_bl(corner))
            .when(ix == last, |button| button.rounded_br(corner))
            .when(ix > 0, |button| button.border_l_1().border_color(line))
    });
    // Drawn, it marks its dialog window as answering, which then dims its owner.
    let mark = canvas(
        |_, window, cx| {
            let handle = window.window_handle();
            let marked = &mut cx.default_global::<Responding>().0;
            if !marked.contains(&handle) {
                marked.push(handle);
                window.refresh();
            }
        },
        |_, _, _, _| {},
    );
    // Out to the dialog's edges, over its page's 16 px padding.
    h_flex().relative().mt_4().mx(px(-16.)).mb(px(-16.)).border_t_1().border_color(line).children(row).child(mark.absolute().size_0()).into_any_element()
}

/// The dialog windows showing `responses` (some may be closed since).
#[derive(Default)]
pub(crate) struct Responding(pub(crate) Vec<AnyWindowHandle>);

impl Global for Responding {}

/// A dialog's button row; Windows' in a band across its foot, as WinUI's ContentDialog.
pub(crate) fn footer(cx: &App) -> Div {
    let native = cx.try_global::<NativeControls>();
    let band = native.and_then(|native| native.command_bar).filter(|_| {
        native.and_then(|native| native.dialog_frame).is_some_and(|frame| frame.close == WindowClose::Fluent)
    });
    // Out to the dialog's edges, over its 24 px padding (the kit's, a content dialog's).
    h_flex().gap_2().justify_end().when_some(band, |row, fill| {
        row.mx(px(-24.)).mb(px(-24.)).mt_3().p(px(24.)).bg(fill).border_t_1().border_color(cx.theme().border).rounded_b(px(7.))
    })
}

/// A dialog's scrolling page or list, its scrollbar at the window's right edge (over the 16 px padding), not afloat in the padding.
pub(crate) fn edge_scrolled(area: Stateful<Div>) -> Div {
    let width = gpui_kit::component::scroll::Scrollbar::width();
    let edge = dialog_window::separate();
    // The margin on a wrapper: the scroll area keeps its element's margin inside, beside the scrollbar.
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .when(edge, |el| el.mr(px(-16.)))
        .child(area.pr(if edge { width + px(16.) } else { width }).overflow_y_scrollbar())
}

/// A list in its frame, the scrollbar inside it beside the rows, as GTK's and Qt's lists; filling its column.
pub(crate) fn framed_list(list: Stateful<Div>, cx: &App) -> Div {
    let theme = cx.theme();
    let width = gpui_kit::component::scroll::Scrollbar::width();
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .child(list.flex_1().min_h_0().pr(width).overflow_y_scrollbar())
}

#[cfg(test)]
mod tests {
    use super::{ButtonOrder, Role, arrange};

    #[test]
    fn buttons_in_each_desktops_order() {
        let written = || vec![(Role::Main, "OK"), (Role::Other, "Don't save"), (Role::Dismiss, "Cancel"), (Role::Apply, "Apply")];
        assert_eq!(arrange(ButtonOrder::Windows, written()), ["OK", "Don't save", "Cancel", "Apply"]);
        assert_eq!(arrange(ButtonOrder::Kde, written()), ["OK", "Don't save", "Apply", "Cancel"]);
        assert_eq!(arrange(ButtonOrder::MainLast, written()), ["Apply", "Don't save", "Cancel", "OK"]);
        let choices = vec![(Role::Main, "Overwrite"), (Role::Other, "Skip"), (Role::Other, "Rename"), (Role::Dismiss, "Cancel")];
        assert_eq!(arrange(ButtonOrder::MainLast, choices), ["Rename", "Skip", "Cancel", "Overwrite"]);
    }
}
