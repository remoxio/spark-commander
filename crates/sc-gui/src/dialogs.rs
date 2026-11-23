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
