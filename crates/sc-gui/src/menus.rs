pub mod context_menu;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub mod desktop_menu;
#[cfg(target_os = "macos")]
pub mod file_menu;
pub mod mark_menu;

use gpui_kit::component::GlobalState;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::Icon;
use gpui_kit::component::menu::{AppMenuBar, MenuIcons, PopupMenu};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use gpui_kit::{
    Action, AnyWindowHandle, App, Context, Entity, KeyDownEvent, DummyKeyboardMapper, Global, KeyBinding,
    KeyBindingContextPredicate, Menu, MenuItem, SharedString, Unbind, Window, WindowId,
};
#[cfg(target_os = "macos")]
use gpui_kit::{SystemMenuType, actions};
use sc_core::keymap::KeySettings;
use sc_core::menu::{Command, Item, MENUS};
use sc_core::system::theme::Appearance;

#[derive(Action, Clone, PartialEq)]
#[action(namespace = sc, no_json)]
pub struct Run(pub Command);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = sc, no_json)]
pub struct SetAppearance(pub Appearance);

/// A tab of the active side.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = sc, no_json)]
pub struct SelectTab(pub usize);

/// A place of the active tab's history, by its index there.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = sc, no_json)]
pub struct GoToHistory(pub usize);

/// One of the active side's recently closed tabs.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = sc, no_json)]
pub struct ReopenTab(pub usize);

/// Show > Workspaces: one, by its place in the list.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = sc, no_json)]
pub struct OpenWorkspace(pub usize);

/// Tools: one of the user's, by its place in the list.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = sc, no_json)]
pub struct RunTool(pub usize);

/// A command's icon in menus, on the toolbar and in Quick Open. Toggles have
/// none: a menu shows their check.
pub fn icon(command: Command) -> Option<Lucide> {
    use Command::*;
    Some(match command {
        View => Lucide::Eye,
        Edit => Lucide::Pencil,
        NewTextFile => Lucide::FilePlus,
        NewFolder => Lucide::FolderPlus,
        DownloadLink => Lucide::Download,
        Copy | CopyToClipboard => Lucide::Copy,
        Move => Lucide::FolderInput,
        RenameInPlace => Lucide::TextCursorInput,
        CreateLink => Lucide::Link,
        Delete => Lucide::Trash,
        Properties => Lucide::Info,
        CalculateOccupiedSpace => Lucide::ChartPie,
        MultiRenameTool => Lucide::Replace,
        CompareByContent => Lucide::Diff,
        Pack => Lucide::Package,
        UnpackSpecificFiles => Lucide::PackageOpen,
        SplitFile => Lucide::Split,
        CombineFiles => Lucide::Merge,
        CreateChecksumFiles => Lucide::Hash,
        VerifyChecksums => Lucide::ShieldCheck,
        Quit => Lucide::LogOut,
        SelectAll => Lucide::ListChecks,
        CutToClipboard => Lucide::Scissors,
        PasteFromClipboard => Lucide::ClipboardPaste,
        GoBack => Lucide::ArrowLeft,
        GoForward => Lucide::ArrowRight,
        FolderHistory => Lucide::FolderClock,
        GoHome => Lucide::House,
        LeftDrives | RightDrives => Lucide::HardDrive,
        Bookmarks => Lucide::Bookmark,
        BookmarkFolder => Lucide::BookmarkPlus,
        QuickOpen => Lucide::Zap,
        Search => Lucide::Search,
        SynchronizeDirs => Lucide::FolderSync,
        CompareDirectories => Lucide::GitCompareArrows,
        Undo => Lucide::Undo2,
        Redo => Lucide::Redo2,
        OperationQueue => Lucide::ListOrdered,
        ActivityLog => Lucide::ScrollText,
        // The system's terminal; the built-in one, a panel, is the framed prompt.
        OpenTerminal => Lucide::Terminal,
        NewTerminal => Lucide::SquareTerminal,
        SwapPanels => Lucide::ArrowLeftRight,
        TargetEqualsSource => Lucide::Equal,
        RereadSource => Lucide::RefreshCw,
        DuplicateTab => Lucide::Plus,
        CloseTab => Lucide::X,
        ManageTools => Lucide::Wrench,
        Settings => Lucide::Settings,
        Shortcuts => Lucide::Keyboard,
        Website => Lucide::Globe,
        Disconnect => Lucide::Unplug,
        _ => return None,
    })
}

/// A command's icon on the toolbar: toggles too, shown pressed while on.
pub fn toolbar_icon(command: Command) -> Option<Lucide> {
    use Command::*;
    icon(command).or_else(|| Some(match command {
        HiddenFiles => Lucide::EyeOff,
        ToggleSidebar => Lucide::PanelLeft,
        QuickViewPanel => Lucide::PanelRight,
        TerminalPanel => Lucide::SquareTerminal,
        VerticalArrangement => Lucide::Rows2,
        Brief => Lucide::Columns3,
        Full => Lucide::List,
        ThumbnailView => Lucide::LayoutGrid,
        BranchView => Lucide::ListTree,
        SyncBrowsing => Lucide::Link2,
        QuickFilter => Lucide::Funnel,
        _ => return None,
    }))
}

/// A command's icon in a menu, where the desktop's own apps show them.
pub fn menu_icon(command: Command) -> Option<Lucide> {
    static SHOWN: OnceLock<bool> = OnceLock::new();
    SHOWN.get_or_init(sc_core::system::theme::menu_icons).then(|| icon(command)).flatten()
}

/// A popup's item running a command, with its icon.
pub trait CommandEntry: Sized {
    /// An item doing what `like` does, its own way, with that icon.
    fn action_as(self, label: impl Into<SharedString>, like: Command, action: Box<dyn Action>) -> Self;

    fn command(self, label: impl Into<SharedString>, command: Command) -> Self {
        self.action_as(label, command, Box::new(Run(command)))
    }
}

impl CommandEntry for PopupMenu {
    fn action_as(self, label: impl Into<SharedString>, like: Command, action: Box<dyn Action>) -> Self {
        match menu_icon(like) {
            Some(icon) => self.menu_with_icon(label, icon, action),
            None => self.menu(label, action),
        }
    }
}

/// macOS greys out a menu item only when no one handles its action.
#[derive(Action, Clone, PartialEq)]
#[action(namespace = sc, no_json)]
struct Unavailable;

#[cfg(target_os = "macos")]
actions!(sc, [HideApp, HideOtherApps, ShowAllApps, MinimizeWindow, ZoomWindow]);

/// macOS: the application menu has these, not the window's menus; Configuration, left empty, goes.
#[cfg(target_os = "macos")]
const IN_APP_MENU: [Command; 4] = [Command::About, Command::Settings, Command::Shortcuts, Command::Quit];

/// The commands' keys. Keys bound `before` are unbound first: the new
/// bindings come after that, so they win.
pub fn bind(keys: &KeySettings, before: Option<&KeySettings>, cx: &mut App) {
    let binding = |key: &str, action: Box<dyn Action>, context: &str| {
        let context = Some(Rc::new(KeyBindingContextPredicate::parse(context).unwrap()));
        KeyBinding::load(key, action, context, false, None, &DummyKeyboardMapper).ok()
    };
    // The terminal takes every other key, and keeps its own (Ctrl+Shift+T opens a tab there);
    // the terminal's keys hide it or open another tab there, the tab keys go round its tabs.
    let targets = |command: Command, key: &str| {
        // Workspace 1 to 9 open it wherever the keys are, their keys shown in Show > Workspaces.
        let own: Box<dyn Action> = match command.workspace() {
            Some(ix) => Box::new(OpenWorkspace(ix)),
            None => Box::new(Run(command)),
        };
        let mut targets: Vec<(&str, Box<dyn Action>)> = vec![(crate::CONTEXT, own)];
        let terminal: Option<Box<dyn Action>> = match command {
            _ if crate::terminal::own_key(key) => None,
            Command::TerminalPanel | Command::NewTerminal => Some(Box::new(Run(command))),
            _ if command.workspace().is_some() => command.workspace().map(|ix| Box::new(OpenWorkspace(ix)) as Box<dyn Action>),
            Command::NextTab => Some(Box::new(crate::terminal::NextTerminalTab)),
            Command::PreviousTab => Some(Box::new(crate::terminal::PreviousTerminalTab)),
            _ => None,
        };
        targets.extend(terminal.map(|action| ("Terminal", action)));
        targets
    };
    let mut unbind = Vec::new();
    for (key, command) in before.into_iter().flat_map(KeySettings::bindings) {
        for (context, action) in targets(command, &key) {
            unbind.extend(binding(&key, Box::new(Unbind(action.name().into())), context));
        }
    }
    cx.bind_keys(unbind);
    // A key the settings file got wrong is left out.
    let mut bound = keys.bindings();
    let alt = bound.iter().filter_map(|(key, _)| {
        let key = sc_core::keymap::normalize(key, cfg!(target_os = "macos"));
        let letter = key.strip_prefix("alt-")?;
        letter.chars().next().filter(|_| letter.chars().count() == 1)
    });
    *ALT_TAKEN.lock().unwrap_or_else(|err| err.into_inner()) = alt.collect();
    // GPUI Kit shows a command's last key, macOS's menu bar its first: there
    // the shortcut goes before the alternate.
    if cfg!(target_os = "macos") {
        for keys in bound.chunk_by_mut(|a, b| a.1 == b.1) {
            keys.reverse();
        }
    }
    let mut bindings = Vec::new();
    for (key, command) in bound {
        for (context, action) in targets(command, &key) {
            bindings.extend(binding(&key, action, context));
        }
    }
    cx.bind_keys(bindings);
    // The menu bar shows the keys: the next window to render puts it up again.
    #[cfg(target_os = "macos")]
    cx.set_global(Installed(None));
}

/// Letters Alt has a command on: the menu bar's names take others.
static ALT_TAKEN: std::sync::Mutex<Vec<char>> = std::sync::Mutex::new(Vec::new());

/// Alt and a menu's letter opens that menu of `bar`.
pub fn alt_letter(bar: &Entity<AppMenuBar>, event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
    if bar.update(cx, |bar, cx| bar.open_by_key(&event.keystroke, window, cx)) {
        cx.stop_propagation();
    }
}

/// Where the letters show only while Alt is held (`MnemonicsOnAlt`): the
/// window drawn again as Alt goes down or up.
pub fn alt_changed(event: &gpui_kit::ModifiersChangedEvent, window: &mut Window, cx: &mut App) {
    thread_local! { static ALT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
    if cx.has_global::<gpui_kit::component::menu::MnemonicsOnAlt>() && ALT.with(|alt| alt.replace(event.modifiers.alt)) != event.modifiers.alt {
        window.refresh();
    }
}

pub fn init(cx: &mut App) {
    let icons = |action: &dyn Action| action.as_any().downcast_ref::<Run>().and_then(|Run(command)| menu_icon(*command)).map(Icon::new);
    cx.set_global(MenuIcons(Rc::new(icons)));
    // macOS's menus have no letters to press.
    #[cfg(not(target_os = "macos"))]
    // GTK's desktops underline them only while Alt is held, KDE's always.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    if !sc_core::system::theme::on_kde() {
        cx.set_global(gpui_kit::component::menu::MnemonicsOnAlt);
    }
    #[cfg(not(target_os = "macos"))]
    cx.set_global(gpui_kit::component::menu::MenuMnemonics(Rc::new(|labels, bar| {
        let taken = match bar {
            true => ALT_TAKEN.lock().unwrap_or_else(|err| err.into_inner()).clone(),
            false => Vec::new(),
        };
        sc_core::menu::mnemonics(labels, &taken)
    })));
    cx.on_action(|Run(command): &Run, cx| {
        if *command == Command::Quit {
            cx.quit()
        }
    });
    #[cfg(target_os = "macos")]
    {
        cx.on_action(|_: &HideApp, cx| cx.hide());
        cx.on_action(|_: &HideOtherApps, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAllApps, cx| cx.unhide_other_apps());
        cx.on_action(|_: &MinimizeWindow, cx| on_active_window(cx, Window::minimize_window));
        cx.on_action(|_: &ZoomWindow, cx| on_active_window(cx, Window::zoom_window));
        cx.bind_keys([
            KeyBinding::new("cmd-h", HideApp, None),
            KeyBinding::new("alt-cmd-h", HideOtherApps, None),
            KeyBinding::new("cmd-m", MinimizeWindow, None),
        ]);
    }
    cx.on_window_closed(|cx, closed| {
        cx.default_global::<Lent>().0.remove(&closed);
        #[cfg(target_os = "macos")]
        if cx.windows().is_empty() {
            grey_out(cx);
        }
    })
    .detach();
}

/// Windows that work for another, as Find Files for the main one: while one
/// is active, the menu bar shows the other's menus and their commands go to
/// the other, as a Mac app's panels leave the menus to its main window.
#[derive(Default)]
struct Lent(HashMap<WindowId, AnyWindowHandle>);

impl Global for Lent {}

/// `window` works for `owner` (see `Lent`).
pub fn lend(window: &Window, owner: AnyWindowHandle, cx: &mut App) {
    cx.default_global::<Lent>().0.insert(window.window_handle().window_id(), owner);
}

/// The window the active one works for, if it works for one.
pub fn owner_of_active(cx: &App) -> Option<AnyWindowHandle> {
    let active = cx.active_window()?;
    cx.try_global::<Lent>()?.0.get(&active.window_id()).copied()
}

/// macOS: no window is left to act on; the menu bar stays as it was, greyed
/// out but for hiding and quitting.
#[cfg(target_os = "macos")]
fn grey_out(cx: &mut App) {
    use gpui_kit::OwnedMenuItem;
    fn items(owned: Vec<OwnedMenuItem>) -> Vec<MenuItem> {
        let stay: [&dyn Action; 4] = [&HideApp, &HideOtherApps, &ShowAllApps, &Run(Command::Quit)];
        let each = owned.into_iter().map(|entry| match entry {
            OwnedMenuItem::Separator => MenuItem::Separator,
            OwnedMenuItem::Submenu(menu) => MenuItem::submenu(Menu::new(menu.name).items(items(menu.items))),
            OwnedMenuItem::SystemMenu(menu) => MenuItem::os_submenu(menu.name, menu.menu_type),
            OwnedMenuItem::Action { name, action, os_action, checked, .. } => {
                let kept = stay.iter().any(|stays| stays.partial_eq(&*action));
                MenuItem::Action {
                    name: name.into(),
                    action: if kept { action } else { Box::new(Unavailable) },
                    os_action,
                    checked,
                    disabled: !kept,
                }
            }
        });
        each.collect()
    }
    let Some(menus) = cx.get_menus() else {
        return;
    };
    cx.set_menus(menus.into_iter().map(|menu| Menu::new(menu.name).items(items(menu.items))));
    // The next window to open puts its own up.
    cx.set_global(Installed(None));
}

/// The action reaches the app while its window is being updated: after that.
#[cfg(target_os = "macos")]
fn on_active_window(cx: &mut App, act: fn(&Window)) {
    if let Some(handle) = cx.active_window() {
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| act(window));
        });
    }
}

/// The Show menu's last custom filter, the name of the filter item after
/// Programs, as Total Commander's; "*.*" before one.
static LAST_FILTER: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

pub fn set_last_filter(masks: &str) {
    *LAST_FILTER.lock().unwrap_or_else(|err| err.into_inner()) = masks.to_owned();
}

pub fn last_filter() -> String {
    let masks = LAST_FILTER.lock().unwrap_or_else(|err| err.into_inner()).clone();
    if masks.is_empty() { "*.*".to_owned() } else { masks }
}

/// Go > Back's and Forward's places: the active tab's history.
static HISTORY: std::sync::Mutex<sc_core::History> = std::sync::Mutex::new(sc_core::History { places: Vec::new(), at: 0 });

/// Whether it changed: then `set` again.
pub fn set_history(history: &sc_core::History) -> bool {
    let mut kept = HISTORY.lock().unwrap_or_else(|err| err.into_inner());
    let changed = *kept != *history;
    if changed {
        *kept = history.clone();
    }
    changed
}

/// Back's or Forward's places, the nearest first: each one's place in the
/// history, and its label.
pub fn history_ways(command: Command) -> Vec<(usize, String)> {
    let history = HISTORY.lock().unwrap_or_else(|err| err.into_inner());
    let ways: Vec<usize> = match command {
        Command::GoBack => (0..history.at.min(history.places.len())).rev().collect(),
        _ => (history.at + 1..history.places.len()).collect(),
    };
    ways.into_iter().map(|ix| (ix, place_label(&history.places[ix]))).collect()
}

/// A history place as menus list it: its name first, as long paths are cut
/// off at the end, where they differ; home as ~.
pub fn place_label(place: &str) -> String {
    let name = std::path::Path::new(place).file_name().map_or_else(|| place.to_owned(), |name| name.to_string_lossy().into_owned());
    let home = std::env::home_dir().map(|home| home.display().to_string());
    let path = match home.as_deref().and_then(|home| place.strip_prefix(home)) {
        Some(rest) => format!("~{rest}"),
        None => place.to_owned(),
    };
    format!("{name}   {path}")
}

/// The Tools menu's names.
static TOOLS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Call `set` after.
pub fn set_tools(tools: &[sc_core::tools::Tool]) {
    *TOOLS.lock().unwrap_or_else(|err| err.into_inner()) = tools.iter().map(|tool| tool.name.clone()).collect();
}

/// The tools' keys, `before`'s unbound first; bound after the commands', so they win.
pub fn bind_tools(tools: &[sc_core::tools::Tool], before: &[sc_core::tools::Tool], cx: &mut App) {
    let binding = |key: &str, action: Box<dyn Action>| {
        let context = Some(Rc::new(KeyBindingContextPredicate::parse(crate::CONTEXT).unwrap()));
        KeyBinding::load(key, action, context, false, None, &DummyKeyboardMapper).ok()
    };
    let unbind = before.iter().filter(|tool| !tool.key.is_empty());
    cx.bind_keys(unbind.filter_map(|tool| binding(&tool.key, Box::new(Unbind(RunTool::name_for_type().into())))).collect::<Vec<_>>());
    let keyed = tools.iter().enumerate().filter(|(_, tool)| !tool.key.is_empty());
    cx.bind_keys(keyed.filter_map(|(ix, tool)| binding(&tool.key, Box::new(RunTool(ix)))).collect::<Vec<_>>());
}

/// Show > Workspaces' names, and which is open.
static WORKSPACES: std::sync::Mutex<(Vec<String>, usize)> = std::sync::Mutex::new((Vec::new(), 0));

/// Call `set` after.
pub fn set_workspaces(workspaces: &sc_core::session::Workspaces) {
    let names = workspaces.list.iter().map(|workspace| workspace.name.clone()).collect();
    *WORKSPACES.lock().unwrap_or_else(|err| err.into_inner()) = (names, workspaces.current);
}

/// Call `AppMenuBar::reload` after.
/// The menu bar; `checked` commands show a check mark.
pub fn set(chosen: Appearance, checked: &dyn Fn(Command) -> bool, cx: &mut App) {
    let menus = main_menus(chosen, checked).into_iter().map(Menu::owned).collect();
    // AppMenuBar draws these, except on macOS: there `show` gives them to the system.
    GlobalState::global_mut(cx).set_app_menus(menus);
}

/// The main window's menus; `checked` commands show a check mark.
pub fn main_menus(chosen: Appearance, checked: &dyn Fn(Command) -> bool) -> Vec<Menu> {
    let menus = MENUS.iter().map(|menu| Menu::new(sc_core::menu::menu_label(menu.label)).items(items(menu.items, chosen, checked)));
    menus.filter(|menu| !menu.items.is_empty()).collect()
}

fn items(list: &[Item], chosen: Appearance, checked: &dyn Fn(Command) -> bool) -> Vec<MenuItem> {
    let one = |entry: &Item| match *entry {
        Item::Separator => Some(MenuItem::separator()),
        Item::Submenu { label, items: sub } => Some(MenuItem::submenu(
            Menu::new(sc_core::menu::menu_label(label)).items(items(sub, chosen, checked)),
        )),
        #[cfg(target_os = "macos")]
        Item::Command { command, .. } if IN_APP_MENU.contains(&command) => None,
        Item::Command { command: Command::AllWithExtension, .. } => {
            Some(item(last_filter().into(), checked(Command::AllWithExtension), Box::new(Run(Command::AllWithExtension)), false))
        }
        Item::Command { command, .. } => {
            Some(item(sc_core::menu::label(command).into(), checked(command), Box::new(Run(command)), false))
        }
        // Xfce's, MATE's and Cinnamon's look is their theme's: no other to choose.
        Item::Appearance { .. } if sc_core::system::theme::desktop_dark().is_some() => None,
        Item::Appearance { label } => {
            let each = [
                (sc_core::t!("appearance.system"), Appearance::System),
                (sc_core::t!("appearance.light"), Appearance::Light),
                (sc_core::t!("appearance.dark"), Appearance::Dark),
            ]
            .map(|(name, appearance)| {
                MenuItem::action(name, SetAppearance(appearance)).checked(chosen == appearance)
            });
            Some(MenuItem::submenu(Menu::new(sc_core::menu::menu_label(label)).items(each)))
        }
        Item::ClosedTabs { .. } => None,
        // A submenu whose first item is its own name: GPUI Kit runs that when it is
        // clicked (macOS shows it first in the submenu); the places that way after.
        Item::History { command, .. } => {
            let label = sc_core::menu::label(command);
            let own = item(label.clone().into(), checked(command), Box::new(Run(command)), false);
            let ways = history_ways(command);
            if ways.is_empty() {
                return Some(own);
            }
            let places = ways.into_iter().map(|(ix, label)| item(label.into(), false, Box::new(GoToHistory(ix)), false));
            let all = [own, MenuItem::separator()].into_iter().chain(places);
            Some(MenuItem::submenu(Menu::new(label).items(all)))
        }
        Item::Workspaces { label, items: rest } => {
            let label = sc_core::menu::menu_label(label);
            let (names, current) = WORKSPACES.lock().unwrap_or_else(|err| err.into_inner()).clone();
            let each = names.into_iter().enumerate();
            let each = each.map(|(ix, name)| item(name.into(), current == ix, Box::new(OpenWorkspace(ix)), false));
            let all = each.chain(items(rest, chosen, checked));
            Some(MenuItem::submenu(Menu::new(label).items(all)))
        }
        Item::Tools => None,
    };
    let all = list.iter().flat_map(|entry| match entry {
        Item::Tools => {
            let names = TOOLS.lock().unwrap_or_else(|err| err.into_inner()).clone();
            let tools = names.into_iter().enumerate().map(|(ix, name)| item(name.into(), false, Box::new(RunTool(ix)), false));
            let mut tools: Vec<MenuItem> = tools.collect();
            if !tools.is_empty() {
                tools.push(MenuItem::separator());
            }
            tools
        }
        entry => one(entry).into_iter().collect(),
    });
    // What the application menu took may leave a separator last.
    let mut items: Vec<MenuItem> = all.collect();
    while matches!(items.last(), Some(MenuItem::Separator)) {
        items.pop();
    }
    items
}

/// A menu bar item; macOS writes "…" for "...".
fn item(label: SharedString, checked: bool, action: Box<dyn Action>, disabled: bool) -> MenuItem {
    let name = match cfg!(target_os = "macos") {
        true => label.replace("...", "…").into(),
        false => label,
    };
    MenuItem::Action {
        name,
        action: if disabled { Box::new(Unavailable) } else { action },
        os_action: None,
        checked,
        disabled,
    }
}

/// What a window's menus are built into: GPUI Kit's popups, as the menu
/// bar in the window shows them, or the items of macOS's menu bar.
pub trait Entries: Sized {
    fn entry(self, label: impl Into<SharedString>, checked: bool, action: Box<dyn Action>, disabled: bool) -> Self;
    fn separator(self) -> Self;
}

impl Entries for PopupMenu {
    fn entry(self, label: impl Into<SharedString>, checked: bool, action: Box<dyn Action>, disabled: bool) -> Self {
        self.menu_with_check_and_disabled(label, checked, action, disabled)
    }

    fn separator(self) -> Self {
        PopupMenu::separator(self)
    }
}

impl Entries for Vec<MenuItem> {
    fn entry(mut self, label: impl Into<SharedString>, checked: bool, action: Box<dyn Action>, disabled: bool) -> Self {
        self.push(item(label.into(), checked, action, disabled));
        self
    }

    fn separator(mut self) -> Self {
        self.push(MenuItem::separator());
        self
    }
}

/// What macOS's menu bar shows, so windows that render again with the same
/// menus leave it alone; `None` puts it up again.
#[cfg(target_os = "macos")]
struct Installed(Option<u64>);

#[cfg(target_os = "macos")]
impl Global for Installed {}

/// macOS: the active window's menus on the system's menu bar, between the
/// application menu and Window, Help last. Each window calls this as it renders.
#[cfg(target_os = "macos")]
pub fn show(window: &Window, mut menus: Vec<Menu>, cx: &mut App) {
    use std::hash::{DefaultHasher, Hash, Hasher};
    fn hash_items(items: &[MenuItem], state: &mut DefaultHasher) {
        for item in items {
            match item {
                MenuItem::Separator | MenuItem::SystemMenu(_) => 0.hash(state),
                MenuItem::Submenu(menu) => {
                    menu.name.hash(state);
                    hash_items(&menu.items, state);
                }
                MenuItem::Action { name, action, checked, disabled, .. } => {
                    (name, action.name(), checked, disabled).hash(state)
                }
            }
        }
    }
    // Its own while active, and while a window that works for it is.
    let lent = owner_of_active(cx).is_some_and(|owner| owner.window_id() == window.window_handle().window_id());
    if !window.is_window_active() && !lent {
        return;
    }
    let mut state = DefaultHasher::new();
    for menu in &menus {
        menu.name.hash(&mut state);
        hash_items(&menu.items, &mut state);
    }
    let hash = Some(state.finish());
    if cx.try_global::<Installed>().is_some_and(|installed| installed.0 == hash) {
        return;
    }
    cx.set_global(Installed(hash));
    let label = |command| SharedString::from(sc_core::menu::label(command));
    let app = Menu::new("Spark Commander").items([
        item(label(Command::About), false, Box::new(Run(Command::About)), false),
        MenuItem::separator(),
        item(label(Command::Settings), false, Box::new(Run(Command::Settings)), false),
        item(label(Command::Shortcuts), false, Box::new(Run(Command::Shortcuts)), false),
        MenuItem::separator(),
        MenuItem::os_submenu(sc_core::t!("menus.services"), SystemMenuType::Services),
        MenuItem::separator(),
        MenuItem::action(sc_core::t!("menus.hide_app"), HideApp),
        MenuItem::action(sc_core::t!("menus.hide_others"), HideOtherApps),
        MenuItem::action(sc_core::t!("menus.show_all"), ShowAllApps),
        MenuItem::separator(),
        MenuItem::action(sc_core::t!("menus.quit_app"), Run(Command::Quit)),
    ]);
    // macOS lists the windows under these; GPUI finds the menu by its English name, so it keeps it.
    let windows = Menu::new("Window").items([
        MenuItem::action(sc_core::t!("menus.minimize"), MinimizeWindow),
        MenuItem::action(sc_core::t!("menus.zoom"), ZoomWindow),
    ]);
    // Help stays last, as in Mac apps.
    let help_name = sc_core::menu::menu_label("Help");
    let help = menus.iter().position(|menu| menu.name == help_name).map(|ix| menus.remove(ix));
    cx.set_menus(std::iter::once(app).chain(menus).chain([windows]).chain(help));
}

/// `checked` puts a check mark on a command; `closed` lists the recently closed tabs.
pub fn context_menu(
    mut menu: PopupMenu,
    items: &[Item],
    checked: impl Fn(Command) -> bool,
    closed: Vec<SharedString>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    for item in items {
        menu = match *item {
            Item::Command { command, .. } if !checked(command) => menu.command(sc_core::menu::tab_label(command), command),
            Item::Command { command, .. } => menu.menu_with_check(sc_core::menu::tab_label(command), true, Box::new(Run(command))),
            Item::Separator => menu.separator(),
            Item::ClosedTabs { label } => {
                let closed = closed.clone();
                menu.submenu(sc_core::menu::menu_label(label), window, cx, move |mut menu, _, _| {
                    if closed.is_empty() {
                        return menu.menu_with_check_and_disabled(
                            sc_core::t!("tab_menu.none"),
                            false,
                            Box::new(ReopenTab(0)),
                            true,
                        );
                    }
                    for (ix, path) in closed.iter().enumerate() {
                        menu = menu.menu(path.clone(), Box::new(ReopenTab(ix)));
                    }
                    menu
                })
            }
            // shortcut: no submenus or appearances in context menus yet; add them when one has them.
            Item::Submenu { .. } | Item::Appearance { .. } | Item::Workspaces { .. } | Item::History { .. } | Item::Tools => menu,
        };
    }
    menu
}
