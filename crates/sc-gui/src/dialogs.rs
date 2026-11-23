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

use gpui_kit::component::{ActiveTheme, NativeControls, WindowClose, h_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{App, Div, Styled, px};
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
