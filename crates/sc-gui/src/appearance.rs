use gpui_kit::component::scroll::ScrollbarMode;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use std::sync::Arc;

#[cfg(target_os = "macos")]
use gpui_kit::component::MacControls;
use gpui_kit::component::{DialogFrame, NativeControls, Theme as KitTheme, ThemeMode, WindowClose};
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use gpui_kit::component::{ActiveTheme, StateImages, ToolkitControls};
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use gpui_kit::{Global, RenderImage, Window};
use gpui_kit::{App, Hsla, hsla, px, rgb};
use sc_core::settings::FontChoice;
use sc_core::system::theme::{ControlColors, ControlStyle, Palette};

/// The fixed-width font chosen in Settings > Font, for `apply`: the
/// viewer's, the editor's and comparison's. The desktop's when none.
pub struct FixedFont(pub Option<FontChoice>);

impl gpui_kit::Global for FixedFont {}

/// GPUI panics laying out text in a family the system lacks: one chosen on
/// another machine, or since uninstalled, is not used.
pub fn installed(family: &str, cx: &App) -> bool {
    cx.text_system().all_font_names().iter().any(|name| name == family)
}

#[derive(Clone, Copy)]
pub struct PanelColors {
    /// The active panel's cursor: its fill and its dotted frame.
    pub cursor: Hsla,
    pub cursor_frame: Hsla,
    /// While another window is in front.
    pub cursor_inactive: Hsla,
    /// macOS: the cursor's names, on the accent.
    pub cursor_text: Option<Hsla>,
    pub sidebar_selection: Hsla,
    pub alternate_row: Hsla,
    pub header_border: Hsla,
    pub active_path: Hsla,
    pub inactive_path: Hsla,
    /// The menu bar.
    pub bar: Hsla,
    /// The sidebar, under its tint.
    pub window: Hsla,
    /// Marked files' names.
    pub marked: Hsla,
}

/// The system's look on GPUI Kit's colours, and the panels'.
pub fn apply(palette: &Palette, cx: &mut App) -> PanelColors {
    let color = |value: u32| Hsla::from(rgb(value));
    let mode = match palette.dark {
        true => ThemeMode::Dark,
        false => ThemeMode::Light,
    };
    let list = palette.list();
    let mut menu = palette.menu();
    // Windows 11's menus: the item under the pointer a faint fill, unframed (see `MenuItemElement`).
    match sc_core::system::theme::control_style() {
        Some(ControlStyle::Fluent) => (menu.popup, menu.hover) = if palette.dark { (0x2c2c2c, 0x383838) } else { (0xf9f9f9, 0xf0f0f0) },
        // libadwaita's popovers (popover_bg_color) and its rows' hover, a tenth of the text.
        Some(ControlStyle::Adwaita) => {
            let popover = sc_core::system::theme::adwaita_color(palette.dark, "popover_bg_color");
            menu.popup = popover.unwrap_or(if palette.dark { 0x36363a } else { 0xffffff });
            menu.hover = sc_core::system::theme::mix(menu.popup, palette.text, 0.1);
        }
        _ => {}
    }
    KitTheme::change(mode, None, cx);
    let font = palette.font.clone().filter(|font| installed(font, cx));
    let fixed = match cx.try_global::<FixedFont>().and_then(|fixed| fixed.0.clone()) {
        Some(font) => Some((font.family, font.size)),
        None => sc_core::system::theme::desktop_fixed_font(),
    };
    let fixed = fixed.filter(|(family, _)| installed(family, cx));
    let scripts = sc_core::system::theme::script_fonts(sc_core::l10n::language());
    KitTheme::update(cx, |kit| {
        // Fonts for the scripts the UI font lacks: else GPUI may take one that only half has them (Hangul as loose letters).
        kit.font_fallbacks = (!scripts.is_empty()).then(|| gpui_kit::FontFallbacks::from_fonts(scripts));
        if let Some(font) = font {
            kit.font_family = font.into();
        }
        if let Some((family, points)) = fixed {
            kit.mono_font_family = family.into();
            kit.mono_font_size = px(sc_core::system::theme::font_pixels(points));
        }
        // The app writes in `text_sm`, 0.875 of the base: that is the desktop's size.
        if let Some(size) = palette.font_size {
            kit.font_size = px(size / 0.875);
        }
        kit.scrollbar_mode = ScrollbarMode::Always;
        // macOS: as wide as AppKit's, its knob as AppKit draws it.
        #[cfg(target_os = "macos")]
        {
            kit.scrollbar_size = mac_scrollbar();
        }
        // A focused field's border takes the accent, as Breeze's; the ring
        // around it was cut off by the dialogs.
        kit.focus_ring = false;
        let kit = &mut kit.colors;
        kit.background = color(palette.view);
        kit.foreground = color(palette.text);
        kit.border = color(palette.border);
        // Dialogs' main buttons, the active tab's line.
        kit.primary = color(palette.selection);
        kit.primary_foreground = color(palette.selection_text);
        // Quiet fills, such as group titles.
        kit.muted = color(list.header);
        kit.muted_foreground = color(palette.dim);
        // Menus and popups; `ring` frames the highlighted item, and focus.
        kit.popover = color(menu.popup);
        kit.popover_foreground = color(palette.text);
        kit.accent = color(menu.hover);
        kit.accent_foreground = color(palette.text);
        kit.ring = color(menu.hover_frame);
        kit.list_active = color(palette.selection);
        // Selected text as the panels' cursor row: the editor's and the fields'.
        kit.selection = color(if cfg!(target_os = "macos") { palette.selection } else { list.selected_row });
        // Column titles and the button bar.
        kit.list_head = color(list.header);
        kit.tab_bar = color(palette.window);
        kit.tab_active = color(palette.view);
        kit.tab_foreground = color(palette.dim);
        kit.tab_active_foreground = color(palette.text);
    });
    // The desktop style's controls (KDE's Breeze, Windows 11's), in its colours.
    if let Some(style) = sc_core::system::theme::control_style() {
        cx.set_global(native_controls(style, &palette.controls(style), dialog_frame(style, palette), palette.dark));
        // Windows' and GNOME's corners, and their fields' frame, a button's
        // (GNOME's unframed: its fill).
        let radius = match style {
            ControlStyle::Breeze => None,
            ControlStyle::Fluent => Some(px(4.)),
            ControlStyle::Adwaita => Some(px(9.)),
        };
        if let Some(radius) = radius {
            let frame = color(palette.controls(style).button[0][1]);
            KitTheme::update(cx, |kit| {
                kit.radius = radius;
                kit.colors.input = frame;
            });
        }
    }
    // The toolkit's drawings are in the desktop's colours: not for the other look.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    match cx.try_global::<ToolkitDrawn>().filter(|drawn| drawn.dark == palette.dark) {
        Some(drawn) => cx.set_global(drawn.controls.clone()),
        None if cx.has_global::<ToolkitControls>() => drop(cx.remove_global::<ToolkitControls>()),
        None => {}
    }
    #[cfg(target_os = "macos")]
    cx.set_global(mac_controls(palette));
    // macOS's cursor is the accent, filled, as Finder's; elsewhere a tint, framed.
    let filled = cfg!(target_os = "macos");
    PanelColors {
        cursor: color(if filled { palette.selection } else { list.selected_row }),
        cursor_frame: if filled { gpui_kit::transparent_black() } else { color(palette.text) },
        // AppKit's unemphasized selection.
        cursor_inactive: match (filled, palette.dark) {
            (false, _) => color(list.selected_row),
            (true, true) => color(0x464646),
            (true, false) => color(0xdcdcdc),
        },
        cursor_text: filled.then(|| color(palette.selection_text)),
        // AppKit's sidebar selection: faint white or black over the blur.
        sidebar_selection: match (filled, palette.dark) {
            // libadwaita's navigation sidebar: a tenth of the text, as Builder's and Files'.
            (false, _) if sc_core::system::theme::control_style() == Some(ControlStyle::Adwaita) => color(palette.text).opacity(0.1),
            (false, _) => color(list.selected_row),
            (true, true) => hsla(0., 0., 1., 0.12),
            (true, false) => hsla(0., 0., 0., 0.1),
        },
        alternate_row: color(list.alternate_row),
        header_border: color(list.header_border),
        active_path: color(list.active_path),
        inactive_path: color(list.inactive_path),
        bar: color(palette.bar),
        // GNOME's: libadwaita's sidebar colour, as Files' (#ebebed, dark #2e2e32), its bars'.
        window: color(match sc_core::system::theme::control_style() {
            Some(ControlStyle::Adwaita) => palette.bar,
            _ => palette.window,
        }),
        marked: color(palette.marked),
    }
}

/// macOS: the controls as AppKit draws them, measured on macOS 26: fills of
/// the text's colour, white in dark mode, black in light.
#[cfg(target_os = "macos")]
fn mac_controls(palette: &Palette) -> MacControls {
    let dark = palette.dark;
    let ink = |alpha: f32| hsla(0., 0., if dark { 1. } else { 0. }, alpha);
    let accent = mac_color(dark, objc2_app_kit::NSColor::controlAccentColor).unwrap_or(0x007aff);
    MacControls {
        accent: Hsla::from(rgb(accent)),
        on_accent: hsla(0., 0., 1., 1.),
        button: ink(0.075),
        button_pressed: ink(0.16),
        button_disabled: ink(0.035),
        well: ink(0.1),
        groove: ink(0.07),
        groove_rim: ink(0.12),
        knob: if dark { Hsla::from(rgb(0xdfdfdf)) } else { hsla(0., 0., 1., 1.) },
        group: ink(0.03),
        field_frame: ink(0.1),
        text: ink(0.85),
        disabled_text: ink(0.25),
        window_frame: ink(if dark { 0.18 } else { 0.12 }),
        alert: Hsla::from(rgb(if dark { 0x1b1b1b } else { 0xf5f5f5 })),
        alert_rim: hsla(0., 0., 1., if dark { 0.3 } else { 1. }),
        alert_edge: hsla(0., 0., 0., if dark { 0.9 } else { 0.3 }),
        destructive: Hsla::from(rgb(if dark { 0x552627 } else { 0xfac8c8 })),
        destructive_text: Hsla::from(rgb(if dark { 0xef1e14 } else { 0xff393c })),
        tab_track: ink(if dark { 0.06 } else { 0.1 }),
        tab: if dark { ink(0.19) } else { Hsla::from(rgb(0xf8f8f8)) },
        tab_rim: hsla(0., 0., 1., if dark { 0.2 } else { 1. }),
        tooltip: Hsla::from(rgb(if dark { 0x282828 } else { 0xe9e9e9 })),
        tooltip_frame: hsla(0., 0., 0., if dark { 0.75 } else { 0.2 }),
    }
}

/// A dialog's title bar as the desktop's windows': KWin's decoration (Breeze,
/// Klassy, another) in the scheme's header colours with the app's icon, GNOME's
/// header bar, Windows 11's caption.
fn dialog_frame(style: ControlStyle, palette: &Palette) -> DialogFrame {
    use sc_core::system::theme::{Decoration, Outline};
    let color = |value: u32| Hsla::from(rgb(value));
    let text = [color(palette.text), color(palette.dim)];
    let decoration = (style == ControlStyle::Breeze).then(sc_core::system::theme::kwin_decoration).flatten();
    let outline = match decoration {
        Some(Decoration::Klassy(Outline::Accent)) => Some(color(palette.selection)),
        Some(Decoration::Klassy(Outline::Custom(rgb, opacity))) => Some(color(rgb).opacity(f32::from(opacity) / 100.)),
        _ => None,
    };
    let (height, fill, close) = match style {
        ControlStyle::Breeze => {
            let close = match decoration {
                Some(Decoration::Klassy(_)) => WindowClose::Klassy,
                Some(Decoration::Other) => WindowClose::Plain,
                _ => WindowClose::Breeze,
            };
            (30., [palette.bar, palette.window], close)
        }
        ControlStyle::Adwaita => {
            let header = sc_core::system::theme::header_bar();
            let fill = |focused| header.map_or(palette.bar, |header| header.color(palette.dark, focused));
            (header.map_or(47., |header| header.height), [fill(true), fill(false)], WindowClose::Adwaita)
        }
        ControlStyle::Fluent => {
            let caption = if palette.dark { 0x202020 } else { 0xffffff };
            (32., [caption, caption], WindowClose::Fluent)
        }
    };
    DialogFrame { height: px(height), fill: fill.map(color), text, close, icon: style == ControlStyle::Breeze, outline }
}

/// The controls in `colors`, at the style's sizes.
fn native_controls(style: ControlStyle, colors: &ControlColors, dialog_frame: DialogFrame, dark: bool) -> NativeControls {
    let color = |value: u32| Hsla::from(rgb(value));
    let pair = |[fill, frame]: [u32; 2]| (color(fill), color(frame));
    // The sizes: a check box, its corners and frame, a radio's dot; a slider's
    // groove, a progress bar's groove and fill; the rim of the handle's middle.
    let (check, radius, frame, dot, groove, progress, rim) = match style {
        ControlStyle::Breeze => (16., 4., 1., 6., 6., [6., 6.], 1.),
        // WinUI's.
        ControlStyle::Fluent => (20., 4., 1., 12., 4., [1., 3.], 4.),
        // libadwaita's: 14 px and 3 px of padding.
        ControlStyle::Adwaita => (20., 6., 2., 8., 6., [8., 8.], 0.),
    };
    NativeControls {
        check_size: px(check),
        check_radius: px(radius),
        check_border: px(frame),
        radio_dot: px(dot),
        check_frame: color(colors.check_frame),
        check_fill: color(colors.check_fill),
        checked_frame: color(colors.checked_frame),
        checked_fill: color(colors.checked_fill),
        mark: color(colors.mark),
        groove: color(colors.groove),
        fill: color(colors.fill),
        slider_groove: px(groove),
        progress: progress.map(px),
        handle: color(colors.handle),
        handle_middle: color(colors.handle_middle),
        handle_rim: px(rim),
        handle_frame: colors.handle_frame.map(color),
        button: colors.button.map(pair),
        default_button: colors.default_button.map(pair),
        button_text: color(colors.button_text),
        default_button_text: color(colors.default_button_text),
        button_edge: colors.button_edge.map(|edges| edges.map(color)),
        tabs: colors.tabs.map(|tabs| tabs.map(color)),
        field_line: colors.field_line.map(color),
        field_fill: colors.field_fill.map(color),
        combo_button: style != ControlStyle::Breeze,
        bold: style == ControlStyle::Adwaita,
        // WinUI's CardBackgroundFillColorDefault and CardStrokeColorDefault.
        card: (style == ControlStyle::Fluent).then(|| match dark {
            true => (hsla(0., 0., 1., 0.05), hsla(0., 0., 0., 0.1)),
            false => (hsla(0., 0., 1., 0.7), hsla(0., 0., 0., 0.06)),
        }),
        command_bar: (style == ControlStyle::Fluent).then(|| hsla(0., 0., 0., if dark { 0.15 } else { 0.03 })),
        dialog_frame: Some(dialog_frame),
        popover_menus: style == ControlStyle::Adwaita,
        // libadwaita's tab: selected a tenth of the text, hovered 7%.
        tab_pills: (style == ControlStyle::Adwaita).then(|| (color(colors.button_text).opacity(0.1), color(colors.button_text).opacity(0.07))),
    }
}

/// macOS: the selected rows' colour, the user's accent, as Finder has it.
#[cfg(target_os = "macos")]
pub fn mac_selection(dark: bool) -> Option<u32> {
    mac_color(dark, objc2_app_kit::NSColor::selectedContentBackgroundColor)
}

/// macOS: AppKit's own parts (menus, pop-ups, the window's buttons) in the
/// app's light or dark when Settings fixes one, else the system's.
#[cfg(target_os = "macos")]
pub fn mac_force(choice: sc_core::system::theme::Appearance) {
    use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication};
    use sc_core::system::theme::Appearance;
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let name = match choice {
        Appearance::System => None,
        Appearance::Light => Some(unsafe { NSAppearanceNameAqua }),
        Appearance::Dark => Some(unsafe { NSAppearanceNameDarkAqua }),
    };
    let appearance = name.and_then(NSAppearance::appearanceNamed);
    NSApplication::sharedApplication(mtm).setAppearance(appearance.as_deref());
}

/// A system colour in light or dark mode.
#[cfg(target_os = "macos")]
fn mac_color(dark: bool, color: fn() -> objc2::rc::Retained<objc2_app_kit::NSColor>) -> Option<u32> {
    use std::cell::Cell;

    use block2::StackBlock;
    use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSColorSpace};
    let name = unsafe { if dark { NSAppearanceNameDarkAqua } else { NSAppearanceNameAqua } };
    let rgb = Cell::new(None);
    let resolve = StackBlock::new(|| {
        let color = color().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace());
        rgb.set(color.map(|color| [color.redComponent(), color.greenComponent(), color.blueComponent()]));
    });
    NSAppearance::appearanceNamed(name)?.performAsCurrentDrawingAppearance(&resolve);
    let [r, g, b] = rgb.get()?.map(|channel| (channel * 255.).round() as u32);
    Some(r << 16 | g << 8 | b)
}

/// The controls as the desktop's toolkit drew them, and whether in dark colours.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
struct ToolkitDrawn {
    dark: bool,
    controls: ToolkitControls,
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
impl Global for ToolkitDrawn {}

/// The controls as the desktop's toolkit draws them (KDE's Qt style, the
/// GTK 3 theme of Xfce, MATE and Cinnamon; `sc_core::system::toolkit_style`),
/// once drawn; until then, and without it, the app's own.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub fn draw_toolkit_controls(window: &Window, cx: &mut App) {
    if sc_core::system::toolkit_style::helper().is_none() {
        return;
    }
    let scale = window.scale_factor();
    // Buttons are 2 rem high.
    let height = (f32::from(cx.theme().font_size) * 2.).round() as u32;
    cx.spawn(async move |cx| {
        let drawn = cx.background_executor().spawn(async move { sc_core::system::toolkit_style::draw(scale, height) }).await;
        let controls = match drawn {
            Ok(mut drawn) => toolkit_controls(&mut drawn, scale).map(|controls| (drawn.dark, controls)),
            Err(err) => {
                eprintln!("{err}");
                None
            }
        };
        let Some((drawn_dark, controls)) = controls else {
            return;
        };
        cx.update(|cx| {
            // The look now, not at the start: a window starts light until the
            // desktop says it is dark, which may come while this draws.
            if drawn_dark == cx.theme().mode.is_dark() {
                cx.set_global(controls.clone());
                cx.refresh_windows();
            }
            cx.set_global(ToolkitDrawn { dark: drawn_dark, controls });
        });
    })
    .detach();
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn toolkit_controls(drawn: &mut sc_core::system::toolkit_style::Drawn, scale: f32) -> Option<ToolkitControls> {
    let sc_core::system::toolkit_style::Drawn { images, colors, sizes, .. } = drawn;
    let mut image = |name: &str| -> Option<Arc<RenderImage>> {
        let picture = images.remove(name)?;
        // GPUI takes images as BGRA.
        let mut bgra = picture.rgba;
        bgra.chunks_exact_mut(4).for_each(|pixel| pixel.swap(0, 2));
        let image = image::RgbaImage::from_raw(picture.width, picture.height, bgra)?;
        Some(Arc::new(RenderImage::new([image::Frame::new(image)])))
    };
    let mut states = |name: &str| -> Option<StateImages> {
        let [normal, hovered, pressed, disabled] = ["normal", "hovered", "pressed", "disabled"].map(|state| image(&name.replace("{}", state)));
        Some(StateImages([normal?, hovered?, pressed?, disabled?]))
    };
    let mut button = |kind: &str| -> Option<[StateImages; 3]> {
        let [left, middle, right] = ["left", "middle", "right"].map(|part| states(&format!("button.{kind}.{{}}.{part}")));
        Some([left?, middle?, right?])
    };
    let button = [button("plain")?, button("default")?];
    let checkbox = [states("checkbox.off.{}")?, states("checkbox.on.{}")?];
    let tab_close = states("tabclose.{}");
    let tabs = toolkit_tabs(&mut image, colors, sizes);
    let menus = toolkit_menus(&mut image, colors, sizes);
    Some(ToolkitControls { scale, checkbox, button, tab_close, tabs, menus })
}

/// GTK's menus, where sc-gtkstyle drew them.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn toolkit_menus(
    image: &mut impl FnMut(&str) -> Option<Arc<RenderImage>>,
    colors: &std::collections::HashMap<String, u32>,
    sizes: &std::collections::HashMap<String, f32>,
) -> Option<gpui_kit::component::ToolkitMenus> {
    let mut parts = |name: &str| -> Option<[Arc<RenderImage>; 3]> {
        let [left, middle, right] = ["left", "middle", "right"].map(|part| image(&format!("{name}.{part}")));
        Some([left?, middle?, right?])
    };
    let color = |name: &str| colors.get(name).map(|color| Hsla::from(rgb(*color)));
    let size = |name: &str| sizes.get(name).map(|size| px(*size));
    Some(gpui_kit::component::ToolkitMenus {
        open: parts("menubar.open")?,
        open_text: color("menubar.open")?,
        row: parts("menu.row")?,
        row_text: color("menu.row")?,
        row_height: size("menu.row.height")?,
        row_inset: size("menu.row.inset")?,
        fill: color("menu")?,
        border: color("menu.border")?,
        padding: size("menu.padding")?,
        radius: size("menu.radius")?,
        context_radius: size("menu.context.radius")?,
    })
}

/// GTK's notebook tabs, where sc-gtkstyle drew them.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn toolkit_tabs(
    image: &mut impl FnMut(&str) -> Option<Arc<RenderImage>>,
    colors: &std::collections::HashMap<String, u32>,
    sizes: &std::collections::HashMap<String, f32>,
) -> Option<gpui_kit::component::ToolkitTabs> {
    let mut tab = |state: &str| -> Option<[Arc<RenderImage>; 3]> {
        let [left, middle, right] = ["left", "middle", "right"].map(|part| image(&format!("tab.{state}.{part}")));
        Some([left?, middle?, right?])
    };
    let tab = [tab("normal")?, tab("hovered")?, tab("selected")?];
    let text = ["normal", "hovered", "selected"].map(|state| colors.get(&format!("tab.{state}")).map(|color| Hsla::from(rgb(*color))));
    Some(gpui_kit::component::ToolkitTabs {
        tab,
        bar: image("tabbar")?,
        height: px(*sizes.get("tab.height")?),
        inset: px(*sizes.get("tab.inset")?),
        text: [text[0]?, text[1]?, text[2]?],
    })
}

/// macOS's look, whose tab bars are Finder's.
pub fn mac_tabs(cx: &App) -> Option<gpui_kit::component::MacControls> {
    cx.try_global::<gpui_kit::component::MacControls>().copied().filter(|_| !cx.has_global::<NativeControls>())
}

/// A tab bar ending in `new`, its + button: on macOS a circle beside the bar, as Finder's.
pub fn tab_bar_with_new(bar: gpui_kit::component::tab::TabBar, new: gpui_kit::component::button::Button, cx: &App) -> gpui_kit::AnyElement {
    use gpui_kit::component::button::{ButtonCustomVariant, ButtonVariants};
    use gpui_kit::component::{Sizable, h_flex};
    use gpui_kit::{IntoElement, ParentElement, Styled, div};
    let Some(mac) = mac_tabs(cx) else {
        return bar.suffix(new.ghost().small()).into_any_element();
    };
    let style = ButtonCustomVariant::new(cx).color(mac.tab_track).hover(mac.group).active(mac.button_pressed).foreground(mac.text);
    let circle = div()
        .flex_none()
        .size(px(28.))
        .mr(px(4.))
        .rounded_full()
        .border(px(0.5))
        .border_color(mac.tab_rim)
        .child(new.custom(style).small().size_full().rounded(px(14.)));
    h_flex().gap(px(4.)).child(bar.flex_1().min_w_0()).child(circle).into_any_element()
}

/// macOS: a tool window's title bar buttons, as AppKit's panels have them:
/// close only (Find Files).
#[cfg(target_os = "macos")]
pub fn close_button_only(window: &gpui_kit::Window) {
    use objc2_app_kit::{NSView, NSWindowButton};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(RawWindowHandle::AppKit(handle)) = HasWindowHandle::window_handle(window).map(|handle| handle.as_raw()) else {
        return;
    };
    // SAFETY: the window's own view, alive while the window is, on the main thread.
    let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
    let Some(ns_window) = view.window() else {
        return;
    };
    for which in [NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton] {
        if let Some(button) = ns_window.standardWindowButton(which) {
            button.setHidden(true);
        }
    }
}

/// macOS: AppKit's always-shown (legacy) scroller, as the app's scrollbars
/// are: its width and its knob's, read from AppKit (17, 11 and 3 on macOS 26).
#[cfg(target_os = "macos")]
fn mac_scrollbar() -> Option<gpui_kit::component::ScrollbarSize> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSControlSize, NSScroller, NSScrollerPart, NSScrollerStyle};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    let mtm = MainThreadMarker::new()?;
    let width = NSScroller::scrollerWidthForControlSize_scrollerStyle(NSControlSize::Regular, NSScrollerStyle::Legacy, mtm);
    let scroller = NSScroller::initWithFrame(mtm.alloc(), NSRect::new(NSPoint::new(0., 0.), NSSize::new(width, 300.)));
    scroller.setScrollerStyle(NSScrollerStyle::Legacy);
    scroller.setKnobProportion(0.3);
    scroller.setEnabled(true);
    let knob = scroller.rectForPart(NSScrollerPart::Knob);
    (knob.size.width > 0.).then(|| gpui_kit::component::ScrollbarSize {
        track: px(width as f32),
        thumb: px(knob.size.width as f32),
        inset: px(knob.origin.x as f32),
    })
}
