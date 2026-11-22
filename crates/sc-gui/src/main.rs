// No console window beside the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod addons;
mod appearance;
mod dialogs;
mod editor;
mod file_tools;
mod header;
mod menus;
mod panel;
mod quick_open;
mod remote;
mod search_window;
mod sidebar;
mod sync_window;
#[cfg(debug_assertions)]
mod style_preview;
mod terminal;
mod transfer;
mod viewer;
mod workspaces;

use std::borrow::Cow;
use std::cell::Cell;
use std::rc::Rc;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants, DropdownButton};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{AppMenuBar, ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::command::{self, Outcome};
use sc_core::menu::{BUTTON_BAR, Command, Item, TAB_MENU, ToolbarItem};
use sc_core::remote::Address;
use sc_core::session::{Session, SideState, WindowPlace, Workspaces};
use sc_core::settings::{self, Settings};
use sc_core::sidebar::SidebarTab;
use sc_core::system::theme::{self, Appearance};
use sc_core::{FileFilter, Panel, Sort, SortKey, Tabs, Typed, View, archive, edit, remote::mounts, ops, viewer::origin};

use crate::appearance::PanelColors;
use crate::panel::drag_drop::{DragLabel, Dragged};
use crate::menus::{GoToHistory, OpenWorkspace, ReopenTab, Run, RunTool, SelectTab, SetAppearance};

actions!(
    sc,
    [
        SwitchPanel,
        Up,
        Down,
        MarkUp,
        MarkDown,
        PageUp,
        PageDown,
        Home,
        End,
        Open,
        Parent,
        FocusPanels,
        CommandHistory,
        CancelPathEdit,
        CancelRename,
        Left,
        Right
    ]
);

include!(concat!(env!("OUT_DIR"), "/material_icons.rs"));

// Lucide icons beyond GPUI Kit's own set.
gpui_kit::assets::icon_assets!(ExtraIcons, [Bookmark, HardDrive, FolderClock, Wrench, SquareTerminal, Layers, PanelsTopLeft, Type, CalendarClock, Languages, FilePen, Puzzle, Server, Smartphone, Tablet, Laptop, Monitor, Tv, Usb, Menu, Cloud, CaseSensitive, WholeWord, Regex, Replace, ReplaceAll, CaseUpper, SquareDashedText, X, Funnel, Link, Pencil, FilePlus, FolderPlus, FolderInput, TextCursorInput, Trash, Diff, Package, PackageOpen, Split, Merge, Hash, ShieldCheck, LogOut, ListChecks, Scissors, ClipboardPaste, House, BookmarkPlus, Zap, FolderSync, GitCompareArrows, ListOrdered, ScrollText, ArrowLeftRight, Equal, Keyboard, Unplug, Rows2, Columns3, List, LayoutGrid, ListTree, Link2, FolderTree, PanelTop, KeyRound, Terminal, Lock, Download, Clock, AudioLines, Music]);

/// GPUI Kit's icons, the file icons as `material/<name>.svg`, the player's as `media/`.
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(name) = path.strip_prefix("media/") {
            let found = viewer::media_view::ICONS.iter().find(|(icon, _)| *icon == name);
            return Ok(found.map(|(_, svg)| Cow::Borrowed(*svg)));
        }
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        let Some(name) = path.strip_prefix("material/") else {
            return gpui_kit::assets::Assets.load(path);
        };
        let found = MATERIAL_ICONS.binary_search_by_key(&name, |(name, _)| name);
        Ok(found.ok().map(|ix| Cow::Borrowed(MATERIAL_ICONS[ix].1)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        gpui_kit::assets::Assets.list(path)
    }
}

const CONTEXT: &str = "Commander";
const ROW: Pixels = px(22.);
/// Refused for want of rights, as the system says it (the trash's errors keep only the text).
/// What permission refused, offered as administrator over `window`: the
/// system asks for the password (polkit, UAC, macOS's). `done` hears whether
/// it was done; declined at the system's prompt, nothing is said.
pub(crate) fn offer_admin_in(
    what: String,
    action: sc_core::system::elevate::Action,
    window: &mut Window,
    cx: &mut App,
    done: impl FnOnce(bool, &mut Window, &mut App) + 'static,
) {
    let text = match action {
        sc_core::system::elevate::Action::Delete(_) => sc_core::t!("main.admin.offer_delete", what = what),
        _ => sc_core::t!("main.admin.offer", what = what),
    };
    type Done = Box<dyn FnOnce(bool, &mut Window, &mut App)>;
    let pending = Rc::new(std::cell::RefCell::new(Some((action, Box::new(done) as Done))));
    // After the dialog that asked for this closes: closing takes the topmost.
    window.defer(cx, move |window, cx| {
        let (title, button) = (sc_core::t!("main.admin.title"), sc_core::t!("main.admin.button"));
        crate::dialogs::dialog_window::question(title, text, button, false, window, cx, move |window, cx| {
            {
                let Some((action, done)) = pending.borrow_mut().take() else {
                    return;
                };
                let what = what.clone();
                let ran = cx.background_executor().spawn(async move {
                    let ran = sc_core::system::elevate::run(&action);
                    sc_core::activity::note(format!("{what}, as administrator"), &ran);
                    ran
                });
                window
                    .spawn(cx, async move |cx| {
                        let ran = ran.await;
                        let _ = cx.update(|window, cx| {
                            if let Err(err) = &ran
                                && err.kind() != io::ErrorKind::Interrupted
                            {
                                window.push_notification(Notification::error(sc_core::t!("main.admin.failed", err = err)), cx);
                            }
                            done(ran.is_ok(), window, cx);
                        });
                    })
                    .detach();
            }
        });
    });
}

pub(crate) fn denied(err: &io::Error) -> bool {
    let text = err.to_string();
    err.kind() == io::ErrorKind::PermissionDenied || text.contains("ermission denied") || text.contains("Access is denied")
}

/// Longer than a key's repeat delay (X's 660 ms, KDE's 600 by default, a second at most).
const REPEAT: Duration = Duration::from_millis(1200);

/// The path title's height, typing a path or not.
const PATH_HEIGHT: Pixels = px(30.);
/// Name, Ext, Size, Date and Attr; Name 0 takes the rest, until resized.
/// Name, Ext, Size, Date, Attr; they keep their widths whatever the window or sidebar do.
const COLUMN_WIDTHS: [f32; 5] = [240., 56., 100., 132., 92.];
const COLUMN_MIN: f32 = 24.;
/// Row and header padding plus border, left of Name and right of Attr.
const ROW_INSET: Pixels = px(5.);
/// The rows' scrollbar, right of them and of their titles.
const SCROLLBAR: Pixels = px(16.);
/// Within this share of the middle, the splitter snaps to it.
const SPLIT_MAGNET: f32 = 0.02;

/// Dragging the left edge of a column (Ext 0 … Attr 3) in a panel.
struct ColumnDrag {
    panel: usize,
    column: usize,
}

struct SplitDrag;

fn main() {
    #[cfg(target_os = "macos")]
    name_the_app();
    let app = gpui_kit::application().with_assets(AppAssets);
    app.run(|cx| {
        #[cfg(target_os = "macos")]
        dock_icon();
        #[cfg(target_os = "macos")]
        alert_icon(cx);
        // KDE: at the left of a dialog's title bar, as KWin draws it on windows.
        #[cfg(not(target_os = "macos"))]
        cx.set_global(gpui_kit::component::AlertIcon(ImageSource::Image(std::sync::Arc::new(gpui_kit::Image::from_bytes(
            gpui_kit::ImageFormat::Png,
            include_bytes!("../../../packaging/linux/hicolor/32x32/apps/sparkcommander.png").to_vec(),
        )))));
        // Windows: one taskbar app for all its windows and copies, wherever the exe is.
        #[cfg(windows)]
        cx.set_app_identity("Remoxio.SparkCommander", "Spark Commander");
        gpui_kit::init(cx);
        cx.set_global(gpui_kit::component::dialog::CancelLabel(|| sc_core::t!("alert.cancel").into()));
        let keys = Some(CONTEXT);
        cx.bind_keys([
            KeyBinding::new("tab", SwitchPanel, keys),
            KeyBinding::new("up", Up, keys),
            KeyBinding::new("down", Down, keys),
            KeyBinding::new("shift-up", MarkUp, keys),
            KeyBinding::new("shift-down", MarkDown, keys),
            KeyBinding::new("left", Left, keys),
            KeyBinding::new("right", Right, keys),
            KeyBinding::new("pageup", PageUp, keys),
            KeyBinding::new("pagedown", PageDown, keys),
            KeyBinding::new("home", Home, keys),
            KeyBinding::new("end", End, keys),
            KeyBinding::new("enter", Open, keys),
            KeyBinding::new("backspace", Parent, keys),
            KeyBinding::new("ctrl-pageup", Parent, keys),
            // Bound after the input's own keys, so it comes first.
            KeyBinding::new("escape", FocusPanels, Some("CommandLine > Input")),
            // A drop-down's, as Total Commander's command line.
            KeyBinding::new("alt-down", CommandHistory, Some("CommandLine > Input")),
            KeyBinding::new("ctrl-down", CommandHistory, Some("CommandLine > Input")),
            KeyBinding::new("escape", CancelPathEdit, Some("PathEdit > Input")),
            KeyBinding::new("escape", CancelRename, Some("RenameInPlace > Input")),
        ]);
        menus::init(cx);
        // Settings and Shortcuts where the panels do not take them, as from
        // the menu bar over another window: over the main window. Other
        // commands from a window working for the main one go to its panels.
        cx.on_action(|Run(command): &Run, cx| {
            let page = match command {
                Command::Settings => Some(0),
                Command::Shortcuts => None,
                // To the next listener: Quit's.
                Command::Quit => return cx.propagate(),
                _ => {
                    if !to_main_window(Box::new(Run(*command)), cx) {
                        cx.propagate();
                    }
                    return;
                }
            };
            // The window the action came to may be the main one, being updated.
            cx.defer(move |cx| configure(page, cx));
        });
        cx.on_action(|appearance: &SetAppearance, cx| {
            if !to_main_window(appearance.boxed_clone(), cx) {
                cx.propagate();
            }
        });
        // Files taken out of archives for the viewer; this run's go at quit, once the jobs stopped.
        std::thread::spawn(archive::remove_stale_scratch);
        viewer::bind_keys(cx);
        search_window::bind_keys(cx);
        dialogs::dialog_window::bind_keys(cx);
        sync_window::bind_keys(cx);
        editor::bind_keys(cx);
        terminal::bind_keys(cx);
        addons::init(cx);
        dialogs::passwords_dialog::init(cx);
        register_xml();
        header::round_corners(cx);
        #[cfg(debug_assertions)]
        if std::env::var_os("SC_STYLE_PREVIEW").is_some() {
            return style_preview::open(cx);
        }
        let saved = Session::load().ok().and_then(|session| session.window).and_then(|place| placed(place, cx));
        let display_id = saved.as_ref().and_then(|(_, display)| *display);
        let options = WindowOptions {
            window_bounds: Some(saved.map_or_else(|| WindowBounds::centered(size(px(1200.), px(800.)), cx), |(bounds, _)| bounds)),
            display_id,
            titlebar: Some(TitlebarOptions {
                title: Some("Spark Commander".into()),
                // macOS: the toolbar in a taller title bar, as Finder's and ForkLift's.
                appears_transparent: cfg!(target_os = "macos"),
                traffic_light_position: cfg!(target_os = "macos").then_some(header::MAC_LIGHTS),
            }),
            app_owns_titlebar_drag: cfg!(target_os = "macos"),
            // Our own frame from the start: asked for once mapped, GNOME's X11 grows the window by its title and our shadow.
            window_decorations: Some(match Settings::load().unwrap_or_default().menu.button() {
                true => WindowDecorations::Client,
                false => WindowDecorations::Server,
            }),
            window_min_size: Some(size(px(600.), px(400.))),
            app_id: Some("sparkcommander".into()),
            icon: window_icon(),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            let commander = cx.new(|cx| Commander::new(window, cx));
            commander.read(cx).request_frame(window);
            commander
        })
        .expect("open the main window");
        // Debug builds open Settings on a page by its name, for looking at it.
        #[cfg(debug_assertions)]
        if let Some(page) = std::env::var("SC_SETTINGS_PAGE").ok().and_then(|name| settings::PAGES.iter().position(|&page| page == name)) {
            cx.defer(move |cx| configure(Some(page), cx));
        }
    });
}

/// macOS's menu bar and Dock show a bundle's name; an executable without
/// one is named after its file. Before the app starts, which reads it.
#[cfg(target_os = "macos")]
fn name_the_app() {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSBundle, NSMutableDictionary, NSString, ns_string};
    let Some(info) = NSBundle::mainBundle().infoDictionary() else {
        return;
    };
    // A bundle's Info.plist names it already.
    if info.objectForKey(ns_string!("CFBundleName")).is_some() {
        return;
    }
    // SAFETY: the dictionary made for an executable without a bundle is a mutable one.
    let info: Retained<NSMutableDictionary<NSString, AnyObject>> = unsafe { Retained::cast_unchecked(info) };
    info.insert(ns_string!("CFBundleName"), ns_string!("Spark Commander"));
}

/// Run outside its bundle, as by `cargo run`: the Dock shows its icon all the same.
#[cfg(target_os = "macos")]
fn dock_icon() {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::{NSBundle, NSString, ns_string};
    const ICON: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../packaging/macos/SparkCommander.icns");
    let bundled = NSBundle::mainBundle().infoDictionary().is_some_and(|info| info.objectForKey(ns_string!("CFBundleIconFile")).is_some());
    let Some(mtm) = MainThreadMarker::new().filter(|_| !bundled) else {
        return;
    };
    if let Some(icon) = NSImage::initWithContentsOfFile(NSImage::alloc(), &NSString::from_str(ICON)) {
        unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&icon)) };
    }
}

/// On top of alerts, as AppKit's: the app's icon in the Dock, 64 px.
#[cfg(target_os = "macos")]
fn alert_icon(cx: &mut App) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    let Some(icon) = MainThreadMarker::new().and_then(|mtm| NSApplication::sharedApplication(mtm).applicationIconImage()) else {
        return;
    };
    // Twice as large: sharp on a Retina display.
    if let Some(drawn) = panel::app_icons::drawn(&icon, 128) {
        cx.set_global(gpui_kit::component::AlertIcon(ImageSource::Render(drawn)));
    }
}

/// Where Settings opens from other windows.
#[derive(Clone)]
struct MainWindow(AnyWindowHandle, WeakEntity<Commander>);

impl Global for MainWindow {}

/// The settings in use, once the main window is up.
fn current_settings(cx: &App) -> Option<Settings> {
    let MainWindow(_, commander) = cx.try_global::<MainWindow>()?;
    Some(commander.upgrade()?.read(cx).settings.clone())
}

/// A notification on the main window, for what happens apart from any.
fn notify_main(notification: Notification, cx: &mut App) {
    let Some(MainWindow(main, _)) = cx.try_global::<MainWindow>().cloned() else {
        return;
    };
    let _ = main.update(cx, |_, window, cx| window.push_notification(notification, cx));
}

/// While a command goes to the main window: one it does not take goes on.
struct Forwarding;

impl Global for Forwarding {}

/// A command of the main window's, chosen in a window that works for it
/// (menus::lend), to its panels, without leaving that window; false when the
/// active window does not work for the main one.
/// Phones whose files stopped answering, connected again, from a window
/// that found out (Quick View's viewer).
pub(crate) fn reconnect_devices(cx: &mut App) {
    let Some(MainWindow(main, commander)) = cx.try_global::<MainWindow>().cloned() else {
        return;
    };
    cx.defer(move |cx| {
        let _ = main.update(cx, |_, window, cx| commander.update(cx, |this, cx| this.reconnect_devices(window, cx)));
    });
}

fn to_main_window(action: Box<dyn Action>, cx: &mut App) -> bool {
    let Some(MainWindow(main, commander)) = cx.try_global::<MainWindow>().cloned() else {
        return false;
    };
    if cx.has_global::<Forwarding>() || menus::owner_of_active(cx) != Some(main) {
        return false;
    }
    // The window it came to is being updated.
    cx.defer(move |cx| {
        let _ = main.update(cx, |_, window, cx| {
            let Ok(focus) = commander.read_with(cx, |this, _| this.focus.clone()) else {
                return;
            };
            cx.set_global(Forwarding);
            focus.dispatch_action(&*action, window, cx);
            cx.remove_global::<Forwarding>();
        });
    });
    true
}

/// Settings on its Edit/View page, over the main window.
fn configure_viewer(cx: &mut App) {
    let page = settings::PAGES
        .iter()
        .position(|&name| name == "Edit/View");
    configure(Some(page.unwrap_or(0)), cx);
}

/// Settings on `page`, or Shortcuts, over the main window.
fn configure(page: Option<usize>, cx: &mut App) {
    let Some(MainWindow(main, commander)) = cx.try_global::<MainWindow>().cloned() else {
        return;
    };
    let _ = main.update(cx, |_, window, cx| {
        window.activate_window();
        let Ok(settings) = commander.read_with(cx, |this, _| this.settings.clone()) else {
            return;
        };
        match page {
            Some(page) => dialogs::settings_dialog::open(commander, settings, page, window, cx),
            None => dialogs::keys_dialog::open(commander, settings.keys, window, cx),
        }
    });
}

/// The setting, for windows that render while the main window updates.
struct ShowMinimap(bool);

impl Global for ShowMinimap {}

/// Whether the Viewer and the Editor show their minimap.
fn minimap_shown(cx: &App) -> bool {
    cx.try_global::<ShowMinimap>().is_none_or(|shown| shown.0)
}

/// XML's colours, which GPUI Kit's own languages lack: its tree-sitter grammar.
fn register_xml() {
    use gpui_kit::component::highlighter::{GrammarConfig, LanguageRegistry};
    let language = tree_sitter::Language::new(tree_sitter_xml::LANGUAGE_XML);
    let config = GrammarConfig::new("xml", language, Vec::new(), tree_sitter_xml::XML_HIGHLIGHT_QUERY, "", "");
    LanguageRegistry::singleton().register("xml", &config);
}

/// Shows or hides the minimap, and saves the setting.
fn toggle_minimap(cx: &mut App) {
    change_settings(cx, |settings| settings.viewer.minimap = !settings.viewer.minimap);
}

/// Changes the main window's settings, as other windows' menus do, and saves them.
fn change_settings(cx: &mut App, change: impl FnOnce(&mut Settings)) {
    let Some(MainWindow(main, commander)) = cx.try_global::<MainWindow>().cloned() else {
        return;
    };
    let _ = main.update(cx, |_, window, cx| {
        let _ = commander.update(cx, |this, cx| {
            let mut settings = this.settings.clone();
            change(&mut settings);
            this.apply_settings(settings, window, cx);
        });
    });
}

/// Quick search: what was typed in a panel, and the row it found.
struct Typing {
    side: usize,
    typed: String,
    row: usize,
    /// The quick filter's (Ctrl+I), open while the panel stays here: what is
    /// typed hides the names without it, arrows move among the rest.
    filter: Option<String>,
}

struct Commander {
    panels: [Side; 2],
    active: usize,
    focus: FocusHandle,
    typing: Option<Typing>,
    /// The last file F3 or F4 opened, how and when: a held key's repeats open no more.
    last_open: Option<(Instant, remote::Open, String)>,
    menu_bar: Entity<AppMenuBar>,
    settings: Settings,
    colors: PanelColors,
    /// Show list of all tabs: the side and its popup.
    panel_list: Option<(usize, Entity<PopupMenu>, Subscription)>,
    /// Its items a letter reaches: the letter, the item, and whether typing it
    /// opens it (Windows' drive letters, as Total Commander's) or highlights it.
    panel_letters: Vec<(char, usize, bool)>,
    /// The command line's last lines, the latest first, and their list when shown.
    commands: Vec<String>,
    command_list: Option<(Entity<PopupMenu>, Subscription)>,
    /// A side whose path is being typed.
    path_edit: Option<(usize, Entity<InputState>, Subscription)>,
    renaming: Option<panel::panel_keys::Renaming>,
    _appearance: Subscription,
    command: Entity<InputState>,
    /// Widths of Name, Ext, Size, Date and Attr, both panels'.
    columns: [f32; 5],
    /// The left panel's share of the width.
    split: f32,
    _quit: Subscription,
    _command: Subscription,
    transfers: transfer::Transfers,
    /// Save Selection's names, for Restore Selection.
    saved_selection: Vec<std::ffi::OsString>,
    /// Ctrl+C's or Ctrl+X's files.
    file_clip: Option<panel::drag_drop::FileClip>,
    sidebar: sidebar::Sidebar,
    quick: Option<viewer::quick_view::QuickView>,
    thumbnails: panel::views::Thumbnails,
    app_icons: panel::app_icons::AppIcons,
    /// The panels one above the other.
    vertical: bool,
    /// Synchronized browsing: the other panel makes the active one's moves too.
    sync_browsing: bool,
    /// The menus show Branch View checked: the active tab's, as last built.
    branch_ticked: bool,
    /// The window's place and size, kept for the next start, and its display's UUID.
    bounds: WindowBounds,
    display: Option<String>,
    workspaces: Workspaces,
    /// The workspaces left this run, by their place in the list: as they were, shells running.
    parked: Vec<Option<workspaces::Parked>>,
    /// Workspaces opened this run: a server's late answer for one left is dropped.
    switches: u64,
    /// The built-in terminal's shells and the one shown; shown or not, and its height.
    terminals: Vec<terminal::pane::TerminalTab>,
    terminal_tab: usize,
    terminal_shown: bool,
    /// Its share of the window's height, so it grows and shrinks with the window.
    terminal_share: f32,
    /// How much it covers the desktop behind it (none: solid), and whether that is blurred.
    terminal_look: (Option<f32>, bool),
    /// Quick View's viewer has the keys: no panel shows a cursor.
    quick_keys: bool,
    /// macOS: the cursor turns grey while another window is in front, as Finder's.
    window_active: bool,
    /// The last right click was on an entry, not off them (the desktop's menu on Linux).
    #[cfg_attr(not(any(target_os = "linux", target_os = "freebsd")), allow(dead_code))]
    menu_on_row: bool,
}

struct Side {
    tabs: Tabs,
    scroll: UniformListScrollHandle,
    /// Brief's columns scroll sideways.
    brief: ScrollHandle,
    /// The entries' area at the last frame: Brief's and Thumbnails' layout.
    area: Rc<Cell<Size<Pixels>>>,
}

impl Commander {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.set_global(MainWindow(window.window_handle(), cx.entity().downgrade()));
        let mut problems = Vec::new();
        let settings = Settings::load().unwrap_or_else(|err| {
            problems.push(sc_core::t!("main.error.read_settings", err = err));
            Settings::default()
        });
        cx.set_global(ShowMinimap(settings.viewer.minimap));
        settings.apply_globally();
        let session = Session::load().unwrap_or_else(|err| {
            problems.push(sc_core::t!("main.error.read_tabs", err = err));
            Session::default()
        });
        let workspaces = Workspaces::load().unwrap_or_else(|err| {
            problems.push(sc_core::t!("main.error.read_workspaces", err = err));
            Workspaces::default()
        });
        menus::set_workspaces(&workspaces);
        if !problems.is_empty() {
            // Notifications need the window's root first.
            window.defer(cx, move |window, cx| {
                let text = problems.join("\n");
                window.push_notification(Notification::error(text).autohide(false), cx)
            });
        }
        cx.set_global(appearance::FixedFont(settings.fixed_font.clone()));
        let colors = appearance::apply(&palette(&settings, window), cx);
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        appearance::draw_toolkit_controls(window, cx);
        menus::bind(&settings.keys, None, cx);
        menus::set_tools(&settings.tools);
        menus::bind_tools(&settings.tools, &[], cx);
        let opacity = settings.sidebar_opacity.0;
        let blur = settings.sidebar_blur.0;
        let sidebar = sidebar::Sidebar::new(session.sidebar, session.sidebar_width, settings.show_hidden, opacity, blur);
        let terminal_look = (sc_core::system::theme::backdrop_tint(settings.terminal_opacity.0), settings.terminal_blur.0);
        menus::set_last_filter(&settings.last_filter);
        let synced = session.sync_browsing;
        menus::set(settings.appearance, &|command| sidebar.shows(command) || (command == Command::SyncBrowsing && synced), cx);
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            if this.settings.appearance == Appearance::System {
                this.colors = appearance::apply(&palette(&this.settings, window), cx);
                this.set_backdrop(window, cx);
            }
        });
        let mut servers = [Vec::new(), Vec::new()];
        let mut side = |ix: usize| {
            let (tabs, found) = restore_side(session.open.sides.get(ix), settings.filter());
            servers[ix] = found;
            Side { tabs, scroll: UniformListScrollHandle::new(), brief: ScrollHandle::new(), area: Rc::default() }
        };
        let panels = [side(0), side(1)];
        // Closing the window drops this view before the app quits, so both save.
        let quit = cx.on_app_quit(|this, _| {
            this.save_session();
            this.transfers.queue.stop_all();
            archive::remove_scratch();
            async {}
        });
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = this.update(cx, |this, cx| {
                this.track_place(window, cx);
                this.save_session();
                this.transfers.queue.stop_all();
            });
            true
        });
        cx.observe_window_bounds(window, |this, window, cx| this.track_place(window, cx)).detach();
        // GPUI learns of another display after the bounds that took the window there.
        cx.observe_window_display(window, |this, window, cx| this.track_place(window, cx)).detach();
        Self::watch_queue(window, cx);
        Self::watch_devices(cx);
        // Back from another program, which may have changed the folders.
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.reread_local(cx);
            }
            cx.notify();
        })
        .detach();
        let command = cx.new(|cx| InputState::new(window, cx));
        let command_entered = cx.subscribe_in(&command, window, |this, _, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.run_command(window, cx);
            }
        });
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // The window's root, which paints its background, comes after this.
        let (shown, terminals, terminal_tab) = (session.open.terminal, session.open.terminals.clone(), session.open.terminal_tab);
        cx.defer_in(window, move |this, window, cx| {
            this.set_backdrop(window, cx);
            for (side, found) in servers.into_iter().enumerate() {
                this.reconnect_tabs(side, found, window, cx);
            }
            // Saved before its tabs were: a shell, the panels keeping the keys after the terminal takes them.
            if shown && terminals.is_empty() {
                this.toggle_terminal(window, cx);
                cx.defer_in(window, |this, window, cx| window.focus(&this.focus, cx));
            } else {
                this.open_terminals(&terminals, terminal_tab, shown, window, cx);
            }
        });
        Self {
            panels,
            active: session.open.active.min(1),
            // Name 0: it filled the panel once.
            columns: match session.columns[..] {
                [name, ext, size, date, attr] if name > 0. => [name, ext, size, date, attr],
                [_, ext, size, date, attr] | [ext, size, date, attr] => [COLUMN_WIDTHS[0], ext, size, date, attr],
                _ => COLUMN_WIDTHS,
            },
            split: match session.split {
                split if (0.15..=0.85).contains(&split) => split,
                _ => 0.5,
            },
            focus,
            typing: None,
            last_open: None,
            menu_bar: AppMenuBar::new(cx),
            settings,
            colors,
            panel_list: None,
            panel_letters: Vec::new(),
            commands: session.commands.clone(),
            command_list: None,
            path_edit: None,
            renaming: None,
            _appearance: appearance,
            command,
            _quit: quit,
            _command: command_entered,
            transfers: transfer::Transfers::default(),
            saved_selection: Vec::new(),
            file_clip: None,
            sidebar,
            quick: None,
            quick_keys: false,
            window_active: true,
            menu_on_row: false,
            thumbnails: panel::views::Thumbnails::default(),
            app_icons: panel::app_icons::AppIcons::default(),
            vertical: session.vertical,
            sync_browsing: session.sync_browsing,
            branch_ticked: false,
            bounds: kept_bounds(window, cx),
            display: display_uuid(window, cx),
            parked: workspaces.list.iter().map(|_| None).collect(),
            workspaces,
            switches: 0,
            terminals: Vec::new(),
            terminal_tab: 0,
            terminal_shown: false,
            terminal_look,
            terminal_share: match session.terminal_share {
                0. => 0.3,
                share => share,
            },
        }
    }

    fn apply_settings(&mut self, settings: Settings, window: &mut Window, cx: &mut Context<Self>) {
        let look = |settings: &Settings| (settings.sidebar_opacity, settings.sidebar_blur, settings.terminal_opacity, settings.terminal_blur);
        if look(&settings) != look(&self.settings) {
            self.show_backdrop(&settings, window, cx);
        }
        if settings.filter() != self.settings.filter() {
            for side in &mut self.panels {
                side.tabs.set_filter(settings.filter());
                side.scroll_to_cursor();
            }
            self.sidebar.refresh(settings.show_hidden);
        }
        if settings.viewer.minimap != self.settings.viewer.minimap {
            cx.set_global(ShowMinimap(settings.viewer.minimap));
            cx.refresh_windows();
        }
        if settings.keys != self.settings.keys {
            menus::bind(&settings.keys, Some(&self.settings.keys), cx);
            // The tools' keys come after again, so they keep winning.
            menus::bind_tools(&settings.tools, &self.settings.tools, cx);
            self.menu_bar.update(cx, |bar, cx| bar.reload(cx));
        }
        // The download limit too: running transfers follow it at once.
        settings.apply_globally();
        let language = settings.language != self.settings.language;
        if (&settings.date, settings.folders_like_files) != (&self.settings.date, self.settings.folders_like_files) {
            if settings.folders_like_files != self.settings.folders_like_files {
                for side in &mut self.panels {
                    side.tabs.resort();
                    side.scroll_to_cursor();
                }
            }
            cx.refresh_windows();
        }
        let look = (settings.appearance, &settings.font, &settings.fixed_font)
            != (self.settings.appearance, &self.settings.font, &self.settings.fixed_font);
        cx.set_global(appearance::FixedFont(settings.fixed_font.clone()));
        let menu = settings.menu != self.settings.menu;
        self.settings = settings;
        if menu {
            self.request_frame(window);
        }
        if look {
            self.colors = appearance::apply(&palette(&self.settings, window), cx);
            self.set_backdrop(window, cx);
            self.reload_menus(cx);
            cx.refresh_windows();
        }
        // Words are read as drawn; the menus are made ahead.
        if language {
            self.reload_menus(cx);
            cx.refresh_windows();
        }
        self.save_settings(window, cx);
        cx.notify();
    }

    fn save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(err) = self.settings.save() {
            let text = sc_core::t!("main.error.save_settings", err = err);
            window.push_notification(Notification::error(text), cx);
        }
    }

    fn set_appearance(&mut self, chosen: Appearance, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.appearance = chosen;
        self.save_settings(window, cx);
        self.colors = appearance::apply(&palette(&self.settings, window), cx);
        self.set_backdrop(window, cx);
        self.reload_menus(cx);
        cx.notify();
    }

    fn reload_menus(&self, cx: &mut Context<Self>) {
        menus::set(self.settings.appearance, &|command| self.checked(command), cx);
        self.menu_bar.update(cx, |bar, cx| bar.reload(cx));
    }

    /// The menus show these commands checked.
    fn checked(&self, command: Command) -> bool {
        self.sidebar.shows(command)
            || (command == Command::QuickViewPanel && self.quick.is_some())
            || (command == Command::TerminalPanel && self.terminal_shown)
            || (command == Command::HiddenFiles && self.settings.show_hidden)
            || (command == Command::SyncBrowsing && self.sync_browsing)
            || (command == Command::BranchView && self.panels[self.active].tabs.current().found_label() == Some("Branch view:"))
            || self.filter_checked(command)
    }

    /// What is typed in panel `ix`, while its cursor stays on the row it found.
    fn typed(&self, ix: usize) -> Option<&str> {
        let typing = self.typing.as_ref().filter(|t| t.side == ix && ix == self.active)?;
        let panel = self.panels[ix].tabs.current();
        let here = match &typing.filter {
            Some(place) => panel.location() == *place,
            None => panel.cursor() == typing.row,
        };
        here.then_some(typing.typed.as_str())
    }

    /// The quick filter is open in panel `ix`.
    fn filtering(&self, ix: usize) -> bool {
        self.typed(ix).is_some() && self.typing.as_ref().is_some_and(|typing| typing.filter.is_some())
    }

    /// Ctrl+I: the quick filter, on what it had if it is on.
    fn quick_filter(&mut self, cx: &mut Context<Self>) {
        let side = self.active;
        let panel = self.panels[side].tabs.current();
        let typed = match panel.file_filter() {
            FileFilter::Quick(text) => text.clone(),
            _ => String::new(),
        };
        self.typing = Some(Typing { side, typed, row: panel.cursor(), filter: Some(panel.location()) });
        cx.notify();
    }

    /// The panel lists the names with `text` in them; all for none.
    fn filter_by(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let filter = match text.is_empty() {
            true => FileFilter::All,
            false => FileFilter::Quick(text),
        };
        self.show_files(filter, window, cx);
    }

    /// Quick search, with no key to start it: a name's first letters typed in
    /// a panel move the cursor to it (Total Commander's).
    /// Backspace takes a letter back (`quick_back`, by Parent's binding), Escape
    /// or another key ends it; a key with a command of its own keeps it.
    fn quick_search(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Not while a field in the panels takes the keys: the path's, a name's.
        if !self.focus.is_focused(window) {
            return;
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let side = self.active;
        let typed = self.typed(side).unwrap_or_default().to_owned();
        let filtering = self.filtering(side);
        let key = keystroke.key.as_str();
        // A key with a command ends a search; the filter stays open.
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
            if !filtering {
                self.typing = None;
            }
            return;
        }
        // Escape ends either; the filter's, left on or open, shows all again.
        let filtered = matches!(self.panels[side].tabs.current().file_filter(), FileFilter::Quick(_));
        if key == "escape" && (filtering || filtered || !typed.is_empty()) {
            self.typing = None;
            if filtering || filtered {
                self.filter_by(String::new(), window, cx);
            }
            cx.stop_propagation();
            return cx.notify();
        }
        let bound = self.settings.keys.bindings().iter().any(|(bound, _)| bound == key);
        let text = keystroke.key_char.as_deref().filter(|text| !text.is_empty() && !text.chars().any(char::is_control));
        let Some(text) = text.filter(|_| !bound && key != "backspace") else {
            if !filtering {
                self.typing = None;
            }
            return;
        };
        let wanted = format!("{typed}{text}");
        if filtering {
            if let Some(typing) = self.typing.as_mut() {
                typing.typed = wanted.clone();
            }
            self.filter_by(wanted, window, cx);
            cx.stop_propagation();
            return cx.notify();
        }
        let panel = self.panels[side].tabs.current();
        // A new search starts below the cursor; typing on may stay on its row.
        let from = panel.cursor() + usize::from(typed.is_empty());
        if let Some(row) = panel.quick_find(&wanted, from) {
            let found = &mut self.panels[side];
            found.tabs.current_mut().set_cursor(row);
            found.scroll_to_cursor();
            self.typing = Some(Typing { side, typed: wanted, row, filter: None });
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Backspace in a quick search: its last letter back, the cursor where it is.
    fn quick_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(typing) = self.typing.as_mut() {
            typing.typed.pop();
            if typing.filter.is_some() {
                let text = typing.typed.clone();
                return self.filter_by(text, window, cx);
            }
            if typing.typed.is_empty() {
                self.typing = None;
            }
        }
        cx.notify();
    }

    fn move_cursor(&mut self, rows: isize, cx: &mut Context<Self>) {
        let side = &mut self.panels[self.active];
        side.tabs.current_mut().move_cursor(rows);
        side.scroll_to_cursor();
        cx.notify();
    }

    // shortcut: a failed save on quit goes unreported; there is no window left to say it.
    fn save_session(&mut self) {
        let _ = self.session().save();
        let current = self.workspaces.current;
        self.workspaces.list[current].open = self.open_tabs();
        let _ = self.workspaces.save();
    }

    /// F3 views, F4 edits the file under the cursor.
    /// F3, F4 and Ctrl+Shift+F4. A server's or an archive's file is copied
    /// out in the background; what the editors save goes back.
    fn open_file(&mut self, how: remote::Open, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        let Some(origin) = panel.origin() else {
            return;
        };
        let around = (how == remote::Open::View && origin.is_local()).then(|| panel.files_around());
        let (settings, bounds, name) = (self.settings.clone(), Over::window(window, cx), origin.name());
        // F3 held down repeats: the same file, the same way, this soon after is the same ask.
        let again = self.last_open.as_ref().is_some_and(|(at, was, file)| *was == how && *file == name && at.elapsed() < REPEAT);
        self.last_open = Some((Instant::now(), how, name.clone()));
        if again {
            return;
        }
        let commander = cx.entity().downgrade();
        // The file's own viewer or editor, by its association.
        let entry = panel
            .current_entry()
            .map(|e| e.display_name())
            .unwrap_or_default();
        let own = sc_core::associations::find(&settings.associations, &entry).map(|a| match how {
            remote::Open::View => a.view.clone(),
            _ => a.edit.clone(),
        });
        let program = match (how, own.filter(|line| !line.trim().is_empty())) {
            (remote::Open::External, _) => Some(settings.editor.external.clone()),
            (_, own) => own,
        };
        let finish = move |prepared: io::Result<(PathBuf, Option<origin::Upload>)>,
                           window: &mut Window,
                           cx: &mut App| {
            let opened = prepared.and_then(|(file, upload)| match (how, upload) {
                (remote::Open::View, _) if let Some(program) = &program => {
                    edit::edit_externally(program, &file)
                }
                (_, Some(upload)) if let Some(program) = &program => {
                    remote::edit_externally(file, upload, program, window, cx)
                }
                (_, None) if let Some(program) = &program => edit::edit_externally(program, &file),
                (remote::Open::View, _) => {
                    let (files, at) = around.unwrap_or_else(|| (vec![file], 0));
                    let wanted = sc_core::addons::wanted_for(&files[at]).filter(|_| !settings.never_offer_addons);
                    let Some(addon) = wanted else {
                        return viewer::open(files, at, settings.viewer, bounds, cx);
                    };
                    let shown = files[at].file_name().map_or_else(|| name.clone(), |file| file.to_string_lossy().into_owned());
                    let viewer = settings.viewer.clone();
                    let go_on = move |window: &mut Window, cx: &mut App| {
                        if let Err(err) = viewer::open(files.clone(), at, viewer.clone(), bounds, cx) {
                            window.push_notification(Notification::error(err.to_string()), cx);
                        }
                    };
                    addons::offer(addon, &shown, commander.clone(), window, cx, go_on);
                    Ok(())
                }
                (remote::Open::Edit | remote::Open::External, upload) => {
                    editor::open(file, settings.editor.clone(), upload, bounds, cx)
                }
            });
            if let Err(err) = opened {
                window.push_notification(Notification::error(format!("{name}: {err}")), cx);
            }
        };
        // A server's file comes by a task the queue shows, which can be cancelled.
        if let origin::Origin::Server(fs, path) = origin {
            let upload =
                (how != remote::Open::View).then(|| origin::upload_to(fs.clone(), path.clone()));
            let file = path.file_name().unwrap_or_default().display().to_string();
            let title = match how {
                remote::Open::View => sc_core::t!("main.download.view", file = file),
                _ => sc_core::t!("main.download.edit", file = file),
            };
            let (sent, received) = futures::channel::oneshot::channel();
            let task = origin::fetch_task(fs, path, title, move |fetched| {
                let _ = sent.send(fetched);
            });
            self.transfers.queue.run_now(task);
            cx.spawn_in(window, async move |_, cx| {
                // The queue shows a failure; a cancel opens nothing.
                if let Ok(Ok(file)) = received.await {
                    let _ = cx.update(|window, cx| finish(Ok((file, upload)), window, cx));
                }
            })
            .detach();
            return cx.notify();
        }
        cx.spawn_in(window, async move |_, cx| {
            let prepare = async move {
                match how {
                    remote::Open::View => origin.view().map(|file| (file, None)),
                    _ => origin.edit(),
                }
            };
            let prepared = cx.background_executor().spawn(prepare).await;
            let _ = cx.update(|window, cx| finish(prepared, window, cx));
        })
        .detach();
    }

    fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        let commander = cx.entity().downgrade();
        let label = sc_core::t!("main.new_folder.label");
        dialogs::name_dialog::open(
            sc_core::t!("main.new_folder.title"),
            label,
            String::new(),
            window,
            cx,
            move |name, window, cx| {
                if name.trim().is_empty() {
                    return;
                }
                let _ = commander.update(cx, |this, cx| {
                    let panel = this.panels[side].tabs.current_mut();
                    // A server's, and 7-Zip rewriting an archive, in the background.
                    if panel.remote().is_some() || panel.in_archive() {
                        let made = move |panel: &mut Panel| panel.new_folder(&name);
                        return this.navigate_on(side, true, made, window, cx);
                    }
                    let folder = panel.folder().to_owned();
                    match panel.new_folder(&name) {
                        Err(err) if denied(&err) => {
                            let action = sc_core::system::elevate::Action::MakeFolder(folder.join(name.trim()));
                            this.offer_admin(sc_core::t!("main.admin.making", name = name.trim()), action, window, cx);
                        }
                        Err(err) => window.push_notification(Notification::error(sc_core::t!("main.error.make_folder", err = err)), cx),
                        Ok(()) => {}
                    }
                    this.refresh_folder(&folder, cx);
                });
            },
        );
    }

    fn mark(&mut self, change: impl FnOnce(&mut Panel), cx: &mut Context<Self>) {
        let side = &mut self.panels[self.active];
        change(side.tabs.current_mut());
        side.scroll_to_cursor();
        cx.notify();
    }

    fn confirm_delete(&mut self, to_trash: bool, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.active;
        let panel = self.panels[side].tabs.current();
        let chosen = panel.chosen();
        if chosen.is_empty() {
            return;
        }
        // Before asking: an archive 7-Zip cannot change says so.
        let archive = match panel.in_archive() {
            true => match panel.archive_place() {
                Ok((archive, _)) => Some(archive.file_name().unwrap_or_default().to_string_lossy().into_owned()),
                Err(err) => return window.push_notification(Notification::warning(err.to_string()), cx),
            },
            false => None,
        };
        let (kind, name, count) = match chosen.as_slice() {
            [one] if one.is_dir => ("folder", one.display_name(), 1),
            [one] => ("file", one.display_name(), 1),
            many => ("other", String::new(), many.len()),
        };
        let server = panel.remote().map(|remote| remote.url());
        let (title, text) = match to_trash {
            // An archive has no trash.
            _ if let Some(archive) = &archive => (sc_core::t!("main.delete.title"), sc_core::t!("main.delete.from_archive", kind = kind, name = name, count = count, archive = archive)),
            // A server has no trash.
            _ if let Some(server) = &server => (sc_core::t!("main.delete.title"), sc_core::t!("main.delete.on_server", kind = kind, name = name, count = count, server = server)),
            true => (sc_core::t!("main.delete.title"), sc_core::t!("main.delete.to_trash", kind = kind, name = name, count = count)),
            false => (sc_core::t!("main.delete.title_permanently"), sc_core::t!("main.delete.permanently", kind = kind, name = name, count = count)),
        };
        let commander = cx.entity().downgrade();
        crate::dialogs::dialog_window::question(title.clone(), text, title, true, window, cx, move |window, cx| {
            let _ = commander.update(cx, |this, cx| this.delete(side, to_trash, window, cx));
        });
    }

    fn delete(&mut self, side: usize, to_trash: bool, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[side].tabs.current_mut();
        if panel.remote().is_some() || panel.in_archive() {
            return self.navigate_on(side, true, move |panel| panel.delete(false), window, cx);
        }
        let (folder, cursor) = (panel.folder().to_owned(), panel.cursor());
        let paths: Vec<PathBuf> = panel.chosen().iter().map(|entry| folder.join(&entry.name)).collect();
        // In the queue, beside the copies: a big folder takes a while, and may be cancelled.
        let title = sc_core::ops::delete::title(&paths, &folder, to_trash);
        let (sent, ended) = futures::channel::oneshot::channel();
        let task = sc_core::ops::delete::task(title, paths.clone(), to_trash, move |failed| {
            let _ = sent.send(failed);
        });
        self.transfers.queue.run_now(task);
        cx.spawn_in(window, async move |this, cx| {
            let failed = ended.await;
            let _ = this.update_in(cx, |this, window, cx| {
                // The cursor stays where it was: on what came after the deleted.
                let panel = this.panels[side].tabs.current_mut();
                if panel.remote().is_none() && panel.folder() == folder && panel.reload().is_ok() {
                    panel.set_cursor(cursor);
                }
                cx.notify();
                // The queue shows how it ended; what permission kept is offered as administrator, for good.
                if let Ok(Some(io::ErrorKind::PermissionDenied)) = failed {
                    let left: Vec<PathBuf> = paths.into_iter().filter(|path| path.symlink_metadata().is_ok()).collect();
                    let what = sc_core::t!("main.admin.deleting", count = left.len());
                    this.offer_admin(what, sc_core::system::elevate::Action::Delete(left), window, cx);
                }
            });
        })
        .detach();
    }

    /// Permission refused `what`: asked whether as administrator, the system
    /// asking for the password, in the background; then the panels read again.
    pub(crate) fn offer_admin(&mut self, what: String, action: sc_core::system::elevate::Action, window: &mut Window, cx: &mut Context<Self>) {
        let commander = cx.entity().downgrade();
        offer_admin_in(what, action, window, cx, move |_, window, cx| {
            let _ = commander.update(cx, |this, cx| {
                for ix in 0..2 {
                    let remote = this.panels[ix].tabs.current().remote().is_some();
                    this.navigate_on(ix, remote, Panel::reload, window, cx);
                }
            });
        });
    }

    /// Every tab on `folder`, on both sides, reads it again.
    /// Ctrl+Z: the newest copy, move, rename or move to the trash undone, or
    /// with `redo` (Ctrl+Shift+Z) the newest undone done again; a move goes
    /// through the queue, the rest in the background.
    fn undo(&mut self, redo: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (taken, way) = match redo {
            false => (sc_core::ops::undo::take(), "undo"),
            true => (sc_core::ops::undo::take_undone(), "redo"),
        };
        let Some(undo) = taken else {
            return window.push_notification(Notification::info(sc_core::t!("main.undo.nothing", way = way)), cx);
        };
        let (title, queue) = (undo.title.clone(), self.transfers.queue.clone());
        // A server no tab keeps open is connected again, its login asked in a dialog.
        let (ask, _) = self.dialog_ask(window, cx);
        let undone = cx.background_executor().spawn(async move { sc_core::ops::undo::run(undo, &queue, redo, ask) });
        cx.spawn_in(window, async move |this, cx| {
            let undone = undone.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let note = match undone {
                    Ok(true) => Notification::info(sc_core::t!("main.undo.done", way = way, title = title)),
                    Ok(false) => Notification::info(sc_core::t!("main.undo.queued", way = way, title = title)),
                    Err(err) => Notification::error(sc_core::t!("main.undo.failed", way = way, title = title, err = err)),
                };
                window.push_notification(note, cx);
                for ix in 0..2 {
                    let remote = this.panels[ix].tabs.current().remote().is_some();
                    this.navigate_on(ix, remote, Panel::reload, window, cx);
                }
            });
        })
        .detach();
    }

    fn refresh_folder(&mut self, folder: &Path, cx: &mut Context<Self>) {
        for side in &mut self.panels {
            side.tabs.reload_folder(folder);
            side.scroll_to_cursor();
        }
        self.sidebar.refresh(self.settings.show_hidden);
        cx.notify();
    }

    fn set_sort(&mut self, ix: usize, sort: Sort, cx: &mut Context<Self>) {
        let side = &mut self.panels[ix];
        side.tabs.current_mut().set_sort(sort);
        side.scroll_to_cursor();
        cx.notify();
    }

    /// The sorted column turns around, another sorts by its default.
    fn sort_by_column(&mut self, ix: usize, key: SortKey, cx: &mut Context<Self>) {
        self.active = ix;
        let sort = self.panels[ix].tabs.current().sort();
        let sort = match sort.key == key {
            true => Sort {
                descending: !sort.descending,
                ..sort
            },
            false => Sort::by(key),
        };
        self.set_sort(ix, sort, cx);
    }

    fn edit_path(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = ix;
        let path = self.panels[ix].tabs.current().location();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(path));
        let events = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            InputEvent::PressEnter { .. } => this.apply_path_edit(window, cx),
            // Clicking elsewhere drops it, focus going where clicked.
            InputEvent::Blur => {
                this.path_edit = None;
                cx.notify();
            }
            _ => {}
        });
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        self.path_edit = Some((ix, input, events));
        cx.notify();
    }

    /// The path, each folder in it a link there, as Total Commander's and
    /// WinUI's BreadcrumbBar; a click on the last or past the end edits it.
    fn render_crumbs(&self, ix: usize, location: &str, cx: &mut Context<Self>) -> Div {
        let crumbs = crumbs(location);
        let last = crumbs.len().saturating_sub(1);
        let parts = crumbs.into_iter().enumerate().map(|(n, (name, end))| {
            let part = div().id(("crumb", n)).flex_none().child(name.to_owned());
            if n == last {
                return part.into_any_element();
            }
            let target = location[..end].to_owned();
            let link = part.cursor_pointer().hover(|style| style.underline()).on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.active = ix;
                this.open_location(ix, &target, window, cx);
                cx.notify();
            }));
            // The path's own separator, `\` or `/`; the root's is in it.
            let sep = location[end..].chars().next().filter(|c| matches!(c, '/' | '\\'));
            h_flex().flex_none().child(link).children(sep.map(|sep| sep.to_string())).into_any_element()
        });
        // Too long, its start goes out of sight: the folder it is in stays.
        h_flex().flex_1().min_w_0().overflow_hidden().justify_end().children(parts).child(div().flex_1())
    }

    /// Windows: the terminal's shell (Settings), else Windows PowerShell, in a
    /// console of its own in the active panel's folder; Windows 11 shows it in its default terminal.
    #[cfg(windows)]
    fn open_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x10;
        let panel = self.panels[self.active].tabs.current();
        if panel.remote().is_some() {
            return window.push_notification(Notification::warning(sc_core::t!("context_menu.terminal_not_local")), cx);
        }
        let chosen = self.settings.terminal_shell.as_deref();
        let shell = sc_core::system::shells::others().into_iter().find(|shell| Some(shell.program.as_str()) == chosen);
        let (program, args) = shell.map_or(("powershell.exe".into(), Vec::new()), |shell| (shell.program, shell.args));
        let started = std::process::Command::new(program).args(args).current_dir(panel.folder()).creation_flags(CREATE_NEW_CONSOLE).spawn();
        if let Err(err) = started {
            window.push_notification(Notification::error(sc_core::t!("context_menu.terminal_failed", err = err)), cx);
        }
    }

    /// Goes to a folder that exists; otherwise says why and stays open.
    fn apply_path_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (ix, typed) = match &self.path_edit {
            Some((ix, input, _)) => (*ix, input.read(cx).value().trim().to_owned()),
            None => return,
        };
        if typed.is_empty() || self.open_location(ix, &typed, window, cx) {
            self.cancel_path_edit(window, cx);
        }
        cx.notify();
    }

    /// Goes where `typed` says, as the path bar takes it: a server, a share,
    /// a folder on this computer or on the panel's server. False, said why,
    /// where it cannot.
    fn open_location(&mut self, ix: usize, typed: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let panel = self.panels[ix].tabs.current();
        // The panel's own server: no new connection.
        if let Some(path) = panel.remote().and_then(|remote| typed.strip_prefix(&remote.url())).filter(|path| path.starts_with('/')) {
            let target = PathBuf::from(path);
            self.navigate_on(ix, true, move |panel: &mut Panel| panel.go_to_folder(target), window, cx);
            return true;
        }
        if let Some(address) = Address::parse(typed) {
            self.connect(ix, address, window, cx);
            return true;
        }
        if mounts::is_url(typed) {
            self.mount_url(ix, typed.to_owned(), window, cx);
            return true;
        }
        let target = match panel.typed_location(typed) {
            // The server's, in the background.
            Typed::Here(target) if panel.remote().is_some() => {
                let go = move |panel: &mut Panel| panel.go_to_folder(target);
                self.navigate_on(ix, true, go, window, cx);
                return true;
            }
            Typed::Here(target) => target,
            // A server's path left without its address: this computer's.
            Typed::Local(target) if target.is_dir() => {
                let go = move |panel: &mut Panel| panel.disconnect(target);
                self.navigate_on(ix, false, go, window, cx);
                return true;
            }
            Typed::Local(target) => {
                let text = sc_core::t!("main.go.no_such_folder", path = target.display());
                window.push_notification(Notification::error(text), cx);
                return false;
            }
        };
        let side = &mut self.panels[ix];
        let before = side.tabs.current().spot();
        match side.tabs.navigate(|panel| panel.go_to_folder(target.clone())) {
            Ok(()) => {
                side.scroll_to_cursor();
                self.follow(ix, before, window, cx);
                true
            }
            Err(err) => {
                let text = format!("{}: {err}", target.display());
                window.push_notification(Notification::error(text), cx);
                false
            }
        }
    }

    /// Back (-1) or Forward (1) in the active tab's history.
    fn go_history(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.active;
        if let Some(place) = self.panels[ix].tabs.current_mut().history.step(by) {
            self.open_location(ix, &place, window, cx);
            cx.notify();
        }
    }

    /// This computer's home folder; on a server, the server's, as `cd ~`.
    fn go_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.active;
        match self.panels[ix].tabs.current().remote().map(|remote| remote.home()) {
            Some(home) => self.navigate_on(ix, true, move |panel| panel.go_to_folder(home), window, cx),
            None => {
                let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                self.navigate_on(ix, false, move |panel| panel.go_to_folder(home), window, cx)
            }
        }
    }

    fn cancel_path_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.path_edit.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    fn run_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let line = self.command.read(cx).value().to_string();
        if !line.trim().is_empty() {
            self.commands.retain(|kept| *kept != line);
            self.commands.insert(0, line.clone());
            self.commands.truncate(20);
        }
        let target = line.trim().strip_prefix("cd ").unwrap_or(line.trim());
        if let Some(address) = Address::parse(target) {
            self.focus_panels(window, cx);
            return self.connect(self.active, address, window, cx);
        }
        if mounts::is_url(target) {
            let url = target.to_owned();
            self.focus_panels(window, cx);
            return self.mount_url(self.active, url, window, cx);
        }
        let panel = self.panels[self.active].tabs.current();
        if let Some(remote) = panel.remote().filter(|_| !line.trim().is_empty()) {
            // shortcut: commands do not run on servers; cd moves there.
            match line.trim().strip_prefix("cd ").map(str::trim) {
                Some(path) => {
                    let path = match path.strip_prefix('~') {
                        Some(rest) => remote.home().join(rest.trim_start_matches('/')),
                        None => command::resolve(path, panel.folder()),
                    };
                    self.navigate(move |panel| panel.go_to_folder(path), window, cx);
                }
                None => {
                    let text = sc_core::t!("command_line.on_server");
                    window.push_notification(Notification::warning(text), cx);
                }
            }
            return self.focus_panels(window, cx);
        }
        let folder = self.panels[self.active].tabs.current().folder().to_owned();
        // With the terminal open, its shell runs them, in the panel's folder.
        if !line.trim().is_empty() && command::change_dir(&line, &folder).is_none() {
            match self.run_in_terminal(&line, &folder, window, cx) {
                Some(true) => return self.focus_panels(window, cx),
                Some(false) => return,
                None => {}
            }
        }
        if !line.trim().is_empty() {
            self.start_command(&line, &folder, window, cx);
        }
        self.focus_panels(window, cx);
    }

    /// `line` by the shell in `folder`, on its own; `cd` moves the panel.
    pub(crate) fn start_command(&mut self, line: &str, folder: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let (sent, failed) = futures::channel::oneshot::channel();
        match command::run(line, folder, move |why| drop(sent.send(why))) {
            Ok(Outcome::ChangeDir(path)) => self.navigate(move |panel| panel.go_to_folder(path), window, cx),
            // Said when the shell gives up; nothing while it runs.
            Ok(Outcome::Started) => cx
                .spawn_in(window, async move |_, cx| {
                    if let Ok(why) = failed.await {
                        let _ = cx.update(|window, cx| window.push_notification(Notification::error(why), cx));
                    }
                })
                .detach(),
            Err(err) => window.push_notification(Notification::error(format!("{line}: {err}")), cx),
        }
    }

    fn focus_panels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.command
            .update(cx, |input, cx| input.set_value("", window, cx));
        window.focus(&self.focus, cx);
    }

    /// Total Commander's command line drop-down: the last lines, the latest
    /// first; the one picked is put in the line, to run with Enter.
    fn show_command_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (lines, input) = (self.commands.clone(), self.command.clone());
        let list = PopupMenu::build(window, cx, move |mut menu, _, _| {
            if lines.is_empty() {
                return menu.label(sc_core::t!("command_line.history.empty"));
            }
            for line in &lines {
                let (input, line) = (input.clone(), line.clone());
                menu = menu.item(PopupMenuItem::new(line.clone()).on_click(move |_, window, cx| {
                    input.update(cx, |input, cx| input.set_value(line.clone(), window, cx));
                }));
            }
            menu.min_w(px(240.)).max_w(px(900.))
        });
        let dismissed = cx.subscribe_in(&list, window, |this, _, _: &DismissEvent, window, cx| {
            this.command_list = None;
            this.command.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
        });
        list.read(cx).focus_handle(cx).focus(window, cx);
        self.command_list = Some((list, dismissed));
        cx.notify();
    }

    fn render_command_line(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let location = self.panels[self.active].tabs.current().location();
        let history = Button::new("command-history")
            .ghost()
            .xsmall()
            .icon(IconName::ChevronDown)
            .on_click(cx.listener(|this, _, window, cx| this.show_command_history(window, cx)));
        let list = self.command_list.as_ref().map(|(list, _)| list.clone());
        h_flex()
            .key_context("CommandLine")
            .on_action(
                cx.listener(|this, _: &FocusPanels, window, cx| this.focus_panels(window, cx)),
            )
            .on_action(cx.listener(|this, _: &CommandHistory, window, cx| this.show_command_history(window, cx)))
            // The panels' or terminal's frame above is its edge.
            .px_1()
            .py_1()
            .gap_1()
            .child(
                div()
                    .max_w(relative(0.5))
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis_start()
                    // The font's capitals sit low in its line: centred on the field.
                    .relative()
                    .top(px(-1.))
                    .child(format!("{location}>")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.))
                    // Up from the line: the button bar is under it.
                    .when_some(list, |el, list| {
                        el.child(deferred(
                            anchored()
                                .anchor(Anchor::BottomLeft)
                                .snap_to_window_with_margin(px(8.))
                                .child(div().occlude().child(list)),
                        ))
                    })
                    // A plain 1 px frame, as Total Commander's.
                    .child(
                        Input::new(&self.command)
                            .small()
                            .bordered(false)
                            .border_1()
                            .border_color(self.colors.header_border)
                            .suffix(history),
                    ),
            )
    }

    fn render_button_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let buttons = BUTTON_BAR.iter().filter_map(|item| match *item {
            Item::Command { command, .. } => {
                let label = sc_core::menu::button_label(command);
                let [key, _] = self.settings.keys.keys_of(command);
                Some((label, key, command))
            }
            _ => None,
        });
        let divider = self.colors.header_border;
        // The command line's colour, the path's beside it.
        h_flex()
            .items_center()
            .children(buttons.enumerate().flat_map(|(ix, (label, keys, command))| {
                let key = dialogs::keys_dialog::shown(&keys);
                // A rule between the buttons, as Total Commander's are apart.
                let rule = (ix > 0).then(|| div().flex_none().w_px().h(px(14.)).bg(divider).into_any_element());
                let button = Button::new(("button-bar", command as usize))
                    .ghost()
                    .small()
                    .flex_1()
                    .label(format!("{key} {label}"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&this.focus, cx);
                        window.dispatch_action(Box::new(Run(command)), cx);
                    }));
                rule.into_iter().chain([button.into_any_element()])
            }))
    }

    /// The toolbar: icons, their names and keys as tooltips, a toggle pressed while on.
    fn render_toolbar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let shown = &self.settings.toolbar.0;
        if shown.is_empty() {
            return None;
        }
        let mac = cfg!(target_os = "macos");
        let button_for = |command: Command| {
            let icon = menus::toolbar_icon(command)?;
            let [key, _] = self.settings.keys.keys_of(command);
            let name = sc_core::menu::name(command);
            let tip = match key.is_empty() {
                true => name.to_owned(),
                false => format!("{name} ({})", dialogs::keys_dialog::shown(&key)),
            };
            let size = if cfg!(target_os = "macos") { header::MAC_BUTTON } else { gpui_kit::component::Size::Medium };
            let button = Button::new(("toolbar", command as usize)).ghost().with_size(size).icon(Icon::new(icon)).selected(self.checked(command)).tooltip(tip);
            // macOS: as wide as AppKit's in a group's capsule.
            let button = if mac { button.w(px(34.)).h(px(26.)).rounded(px(13.)) } else { button };
            let button = button.on_click(cx.listener(move |this, _, window, cx| {
                // Pressed, the terminal's button hides it; the command would give it the keys, the panels having them.
                if command == Command::TerminalPanel && this.terminal_shown {
                    return this.hide_terminal(window, cx);
                }
                window.focus(&this.focus, cx);
                window.dispatch_action(Box::new(Run(command)), cx);
            }));
            if !matches!(command, Command::GoBack | Command::GoForward) {
                return Some(button.into_any_element());
            }
            // Back and Forward go one place; their arrow lists the places that way, as Go's menu.
            let ways = menus::history_ways(command);
            let none = ways.is_empty();
            let focus = self.focus.clone();
            let places = move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
                let menu = menu.action_context(focus.clone());
                ways.iter().fold(menu, |menu, (at, label)| menu.menu(label.clone(), Box::new(GoToHistory(*at))))
            };
            let split = DropdownButton::new(("toolbar-history", command as usize)).ghost().with_size(size).button(button).disabled(none);
            Some(split.dropdown_menu_with_anchor(Anchor::TopLeft, places).into_any_element())
        };
        // The buttons in groups, as the separators part them.
        let mut groups: Vec<Vec<AnyElement>> = vec![Vec::new()];
        // Windows and Linux: the sidebar's button first, apart (macOS has its own by the window's buttons).
        let sidebar = [ToolbarItem::Button(Command::ToggleSidebar), ToolbarItem::Separator];
        let first = if mac || shown.contains(&sidebar[0]) { &[][..] } else { &sidebar[..] };
        for item in first.iter().chain(shown) {
            match *item {
                ToolbarItem::Separator => groups.push(Vec::new()),
                ToolbarItem::Button(command) => groups.last_mut().expect("a group").extend(button_for(command)),
            }
        }
        groups.retain(|group| !group.is_empty());
        // macOS: each group a capsule, as AppKit's; elsewhere a line between them.
        let buttons: Vec<AnyElement> = match mac {
            true => groups.into_iter().map(|group| header::mac_capsule(group, cx).into_any_element()).collect(),
            false => {
                let line = || div().flex_none().w(px(1.)).h(px(18.)).mx_1().bg(cx.theme().border).into_any_element();
                let mut all = Vec::new();
                for (at, group) in groups.into_iter().enumerate() {
                    if at > 0 {
                        all.push(line());
                    }
                    all.extend(group);
                }
                all
            }
        };
        // A press on it must not start moving the window from the title bar.
        let bar = h_flex().id("toolbar").when(mac, |bar| bar.gap_2()).when(!mac, |bar| bar.gap_0p5()).px_1().py_0p5().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        Some(bar.children(buttons).into_any_element())
    }

    fn track_place(&mut self, window: &Window, cx: &App) {
        self.bounds = kept_bounds(window, cx);
        self.display = display_uuid(window, cx);
    }

    /// Settings, on the page named `page`.
    pub(crate) fn open_settings_at(&mut self, page: &str, window: &mut Window, cx: &mut Context<Self>) {
        let page = settings::PAGES.iter().position(|&name| name == page).unwrap_or(0);
        dialogs::settings_dialog::open(cx.entity().downgrade(), self.settings.clone(), page, window, cx);
    }

    fn session(&self) -> Session {
        Session {
            open: self.open_tabs(),
            columns: self.columns.to_vec(),
            split: self.split,
            sidebar: self.sidebar.tab,
            sidebar_width: self.sidebar.width,
            vertical: self.vertical,
            terminal_share: self.terminal_share,
            sync_browsing: self.sync_browsing,
            window: Some(window_place(self.bounds, self.display.clone())),
            commands: self.commands.clone(),
        }
    }

    fn rename_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (side, tab) = (self.active, self.panels[self.active].tabs.active());
        let name = self.panels[side].tabs.current().name();
        let commander = cx.entity().downgrade();
        dialogs::name_dialog::open(
            sc_core::t!("tabs.rename.title"),
            sc_core::t!("tabs.rename.label"),
            name,
            window,
            cx,
            move |name, _, cx| {
                let _ = commander.update(cx, |this, cx| {
                    this.panels[side].tabs.rename(tab, &name);
                    cx.notify();
                });
            },
        );
    }

    fn copy_tab_to_other_panel(&mut self, cx: &mut Context<Self>) {
        let copy = self.panels[self.active].tabs.current().clone();
        let other = &mut self.panels[1 - self.active];
        other.tabs.add(copy);
        other.scroll_to_cursor();
        cx.notify();
    }

    fn reopen_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (active, filter) = (self.active, self.settings.filter());
        let side = &mut self.panels[active];
        match side.tabs.reopen(ix) {
            Err(err) => window.push_notification(Notification::error(sc_core::t!("tabs.error.reopen", err = err)), cx),
            // A server's: a tab of home's meanwhile, its server's once connected.
            Ok(Some(state)) => {
                side.tabs.add(home_panel(filter));
                let tab = side.tabs.active();
                self.reconnect_tabs(active, vec![(tab, state)], window, cx);
            }
            Ok(None) => {}
        }
        let side = &mut self.panels[active];
        side.scroll_to_cursor();
        cx.notify();
    }

    /// Alt+Down: the tab's last places, newest first, the one it is at checked.
    fn show_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let history = &self.panels[self.active].tabs.current().history;
        let places: Vec<(usize, String, bool)> = history.places.iter().enumerate().rev().map(|(ix, place)| (ix, menus::place_label(place), ix == history.at)).collect();
        if places.is_empty() {
            return window.push_notification(Notification::info(sc_core::t!("tabs.history.empty")), cx);
        }
        let focus = self.focus.clone();
        let list = PopupMenu::build(window, cx, move |mut menu, _, _| {
            for (ix, place, current) in places {
                menu = menu.menu_with_check(place, current, Box::new(GoToHistory(ix)));
            }
            menu.action_context(focus)
        });
        self.show_panel_list(list, window, cx);
    }

    fn show_tab_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tabs = &self.panels[self.active].tabs;
        let titles: Vec<(String, bool)> = tabs
            .panels()
            .iter()
            .enumerate()
            .map(|(ix, panel)| {
                (
                    format!("{}: {}", ix + 1, panel.title()),
                    ix == tabs.active(),
                )
            })
            .collect();
        let focus = self.focus.clone();
        let list = PopupMenu::build(window, cx, move |mut menu, _, _| {
            for (ix, (title, current)) in titles.into_iter().enumerate() {
                menu = menu.menu_with_check(title, current, Box::new(SelectTab(ix)));
            }
            menu.action_context(focus)
        });
        self.show_panel_list(list, window, cx);
    }

    /// A letter typed in a list over a panel: the next item of that letter
    /// highlighted, round to the first; a Windows drive letter opens the drive.
    fn panel_list_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let held = keystroke.modifiers;
        if held.control || held.alt || held.platform || held.function {
            return;
        }
        let mut chars = keystroke.key_char.as_deref().unwrap_or_default().chars();
        let (Some(typed), None) = (chars.next(), chars.next()) else {
            return;
        };
        let typed = typed.to_ascii_lowercase();
        let found: Vec<(usize, bool)> = self.panel_letters.iter().filter(|(letter, ..)| *letter == typed).map(|&(_, ix, opens)| (ix, opens)).collect();
        let Some((_, list, _)) = self.panel_list.as_ref().filter(|_| !found.is_empty()) else {
            return;
        };
        cx.stop_propagation();
        let after = list.read(cx).highlighted();
        let (ix, opens) = found.iter().find(|(ix, _)| Some(*ix) > after).unwrap_or(&found[0]).to_owned();
        // After this update: opening goes back to the Commander.
        let list = list.clone();
        window.defer(cx, move |window, cx| {
            list.update(cx, |menu, cx| match opens {
                true => menu.pick(ix, window, cx),
                false => menu.highlight(ix, cx),
            })
        });
    }

    /// A list over the active panel: its tabs, its history.
    pub(crate) fn show_panel_list(&mut self, list: Entity<PopupMenu>, window: &mut Window, cx: &mut Context<Self>) {
        let dismissed = cx.subscribe_in(&list, window, |this, _, _: &DismissEvent, window, cx| {
            this.panel_list = None;
            window.focus(&this.focus, cx);
            cx.notify();
        });
        list.read(cx).focus_handle(cx).focus(window, cx);
        self.panel_list = Some((self.active, list, dismissed));
        self.panel_letters.clear();
        cx.notify();
    }

    fn change_tabs(&mut self, change: impl FnOnce(&mut Tabs), cx: &mut Context<Self>) {
        let side = &mut self.panels[self.active];
        change(&mut side.tabs);
        side.scroll_to_cursor();
        cx.notify();
    }

    fn select_tab(&mut self, ix: usize, tab: usize, cx: &mut Context<Self>) {
        self.active = ix;
        let side = &mut self.panels[ix];
        side.tabs.select(tab);
        side.scroll_to_cursor();
        cx.notify();
    }

    fn page(&self) -> isize {
        self.panels[self.active].page_step()
    }

    fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        let side = &mut self.panels[self.active];
        side.tabs.current_mut().set_view(view);
        side.scroll_to_cursor();
        cx.notify();
    }

    /// On a server, in the background.
    /// Enter: into folders and archives; files open with their program.
    fn enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        // macOS's applications run, but where they are only folders: in archives, on servers.
        let local = panel.remote().is_none() && !panel.in_archive();
        let file = panel.current_entry().is_some_and(|e| {
            (!e.is_dir || e.is_app() && local) && (panel.in_archive() || !archive::is_archive(&e.name))
        });
        let archive = !file && panel.current_entry().is_some_and(|e| !e.is_dir && archive::is_archive(&e.name));
        match file {
            true => self.launch(window, cx),
            false if archive => self.open_archive(self.active, window, cx),
            false => self.navigate(|panel| panel.open().map(drop), window, cx),
        }
    }

    /// The archive at panel `ix`'s cursor lists in the background: a big compressed
    /// tar is unpacked whole for it. One that takes a while shows its progress, to cancel.
    fn open_archive(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[ix].tabs.current();
        let name = panel.current_entry().map(|e| e.name.to_string_lossy().into_owned()).unwrap_or_default();
        let watch = archive::Watch::default();
        let done = Arc::new(AtomicBool::new(false));
        let (listing, finished) = (watch.clone(), done.clone());
        self.navigate_on(
            ix,
            true,
            move |panel| {
                let opened = panel.open_watched(&listing).map(drop);
                finished.store(true, Ordering::Relaxed);
                opened
            },
            window,
            cx,
        );
        cx.spawn_in(window, async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(400)).await;
            if !done.load(Ordering::Relaxed) {
                let _ = cx.update(|window, cx| panel::opening::show(name, watch, done, window, cx));
            }
        })
        .detach();
    }

    /// A server's or an archive's file opens from a copy, and is never run.
    fn launch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        let Some(entry) = panel.current_entry() else {
            return;
        };
        // An application is a folder, which has no origin of a file: its own.
        let origin = match entry.is_app() {
            true => Some(origin::Origin::Local(panel.folder().join(&entry.name))),
            false => panel.origin(),
        };
        let Some(origin) = origin else {
            return;
        };
        // An application is a folder: the system runs it.
        let executable = origin.is_local() && entry.attributes.contains('x') && !entry.is_app();
        let associations = self.settings.associations.clone();
        let name = origin.name();
        cx.spawn_in(window, async move |_, cx| {
            let opened = cx
                .background_executor()
                .spawn(async move {
                    let file = origin.view()?;
                    sc_core::associations::open(&associations, &file, executable)
                })
                .await;
            if let Err(err) = opened {
                let _ = cx.update(|window, cx| {
                    window.push_notification(Notification::error(format!("{name}: {err}")), cx)
                });
            }
        })
        .detach();
    }

    fn navigate(
        &mut self,
        go: impl FnOnce(&mut Panel) -> io::Result<()> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote = self.panels[self.active].tabs.current().remote().is_some();
        self.navigate_on(self.active, remote, go, window, cx);
    }

    /// `background` for a server: the move runs on a copy of the panel,
    /// which then takes the tab's place. Locally it is instant, and keys
    /// pressed meanwhile must not be lost.
    fn navigate_on(
        &mut self,
        ix: usize,
        background: bool,
        go: impl FnOnce(&mut Panel) -> io::Result<()> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let side = &mut self.panels[ix];
        let before = side.tabs.current().spot();
        if !background {
            match side.tabs.navigate(go) {
                Ok(()) => self.follow(ix, before, window, cx),
                Err(err) => self.move_failed(ix, err, window, cx),
            }
            self.panels[ix].scroll_to_cursor();
            return cx.notify();
        }
        let switches = self.switches;
        let mut moved = side.tabs.start_move();
        let mark = moved.mark();
        cx.spawn_in(window, async move |this, cx| {
            let went = cx
                .background_executor()
                .spawn(async move { go(&mut moved).map(|()| moved) });
            let went = went.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.switches != switches {
                    return;
                }
                // Its tab closed, or moved again since: the newer move stands.
                let Some(tab) = this.panels[ix].tabs.place_of(mark) else {
                    return;
                };
                match went {
                    Ok(moved) => {
                        this.panels[ix].tabs.navigated(tab, moved);
                        this.follow(ix, before, window, cx);
                    }
                    // Cancelled: as it was.
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                    Err(err) => this.move_failed(ix, err, window, cx),
                }
                this.panels[ix].scroll_to_cursor();
                cx.notify();
            });
        })
        .detach();
    }

    fn move_failed(&mut self, ix: usize, err: io::Error, window: &mut Window, cx: &mut Context<Self>) {
        let mut text = format!("{}: {err}", self.panels[ix].tabs.current().location());
        // The other panel could not follow.
        if self.sync_browsing && ix != self.active {
            text = sc_core::t!("main.sync_browsing.lost", err = err);
            self.set_sync_browsing(false, cx);
        }
        window.push_notification(Notification::error(text), cx);
        if sidebar::lost_device(&err) {
            self.reconnect_devices(window, cx);
        }
    }

    fn set_sync_browsing(&mut self, on: bool, cx: &mut Context<Self>) {
        self.sync_browsing = on;
        self.reload_menus(cx);
        cx.notify();
    }

    /// Synchronized browsing: the other panel makes panel `ix`'s move from
    /// `before` too, when both show folders on the same computer or server.
    fn follow(&mut self, ix: usize, before: Option<(Option<String>, PathBuf)>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.sync_browsing || ix != self.active {
            return;
        }
        let other = self.panels[1 - ix].tabs.current();
        let (Some((server, from)), Some((server_now, to)), Some(_)) =
            (before, self.panels[ix].tabs.current().spot(), other.spot())
        else {
            return;
        };
        if server != server_now || from == to {
            return;
        }
        // shortcut: on a server it follows from where it was before its last follow ended; fine unless keys outrun the server.
        let remote = other.remote().is_some();
        self.navigate_on(1 - ix, remote, move |panel| panel.follow(&from, &to), window, cx);
    }

    /// A panel, or Quick View where it covers it.
    fn render_side(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        match self.quick.as_ref().is_some_and(|quick| quick.side == ix) {
            true => {
                let tab_bar = self.render_tab_bar(ix, cx);
                div().size_full().children(self.render_quick_view(self.quick_keys, tab_bar, cx)).into_any_element()
            }
            false => self.render_panel(ix, cx).into_any_element(),
        }
    }

    /// Panel `ix`'s folder tabs, when it has more than one or the setting
    /// shows them always; above Quick View too, as Total Commander's.
    fn render_tab_bar(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let side = &self.panels[ix];
        if side.tabs.panels().len() < 2 && !self.settings.tab_header_always.0 {
            return None;
        }
        let commander = cx.entity();
        let focus = self.focus.clone();
        let tabs = side.tabs.panels().iter().enumerate().map(|(tab, panel)| {
            // Right-click acts on the clicked tab.
            Tab::new().label(panel.title()).on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    this.select_tab(ix, tab, cx);
                    window.focus(&this.focus, cx);
                }),
            )
        });
        let bar = TabBar::new(("tabs", ix))
            .small()
            .selected_index(side.tabs.active())
            .children(tabs)
            .on_click(cx.listener(move |this, tab: &usize, window, cx| {
                this.select_tab(ix, *tab, cx);
                window.focus(&this.focus, cx);
            }));
        let new = Button::new(("tab-new", ix)).icon(IconName::Plus).tooltip(sc_core::t!("tabs.new")).on_click(cx.listener(move |this, _, window, cx| {
            this.active = ix;
            this.change_tabs(Tabs::duplicate, cx);
            window.focus(&this.focus, cx);
        }));
        Some(
            div()
                .child(appearance::tab_bar_with_new(bar, new, cx))
                .context_menu(move |menu, window, cx| {
                    let tabs = &commander.read(cx).panels[ix].tabs;
                    let locked = tabs.current().is_locked();
                    let closed = tabs.closed().iter().map(|closed| closed.label().into()).collect();
                    menus::context_menu(
                        menu.action_context(focus.clone()),
                        TAB_MENU,
                        |c| c == Command::LockTab && locked,
                        closed,
                        window,
                        cx,
                    )
                }),
        )
        .map(IntoElement::into_any_element)
    }

    fn render_panel(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let tab_bar = self.render_tab_bar(ix, cx);
        let theme = cx.theme();
        let side = &self.panels[ix];
        let panel = side.tabs.current();
        let active = ix == self.active && !self.quick_keys;
        let (title_bg, title_fg, stripe) = if active {
            (self.colors.active_path, theme.foreground, theme.primary)
        } else {
            (self.colors.inactive_path, theme.muted_foreground, self.colors.inactive_path)
        };
        let sort = panel.sort();
        let colors = self.colors;
        let accent = theme.primary;
        let title = |id: &'static str, label: String, key: SortKey, right: bool| {
            let sorted = sort.key == key;
            let arrow = match sort.descending {
                true => IconName::ArrowDown,
                false => IconName::ArrowUp,
            };
            h_flex()
                .id((id, ix))
                .h_full()
                .px_1()
                .gap_1()
                .cursor_pointer()
                .border_r_1()
                .border_color(colors.header_border)
                .on_click(cx.listener(move |this, _, _, cx| this.sort_by_column(ix, key, cx)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .when(right, |el| el.text_right())
                        .child(label),
                )
                .when(sorted, |el| el.child(Icon::new(arrow).small().text_color(accent)))
        };
        let [name, ext, size, date, attr] = self.columns.map(px);
        let date = date.max(date_fits(cx));
        let (scrolled, wide) = {
            let scroll = &side.scroll.0.borrow().base_handle;
            (panel::views::scrolled_x(scroll), scroll.max_offset().x > px(0.))
        };
        // A column's right edge, past the columns left of it.
        let handles = (0..5).map(|column| {
            let handle = div()
                .id(("resize", ix * 5 + column))
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(6.))
                .occlude()
                .cursor(CursorStyle::ResizeColumn)
                .on_drag(ColumnDrag { panel: ix, column }, |_, _, _, cx| {
                    cx.new(|_| EmptyView)
                });
            let left: f32 = self.columns[..=column].iter().sum();
            handle.left(scrolled + px(left - 3.))
        });
        let column_titles = h_flex()
            .h(ROW + px(4.))
            .pl_1()
            .pr(px(4.) + SCROLLBAR)
            .bg(theme.list_head)
            .border_x_1()
            .border_b_1()
            .border_color(colors.header_border)
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<ColumnDrag>, _, cx| {
                    let drag = event.drag(cx);
                    if drag.panel != ix {
                        return;
                    }
                    let column = drag.column;
                    let scrolled = panel::views::scrolled_x(&this.panels[ix].scroll.0.borrow().base_handle);
                    let widths = &mut this.columns;
                    let left = event.bounds.left() + ROW_INSET + scrolled;
                    let left = left + px(widths[..column].iter().sum());
                    let width = f32::from(event.event.position.x - left);
                    widths[column] = width.clamp(COLUMN_MIN, 2000.);
                    cx.notify();
                }),
            )
            .child(
                div()
                    .relative()
                    .size_full()
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .size_full()
                            .ml(scrolled)
                            .child(title("Name", sc_core::t!("panel.column.name"), SortKey::Name, false).w(name).flex_none())
                            .child(title("Ext", sc_core::t!("panel.column.ext"), SortKey::Extension, false).w(ext).flex_none())
                            .child(title("Size", sc_core::t!("panel.column.size"), SortKey::Size, true).w(size).flex_none())
                            .child(title("Date", sc_core::t!("panel.column.date"), SortKey::Modified, false).w(date).flex_none())
                            .child(title("Attr", sc_core::t!("panel.column.attr"), SortKey::Attributes, false).w(attr).flex_none()),
                    )
                    .children(handles),
            );
        let editing = self
            .path_edit
            .as_ref()
            .filter(|(side, ..)| *side == ix)
            .map(|(_, input, _)| input.clone());
        let rows = uniform_list(
            ("panel", ix),
            panel.entries().len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range.map(|row| this.render_row(ix, row, cx)).collect()
            }),
        )
        .track_scroll(&side.scroll)
        .size_full()
        .pr(SCROLLBAR)
        // Columns wider than the panel scroll sideways, under a scrollbar of their own.
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .when(wide, |list| list.pb(SCROLLBAR));
        // The list paints its rows over its padding too; these keep it under the scrollbars clear.
        let gutter = || div().absolute().right_0().bottom_0().bg(theme.background);
        let rows = match panel.view() {
            View::Full => div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(panel::views::measure(side.area.clone(), cx.entity_id()))
                .child(rows)
                .child(panel::views::follow_scroll(side.scroll.0.borrow().base_handle.clone(), scrolled, cx.entity_id()))
                .child(gutter().top_0().w(SCROLLBAR))
                .when(wide, |el| el.child(gutter().left_0().h(SCROLLBAR)))
                .child(div().absolute().inset_0().child(Scrollbar::new(&side.scroll)))
                .into_any_element(),
            View::Brief => self.render_brief(ix, cx),
            View::Thumbnails => self.render_thumbnails(ix, cx),
        };
        #[cfg(target_os = "macos")]
        let rows = self.with_file_menu(ix, rows, cx);
        #[cfg(windows)]
        let rows = self.with_shell_menu(ix, rows, cx);
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let rows = self.with_desktop_menu(ix, rows, cx);
        let crumbs = self.render_crumbs(ix, &panel.location(), cx);
        let theme = cx.theme();
        let panel_list = self.panel_list.as_ref().filter(|(side, ..)| *side == ix);
        let ring = theme.ring;
        v_flex()
            .size_full()
            .min_w_0()
            .border_1()
            .border_color(theme.border)
            // A click anywhere in it makes it the active one, as Total Commander's.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.active = ix;
                    window.focus(&this.focus, cx);
                    cx.notify();
                }),
            )
            .drag_over::<Dragged>(move |style, _, _, _| style.border_color(ring))
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.border_color(ring))
            .on_drop(cx.listener(move |this, dragged: &Dragged, window, cx| {
                this.drop_files(*dragged, ix, None, window, cx)
            }))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                this.drop_paths(paths, ix, None, window, cx)
            }))
            .children(tab_bar)
            // Under the tabs, as Total Commander's drive list: an overlay alone
            // starts at the panel's top, so a box of no height holds its place.
            .when_some(panel_list, |el, (_, list, _)| {
                el.child(div().h_0().child(deferred(
                    anchored()
                        .snap_to_window_with_margin(px(8.))
                        .child(div().occlude().on_key_down(cx.listener(Self::panel_list_key)).child(list.clone())),
                )))
            })
            .child(match editing {
                Some(input) => div()
                    .key_context("PathEdit")
                    .on_action(cx.listener(|this, _: &CancelPathEdit, window, cx| {
                        this.cancel_path_edit(window, cx)
                    }))
                    // The field passes its Enter on: the panel's would open what the cursor is on too.
                    .on_action(|_: &gpui_kit::component::input::Enter, _, _| {})
                    // Not the panel's: a click in the field would focus the panel and end the edit.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    // Input::h sizes only multi-line boxes.
                    .child(Styled::h(Input::new(&input).small(), PATH_HEIGHT))
                    .into_any_element(),
                None => div()
                    .id(("path", ix))
                    .h(PATH_HEIGHT)
                    .flex()
                    .items_center()
                    .px_2()
                    .border_l(px(3.))
                    .border_color(stripe)
                    .bg(title_bg)
                    .text_color(title_fg)
                    .cursor_text()
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.edit_path(ix, window, cx)),
                    )
                    // Search results say so, as Total Commander's listbox.
                    .children(panel.found_label().map(|label| div().flex_none().pr_2().font_weight(FontWeight::BOLD).child(found_label(label))))
                    .child(crumbs)
                    // The filter stays in sight however long the path.
                    .children(filter_suffix(panel).map(|filter| div().flex_none().child(filter)))
                    .when(self.sync_browsing, |el| {
                        el.child(
                            Button::new(("sync", ix))
                                .ghost()
                                .xsmall()
                                .icon(gpui_kit::assets::IconName::Link)
                                .tooltip(sc_core::t!("panel.sync_browsing.stop"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.set_sync_browsing(false, cx);
                                })),
                        )
                    })
                    // A server's, a share mounted (SMB, WebDAV, NFS), or a phone's KDE Connect mounted.
                    .when(panel.remote().is_some() || self.device_of(panel.folder()).is_some() || sc_core::remote::mounts::share_root(panel.folder()).is_some(), |el| {
                        el.child(
                            Button::new(("disconnect", ix))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .tooltip(sc_core::t!("panel.disconnect"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.active = ix;
                                    this.disconnect(window, cx);
                                })),
                        )
                    })
                    .into_any_element(),
            })
            .child(column_titles)
            .child(rows)
            .child(
                div()
                    .px_2()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(match self.typed(ix) {
                        // Quick search, or the quick filter: a field where the status was, as
                        // Total Commander's "Search:", in the accent's colour to be seen.
                        Some(typed) => {
                            let filter = self.filtering(ix);
                            let (icon, label) = match filter {
                                true => (Icon::new(gpui_kit::assets::IconName::Funnel), sc_core::t!("panel.quick_filter.label")),
                                false => (Icon::new(IconName::Search), sc_core::t!("panel.quick_search.label")),
                            };
                            let field = h_flex()
                                .flex_1()
                                .min_w_0()
                                .h(px(18.))
                                .px_1p5()
                                .rounded(theme.radius)
                                .border_1()
                                .border_color(theme.primary)
                                .bg(theme.background)
                                .text_color(theme.foreground)
                                .child(typed.to_owned())
                                // The caret.
                                .child(div().flex_none().w(px(1.5)).h(px(13.)).bg(theme.primary))
                                .when(filter && typed.is_empty(), |el| {
                                    el.child(div().pl_1p5().truncate().text_color(theme.muted_foreground).child(sc_core::t!("panel.quick_filter.hint")))
                                });
                            h_flex()
                                .gap_1p5()
                                .child(icon.small().text_color(theme.primary))
                                .child(div().flex_none().font_weight(FontWeight::MEDIUM).text_color(theme.foreground).child(label))
                                .child(field)
                                .into_any_element()
                        }
                        None => div().truncate().child(match panel.view() {
                        // Names only: the line tells the rest, as Total Commander's.
                        View::Brief => panel.current_entry().map_or_else(String::new, |entry| {
                            let size = panel.size_text(entry);
                            format!("{}   {size}   {}   {}", entry.display_name(), entry.modified_text(), entry.attributes)
                        }),
                        View::Full | View::Thumbnails => panel.status_text(),
                    })
                    .into_any_element(),
                    }),
            )
    }

    fn render_row(&self, ix: usize, row: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let panel = self.panels[ix].tabs.current();
        let entry = &panel.entries()[row];
        let icon = self.entry_icon(ix, entry, px(16.), cx);
        let (name, extension) = entry.name_and_extension();
        let [name_width, ext, size, date, attr] = self.columns.map(px);
        let date = date.max(date_fits(cx));
        // Shift+F6 edits the name in the row, over the Ext column too.
        let cells = match self.rename_cell(ix, row, name_width + ext, icon, cx) {
            Ok(field) => vec![field],
            Err(icon) => vec![
                h_flex()
                    .w(name_width)
                    .flex_none()
                    .px_1()
                    .gap_1()
                    .child(icon)
                    .child(div().flex_1().min_w_0().truncate().child(name))
                    .child(tag_dots(&entry.tags, cx.theme().background))
                    .when(entry.online, |el| {
                        let cloud = Icon::new(gpui_kit::assets::IconName::Cloud).xsmall();
                        el.child(cloud.text_color(cx.theme().muted_foreground))
                    })
                    .into_any_element(),
                div().w(ext).flex_none().px_1().truncate().child(extension).into_any_element(),
            ],
        };
        self.entry_base(ix, row, true, cx)
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(ROW)
            .px_1()
            .children(cells)
            .child(
                div()
                    .w(size)
                    .flex_none()
                    .px_1()
                    .text_right()
                    .truncate()
                    .child(panel.size_text(entry)),
            )
            .child(
                div()
                    .w(date)
                    .flex_none()
                    .px_1()
                    .truncate()
                    .child(entry.modified_text()),
            )
            .child(
                div()
                    .w(attr)
                    .flex_none()
                    .px_1()
                    .truncate()
                    .child(entry.attributes.clone()),
            )
    }

    /// An entry as every view has it: clicked, dragged, dropped onto; the
    /// cursor's frame, marked names' colour; `striped` every other one.
    fn entry_base(&self, ix: usize, row: usize, striped: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let panel = self.panels[ix].tabs.current();
        let entry = &panel.entries()[row];
        let cursor = row == panel.cursor() && ix == self.active && !self.quick_keys;
        let marked = panel.is_marked(entry);
        let (labels, payloads) = (cx.entity(), cx.entity());
        let target = cx.theme().accent.opacity(0.35);
        div()
            .id(("row", row))
            // A middle click opens a folder in a new tab, as a browser's link.
            .when(entry.is_dir && !entry.is_parent(), |el| {
                el.on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |this, _, window, cx| {
                        this.active = ix;
                        this.panels[ix].tabs.current_mut().set_cursor(row);
                        this.open_in_new_tab(false, window, cx);
                    }),
                )
            })
            .on_drag(Dragged { side: ix, row }, move |dragged, grab, _, cx| {
                let text = labels.read(cx).drag_label(*dragged);
                cx.new(|_| DragLabel { text, grab })
            })
            .external_drag_payload(move |dragged: &Dragged, _, cx| {
                payloads.read(cx).drag_payload(*dragged)
            })
            // A folder takes what is dropped onto it.
            .when(entry.is_dir, |el| {
                el.drag_over::<Dragged>(move |style, _, _, _| style.bg(target))
                    .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(target))
                    .on_drop(cx.listener(move |this, dragged: &Dragged, window, cx| {
                        this.drop_files(*dragged, ix, Some(row), window, cx)
                    }))
                    .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                        this.drop_paths(paths, ix, Some(row), window, cx)
                    }))
            })
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .when(striped && row % 2 == 1, |el| el.bg(self.colors.alternate_row))
            .when(cursor, |el| {
                let fill = if self.window_active { self.colors.cursor } else { self.colors.cursor_inactive };
                el.bg(fill).border_dashed().border_color(self.colors.cursor_frame)
            })
            .when_some(self.colors.cursor_text.filter(|_| cursor && self.window_active), |el, text| el.text_color(text))
            .when(marked, |el| el.text_color(self.colors.marked))
            // On press, as Total Commander: the panel turns active with the cursor already here.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.active = ix;
                    this.panels[ix].tabs.current_mut().set_cursor(row);
                    cx.notify();
                }),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() == 2 {
                    this.enter(window, cx);
                }
            }))
            // The menu is for the row right-clicked: Finder's (file_menu), Explorer's (context_menu), the desktop's (desktop_menu).
            .map(|el| {
                let el = el.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, _, window, cx| {
                        this.right_click(ix, row, window, cx);
                        // Elsewhere the panel's own menu opens as the click goes on.
                        #[cfg(target_os = "macos")]
                        {
                            this.show_file_menu(ix, window, cx);
                            cx.stop_propagation();
                        }
                    }),
                );
                #[cfg(windows)]
                let el = el.on_mouse_up(
                    MouseButton::Right,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        // A press that closed an open menu never reached the row.
                        this.right_click(ix, row, window, cx);
                        this.shell_menu(ix, Some(row), window, cx)
                    }),
                );
                el
            })
    }
}

impl Render for Commander {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_quick_view(window, cx);
        // Go > Back and Forward list the active tab's places, wherever it went; Branch View's tick
        // follows its tab too, however the branch view came (walked in the background) or went.
        let history = menus::set_history(&self.panels[self.active].tabs.current().history);
        let branch = self.checked(Command::BranchView);
        let branch_changed = branch != std::mem::replace(&mut self.branch_ticked, branch);
        if history || branch_changed {
            self.reload_menus(cx);
        }
        #[cfg(target_os = "macos")]
        menus::show(window, menus::main_menus(self.settings.appearance, &|command| self.checked(command)), cx);
        self.quick_keys = self.quick_focused(window, cx);
        self.window_active = window.is_window_active();
        self.trim_thumbnails(window);
        let vertical = self.vertical;
        // Keeps panel keys out of the menu bar.
        // Typing a path, the panels' keys rest.
        let panels = div()
            .flex()
            .map(|el| if vertical { el.flex_col() } else { el.flex_row() })
            .when(self.path_edit.is_none(), |el| el.key_context(CONTEXT))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::quick_search))
            .relative()
            .flex_1()
            .min_h_0()
            .p_1()
            .on_action(cx.listener(|this, _: &SwitchPanel, window, cx| {
                if !this.tab_quick_view(window, cx) {
                    this.active = 1 - this.active;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Up, _, cx| {
                this.move_cursor(-this.panels[this.active].vertical_step(), cx)
            }))
            .on_action(cx.listener(|this, _: &Down, _, cx| {
                this.move_cursor(this.panels[this.active].vertical_step(), cx)
            }))
            .on_action(cx.listener(|this, _: &MarkUp, _, cx| {
                let step = this.panels[this.active].vertical_step();
                this.mark(|panel| panel.mark_and_move(-step), cx)
            }))
            .on_action(cx.listener(|this, _: &MarkDown, _, cx| {
                let step = this.panels[this.active].vertical_step();
                this.mark(|panel| panel.mark_and_move(step), cx)
            }))
            .on_action(cx.listener(|this, _: &Left, _, cx| {
                if let Some(step) = this.panels[this.active].sideways_step() {
                    this.move_cursor(-step, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &Right, _, cx| {
                if let Some(step) = this.panels[this.active].sideways_step() {
                    this.move_cursor(step, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| this.move_cursor(-this.page(), cx)))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| this.move_cursor(this.page(), cx)))
            .on_action(cx.listener(|this, _: &Home, _, cx| this.move_cursor(isize::MIN, cx)))
            .on_action(cx.listener(|this, _: &End, _, cx| this.move_cursor(isize::MAX, cx)))
            .on_action(cx.listener(|this, _: &Open, window, cx| this.enter(window, cx)))
            .on_action(
                cx.listener(|this, _: &Parent, window, cx| {
                    // Backspace's binding comes before the key reaches the quick search.
                    if this.typed(this.active).is_some() {
                        return this.quick_back(window, cx);
                    }
                    this.navigate(Panel::up, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, Run(command): &Run, window, cx| match command {
                    Command::DuplicateTab => this.change_tabs(Tabs::duplicate, cx),
                    Command::LockTab => this.change_tabs(Tabs::toggle_lock, cx),
                    Command::CloseTab => this.change_tabs(Tabs::close, cx),
                    Command::NextTab => this.change_tabs(|tabs| tabs.step(1), cx),
                    Command::PreviousTab => this.change_tabs(|tabs| tabs.step(-1), cx),
                    Command::CloseAllTabs => this.change_tabs(Tabs::close_all, cx),
                    Command::CloseDuplicateTabs => this.change_tabs(Tabs::close_duplicates, cx),
                    Command::RenameTab => this.rename_tab(window, cx),
                    Command::CopyTabToOtherPanel => this.copy_tab_to_other_panel(cx),
                    Command::ListTabs => this.show_tab_list(window, cx),
                    Command::FolderHistory => this.show_history(window, cx),
                    Command::NewFolder => this.new_folder(window, cx),
                    #[cfg(any(target_os = "linux", target_os = "freebsd", windows))]
                    Command::OpenTerminal => this.open_terminal(window, cx),
                    Command::CalculateOccupiedSpace => this.calculate_space(window, cx),
                    Command::AllFolderSizes => this.all_folder_sizes(window, cx),
                    Command::CreateLink => this.create_link(window, cx),
                    Command::ChangeAttributes => this.change_attributes(window, cx),
                    Command::Properties => this.show_properties(window, cx),
                    Command::Pack => this.pack(window, cx),
                    Command::UnpackSpecificFiles => this.unpack(window, cx),
                    Command::TestArchives => this.test_archives(window, cx),
                    Command::CreateChecksumFiles => this.create_checksums(window, cx),
                    Command::VerifyChecksums => this.verify_checksums(window, cx),
                    Command::SplitFile => this.split_file(window, cx),
                    Command::MultiRenameTool => this.multi_rename(window, cx),
                    Command::CompareByContent => this.compare_files(window, cx),
                    Command::Search => this.find_files(false, window, cx),
                    Command::SearchInSeparateProcess => this.find_files(true, window, cx),
                    Command::SynchronizeDirs => this.synchronize_dirs(window, cx),
                    Command::ManageTools => this.manage_tools(window, cx),
                    Command::AssociateWith => this.associate_with(window, cx),
                    Command::InternalAssociations => this.internal_associations(window, cx),
                    Command::CombineFiles => this.combine_files(window, cx),
                    Command::View => this.open_file(remote::Open::View, window, cx),
                    Command::Edit => this.open_file(remote::Open::Edit, window, cx),
                    Command::EditExternally => this.open_file(remote::Open::External, window, cx),
                    Command::Delete => this.confirm_delete(true, window, cx),
                    Command::DeletePermanently => this.confirm_delete(false, window, cx),
                    Command::ToggleMark => this.mark(Panel::toggle_mark, cx),
                    Command::MarkInPlace => this.mark_in_place(cx),
                    Command::Copy => this.transfer(ops::Kind::Copy, window, cx),
                    Command::CopyToClipboard => this.copy_files(false, window, cx),
                    Command::CutToClipboard => this.copy_files(true, window, cx),
                    Command::PasteFromClipboard => this.paste_files(window, cx),
                    Command::Move => this.transfer(ops::Kind::Move, window, cx),
                    Command::Disconnect => this.disconnect(window, cx),
                    Command::QuickViewPanel => this.toggle_quick_view(window, cx),
                    Command::TerminalPanel => this.toggle_terminal(window, cx),
                    Command::NewTerminal => this.new_terminal_tab(window, cx),
                    Command::DownloadLink => this.download_link(window, cx),
                    Command::Brief => this.set_view(View::Brief, cx),
                    Command::Full => this.set_view(View::Full, cx),
                    Command::ThumbnailView => this.set_view(View::Thumbnails, cx),
                    Command::VerticalArrangement => {
                        this.vertical = !this.vertical;
                        cx.notify()
                    }
                    Command::SidebarTree => this.toggle_sidebar(SidebarTab::Tree, window, cx),
                    Command::SidebarPlaces => this.toggle_sidebar(SidebarTab::Places, window, cx),
                    Command::GoBack => this.go_history(-1, window, cx),
                    Command::GoForward => this.go_history(1, window, cx),
                    Command::GoHome => this.go_home(window, cx),
                    Command::Bookmarks => this.show_bookmarks(window, cx),
                    Command::QuickOpen => this.quick_open(window, cx),
                    Command::LeftDrives => this.show_drives(0, window, cx),
                    Command::RightDrives => this.show_drives(1, window, cx),
                    Command::BookmarkFolder => this.bookmark_folder(window, cx),
                    Command::ManageBookmarks => this.manage_bookmarks(window, cx),
                    Command::NewWorkspace => this.new_workspace(window, cx),
                    Command::DuplicateWorkspace => this.duplicate_workspace(this.workspaces.current, window, cx),
                    Command::ManageWorkspaces => this.manage_workspaces(window, cx),
                    Command::ToggleSidebar => {
                        let tab = this.sidebar.tab.unwrap_or(this.sidebar.last);
                        this.toggle_sidebar(tab, window, cx)
                    }
                    Command::SidebarConnections => {
                        this.toggle_sidebar(SidebarTab::Connections, window, cx)
                    }
                    Command::OperationQueue => this.show_operations(window, cx),
                    Command::SidebarOperations => this.toggle_sidebar(SidebarTab::Operations, window, cx),
                    Command::SelectAll => this.mark(|panel| panel.mark_all(true), cx),
                    Command::UnselectAll => this.mark(|panel| panel.mark_all(false), cx),
                    Command::InvertSelection => this.mark(Panel::invert_marks, cx),
                    // As the column headers: again on the sorted column turns it around.
                    Command::SortByName => this.sort_by_column(this.active, SortKey::Name, cx),
                    Command::SortByExtension => this.sort_by_column(this.active, SortKey::Extension, cx),
                    Command::SortByTime => this.sort_by_column(this.active, SortKey::Modified, cx),
                    Command::SortBySize => this.sort_by_column(this.active, SortKey::Size, cx),
                    Command::Unsorted => {
                        this.set_sort(this.active, Sort::by(SortKey::Unsorted), cx)
                    }
                    Command::ReversedOrder => {
                        let sort = this.panels[this.active].tabs.current().sort();
                        let reversed = Sort {
                            descending: !sort.descending,
                            ..sort
                        };
                        this.set_sort(this.active, reversed, cx)
                    }
                    Command::Settings => {
                        let commander = cx.entity().downgrade();
                        dialogs::settings_dialog::open(commander, this.settings.clone(), 0, window, cx)
                    }
                    Command::RereadSource => this.reread_source(window, cx),
                    Command::SwapPanels => this.swap_panels(false, cx),
                    Command::SwapPanelsAndTabs => this.swap_panels(true, cx),
                    Command::TargetEqualsSource => this.target_equals_source(cx),
                    Command::SyncBrowsing => this.set_sync_browsing(!this.sync_browsing, cx),
                    Command::OpenInLeftPanel => this.show_in(0, window, cx),
                    Command::OpenInRightPanel => this.show_in(1, window, cx),
                    Command::GoToRoot => this.go_to_root(window, cx),
                    Command::OpenInNewTab => this.open_in_new_tab(false, window, cx),
                    Command::OpenInOtherNewTab => this.open_in_new_tab(true, window, cx),
                    Command::PathToCommandLine => this.to_command_line(true, false, window, cx),
                    Command::NameToCommandLine => this.to_command_line(false, true, window, cx),
                    Command::PathAndNameToCommandLine => this.to_command_line(true, true, window, cx),
                    // After a menu gives the panel its focus back, which would end the edit.
                    Command::RenameInPlace => cx.defer_in(window, |this, window, cx| this.rename_in_place(window, cx)),
                    Command::NewTextFile => this.new_text_file(window, cx),
                    Command::AllFiles => this.show_files(sc_core::FileFilter::All, window, cx),
                    Command::Programs => this.show_files(sc_core::FileFilter::Programs, window, cx),
                    Command::AllWithExtension => {
                        this.show_files(sc_core::FileFilter::Masks(menus::last_filter()), window, cx)
                    }
                    Command::CustomFilter => this.custom_filter(window, cx),
                    Command::QuickFilter => this.quick_filter(cx),
                    Command::Undo => this.undo(false, window, cx),
                    Command::Redo => this.undo(true, window, cx),
                    Command::ActivityLog => dialogs::activity_dialog::open(window, cx),
                    // Walked in the background: a big tree takes a while.
                    Command::BranchView | Command::BranchViewSelected => {
                        let selected = *command == Command::BranchViewSelected;
                        this.navigate_on(this.active, true, move |panel| panel.toggle_branch(selected), window, cx);
                    }
                    Command::HiddenFiles => {
                        // As Total Commander's button: hidden and system files together.
                        let mut settings = this.settings.clone();
                        settings.show_hidden = !settings.show_hidden;
                        settings.show_system = settings.show_hidden;
                        this.apply_settings(settings, window, cx);
                        this.reload_menus(cx);
                    }
                    Command::About => dialogs::about::open(window, cx),
                    Command::Website => cx.open_url(sc_core::about::WEBSITE),
                    Command::OnlySelectedFiles => {
                        this.show_files(sc_core::FileFilter::Selected(Default::default()), window, cx)
                    }
                    Command::Shortcuts => {
                        let commander = cx.entity().downgrade();
                        dialogs::keys_dialog::open(commander, this.settings.keys.clone(), window, cx)
                    }
                    other => {
                        if !this.mark_command(*other, window, cx) {
                            cx.propagate()
                        }
                    }
                }),
            )
            .on_action(cx.listener(|this, SelectTab(tab): &SelectTab, _, cx| {
                this.select_tab(this.active, *tab, cx)
            }))
            .on_action(cx.listener(|this, ReopenTab(ix): &ReopenTab, window, cx| {
                this.reopen_tab(*ix, window, cx)
            }))
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<SplitDrag>, _, cx| {
                    let (bounds, at) = (event.bounds, event.event.position);
                    let share = match this.vertical {
                        true => (at.y - bounds.top()) / bounds.size.height,
                        false => (at.x - bounds.left()) / bounds.size.width,
                    };
                    this.split = match (share - 0.5).abs() < SPLIT_MAGNET {
                        true => 0.5,
                        false => share.clamp(0.15, 0.85),
                    };
                    cx.notify();
                }),
            )
            .child(
                div()
                    .min_w_0()
                    .min_h_0()
                    .flex_none()
                    .map(|el| match vertical {
                        true => el.w_full().h(relative(self.split)).pb(px(2.)),
                        false => el.h_full().w(relative(self.split)).pr(px(2.)),
                    })
                    .child(self.render_side(0, cx)),
            )
            .child(
                div()
                    .min_w_0()
                    .min_h_0()
                    .flex_1()
                    .map(|el| match vertical {
                        true => el.w_full().pt(px(2.)),
                        false => el.h_full().pl(px(2.)),
                    })
                    .child(self.render_side(1, cx)),
            )
            .child(
                div()
                    .id("splitter")
                    .absolute()
                    .occlude()
                    .map(|el| match vertical {
                        true => el.left_0().right_0().top(relative(self.split)).mt(px(-3.)).h(px(6.)),
                        false => el.top_0().bottom_0().left(relative(self.split)).ml(px(-3.)).w(px(6.)),
                    })
                    .cursor(if vertical { CursorStyle::ResizeUpDown } else { CursorStyle::ResizeLeftRight })
                    .hover(|style| style.bg(cx.theme().list_active))
                    .on_drag(SplitDrag, |_, _, _, cx| cx.new(|_| EmptyView)),
            );
        #[cfg(target_os = "macos")]
        let panels = self.file_menu_actions(panels, cx);
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let panels = self.desktop_menu_actions(panels, cx);
        // In the title bar where there is one, as GNOME's apps; else over the panels.
        let mut toolbar = self.render_toolbar(cx);
        let header = self.render_header(&mut toolbar, window, cx);
        // macOS: the title bar beside the sidebar, which runs the window's height.
        let (header, beside) = match cfg!(target_os = "macos") {
            true => (None, header),
            false => (header, None),
        };
        // The terminal paints its own background, which the desktop may show through.
        let background = cx.theme().background;
        let main = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .children(beside)
            .children(toolbar.map(|bar| div().id("toolbar-row").bg(self.colors.bar).child(bar).context_menu(toolbar_menu(cx.entity().downgrade()))))
            .child(v_flex().flex_1().min_h_0().bg(background).child(panels))
            .children(self.render_terminal(window, cx))
            .child(
                v_flex()
                    // macOS: as the title bar, in the sidebar's colour.
                    .bg(match cfg!(target_os = "macos") {
                        true => self.sidebar.chrome(self.colors.window),
                        false => background,
                    })
                    .child(self.render_command_line(cx))
                    .child(self.render_button_bar(cx)),
            );
        // macOS shows the menus on the system's menu bar.
        let menu_bar = !cfg!(target_os = "macos") && !self.menus_behind_button(window);
        // The sidebar may let the desktop through: everything else paints its own background.
        v_flex()
            .size_full()
            .text_color(cx.theme().foreground)
            .text_sm()
            .children(header)
            .when(menu_bar, |el| {
                let bar = self.menu_bar.clone();
                el.on_key_down(move |event, window, cx| menus::alt_letter(&bar, event, window, cx)).child(
                    div()
                        .bg(self.colors.bar)
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(self.menu_bar.clone()),
                )
            })
            // The menus' own choices, wherever the keys are: the terminal, the sidebar.
            .on_action(cx.listener(|this, OpenWorkspace(ix): &OpenWorkspace, window, cx| {
                this.open_workspace(*ix, window, cx)
            }))
            .on_action(cx.listener(|this, RunTool(ix): &RunTool, window, cx| this.run_tool(*ix, window, cx)))
            .on_action(cx.listener(|this, GoToHistory(at): &GoToHistory, window, cx| {
                let ix = this.active;
                if let Some(place) = this.panels[ix].tabs.current_mut().history.jump(*at) {
                    this.open_location(ix, &place, window, cx);
                }
            }))
            .on_action(cx.listener(|this, SetAppearance(chosen): &SetAppearance, window, cx| {
                this.set_appearance(*chosen, window, cx)
            }))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .on_drag_move(cx.listener(
                        |this, event: &DragMoveEvent<sidebar::SidebarDrag>, _, cx| {
                            this.drag_sidebar(event.bounds, event.event.position.x, cx)
                        },
                    ))
                    .children(self.render_sidebar(cx))
                    .child(main),
            )
    }
}

/// The toolbar's right-click menu: Customize Toolbar, Settings on its page.
pub(crate) fn toolbar_menu(commander: WeakEntity<Commander>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
    move |menu, _, _| {
        let commander = commander.clone();
        let item = PopupMenuItem::new(sc_core::t!("toolbar.customize")).when_some(menus::menu_icon(Command::Settings), |item, icon| item.icon(Icon::new(icon)));
        menu.item(item.on_click(move |_, window, cx| {
            let _ = commander.update(cx, |this, cx| this.open_settings_at("Toolbar", window, cx));
        }))
    }
}

/// Where a window opens over another: its bounds, on its display. A
/// window's bounds are its display's; without the display they would be
/// taken as the primary display's, another monitor.
#[derive(Clone, Copy)]
pub(crate) struct Over {
    bounds: WindowBounds,
    display: Option<DisplayId>,
}

impl Over {
    /// As large as `window`, over it.
    pub(crate) fn window(window: &Window, cx: &App) -> Self {
        Self::at(window.window_bounds(), window, cx)
    }

    /// `bounds` on `window`'s display.
    pub(crate) fn at(bounds: WindowBounds, window: &Window, cx: &App) -> Self {
        let display = window.display(cx).map(|display| display.id());
        Over { bounds, display }
    }
}

/// The viewer's and editor's windows: where the main one is. Wayland lets
/// no app place its windows; as the main window's child, the compositor
/// centres it over the main window.
pub(crate) fn over_main_window(title: String, over: Over, cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(over.bounds),
        display_id: over.display,
        kind: match cx.compositor_name() {
            "Wayland" => WindowKind::Floating,
            _ => WindowKind::Normal,
        },
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        app_id: Some("sparkcommander".into()),
        icon: window_icon(),
        ..Default::default()
    }
}

/// The app's icon on its windows: X11's, and Wayland's where the compositor
/// takes a window's icon (KWin); other Wayland desktops find it by the app id,
/// through the installed `sparkcommander.desktop`.
pub(crate) fn window_icon() -> Option<std::sync::Arc<image::RgbaImage>> {
    static ICON: std::sync::OnceLock<Option<std::sync::Arc<image::RgbaImage>>> = std::sync::OnceLock::new();
    let png = include_bytes!("../../../packaging/linux/hicolor/256x256/apps/sparkcommander.png");
    let decode = || image::load_from_memory_with_format(png, image::ImageFormat::Png).ok();
    ICON.get_or_init(|| decode().map(|icon| std::sync::Arc::new(icon.into_rgba8()))).clone()
}

/// The system's look, light or dark as chosen; on Xfce, MATE and Cinnamon
/// always their theme's, which GTK draws only as it is.
/// The saved window place, if a display still shows it; else none.
fn placed(place: WindowPlace, cx: &App) -> Option<(WindowBounds, Option<DisplayId>)> {
    let bounds = Bounds {
        origin: point(px(place.x), px(place.y)),
        size: size(px(place.width.max(600.)), px(place.height.max(400.))),
    };
    // The display it was on, if it is still there; else any that shows it.
    let displays = cx.displays();
    let display = place.display.as_deref().and_then(|uuid| displays.iter().find(|display| display.uuid().is_ok_and(|id| id.to_string() == uuid)));
    let shown = match display {
        // macOS gives the place on the display, the others on the desktop.
        Some(display) if cfg!(target_os = "macos") => Bounds::new(point(px(0.), px(0.)), display.bounds().size).intersects(&bounds),
        Some(display) => display.bounds().intersects(&bounds),
        None => displays.iter().any(|display| display.bounds().intersects(&bounds)),
    };
    let bounds = match place.maximized {
        true => WindowBounds::Maximized(bounds),
        false => WindowBounds::Windowed(bounds),
    };
    (shown && place.width > 0.).then(|| (bounds, display.map(|display| display.id())))
}

/// Full screen comes back maximized.
fn window_place(bounds: WindowBounds, display: Option<String>) -> WindowPlace {
    let (maximized, size) = match bounds {
        WindowBounds::Windowed(size) => (false, size),
        WindowBounds::Maximized(size) | WindowBounds::Fullscreen(size) => (true, size),
    };
    let (origin, size) = (size.origin, size.size);
    WindowPlace { x: origin.x.into(), y: origin.y.into(), width: size.width.into(), height: size.height.into(), maximized, display }
}

/// The window's place to open at again: on X11 without the shadow of a frame
/// it draws, as the window manager grows it by that once it is open; Wayland's
/// keeps the size it opens at.
fn kept_bounds(window: &Window, cx: &App) -> WindowBounds {
    match cx.compositor_name() {
        "X11" => window.inner_window_bounds(),
        _ => window.window_bounds(),
    }
}

/// The UUID of the display `window` is on.
fn display_uuid(window: &Window, cx: &App) -> Option<String> {
    Some(window.display(cx)?.uuid().ok()?.to_string())
}

/// A side's tabs as saved; without them, the home folder's (or /).
/// A side's saved tabs; a server's is the home folder's until it is connected
/// again: those, by where they are.
/// A tab on the home folder, else on /.
fn home_panel(filter: sc_core::Filter) -> Panel {
    let home = std::env::home_dir().unwrap_or_else(|| "/".into());
    Panel::new(&home, filter).or_else(|_| Panel::new("/", filter)).expect("list the home folder or /")
}

fn restore_side(state: Option<&SideState>, filter: sc_core::Filter) -> (Tabs, Vec<(usize, sc_core::session::TabState)>) {
    let fallback = || home_panel(filter);
    match state {
        Some(state) => Tabs::restore_servers(state, filter, fallback),
        None => (Tabs::new(fallback()), Vec::new()),
    }
}

fn palette(settings: &Settings, window: &Window) -> theme::Palette {
    #[cfg(target_os = "macos")]
    appearance::mac_force(settings.appearance);
    let dark = theme::desktop_dark().unwrap_or_else(|| match settings.appearance {
        Appearance::System => matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark),
        Appearance::Light => false,
        Appearance::Dark => true,
    });
    let palette = theme::system(dark);
    #[cfg(target_os = "macos")]
    let palette = theme::Palette { selection: appearance::mac_selection(dark).unwrap_or(palette.selection), ..palette };
    // The main window's font, chosen in Settings > Font.
    match &settings.font {
        Some(font) => theme::Palette { font: Some(font.family.clone()), font_size: Some(theme::font_pixels(font.size)), ..palette },
        None => palette,
    }
}

/// The path's folders, each its text and where its path ends in `location`:
/// the root first with its separator (a drive, a share, a server, `/`).
fn crumbs(location: &str) -> Vec<(&str, usize)> {
    let sep = |c: char| c == '/' || c == '\\';
    let after = |from: usize, n: usize| location[from..].match_indices(sep).nth(n).map_or(location.len(), |(at, _)| from + at + 1);
    let root = match location.find("://") {
        Some(at) => after(at + 3, 0),
        None if location.starts_with("\\\\") => after(2, 1),
        None if location.starts_with(sep) => 1,
        None => location.find(sep).filter(|&at| location[..at].ends_with(':')).map_or(0, |at| at + 1),
    };
    let mut crumbs = Vec::new();
    if root > 0 {
        crumbs.push((&location[..root], root));
    }
    let mut start = root;
    for part in location[root..].split(sep) {
        let end = start + part.len();
        if !part.is_empty() {
            crumbs.push((part, end));
        }
        start = end + 1;
    }
    crumbs
}

/// The path line's start in search results or Branch View, in words.
fn found_label(label: &str) -> String {
    match label {
        "Branch view:" => sc_core::t!("panel.found.branch_view"),
        _ => sc_core::t!("panel.found.search_results"),
    }
}

/// The path line's end: the panel's filter, as Total Commander's
/// `c:\docs\*.txt`; one in words in brackets.
fn filter_suffix(panel: &Panel) -> Option<String> {
    let filter = panel.file_filter().label()?;
    if !filter.contains(['*', '?']) {
        return Some(format!("  ({filter})"));
    }
    match panel.location().ends_with(['/', '\\']) {
        true => Some(filter),
        false => Some(format!("{}{filter}", std::path::MAIN_SEPARATOR)),
    }
}

/// The Date column's least width, for the date format's longest text: an
/// estimate, digits being about 0.58 of the text's size.
fn date_fits(cx: &App) -> Pixels {
    let text = f32::from(cx.theme().font_size) * 0.875;
    px(sc_core::dates::widest().chars().count() as f32 * text * 0.58 + 10.)
}

/// Finder's tag dots: overlapping, the first on the right, three at most.
fn tag_dots(tags: &[u8], edge: Hsla) -> Div {
    h_flex().flex_none().children(tags.iter().take(3).rev().enumerate().filter_map(|(n, &colour)| {
        let [r, g, b] = sc_core::system::tags::rgb(colour)?;
        let dot = div().size(px(10.)).rounded_full().border_1().border_color(edge);
        Some(dot.bg(rgb(u32::from_be_bytes([0, r, g, b]))).when(n > 0, |dot| dot.ml(px(-4.))))
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn crumbs() {
        let shown = |location| super::crumbs(location).into_iter().map(|(name, end)| (name, &location[..end])).collect::<Vec<_>>();
        assert_eq!(shown(r"C:\Users\me"), [(r"C:\", r"C:\"), ("Users", r"C:\Users"), ("me", r"C:\Users\me")]);
        assert_eq!(shown(r"C:\"), [(r"C:\", r"C:\")]);
        assert_eq!(shown("/home/me/"), [("/", "/"), ("home", "/home"), ("me", "/home/me")]);
        assert_eq!(shown("sftp://me@host/home"), [("sftp://me@host/", "sftp://me@host/"), ("home", "sftp://me@host/home")]);
        assert_eq!(shown(r"\\nas\share\docs"), [(r"\\nas\share\", r"\\nas\share\"), ("docs", r"\\nas\share\docs")]);
    }
}
