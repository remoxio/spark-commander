use std::collections::HashSet;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{ActiveTheme, Disableable, MacControls, Sizable, StyledExt, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::addons::{Addon, Builtin, Presence};
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectState};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{Icon, IconName, IndexPath};
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use sc_core::dates::{self, Clock};
use sc_core::menu::{self, ToolbarItem};
use sc_core::settings::{AddonUpdates, FontChoice, PAGES, Settings, Blur, SidebarButton, SidebarLayout, SidebarOpacity, ToolbarStyle, TerminalOpacity, StartFont, StartMode};
use sc_core::system::theme::Appearance;
use sc_core::t;

use crate::Commander;
use crate::dialogs::Role;
use crate::menus::toolbar_icon;

const WIDTH: f32 = 720.;
const BODY_HEIGHT: f32 = 550.;

const START_MODES: [StartMode; 4] = [
    StartMode::Text,
    StartMode::Binary,
    StartMode::Hex,
    StartMode::Auto,
];
const START_FONTS: [StartFont; 4] = [
    StartFont::Ansi,
    StartFont::Ascii,
    StartFont::Variable,
    StartFont::Auto,
];

fn fluent() -> bool {
    sc_core::system::theme::control_style() == Some(sc_core::system::theme::ControlStyle::Fluent)
}

/// Linux's dialog windows: the page between lines, under its title and over the buttons (or the window's foot), the
/// scrollbar from one to the other at the window's edge, as KDE's System Settings; not macOS's, nor Windows' in-window.
fn lined(cx: &App) -> bool {
    crate::dialogs::dialog_window::separate() && !cx.has_global::<MacControls>()
}

/// GNOME's: each change applied at once, as its preferences, no OK, Cancel or Apply; a × to close.
fn instant() -> bool {
    crate::dialogs::dialog_window::separate() && sc_core::system::theme::control_style() == Some(sc_core::system::theme::ControlStyle::Adwaita)
}

/// The main window's font's sample, as Total Commander's.
const SAMPLE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz äöüéèêàñç 0123456789";
const SIZES: [&str; 15] = ["7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "18", "20", "22", "24", "28"];
/// Each page's icon in the rail, as `PAGES`.
const ICONS: [Lucide; 11] = [
    Lucide::PanelsTopLeft,
    Lucide::PanelTop,
    Lucide::Eye,
    Lucide::Type,
    Lucide::CalendarClock,
    Lucide::Languages,
    Lucide::FilePen,
    Lucide::SquareTerminal,
    Lucide::ListOrdered,
    Lucide::KeyRound,
    Lucide::Puzzle,
];

type Fonts = SelectState<SearchableVec<SharedString>>;
type Sizes = SelectState<Vec<SharedString>>;

/// Opens on `PAGES[page]`.
pub fn open(
    commander: WeakEntity<Commander>,
    settings: Settings,
    page: usize,
    window: &mut Window,
    cx: &mut App,
) {
    let build = move |owner: Option<crate::dialogs::dialog_window::Main>, window: &mut Window, cx: &mut App| {
    // At most `digits` digits; any text with none.
    let mut input = |value: String, digits: usize, cx: &mut App| {
        cx.new(|cx| {
            let state = InputState::new(window, cx).default_value(value);
            match digits {
                0 => state,
                _ => state.validate(move |text, _| text.len() <= digits && text.chars().all(|c| c.is_ascii_digit())),
            }
        })
    };
    let text_width = input(settings.viewer.text_width.to_string(), 4, cx);
    let binary_width = input(settings.viewer.binary_width.to_string(), 4, cx);
    let limit = input(settings.editor.limit_mb.to_string(), 4, cx);
    let external = input(settings.editor.external.clone(), 0, cx);
    let pattern = input(settings.date.custom.clone().unwrap_or_else(|| "yyyy-MM-dd".into()), 0, cx);
    let clear_after = input(settings.queue.clear_after_minutes.to_string(), 4, cx);
    let download_limit = input(settings.queue.download_limit_kb.to_string(), 7, cx);
    let slider = |value: u8, cx: &mut App| cx.new(|_| SliderState::new().min(0.).max(100.).step(5.).default_value(f32::from(value)));
    let opacity = slider(settings.sidebar_opacity.0, cx);
    let terminal_opacity = slider(settings.terminal_opacity.0, cx);
    let mut families: Vec<SharedString> = cx
        .text_system()
        .all_font_names()
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .map(SharedString::from)
        .collect();
    families.sort_by_key(|name| name.to_lowercase());
    families.dedup();
    let installed: HashSet<SharedString> = families.iter().cloned().collect();
    let main = Picker::new(families.clone(), &settings.font, system_font(&settings, cx), window, cx);
    // The fixed-width ones known: all of them until measured once (`fixed_width`).
    let fixed_families = FIXED_WIDTH.get().cloned().unwrap_or_else(|| families.clone());
    let fixed = Picker::new(fixed_families, &settings.fixed_font, system_fixed_font(&settings, cx), window, cx);
    let view = cx.new(|cx| {
        // Tried on the sidebar or the terminal as it moves; closed, the dialog puts back what was applied.
        let tried = |slider: &Entity<SliderState>, set: fn(&mut Settings, u8), window: &mut Window, cx: &mut Context<SettingsDialog>| {
            cx.subscribe_in(slider, window, move |this: &mut SettingsDialog, _, event: &SliderEvent, window, cx| {
                let SliderEvent::Change(value) = event else {
                    return;
                };
                set(&mut this.draft, value.end().round().clamp(0., 100.) as u8);
                this.try_backdrop(window, cx);
            })
        };
        let trying = [
            tried(&opacity, |settings, value| settings.sidebar_opacity = SidebarOpacity(value), window, cx),
            tried(&terminal_opacity, |settings, value| settings.terminal_opacity = TerminalOpacity(value), window, cx),
        ];
        // Each input's text into the draft, as typed.
        let bind = |cx: &mut Context<SettingsDialog>,
                    input: &Entity<InputState>,
                    set: fn(&mut Settings, &str)| {
            cx.subscribe(input, move |this, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    set(&mut this.draft, &input.read(cx).value());
                    cx.notify();
                }
            })
        };
        let pickers = [main.listen(Which::Main, cx), fixed.listen(Which::Fixed, cx)];
        // The list of fixed-width fonts, measured once in the background.
        let measuring = match FIXED_WIDTH.get() {
            Some(_) => Task::ready(()),
            None => {
                let text_system = cx.text_system().clone();
                cx.spawn_in(window, async move |this, cx| {
                    let fixed = cx
                        .background_executor()
                        .spawn(async move { families.into_iter().filter(|family| fixed_width(&text_system, family)).collect::<Vec<_>>() })
                        .await;
                    let fixed = FIXED_WIDTH.get_or_init(|| fixed).clone();
                    let _ = this.update_in(cx, |this, window, cx| this.fixed.set_families(fixed, &this.draft.fixed_font, window, cx));
                })
            }
        };
        SettingsDialog {
            _inputs: [
                bind(cx, &text_width, |s, text| {
                    if let Ok(width @ 1..) = text.parse() {
                        s.viewer.text_width = width;
                    }
                }),
                bind(cx, &binary_width, |s, text| {
                    if let Ok(width @ 1..) = text.parse() {
                        s.viewer.binary_width = width;
                    }
                }),
                bind(cx, &limit, |s, text| {
                    if let Ok(mb @ 1..) = text.parse() {
                        s.editor.limit_mb = mb;
                    }
                }),
                bind(cx, &external, |s, text| {
                    s.editor.external = text.trim().to_owned()
                }),
                // Typed into while "Custom" is chosen.
                bind(cx, &pattern, |s, text| {
                    if s.date.custom.is_some() {
                        s.date.custom = Some(text.to_owned());
                    }
                }),
                // Empty is 0.
                bind(cx, &clear_after, |s, text| s.queue.clear_after_minutes = text.parse().unwrap_or(0)),
                bind(cx, &download_limit, |s, text| s.queue.download_limit_kb = text.parse().unwrap_or(0)),
            ],
            _pickers: pickers.into_iter().flatten().collect(),
            _measuring: measuring,
            text_width,
            binary_width,
            limit,
            external,
            pattern,
            clear_after,
            download_limit,
            main,
            fixed,
            installed,
            opacity,
            terminal_opacity,
            shells: sc_core::system::shells::others(),
            system_language: sc_core::l10n::system_language(),
            passwords: (0, false, false),
            _opacity: trying,
            _addons: cx.observe(&crate::addons::entity(cx), |_, _, cx| cx.notify()),
            commander,
            saved: settings.clone(),
            draft: settings,
            page,
            owner,
        }
    });
    if PAGES[page] == "Add-ons" {
        crate::addons::shown(cx);
    }
    if PAGES[page] == "Passwords" {
        view.update(cx, |this, cx| this.read_passwords(cx));
    }
    // Closed however, the sidebar and the terminal are back as applied.
    view.update(cx, |_, cx| cx.on_release(|this, cx| this.untry_later(cx)).detach());
    view
    };
    if crate::dialogs::dialog_window::separate() {
        let size = size(px(WIDTH), px(BODY_HEIGHT + 90.));
        // GNOME: its title beside its × (`render`).
        if instant() {
            return crate::dialogs::dialog_window::open_headed(t!("settings.title"), size, window, cx, move |main, window, cx| (build(Some(main), window, cx), None));
        }
        return crate::dialogs::dialog_window::open(t!("settings.title"), size, window, cx, move |main, window, cx| (build(Some(main), window, cx), None));
    }
    let view = build(None, window, cx);
    window.open_dialog(cx, move |dialog, _, cx| {
        let closing = view.clone();
        dialog
            .title(t!("settings.title"))
            .width(px(WIDTH))
            .overlay_closable(false)
            .child(view.clone())
            .footer(buttons(&view, view.read(cx).draft != view.read(cx).saved))
            .on_close(move |_, window, cx| closing.update(cx, |this, cx| this.untry(window, cx)))
    });
}

/// The commands a toolbar button can be, by menu, to put at `at` (none: added
/// last); those on the toolbar already greyed.
fn choices(dialog: WeakEntity<SettingsDialog>, shown: Vec<ToolbarItem>, at: Option<usize>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
    move |menu, window, cx| {
        menu::groups().into_iter().fold(menu, |menu, (group, commands)| {
            let commands: Vec<_> = commands.into_iter().filter_map(|(label, command)| Some((label, command, toolbar_icon(command)?))).collect();
            if commands.is_empty() {
                return menu;
            }
            let (dialog, shown) = (dialog.clone(), shown.clone());
            menu.submenu(group, window, cx, move |menu, _, _| {
                commands.iter().fold(menu, |menu, (label, command, icon)| {
                    let (dialog, command, icon) = (dialog.clone(), *command, *icon);
                    let button = ToolbarItem::Button(command);
                    let item = PopupMenuItem::new(label.clone()).icon(Icon::new(icon)).disabled(shown.contains(&button)).on_click(move |_, _, cx| {
                        let _ = dialog.update(cx, |this, cx| {
                            let bar = &mut this.draft.toolbar.0;
                            match at {
                                Some(at) => bar[at] = button,
                                None => bar.push(button),
                            }
                            cx.notify();
                        });
                    });
                    menu.item(item)
                })
            })
        })
    }
}

/// A page's name, shown in the rail and over the page.
fn page_name(page: &str) -> String {
    match page {
        "Layout" => t!("settings.page.layout"),
        "Toolbar" => t!("settings.page.toolbar"),
        "Display" => t!("settings.page.display"),
        "Font" => t!("settings.page.font"),
        "Date" => t!("settings.page.date_time"),
        "Language" => t!("settings.page.language"),
        "Edit/View" => t!("settings.page.edit_view"),
        "Terminal" => t!("settings.page.terminal"),
        "Operations" => t!("settings.page.operations"),
        "Passwords" => t!("settings.page.passwords"),
        "Add-ons" => t!("settings.page.addons"),
        _ => page.to_owned(),
    }
}

/// The desktop's UI font and size in points; GPUI Kit's where it names none.
fn system_font(settings: &Settings, cx: &App) -> FontChoice {
    let theme = cx.theme();
    let palette = sc_core::system::theme::system(theme.mode.is_dark());
    // The theme has the system's font while no other is chosen.
    let (family, pixels) = match &settings.font {
        None => (theme.font_family.to_string(), f32::from(theme.font_size) * 0.875),
        Some(_) => (palette.font.clone().unwrap_or_else(|| ".SystemUIFont".into()), 14.),
    };
    FontChoice {
        family: palette.font.unwrap_or(family),
        size: sc_core::system::theme::font_points(palette.font_size.unwrap_or(pixels)).round(),
    }
}

/// The desktop's fixed-width font; GPUI Kit's where it names none.
fn system_fixed_font(settings: &Settings, cx: &App) -> FontChoice {
    let theme = cx.theme();
    // The theme has the system's while no other is chosen.
    let current = (theme.mono_font_family.to_string(), sc_core::system::theme::font_points(f32::from(theme.mono_font_size)));
    let (family, size) = match &settings.fixed_font {
        None => current,
        Some(_) => sc_core::system::theme::desktop_fixed_font().unwrap_or(current),
    };
    FontChoice { family, size: size.round() }
}

/// The fixed-width families among `families`, measured once.
static FIXED_WIDTH: std::sync::OnceLock<Vec<SharedString>> = std::sync::OnceLock::new();

/// A font whose i, W and m are as wide.
fn fixed_width(text_system: &TextSystem, family: &str) -> bool {
    let id = text_system.resolve_font(&font(family.to_owned()));
    let width = |c| text_system.advance(id, px(16.), c).ok().map(|size| size.width);
    match (width('i'), width('W'), width('m')) {
        (Some(i), Some(w), Some(m)) => (i - w).abs() < px(0.05) && (i - m).abs() < px(0.05),
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum Which {
    Main,
    Fixed,
}

/// A font's family and size, chosen or the system's.
struct Picker {
    fonts: Entity<Fonts>,
    sizes: Entity<Sizes>,
    system: FontChoice,
}

impl Picker {
    fn new(families: Vec<SharedString>, chosen: &Option<FontChoice>, system: FontChoice, window: &mut Window, cx: &mut App) -> Self {
        let shown = chosen.clone().unwrap_or_else(|| system.clone());
        let at = families.iter().position(|name| name.as_ref() == shown.family);
        let fonts = cx.new(|cx| SelectState::new(SearchableVec::new(families), at.map(IndexPath::new), window, cx).searchable(true));
        let size: SharedString = format!("{}", shown.size.round()).into();
        let list: Vec<SharedString> = SIZES.iter().map(|&size| size.into()).collect();
        let at = list.iter().position(|item| *item == size);
        let sizes = cx.new(|cx| SelectState::new(list, at.map(IndexPath::new), window, cx));
        Picker { fonts, sizes, system }
    }

    /// Choosing either sets the draft's font of `which`.
    fn listen(&self, which: Which, cx: &mut Context<SettingsDialog>) -> [Subscription; 2] {
        let family = cx.subscribe(&self.fonts, move |this: &mut SettingsDialog, _, event: &SelectEvent<SearchableVec<SharedString>>, cx| {
            let SelectEvent::Confirm(Some(family)) = event else { return };
            let size = this.chosen(which).map_or(this.picker(which).system.size, |font| font.size);
            *this.chosen_mut(which) = Some(FontChoice { family: family.to_string(), size });
            cx.notify();
        });
        let size = cx.subscribe(&self.sizes, move |this: &mut SettingsDialog, _, event: &SelectEvent<Vec<SharedString>>, cx| {
            let SelectEvent::Confirm(Some(size)) = event else { return };
            let Ok(size) = size.parse() else { return };
            let family = this.chosen(which).map_or_else(|| this.picker(which).system.family.clone(), |font| font.family.clone());
            *this.chosen_mut(which) = Some(FontChoice { family, size });
            cx.notify();
        });
        [family, size]
    }

    /// Another list of families, the shown one kept.
    fn set_families(&self, families: Vec<SharedString>, chosen: &Option<FontChoice>, window: &mut Window, cx: &mut App) {
        let shown: SharedString = chosen.as_ref().map_or_else(|| self.system.family.clone(), |font| font.family.clone()).into();
        self.fonts.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(families), window, cx);
            select.set_selected_value(&shown, window, cx);
        });
    }

    /// Back to the system's font.
    fn reset(&self, window: &mut Window, cx: &mut App) {
        let family: SharedString = self.system.family.clone().into();
        let size: SharedString = format!("{}", self.system.size).into();
        self.fonts.update(cx, |select, cx| select.set_selected_value(&family, window, cx));
        self.sizes.update(cx, |select, cx| select.set_selected_value(&size, window, cx));
    }
}

struct SettingsDialog {
    commander: WeakEntity<Commander>,
    /// In a window of its own: the main window, which it changes.
    owner: Option<crate::dialogs::dialog_window::Main>,
    /// As last applied.
    saved: Settings,
    draft: Settings,
    page: usize,
    text_width: Entity<InputState>,
    binary_width: Entity<InputState>,
    limit: Entity<InputState>,
    external: Entity<InputState>,
    pattern: Entity<InputState>,
    clear_after: Entity<InputState>,
    download_limit: Entity<InputState>,
    main: Picker,
    fixed: Picker,
    /// The fonts this computer has, for the samples.
    installed: HashSet<SharedString>,
    opacity: Entity<SliderState>,
    terminal_opacity: Entity<SliderState>,
    /// The shells installed besides the default one.
    shells: Vec<sc_core::system::shells::Shell>,
    /// The system's language, where the app speaks it.
    system_language: Option<String>,
    /// The saved passwords: how many, a master password set, locked here;
    /// read as the page is shown.
    passwords: (usize, bool, bool),
    _inputs: [Subscription; 7],
    _pickers: Vec<Subscription>,
    _measuring: Task<()>,
    _opacity: [Subscription; 2],
    _addons: Subscription,
}

impl SettingsDialog {
    fn chosen(&self, which: Which) -> Option<&FontChoice> {
        match which {
            Which::Main => self.draft.font.as_ref(),
            Which::Fixed => self.draft.fixed_font.as_ref(),
        }
    }

    fn chosen_mut(&mut self, which: Which) -> &mut Option<FontChoice> {
        match which {
            Which::Main => &mut self.draft.font,
            Which::Fixed => &mut self.draft.fixed_font,
        }
    }

    fn picker(&self, which: Which) -> &Picker {
        match which {
            Which::Main => &self.main,
            Which::Fixed => &self.fixed,
        }
    }

    /// The sidebar and the terminal as last applied, whatever was tried.
    fn untry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (look, commander) = (self.saved.clone(), self.commander.clone());
        crate::dialogs::dialog_window::answer(self.owner, window, cx, move |window, cx| {
            let _ = commander.update(cx, |commander, cx| commander.show_backdrop(&look, window, cx));
        });
    }

    /// As `untry`, as its window closes.
    fn untry_later(&self, cx: &mut App) {
        let (look, commander) = (self.saved.clone(), self.commander.clone());
        if let Some(main) = self.owner {
            main.run(cx, move |window, cx| {
                let _ = commander.update(cx, |commander, cx| commander.show_backdrop(&look, window, cx));
            });
        }
    }

    /// The draft's sidebar and terminal shown, before they are applied.
    fn try_backdrop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (look, commander) = (self.draft.clone(), self.commander.clone());
        crate::dialogs::dialog_window::answer(self.owner, window, cx, move |window, cx| {
            let _ = commander.update(cx, |commander, cx| commander.show_backdrop(&look, window, cx));
        });
        cx.notify();
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft == self.saved {
            return;
        }
        let (draft, commander) = (self.draft.clone(), self.commander.clone());
        crate::dialogs::dialog_window::answer(self.owner, window, cx, move |window, cx| {
            let _ = commander.update(cx, |commander, cx| commander.apply_settings(draft, window, cx));
        });
        self.saved = self.draft.clone();
        cx.notify();
    }

    /// The pages, a row each with its icon: a rail down the left.
    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        // macOS: a sidebar, its selection a rounded row of the accent.
        let mac = cx.try_global::<MacControls>().copied();
        // Windows 11: a navigation pane, unframed; the selection a faint row with the accent's pill.
        let fluent = fluent();
        // GNOME: libadwaita's navigation sidebar, the selection a tenth of the text, as Builder's preferences.
        let gnome = sc_core::system::theme::control_style() == Some(sc_core::system::theme::ControlStyle::Adwaita);
        let subtle = if theme.mode.is_dark() { hsla(0., 0., 1., 0.06) } else { hsla(0., 0., 0., 0.05) };
        let (pill, rail) = (theme.primary, theme.muted);
        v_flex()
            .id("settings-rail")
            // As wide as its longest page name: some languages' are long.
            .min_w(px(172.))
            .flex_none()
            .h_full()
            .gap(px(2.))
            .p(px(6.))
            .map(|el| match mac {
                Some(mac) => el.bg(mac.group).rounded(px(10.)),
                None if fluent => el.p_0(),
                None => el.bg(rail).rounded(px(8.)),
            })
            .children(PAGES.iter().enumerate().map(|(ix, &label)| {
                let selected = ix == self.page;
                h_flex()
                    .id(("settings-page", ix))
                    .relative()
                    .flex_none()
                    .h(px(if mac.is_some() { 28. } else { 34. }))
                    .px_2()
                    .gap_2()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .child(Icon::new(ICONS[ix].clone()).size_4())
                    .child(page_name(label))
                    .when(!selected, |el| el.hover(|el| el.bg(subtle)))
                    .when(selected, |el| match mac {
                        Some(mac) => el.bg(mac.accent).text_color(mac.on_accent),
                        None if fluent => el
                            .bg(subtle)
                            .child(div().absolute().left_0().top(px(9.)).w(px(3.)).h(px(16.)).rounded(px(2.)).bg(pill)),
                        None if gnome => el.bg(theme.foreground.opacity(0.1)),
                        None => el.bg(theme.primary).text_color(theme.primary_foreground),
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.page = ix;
                        if label == "Add-ons" {
                            crate::addons::shown(cx);
                        }
                        if label == "Passwords" {
                            this.read_passwords(cx);
                        }
                        cx.notify();
                    }))
            }))
    }

    /// Light or dark, where the desktop does not decide it alone.
    fn render_appearance(&self, cx: &mut Context<Self>) -> Option<GroupBox> {
        if sc_core::system::theme::desktop_dark().is_some() {
            return None;
        }
        let looks = [Appearance::System, Appearance::Light, Appearance::Dark];
        Some(GroupBox::new().outline().title(t!("settings.appearance.title")).child(
            RadioGroup::horizontal("appearance")
                .children([t!("appearance.system"), t!("appearance.light"), t!("appearance.dark")])
                .selected_index(looks.iter().position(|look| *look == self.draft.appearance))
                .on_click(cx.listener(move |this, at: &usize, _, cx| {
                    this.draft.appearance = looks[*at];
                    cx.notify();
                })),
        ))
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> GroupBox {
        GroupBox::new().outline().title(t!("settings.tabs.title")).child(
            Checkbox::new("tab-header-always")
                .label(t!("settings.tabs.header_always"))
                .checked(self.draft.tab_header_always.0)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.draft.tab_header_always = sc_core::settings::TabHeaderAlways(*checked);
                    cx.notify();
                })),
        )
    }

    fn render_files(&self, cx: &mut Context<Self>) -> Vec<GroupBox> {
        let folders = [t!("settings.sort_folders.by_name"), t!("settings.sort_folders.like_files")];
        vec![
            GroupBox::new().outline().title(t!("settings.files_shown.title")).child(
                h_flex()
                    .gap_6()
                    .child(
                        Checkbox::new("show-hidden")
                            .label(t!("settings.files_shown.hidden"))
                            .checked(self.draft.show_hidden)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.show_hidden = *checked;
                                cx.notify();
                            })),
                    )
                    .child(
                        // Linux and macOS have no system attribute.
                        Checkbox::new("show-system")
                            .label(t!("settings.files_shown.system"))
                            .checked(self.draft.show_system)
                            .disabled(!cfg!(windows))
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.show_system = *checked;
                                cx.notify();
                            })),
                    ),
            ),
            GroupBox::new().outline().title(t!("settings.sort_folders.title")).child(
                RadioGroup::horizontal("folders-sort")
                    .children(folders)
                    .selected_index(Some(usize::from(self.draft.folders_like_files)))
                    .on_click(cx.listener(|this, at: &usize, _, cx| {
                        this.draft.folders_like_files = *at == 1;
                        cx.notify();
                    })),
            ),
        ]
    }

    /// A font of `which`: chosen, or the desktop's; with its sample.
    fn render_font(&self, which: Which, cx: &mut Context<Self>) -> GroupBox {
        let picker = self.picker(which);
        let chosen = self.chosen(which);
        let font = chosen.cloned().unwrap_or_else(|| picker.system.clone());
        // Never a family this computer lacks: GPUI panics on it.
        let family = match self.installed.contains(font.family.as_str()) {
            true => font.family.clone(),
            false => picker.system.family.clone(),
        };
        let theme = cx.theme();
        let dim = theme.muted_foreground;
        let system = &picker.system;
        let state = match chosen {
            None => t!("settings.font.system"),
            Some(_) => t!("settings.font.chosen", family = system.family, size = system.size),
        };
        let (title, note, id) = match which {
            Which::Main => (t!("settings.font.main"), None, "font-system"),
            Which::Fixed => (t!("settings.font.fixed"), Some(t!("settings.font.fixed_note")), "fixed-system"),
        };
        GroupBox::new().outline().title(title).child(
            v_flex()
                .gap_3()
                .children(note.map(|note| div().text_color(dim).child(note)))
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().min_w_0().child(Select::new(&picker.fonts).menu_max_h(px(320.))))
                        .child(div().w(px(84.)).flex_none().child(Select::new(&picker.sizes)))
                        .child(
                            Button::new(id)
                                .outline()
                                .label(t!("settings.font.as_system"))
                                .disabled(chosen.is_none())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    *this.chosen_mut(which) = None;
                                    this.picker(which).reset(window, cx);
                                    cx.notify();
                                })),
                        ),
                )
                .child(div().text_color(dim).child(state))
                .child(
                    div()
                        .p_3()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.background)
                        .font_family(family)
                        .text_size(px(sc_core::system::theme::font_pixels(font.size)))
                        .child(SAMPLE),
                ),
        )
    }

    /// Dates and times as the system shows them, or a pattern and clock of one's own.
    fn render_date(&self, cx: &mut Context<Self>) -> Vec<GroupBox> {
        let now = std::time::SystemTime::now();
        let dim = cx.theme().muted_foreground;
        let custom = self.draft.date.custom.is_some();
        let pattern = self.pattern.clone();
        let clocks = [Clock::System, Clock::H24, Clock::H12];
        vec![
            GroupBox::new().outline().title(t!("settings.date.title")).child(
                v_flex()
                    .gap_2()
                    .child(
                        RadioGroup::vertical("date-kind")
                            .children([t!("settings.date.as_system", date = dates::system_date(now)), t!("settings.date.custom")])
                            .selected_index(Some(usize::from(custom)))
                            .on_click(cx.listener(move |this, at: &usize, _, cx| {
                                this.draft.date.custom = (*at == 1).then(|| pattern.read(cx).value().to_string());
                                cx.notify();
                            })),
                    )
                    .child(
                        v_flex()
                            .pl_6()
                            .gap_1()
                            .child(div().w(px(240.)).child(Input::new(&self.pattern).disabled(!custom)))
                            .child(div().text_color(dim).child(t!("settings.date.pattern_help"))),
                    ),
            ),
            GroupBox::new().outline().title(t!("settings.time.title")).child(
                RadioGroup::horizontal("clock")
                    .children([t!("settings.time.as_system"), t!("settings.time.h24"), t!("settings.time.h12")])
                    .selected_index(clocks.iter().position(|clock| *clock == self.draft.date.clock))
                    .on_click(cx.listener(move |this, at: &usize, _, cx| {
                        this.draft.date.clock = clocks[*at];
                        cx.notify();
                    })),
            ),
            GroupBox::new().outline().title(t!("settings.date.shown_as")).child(
                h_flex()
                    .gap_2()
                    .child(dates::preview(&self.draft.date, now))
                    .child(div().text_color(dim).child(t!("settings.date.shown_as_note"))),
            ),
        ]
    }

    /// The app's languages in a list, as Total Commander's: the system's
    /// first, then each in its own words and English.
    fn render_language(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let system = sc_core::l10n::languages().into_iter().find(|language| Some(&language.tag) == self.system_language.as_ref());
        let first = match system {
            Some(language) => t!("settings.language.system_named", language = language.native),
            None => t!("settings.language.system"),
        };
        let rows = std::iter::once((None, first)).chain(sc_core::l10n::languages().into_iter().map(|language| {
            let text = match language.native == language.name {
                true => language.native,
                false => format!("{} ({})", language.native, language.name),
            };
            (Some(language.tag), text)
        }));
        let list = rows.enumerate().map(|(ix, (tag, text))| {
            let chosen = self.draft.language == tag;
            div()
                .id(("language", ix))
                .px_2()
                .py_0p5()
                .when(chosen, |row| row.bg(theme.primary).text_color(theme.primary_foreground))
                .when(!chosen, |row| row.hover(|style| style.bg(theme.foreground.opacity(0.06))))
                .child(text)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.draft.language = tag.clone();
                    cx.notify();
                }))
        });
        crate::dialogs::framed_list(v_flex().id("languages").py_0p5().children(list), cx)
    }

    fn render_viewer(&self, cx: &mut Context<Self>) -> GroupBox {
        let viewer = &self.draft.viewer;
        let start = START_MODES.iter().position(|&mode| mode == viewer.start);
        let font = START_FONTS.iter().position(|&font| font == viewer.font);
        let width = |label: String, input: &Entity<InputState>| {
            h_flex()
                .gap_2()
                .child(label)
                .child(div().w(px(70.)).child(Input::new(input)))
        };
        let start = v_flex().gap_2().child(t!("settings.viewer.start.label")).child(
            RadioGroup::vertical("viewer-start")
                .children([t!("settings.viewer.start.text"), t!("settings.viewer.start.binary"), t!("settings.viewer.start.hex"), t!("settings.viewer.start.auto")])
                .selected_index(start)
                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                    this.draft.viewer.start = START_MODES[*ix];
                    cx.notify();
                })),
        );
        let font = v_flex().gap_2().child(t!("settings.viewer.font.label")).child(
            RadioGroup::vertical("viewer-font")
                .children([
                    t!("settings.viewer.font.ansi"),
                    t!("settings.viewer.font.ascii"),
                    t!("settings.viewer.font.variable"),
                    t!("settings.viewer.font.auto"),
                ])
                .selected_index(font)
                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                    this.draft.viewer.font = START_FONTS[*ix];
                    cx.notify();
                })),
        );
        GroupBox::new().outline().title(t!("settings.viewer.title")).child(
            v_flex()
                .gap_4()
                .child(
                    h_flex()
                        .items_start()
                        .gap_8()
                        .child(start)
                        .child(font)
                        .child(
                            v_flex()
                                .gap_2()
                                .child(t!("settings.viewer.show"))
                                .child(
                                    Checkbox::new("viewer-wrap")
                                        .label(t!("settings.viewer.wrap"))
                                        .checked(viewer.wrap)
                                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                            this.draft.viewer.wrap = *checked;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    // Beside the editor too.
                                    Checkbox::new("viewer-minimap")
                                        .label(t!("settings.viewer.minimap"))
                                        .checked(viewer.minimap)
                                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                            this.draft.viewer.minimap = *checked;
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .gap_6()
                        .child(t!("settings.viewer.width.label"))
                        .child(width(t!("settings.viewer.width.text"), &self.text_width))
                        .child(width(t!("settings.viewer.width.binary"), &self.binary_width)),
                ),
        )
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> GroupBox {
        let theme = cx.theme();
        let external = self.external.clone();
        let browse = Button::new("editor-browse")
            .outline()
            .label(t!("settings.editor.browse"))
            .on_click(cx.listener(move |_, _, window, cx| {
                let paths = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some(t!("settings.editor.choose").into()),
                });
                let external = external.clone();
                cx.spawn_in(window, async move |_, cx| {
                    let Ok(Ok(Some(paths))) = paths.await else {
                        return;
                    };
                    let program = paths[0].to_string_lossy().into_owned();
                    let program = match program.contains(' ') {
                        true => format!("\"{program}\""),
                        false => program,
                    };
                    let _ = cx.update(|window, cx| {
                        external.update(cx, |input, cx| input.set_value(program, window, cx))
                    });
                })
                .detach();
            }));
        GroupBox::new().outline().title(t!("settings.editor.title")).child(
            v_flex()
                .gap_3()
                .child(
                    h_flex()
                        .gap_2()
                        .child(t!("settings.editor.limit"))
                        .child(div().w(px(70.)).child(Input::new(&self.limit)))
                        .child(t!("settings.editor.limit_unit")),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(t!("settings.editor.external"))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().flex_1().child(Input::new(&self.external)))
                                .child(browse),
                        )
                        .child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child(t!("settings.editor.external_help")),
                        ),
                ),
        )
    }

    /// The operation queue: how long done operations stay, how fast links download.
    fn render_queue(&self, cx: &mut Context<Self>) -> GroupBox {
        let muted = cx.theme().muted_foreground;
        let row = |label: String, input: &Entity<InputState>, unit: String, about: String| {
            v_flex()
                .gap_1()
                .child(h_flex().gap_2().child(label).child(div().w(px(80.)).child(Input::new(input))).child(unit))
                .child(div().whitespace_normal().text_color(muted).child(about))
        };
        GroupBox::new().child(
            v_flex()
                .gap_4()
                .child(row(t!("settings.queue.clear_after"), &self.clear_after, t!("settings.queue.clear_after_unit"), t!("settings.queue.clear_after_help")))
                .child(row(t!("settings.queue.download_limit"), &self.download_limit, t!("settings.queue.download_limit_unit"), t!("settings.queue.download_limit_help"))),
        )
    }

    /// The toolbar's buttons in order: each replaced, moved or removed; more added.
    /// Its style over the list of buttons, what adds to them under it (`render_toolbar_adding`).
    fn render_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let shown = self.draft.toolbar.0.clone();
        let this = cx.weak_entity();
        let small = |id: (&'static str, usize), icon: Lucide| Button::new(id).ghost().small().icon(Icon::new(icon));
        let rows = shown.iter().enumerate().map(|(at, &item)| {
            let replace = Button::new(("toolbar-replace", at)).ghost().small().label(t!("settings.toolbar.replace")).dropdown_menu(choices(this.clone(), shown.clone(), Some(at)));
            let change = |change: fn(&mut Vec<ToolbarItem>, usize)| {
                let this = this.clone();
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    let _ = this.update(cx, |this, cx| {
                        change(&mut this.draft.toolbar.0, at);
                        cx.notify();
                    });
                }
            };
            let line = || div().flex_1().h(px(1.)).bg(cx.theme().border);
            let row = match item {
                ToolbarItem::Button(command) => h_flex()
                    .gap_1()
                    .children(toolbar_icon(command).map(|icon| Icon::new(icon).size_4().mr_1()))
                    .child(div().flex_1().child(menu::name(command)))
                    .child(replace),
                ToolbarItem::Separator => h_flex()
                    .gap_1()
                    .child(line())
                    .child(div().px_1().text_color(cx.theme().muted_foreground).child(t!("settings.toolbar.separator")))
                    .child(line()),
            };
            row
                .child(small(("toolbar-up", at), Lucide::ChevronUp).tooltip(t!("settings.toolbar.move_up")).disabled(at == 0).on_click(change(|bar, at| bar.swap(at, at - 1))))
                .child(small(("toolbar-down", at), Lucide::ChevronDown).tooltip(t!("settings.toolbar.move_down")).disabled(at + 1 == shown.len()).on_click(change(|bar, at| bar.swap(at, at + 1))))
                .child(small(("toolbar-remove", at), Lucide::X).tooltip(t!("settings.toolbar.remove")).on_click(change(|bar, at| {
                    bar.remove(at);
                })))
        });
        let rows: Vec<_> = rows.collect();
        let (styles, names): (Vec<ToolbarStyle>, Vec<String>) = crate::toolbar_styles().into_iter().unzip();
        // GNOME: none to choose, icons alone.
        let offered = !styles.is_empty();
        let style = RadioGroup::horizontal("toolbar-style")
            .children(names)
            .selected_index(styles.iter().position(|style| *style == self.draft.toolbar_style).or(Some(0)))
            .on_click(cx.listener(move |this, at: &usize, _, cx| {
                this.draft.toolbar_style = styles[*at];
                cx.notify();
            }));
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_3()
            // Its own height: the list takes the rest.
            .when(offered, |el| el.child(div().flex_none().child(style)))
            .child(crate::dialogs::framed_list(v_flex().id("toolbar-buttons").p_1().gap_0p5().children(rows.into_iter().map(|row| row.px_1())), cx))
            .child(self.render_toolbar_adding(cx))
    }

    /// Adding to the toolbar, or its default back: under the list, always in sight.
    fn render_toolbar_adding(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.draft.toolbar.0.clone();
        let this = cx.weak_entity();
        h_flex()
            .gap_2()
            .child(Button::new("toolbar-add").outline().label(t!("settings.toolbar.add")).dropdown_menu(choices(this.clone(), shown.clone(), None)))
            .child(Button::new("toolbar-add-separator").outline().label(t!("settings.toolbar.add_separator")).on_click(cx.listener(|this, _, _, cx| {
                this.draft.toolbar.0.push(ToolbarItem::Separator);
                cx.notify();
            })))
            .child(Button::new("toolbar-default").outline().label(t!("settings.toolbar.restore_default")).disabled(self.draft.toolbar == Default::default()).on_click(cx.listener(
                |this, _, _, cx| {
                    this.draft.toolbar = Default::default();
                    cx.notify();
                },
            )))
    }

    /// How many passwords are saved and whether a master password is set, at
    /// once; whether they are locked here, from the keyring, in the background.
    fn read_passwords(&mut self, cx: &mut Context<Self>) {
        use sc_core::remote::secrets;
        self.passwords = (secrets::saved(), secrets::has_master(), self.passwords.2);
        cx.spawn(async move |this, cx| {
            let locked = cx.background_spawn(async { secrets::locked() }).await;
            let _ = this.update(cx, |this, cx| {
                this.passwords.2 = locked;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The saved passwords: where they are, and the master password that
    /// takes them to another computer. Its buttons act at once, not by OK.
    fn render_passwords(&self, cx: &mut Context<Self>) -> GroupBox {
        use crate::dialogs::passwords_dialog::{Ask, ask};
        let (saved, master, locked) = self.passwords;
        let place = t!("settings.passwords.saved", count = saved);
        let state = match (master, locked) {
            (_, true) => t!("settings.passwords.locked"),
            (true, false) => t!("settings.passwords.master_set"),
            (false, false) => t!("settings.passwords.needs_master"),
        };
        let this = cx.weak_entity();
        let read_again = move |_: &mut Window, cx: &mut App| {
            let _ = this.update(cx, |this, cx| this.read_passwords(cx));
        };
        let mut buttons = h_flex().gap_2();
        if locked {
            let done = read_again.clone();
            buttons = buttons.child(Button::new("passwords-unlock").primary().label(t!("settings.passwords.unlock")).on_click(move |_, window, cx| ask(Ask::Unlock, window, cx, done.clone())));
        } else {
            let done = read_again.clone();
            let label = if master { t!("settings.passwords.change_master") } else { t!("settings.passwords.set_master") };
            buttons = buttons.child(Button::new("passwords-master").outline().label(label).on_click(move |_, window, cx| ask(Ask::Set, window, cx, done.clone())));
        }
        GroupBox::new().child(
            v_flex()
                .gap_3()
                .child(div().whitespace_normal().child(place))
                .child(div().whitespace_normal().text_color(cx.theme().muted_foreground).child(state))
                .child(buttons),
        )
    }

    /// The terminal's shell: the default one or another installed.
    fn render_shell(&self, cx: &mut Context<Self>) -> GroupBox {
        let default = t!("settings.shell.default", shell = sc_core::system::shells::default_name());
        let chosen = self.draft.terminal_shell.as_ref();
        let at = chosen.map_or(Some(0), |chosen| self.shells.iter().position(|shell| &shell.program == chosen).map(|ix| ix + 1));
        let programs: Vec<String> = self.shells.iter().map(|shell| shell.program.clone()).collect();
        GroupBox::new().outline().title(t!("settings.shell.title")).child(
            v_flex()
                .gap_2()
                .child(
                    RadioGroup::vertical("terminal-shell")
                        .child(default)
                        .children(self.shells.iter().map(|shell| shell.name.clone()))
                        .selected_index(at)
                        .on_click(cx.listener(move |this, at: &usize, _, cx| {
                            this.draft.terminal_shell = at.checked_sub(1).map(|ix| programs[ix].clone());
                            cx.notify();
                        })),
                )
                .child(div().text_color(cx.theme().muted_foreground).child(t!("settings.shell.note"))),
        )
    }

    /// The sidebar: its sections in a row or one under another; where the
    /// desktop can show through it, how much.
    fn render_sidebar(&self, cx: &mut Context<Self>) -> GroupBox {
        let layouts = [SidebarLayout::Horizontal, SidebarLayout::Vertical];
        let layout = v_flex().gap_2().child(t!("settings.sidebar.sections")).child(
            RadioGroup::vertical("sidebar-layout")
                .children([t!("settings.sidebar.horizontal"), t!("settings.sidebar.vertical")])
                .selected_index(layouts.iter().position(|layout| *layout == self.draft.sidebar_layout))
                .on_click(cx.listener(move |this, at: &usize, _, cx| {
                    this.draft.sidebar_layout = layouts[*at];
                    cx.notify();
                })),
        );
        let buttons = [SidebarButton::TitleBar, SidebarButton::Toolbar];
        let button = v_flex().gap_2().child(t!("settings.sidebar.button")).child(
            RadioGroup::vertical("sidebar-button")
                .children([t!("settings.sidebar.button_title_bar"), t!("settings.sidebar.button_toolbar")])
                .selected_index(buttons.iter().position(|button| *button == self.draft.sidebar_button))
                .on_click(cx.listener(move |this, at: &usize, _, cx| {
                    this.draft.sidebar_button = buttons[*at];
                    cx.notify();
                })),
        );
        let see_through = sc_core::system::theme::see_through();
        GroupBox::new().outline().title(t!("settings.backdrop.sidebar")).child(
            v_flex()
                .gap_4()
                .child(layout)
                // Windows: WinUI's navigation button in the title bar, or the toolbar's.
                .when(cfg!(windows), |el| el.child(button))
                .when(see_through, |el| el.child(self.render_backdrop(false, cx))),
        )
    }

    /// The sidebar's or the `terminal`'s opacity over the desktop, and its blur.
    fn render_backdrop(&self, terminal: bool, cx: &mut Context<Self>) -> Div {
        let (slider, opacity, blur) = match terminal {
            false => (&self.opacity, self.draft.sidebar_opacity.0, self.draft.sidebar_blur.0),
            true => (&self.terminal_opacity, self.draft.terminal_opacity.0, self.draft.terminal_blur.0),
        };
        let blurs = sc_core::system::theme::blurs();
        let shown = match opacity {
            100 => t!("settings.backdrop.solid"),
            0 => t!("settings.backdrop.clear"),
            _ => t!("settings.backdrop.opacity", opacity = opacity),
        };
        v_flex()
            .gap_2()
            .child(t!("settings.backdrop.cover"))
            .child(
                h_flex()
                    .gap_4()
                    .child(div().flex_1().child(Slider::new(slider)))
                    .child(div().w(px(150.)).child(shown)),
            )
            .child(
                Checkbox::new(if terminal { "terminal-blur" } else { "sidebar-blur" })
                    .label(t!("settings.backdrop.blur"))
                    .checked(blur && blurs)
                    .disabled(!blurs)
                    .on_click(cx.listener(move |this, checked: &bool, window, cx| {
                        match terminal {
                            true => this.draft.terminal_blur = Blur(*checked),
                            false => this.draft.sidebar_blur = Blur(*checked),
                        }
                        this.try_backdrop(window, cx);
                    })),
            )
            .when(!blurs, |this| this.child(div().text_color(cx.theme().muted_foreground).child(t!("settings.backdrop.no_blur"))))
    }

    /// Other projects' programs the app uses: the add-ons, each with its
    /// files, versions and download; then those that come with it; then the
    /// look-ups for updates.
    fn render_addons(&self, cx: &mut Context<Self>) -> Vec<GroupBox> {
        let state = crate::addons::entity(cx).read(cx);
        let dim = cx.theme().muted_foreground;
        let row = |label: String, value: AnyElement| {
            h_flex()
                .gap_2()
                .child(div().w(px(80.)).flex_none().text_color(dim).child(label))
                .child(div().flex_1().min_w_0().truncate().child(value))
        };
        let website = |id: &'static str, (shown, url): (&'static str, &'static str)| {
            let link = Button::new(id).link().label(shown).on_click(move |_, _, cx| cx.open_url(url));
            h_flex().child(link).into_any_element()
        };
        let mut groups = vec![GroupBox::new().child(div().text_color(dim).child(t!("settings.addons.intro")))];
        groups.extend(Addon::ALL.into_iter().map(|addon| {
            let offer = state.manifest.as_ref().and_then(|manifest| manifest.offer(addon));
            let offered = offer.map(|(release, download)| (release.version.clone(), download.size));
            let presence = state.presence.get(&addon).cloned().unwrap_or(Presence::Missing);
            let installed = match &presence {
                Presence::Ours(version) => version.clone(),
                Presence::Elsewhere(_) => t!("settings.addons.installed.builtin"),
                Presence::Missing => t!("settings.addons.installed.none"),
            };
            let online = match &offered {
                Some((version, size)) => format!("{version} ({})", crate::addons::megabytes(*size)),
                None if state.checking => t!("settings.addons.online.looking"),
                None if state.check_error.is_some() => t!("settings.addons.online.unknown"),
                None if state.manifest.is_some() => t!("settings.addons.online.none"),
                None => t!("settings.addons.online.not_yet"),
            };
            let progress = state.progress.get(&addon).copied();
            // Something to get: none here, or another version online.
            let newer = offered.as_ref().is_some_and(|(version, _)| match &presence {
                Presence::Ours(installed) => installed != version,
                _ => true,
            });
            let buttons = h_flex()
                .flex_none()
                .gap_2()
                .child(
                    Button::new(("addon-get", addon as usize))
                        .label(t!("settings.addons.download"))
                        .disabled(!newer || progress.is_some())
                        .on_click(move |_, _, cx| crate::addons::install(addon, cx)),
                )
                .when(matches!(presence, Presence::Ours(_)), |el| {
                    el.child(
                        Button::new(("addon-remove", addon as usize))
                            .label(t!("settings.addons.remove"))
                            .disabled(progress.is_some())
                            .on_click(move |_, _, cx| crate::addons::remove(addon, cx)),
                    )
                });
            let site = ["addon-site-0", "addon-site-1", "addon-site-2"][addon as usize];
            GroupBox::new().outline().title(addon.name()).child(
                v_flex()
                    .gap_2()
                    .child(div().text_color(dim).child(addon.purpose()))
                    .child(
                        h_flex()
                            .gap_4()
                            .items_end()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(row(t!("settings.addons.website"), website(site, addon.website())))
                                    .child(row(t!("settings.addons.file"), addon.files().join(", ").into_any_element()))
                                    .child(row(t!("settings.addons.installed.label"), installed.into_any_element()))
                                    .child(row(t!("settings.addons.online.label"), online.into_any_element())),
                            )
                            .child(buttons),
                    )
                    .when_some(progress, |el, (done, of)| {
                        el.child(t!("settings.addons.downloading", done = crate::addons::megabytes(done), size = crate::addons::megabytes(of)))
                    })
                    .when_some(state.failed.get(&addon), |el, why| {
                        el.child(div().text_color(cx.theme().danger).child(why.clone()))
                    }),
            )
        }));
        groups.extend(Builtin::ALL.into_iter().map(|builtin| {
            let site = ["builtin-site-0", "builtin-site-1"][builtin as usize];
            GroupBox::new().outline().title(builtin.name()).child(
                v_flex()
                    .gap_2()
                    .child(div().text_color(dim).child(builtin.purpose()))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(row(t!("settings.addons.website"), website(site, builtin.website())))
                            .child(row(t!("settings.addons.file"), builtin.file().into_any_element()))
                            .child(row(t!("settings.addons.version"), t!("settings.addons.builtin_version", version = builtin.version()).into_any_element())),
                    ),
            )
        }));
        let looked_up = match (state.checking, &state.check_error, &state.manifest) {
            (true, _, _) => t!("settings.addons.checking"),
            (false, Some(err), _) => t!("settings.addons.check_failed", err = err),
            (false, None, Some(_)) => t!("settings.addons.up_to_date"),
            (false, None, None) => String::new(),
        };
        let checking = state.checking;
        groups.push(
            GroupBox::new().outline().title(t!("settings.addons.offers")).child(
                v_flex()
                    .gap_2()
                    .child(
                        Checkbox::new("addon-never-offer")
                            .label(t!("settings.addons.never_offer"))
                            .checked(self.draft.never_offer_addons)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.never_offer_addons = *checked;
                                cx.notify();
                            })),
                    )
                    .child(
                        Checkbox::new("addon-updates")
                            .label(t!("settings.addons.check_daily"))
                            .checked(self.draft.addon_updates.0)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.addon_updates = AddonUpdates(*checked);
                                cx.notify();
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_4()
                            .child(
                                Button::new("addon-check")
                                    .label(t!("settings.addons.check_now"))
                                    .disabled(checking)
                                    .on_click(|_, _, cx| crate::addons::check(false, cx)),
                            )
                            .child(div().text_color(dim).child(looked_up)),
                    ),
            ),
        );
        groups
    }

    fn render_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = PAGES[self.page];
        let list = match title {
            "Toolbar" => Some(self.render_toolbar(cx)),
            "Language" => Some(self.render_language(cx)),
            _ => None,
        };
        let groups = match title {
            "Layout" => {
                let mut groups: Vec<GroupBox> = self.render_appearance(cx).into_iter().collect();
                groups.push(self.render_sidebar(cx));
                groups.push(self.render_tabs(cx));
                groups
            }
            "Display" => self.render_files(cx),
            "Font" => vec![self.render_font(Which::Main, cx), self.render_font(Which::Fixed, cx)],
            "Date" => self.render_date(cx),
            "Edit/View" => vec![self.render_viewer(cx), self.render_editor(cx)],
            "Terminal" => {
                let mut groups = vec![self.render_shell(cx)];
                if sc_core::system::theme::see_through() {
                    groups.push(GroupBox::new().outline().title(t!("settings.backdrop.transparency")).child(self.render_backdrop(true, cx)));
                }
                groups
            }
            "Operations" => vec![self.render_queue(cx)],
            "Passwords" => vec![self.render_passwords(cx)],
            "Add-ons" => self.render_addons(cx),
            _ => Vec::new(),
        };
        let lined = lined(cx);
        // A list fills the page, its scrollbar in its frame; groups scroll, the scrollbar at the window's edge from line to line.
        let body = match list {
            Some(list) => v_flex().flex_1().min_h_0().pt_3().pr(gpui_kit::component::scroll::Scrollbar::width()).child(list),
            None => crate::dialogs::edge_scrolled(v_flex().id("settings-page-body").flex_1().min_h_0().gap_3().pt_3().when(lined, |el| el.pb_4()).children(groups))
                // Down past the rail's foot to the line over the buttons, or to the window's.
                .when(lined, |el| el.mb(px(-16.))),
        };
        // The page's name as a pane's title, bold, unboxed; macOS's smaller.
        let mac = cx.has_global::<MacControls>();
        let fluent = fluent();
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(
                div()
                    .pt_1()
                    .pb_2()
                    .map(|el| match () {
                        _ if mac => el.text_base().font_semibold(),
                        _ if fluent => el.text_xl().font_semibold(),
                        _ => el.text_lg().font_semibold(),
                    })
                    .child(page_name(title)),
            )
            // Out to the window's right edge, as the scrollbar under it.
            .when(lined, |page| page.child(div().h(px(1.)).mr(px(-16.)).bg(cx.theme().border)))
            .child(body)
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // GNOME: a change applied as made; the title and a × over the page instead of the buttons.
        if instant() {
            if self.draft != self.saved {
                cx.defer_in(window, |this, window, cx| this.apply(window, cx));
            }
            let close = Button::new("settings-close")
                .ghost()
                .small()
                .icon(IconName::Close)
                .rounded_full()
                .on_click(|_, window, cx| crate::dialogs::dialog_window::close(window, cx));
            let title = div().text_xl().font_weight(FontWeight::SEMIBOLD).child(t!("settings.title"));
            let page = h_flex().w_full().h(px(BODY_HEIGHT)).items_start().gap_3().text_sm().child(self.render_rail(cx)).child(self.render_page(cx));
            return v_flex().w_full().gap_4().child(h_flex().justify_between().items_center().child(title).child(close)).child(page).into_any_element();
        }
        let page = h_flex()
            .w_full()
            .h(px(BODY_HEIGHT))
            .items_start()
            .gap_3()
            .text_sm()
            .child(self.render_rail(cx))
            .child(self.render_page(cx));
        // In a window of its own: the footer's buttons under it; on Linux under a line across the window, as far under the rail.
        let lined = lined(cx);
        let footer = self.owner.is_some().then(|| {
            div()
                .pt_4()
                .when(lined, |el| el.mt_4().mx(px(-16.)).px_4().border_t_1().border_color(cx.theme().border))
                .child(buttons(&cx.entity(), self.draft != self.saved))
        });
        v_flex().w_full().child(page).children(footer).into_any_element()
    }
}

/// OK, Cancel and Apply, in the dialog's footer.
fn buttons(view: &Entity<SettingsDialog>, changed: bool) -> impl IntoElement {
    let (ok, cancel, apply) = (view.clone(), view.clone(), view.clone());
    h_flex()
        .gap_2()
        .justify_end()
        .children(crate::dialogs::ordered([
            (Role::Main, Button::new("settings-ok")
                .primary()
                .label(t!("settings.ok"))
                .min_w(px(80.))
                .on_click(move |_, window, cx| {
                    ok.update(cx, |this, cx| this.apply(window, cx));
                    crate::dialogs::dialog_window::close(window, cx);
                })),
            (Role::Dismiss, Button::new("settings-cancel")
                .outline()
                .label(t!("settings.cancel"))
                .min_w(px(80.))
                .on_click(move |_, window, cx| {
                    cancel.update(cx, |this, cx| this.untry(window, cx));
                    crate::dialogs::dialog_window::close(window, cx);
                })),
            (Role::Apply, Button::new("settings-apply")
                .outline()
                .label(t!("settings.apply"))
                .min_w(px(80.))
                .disabled(!changed)
                .on_click(move |_, window, cx| apply.update(cx, |this, cx| this.apply(window, cx)))),
        ]))
}
