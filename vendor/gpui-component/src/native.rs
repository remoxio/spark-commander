// SparkCommander patch: this file is new.
use gpui::{Global, Hsla, Pixels};

/// Controls drawn as the desktop's own style draws them (KDE's Breeze,
/// Windows 11's): its colours and the sizes that differ; without it, GPUI
/// Kit's own look.
#[derive(Clone, Copy, Debug)]
pub struct NativeControls {
    /// Check boxes and radio buttons, a check box's corners, their frame's
    /// width (GNOME's is 2 px), and a radio button's dot.
    pub check_size: Pixels,
    pub check_radius: Pixels,
    pub check_border: Pixels,
    pub radio_dot: Pixels,
    /// Unchecked, their frame and fill; checked; the mark or dot on them.
    pub check_frame: Hsla,
    pub check_fill: Hsla,
    pub checked_frame: Hsla,
    pub checked_fill: Hsla,
    pub mark: Hsla,
    /// Sliders' and progress bars' groove, and what fills it.
    pub groove: Hsla,
    pub fill: Hsla,
    /// A slider's groove; a progress bar's groove and fill.
    pub slider_groove: Pixels,
    pub progress: [Pixels; 2],
    /// A slider's 20 px handle, its middle, and the rim around the middle.
    pub handle: Hsla,
    pub handle_middle: Hsla,
    pub handle_rim: Pixels,
    /// Windows': the handle's frame.
    pub handle_frame: Option<Hsla>,
    /// Push buttons' fill and frame, normal, hovered and pressed; the default
    /// button's (a dialog's OK); their text.
    pub button: [(Hsla, Hsla); 3],
    pub default_button: [(Hsla, Hsla); 3],
    pub button_text: Hsla,
    pub default_button_text: Hsla,
    /// Windows': the bottom edge of a button's frame, of the default button's.
    pub button_edge: Option<[Hsla; 2]>,
    /// Breeze's document-mode tabs: the selected one, the others, the selected one's line.
    pub tabs: Option<[Hsla; 3]>,
    /// Windows': a text field's bottom line; focused, the checked fill.
    pub field_line: Option<Hsla>,
    /// Text fields' fill, where the style has its own.
    pub field_fill: Option<Hsla>,
    /// Combo boxes drawn as push buttons (Windows', GNOME's); buttons' labels
    /// and the check mark bold (GNOME's).
    pub combo_button: bool,
    pub bold: bool,
    /// Windows': a group box as a card, its fill and rim, its title bold.
    pub card: Option<(Hsla, Hsla)>,
    /// Windows': a dialog's buttons in a band of this fill.
    pub command_bar: Option<Hsla>,
    /// A dialog's title bar as the desktop's windows have it.
    pub dialog_frame: Option<DialogFrame>,
}

/// The desktop's window title bar, for dialogs: its height, its fill and text
/// with the window focused and not, and its close button; the app's icon at
/// its left (`AlertIcon`), as KWin draws it; the window's outline, focused.
#[derive(Clone, Copy, Debug)]
pub struct DialogFrame {
    pub height: Pixels,
    pub fill: [Hsla; 2],
    pub text: [Hsla; 2],
    pub close: WindowClose,
    pub icon: bool,
    pub outline: Option<Hsla>,
}

/// A window's close button as the desktop draws it, and where the title is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowClose {
    /// KDE's Breeze: a cross, on a red circle under the pointer; the title centred.
    Breeze,
    /// KDE's Klassy: a cross, white on a red rounded cell of the title bar's height under the pointer; the title centred.
    Klassy,
    /// Another KDE decoration's: a cross on a faint cell under the pointer; the title centred.
    Plain,
    /// GNOME's: a cross on a faint circle; the title bold, centred.
    Adwaita,
    /// Windows 11's: a cross in a 46 px wide cell, red under the pointer; the title small, at the left.
    Fluent,
}

impl NativeControls {
    /// The frame of check boxes and radio buttons, as GPUI's border widths go.
    pub(crate) fn check_frame_width<E: gpui::Styled>(&self, element: E) -> E {
        match self.check_border >= gpui::px(2.) {
            true => element.border_2(),
            false => element.border_1(),
        }
    }
}

impl Global for NativeControls {}
