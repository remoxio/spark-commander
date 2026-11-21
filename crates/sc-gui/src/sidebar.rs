//! The sidebar: its sections as a row of icons or as an accordion, one open
//! and filling it;
//! this computer's folders as a tree that follows the active panel, the
//! servers by protocol, and Places: bookmarks, drives and devices.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, MacControls, Side, Sizable, StyledExt, TitleBar, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::remote::connections::{self, Connection, Connections};
use sc_core::system::devices::{self, Device, Volume, VolumeKind};
use sc_core::menu::Command;
use sc_core::remote::{Address, Scheme};
use sc_core::settings::SidebarLayout;
use sc_core::sidebar::{self, FolderTree, Group, SidebarTab};
use sc_core::{icons, system::theme, t};

use crate::panel::drag_drop::{DragLabel, Dragged};
use crate::menus::CommandEntry;
use crate::{Commander, dialogs::connections_dialog, header};

pub(crate) const WIDTH: f32 = 260.;
/// How long it slides in or out, shown or hidden.
const SLIDE: Duration = Duration::from_millis(200);
const ROW: Pixels = px(if cfg!(target_os = "macos") { 24. } else { 22. });
const SECTION: Pixels = px(30.);
/// macOS: the sidebar's inset from the window's edges.
const INSET: Pixels = px(8.);

pub(crate) struct Sidebar {
    /// The section open; none while hidden.
    pub(crate) tab: Option<SidebarTab>,
    /// The section last open, which Show Sidebar opens.
    pub(crate) last: SidebarTab,
    pub(crate) width: f32,
    /// Times shown or hidden since the start, each a slide of its own.
    slides: usize,
    /// Hidden, the section it slides out with.
    closing: Option<SidebarTab>,
    /// Section switches so far.
    switches: usize,
    /// The section sliding shut.
    leaving: Option<SidebarTab>,
    /// How much of the window colour it paints over the desktop; none: solid.
    tint: Option<f32>,
    /// The desktop through it blurred, else sharp.
    blur: bool,
    tree: FolderTree,
    tree_scroll: UniformListScrollHandle,
    /// The active panel's folder, where the tree last went.
    followed: Option<PathBuf>,
    pub(crate) connections: Connections,
    pub(crate) hosts: Vec<String>,
    /// The Connections list's groups shut.
    closed: Vec<Group>,
    selected: Option<Picked>,
    /// KDE Connect's paired devices.
    devices: Vec<Device>,
    pub(crate) volumes: Vec<Volume>,
    /// The folders of the devices mounted, by id.
    device_folders: HashMap<String, Vec<(PathBuf, String)>>,
    /// When each phone was last connected again, its files having stopped answering.
    reconnected: HashMap<String, std::time::Instant>,
}

/// Where a mounted device's files are: KDE Connect mounts it at
/// `$XDG_RUNTIME_DIR/<id>`, its folders below; none while not mounted.
fn device_roots(sidebar: &Sidebar, id: &str) -> Vec<PathBuf> {
    let Some(folders) = sidebar.device_folders.get(id) else {
        return Vec::new();
    };
    let id = std::ffi::OsStr::new(id);
    let root = |path: &PathBuf| path.ancestors().find(|at| at.file_name() == Some(id)).unwrap_or(path).to_owned();
    folders.iter().map(|(path, _)| root(path)).collect()
}

/// The "Opening…" note of a device being mounted.
struct Mounting;

/// The sections in their order, each with its icon and name.
fn sections() -> [(SidebarTab, Lucide, String); 4] {
    [
        (SidebarTab::Places, Lucide::Bookmark, t!("sidebar.section.places")),
        (SidebarTab::Tree, Lucide::FolderTree, t!("sidebar.section.folders")),
        (SidebarTab::Connections, Lucide::Network, t!("sidebar.section.connections")),
        (SidebarTab::Operations, Lucide::ListOrdered, t!("sidebar.section.operations")),
    ]
}

fn section_at(which: SidebarTab) -> f32 {
    sections().iter().position(|section| section.0 == which).unwrap_or(0) as f32
}

/// The sections' row of icons: its track, the open one's pill and icon, their corners.
#[derive(Clone, Copy)]
struct TabsLook {
    track: Hsla,
    pill: Hsla,
    on_pill: Hsla,
    radius: Pixels,
}

/// Dragging the sidebar's edge.
pub(crate) struct SidebarDrag;

/// What is chosen in the Connections list.
#[derive(Clone, PartialEq)]
enum Picked {
    Saved(usize),
    Folder(String),
    /// A host of `~/.ssh/config`, by its place there.
    Host(usize),
}

/// A saved connection dragged, by its place.
#[derive(Clone, Copy)]
struct ServerDrag(usize);

/// A folder of the Connections list dragged.
#[derive(Clone)]
struct ServerFolderDrag(String);

impl Sidebar {
    pub(crate) fn new(tab: Option<SidebarTab>, width: f32, hidden: bool, opacity: u8, blur: bool) -> Self {
        Sidebar {
            tab,
            last: tab.unwrap_or(SidebarTab::Tree),
            width: if width > 0. { width } else { WIDTH },
            slides: 0,
            closing: None,
            switches: 0,
            leaving: None,
            tint: theme::backdrop_tint(opacity),
            blur,
            tree: FolderTree::new(hidden),
            tree_scroll: UniformListScrollHandle::new(),
            followed: None,
            connections: Connections::load().unwrap_or_default(),
            hosts: connections::ssh_hosts(),
            closed: Vec::new(),
            selected: None,
            devices: Vec::new(),
            volumes: Vec::new(),
            device_folders: HashMap::new(),
            reconnected: HashMap::new(),
        }
    }

    /// Show > Sidebar's check marks.
    /// macOS: the title bar's and the bottom bars' colour: the sidebar's, as
    /// see-through as it.
    pub(crate) fn chrome(&self, window: Hsla) -> Hsla {
        window.opacity(self.tint.unwrap_or(1.))
    }

    /// Shown, or sliding out.
    pub(crate) fn drawn(&self) -> bool {
        self.tab.or(self.closing).is_some()
    }

    pub(crate) fn shows(&self, command: Command) -> bool {
        match command {
            Command::SidebarTree => self.tab == Some(SidebarTab::Tree),
            Command::SidebarConnections => self.tab == Some(SidebarTab::Connections),
            Command::SidebarPlaces => self.tab == Some(SidebarTab::Places),
            Command::SidebarOperations => self.tab == Some(SidebarTab::Operations),
            Command::ToggleSidebar => self.tab.is_some(),
            _ => false,
        }
    }

    /// Reads the open folders again; `hidden` shows dot folders.
    pub(crate) fn refresh(&mut self, hidden: bool) {
        self.tree.refresh(hidden);
    }

    pub(crate) fn reload_servers(&mut self) {
        self.connections = Connections::load().unwrap_or_default();
        self.hosts = connections::ssh_hosts();
    }
}

impl Commander {
    /// Opens `which`; the open one slides shut.
    fn open_section(&mut self, which: SidebarTab, cx: &mut Context<Self>) {
        let Some(was) = self.sidebar.tab.filter(|&was| was != which) else {
            return;
        };
        self.sidebar.tab = Some(which);
        self.sidebar.last = which;
        self.sidebar.switches += 1;
        self.sidebar.leaving = Some(was);
        let switch = self.sidebar.switches;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SLIDE).await;
            let _ = this.update(cx, |this, cx| {
                if this.sidebar.switches == switch && this.sidebar.leaving.take().is_some() {
                    cx.notify();
                }
            });
        })
        .detach();
        self.reload_menus(cx);
        cx.notify();
    }

    /// Show > Sidebar: that tab, or hidden when it shows it already.
    pub(crate) fn toggle_sidebar(&mut self, tab: SidebarTab, window: &mut Window, cx: &mut Context<Self>) {
        let was = self.sidebar.tab;
        if was.is_some_and(|was| was != tab) {
            return self.open_section(tab, cx);
        }
        self.sidebar.tab = (was != Some(tab)).then_some(tab);
        self.sidebar.last = tab;
        // Shown or hidden, it slides; hidden, it is drawn until out.
        if was.is_some() != self.sidebar.tab.is_some() {
            self.sidebar.slides += 1;
            self.sidebar.closing = was.filter(|_| self.sidebar.tab.is_none());
            let slide = self.sidebar.slides;
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(SLIDE).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.sidebar.slides == slide && this.sidebar.closing.take().is_some() {
                        this.set_backdrop(window, cx);
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        self.set_backdrop(window, cx);
        self.reload_menus(cx);
        cx.notify();
    }

    /// The sidebar's and the terminal's opacity over the desktop, blurred or
    /// not: in the settings, or tried in their dialog.
    pub(crate) fn show_backdrop(&mut self, look: &sc_core::settings::Settings, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar.tint = theme::backdrop_tint(look.sidebar_opacity.0);
        self.sidebar.blur = look.sidebar_blur.0;
        self.terminal_look = (theme::backdrop_tint(look.terminal_opacity.0), look.terminal_blur.0);
        self.set_backdrop(window, cx);
        cx.notify();
    }

    /// The desktop shows through, blurred if wished, only while the sidebar
    /// or the terminal is there: a see-through window loses subpixel text.
    pub(crate) fn set_backdrop(&self, window: &mut Window, cx: &mut App) {
        // Sliding out too.
        let sidebar = self.sidebar.tab.or(self.sidebar.closing).is_some() && self.sidebar.tint.is_some();
        let terminal = self.terminal_shown && self.terminal_look.0.is_some();
        // shortcut: the window has one backdrop: blurred when either part shown asks for it.
        let blur = (sidebar && self.sidebar.blur || terminal && self.terminal_look.1) && theme::blurs();
        let see_through = sidebar || terminal;
        window.set_background_appearance(match see_through {
            false => WindowBackgroundAppearance::Opaque,
            true if !blur => WindowBackgroundAppearance::Transparent,
            true => WindowBackgroundAppearance::Blurred,
        });
        window.set_see_through(see_through, cx);
        // The title bar as Windows draws it on an opaque window, as the menu bar under it;
        // a see-through backdrop had turned it black.
        #[cfg(windows)]
        if let Ok(raw_window_handle::RawWindowHandle::Win32(handle)) =
            raw_window_handle::HasWindowHandle::window_handle(window).map(|handle| handle.as_raw())
        {
            let rgb = if cx.theme().mode.is_dark() { 0x202020 } else { 0xffffff };
            sc_core::system::theme::set_caption_color(handle.hwnd.get(), rgb);
        }
    }

    /// The tree opens down to the active panel's folder, once that changed.
    fn follow_active(&mut self) {
        let panel = self.panels[self.active].tabs.current();
        let here = panel.remote().is_none().then(|| panel.folder().to_owned());
        if here == self.sidebar.followed {
            return;
        }
        if let Some(path) = &here
            && self.sidebar.tree.reveal(path)
            && let Some(ix) = self.sidebar.tree.rows().iter().position(|row| row.path == path)
        {
            self.sidebar.tree_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        self.sidebar.followed = here;
    }

    /// The active panel shows `path`, leaving its server for it; an error names `path`.
    pub(crate) fn open_folder(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let side = &mut self.panels[self.active];
        let remote = side.tabs.current().remote().is_some();
        let before = side.tabs.current().spot();
        let went = side.tabs.navigate(|panel| match remote {
            true => panel.disconnect(path.clone()),
            false => panel.go_to_folder(path.clone()),
        });
        side.scroll_to_cursor();
        match went {
            Ok(()) => self.follow(self.active, before, window, cx),
            Err(err) => window.push_notification(Notification::error(format!("{}: {err}", path.display())), cx),
        }
        cx.notify();
    }

    /// The saved servers as on disk, changed, saved and shown.
    fn change_servers(&mut self, change: impl FnOnce(&mut Connections), window: &mut Window, cx: &mut Context<Self>) {
        let mut all = Connections::load().unwrap_or_default();
        change(&mut all);
        if let Err(err) = all.save() {
            window.push_notification(Notification::error(t!("connections.save_failed", err = err)), cx);
        }
        self.sidebar.reload_servers();
        cx.notify();
    }

    /// New (none: in the chosen one's folder), edit, or duplicate a saved server.
    fn edit_server(&mut self, ix: Option<usize>, duplicate: bool, window: &mut Window, cx: &mut Context<Self>) {
        let saved = &self.sidebar.connections.saved;
        let mut connection = match ix.and_then(|ix| saved.get(ix).cloned()) {
            Some(connection) => connection,
            None => Connection { folder: self.picked_folder(), ..Connection::default() },
        };
        if duplicate {
            connection.name = t!("connections.duplicate_name", name = connection.name);
        }
        let replace = ix.filter(|_| !duplicate);
        let this = cx.entity().downgrade();
        connections_dialog::open_edit(connection, window, cx, move |connection, window, cx| {
            let _ = this.update(cx, |this, cx| {
                let mut at = None;
                let change = |all: &mut Connections| match replace.filter(|&ix| ix < all.saved.len()) {
                    Some(ix) => all.saved[ix] = connection,
                    None => {
                        // Last in its folder.
                        all.saved.push(connection);
                        at = all.move_connection(all.saved.len() - 1, all.saved.last().and_then(|last| last.folder.clone()), None);
                    }
                };
                this.change_servers(change, window, cx);
                if let Some(at) = at.or(replace) {
                    this.sidebar.selected = Some(Picked::Saved(at));
                }
            });
        });
    }

    /// The folder of what is chosen in the Connections list: a folder, or a connection's.
    fn picked_folder(&self) -> Option<String> {
        match self.sidebar.selected.as_ref()? {
            Picked::Folder(name) => Some(name.clone()),
            Picked::Saved(ix) => self.sidebar.connections.saved.get(*ix)?.folder.clone(),
            Picked::Host(_) => None,
        }
    }

    fn delete_server(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let delete = |all: &mut Connections| {
            if ix < all.saved.len() {
                let gone = all.saved.remove(ix);
                connections::forget_password(&all.saved, &gone);
            }
        };
        self.sidebar.selected = None;
        self.change_servers(delete, window, cx);
    }

    /// A new folder of connections (none), or `old` renamed.
    fn name_server_folder(&mut self, old: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let (title, value) = match &old {
            Some(old) => (t!("connections.folder.rename_title"), old.clone()),
            None => (t!("connections.folder.new_title"), String::new()),
        };
        let this = cx.entity().downgrade();
        crate::dialogs::name_dialog::open(title, t!("connections.folder.name"), value, window, cx, move |name, window, cx| {
            let _ = this.update(cx, |this, cx| {
                let mut named = false;
                this.change_servers(|all| named = all.name_folder(old.as_deref(), &name), window, cx);
                match named {
                    true => this.sidebar.selected = Some(Picked::Folder(name.trim().to_owned())),
                    false if !name.trim().is_empty() => {
                        window.push_notification(Notification::error(t!("connections.folder.exists", name = name.trim())), cx);
                    }
                    false => {}
                }
            });
        });
    }

    /// A folder of connections gone, its connections too: asked first when it has any.
    fn delete_server_folder(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let inside = self.sidebar.connections.saved.iter().filter(|connection| connection.folder.as_deref() == Some(name.as_str())).count();
        let text = t!("connections.folder.delete_confirm", name = name, count = inside);
        let delete = move |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            this.sidebar.selected = None;
            this.change_servers(
                |all| {
                    for gone in all.remove_folder(&name) {
                        connections::forget_password(&all.saved, &gone);
                    }
                },
                window,
                cx,
            );
        };
        if inside == 0 {
            return delete(self, window, cx);
        }
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (this, delete) = (this.clone(), delete.clone());
            alert.title(t!("connections.folder.delete_title")).description(text.clone()).ok_text(t!("connections.delete")).show_cancel(true).on_ok(move |_, window, cx| {
                let delete = delete.clone();
                let _ = this.update(cx, |this, cx| delete(this, window, cx));
                true
            })
        });
    }

    /// The chosen connection or folder a place up or down.
    fn step_server(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        match self.sidebar.selected.clone() {
            Some(Picked::Saved(ix)) => {
                let mut at = None;
                self.change_servers(|all| at = all.step_connection(ix, up), window, cx);
                if let Some(at) = at {
                    self.sidebar.selected = Some(Picked::Saved(at));
                }
            }
            Some(Picked::Folder(name)) => self.change_servers(|all| _ = all.step_folder(&name, up), window, cx),
            _ => {}
        }
    }

    /// Saved connection `ix` into `folder` (none: the top), before `before` there, else last.
    fn move_server(&mut self, ix: usize, folder: Option<String>, before: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let mut at = None;
        self.change_servers(|all| at = all.move_connection(ix, folder, before), window, cx);
        if let Some(at) = at {
            self.sidebar.selected = Some(Picked::Saved(at));
        }
    }

    /// The Connections list's folder `name` before `before`, else last.
    fn move_server_folder(&mut self, name: String, before: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.change_servers(|all| all.move_folder(&name, before.as_deref()), window, cx);
        self.sidebar.selected = Some(Picked::Folder(name));
    }

    pub(crate) fn render_sidebar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tab = self.sidebar.tab.or(self.sidebar.closing)?;
        if tab == SidebarTab::Tree {
            self.follow_active();
        }
        let background = self.colors.window.opacity(self.sidebar.tint.unwrap_or(1.));
        let horizontal = self.settings.sidebar_layout == SidebarLayout::Horizontal;
        if let Some(mac) = crate::appearance::mac_tabs(cx) {
            let sidebar = self.render_mac_sidebar(tab, background, horizontal, mac, cx);
            return Some(self.slide_sidebar(sidebar));
        }
        let sections = match horizontal {
            true => {
                let theme = cx.theme();
                let look = TabsLook { track: theme.muted, pill: theme.primary, on_pill: theme.primary_foreground, radius: theme.radius };
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(self.render_section_tabs(tab, look, cx).mx_2().mt_2())
                    .child(v_flex().flex_1().min_h_0().pt_2().pb_1().child(self.render_open_section(tab, cx)))
            }
            false => self.render_accordion(tab, false, cx),
        };
        let theme = cx.theme();
        let width = self.sidebar.width;
        let drawer = h_flex()
            .h_full()
            .w(px(width))
            .bg(background)
            .border_r_1()
            .border_color(theme.border)
            .child(sections.h_full());
        // Sliding, its edge narrows or widens; the drawer is clipped, not reflowed.
        let sidebar = div()
            .relative()
            .h_full()
            .flex_none()
            .w(px(width))
            .child(div().size_full().overflow_hidden().child(drawer))
            .child(
                div()
                    .id("sidebar-edge")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(-3.))
                    .w(px(6.))
                    .occlude()
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_drag(SidebarDrag, |_, _, _, cx| cx.new(|_| EmptyView)),
            );
        Some(self.slide_sidebar(sidebar))
    }

    fn section_content(&mut self, which: SidebarTab, cx: &mut Context<Self>) -> AnyElement {
        match which {
            SidebarTab::Tree => self.render_tree(cx).into_any_element(),
            SidebarTab::Connections => self.render_connections(cx).into_any_element(),
            SidebarTab::Places => self.render_places(cx).into_any_element(),
            SidebarTab::Operations => self.render_operations(cx).into_any_element(),
        }
    }

    /// The sections' names one under another, the open one's content under its name.
    fn render_accordion(&mut self, tab: SidebarTab, mac: bool, cx: &mut Context<Self>) -> Div {
        let mut accordion = v_flex().flex_1().min_w_0().min_h_0();
        for (at, (which, icon, name)) in sections().into_iter().enumerate() {
            let open = tab == which;
            let theme = cx.theme();
            let chevron = match open {
                true => IconName::ChevronDown,
                false => IconName::ChevronRight,
            };
            // The queue's progress shows also while its section is shut.
            let summary = (which == SidebarTab::Operations).then(|| self.operations_summary()).flatten();
            let line = summary.as_ref().and_then(|(_, done)| *done).map(|done| {
                div().absolute().left_0().bottom_0().h(px(2.)).w(relative(done)).bg(theme.progress_bar)
            });
            let summary = summary.map(|(text, _)| div().text_xs().text_color(theme.muted_foreground).child(text));
            // macOS: rows of the sidebar's, rounded, with a line between them.
            if mac && at > 0 {
                accordion = accordion.child(div().flex_none().h(px(1.)).mx_3().bg(theme.border));
            }
            // A click opens it; the open one stays: one is always open.
            let header = h_flex()
                .id(("sidebar-section", at))
                .relative()
                .flex_none()
                .h(SECTION)
                .px_2()
                .gap_2()
                .cursor_pointer()
                .map(|el| match mac {
                    true => el.mx_1().rounded(px(6.)),
                    false => el.when(at > 0, |el| el.border_t_1().border_color(theme.border)),
                })
                .hover(|style| style.bg(theme.foreground.opacity(0.06)))
                .child(Icon::new(icon).size_4().flex_none())
                .child(div().flex_1().font_medium().child(name))
                .children(summary)
                .children(line)
                .child(Icon::new(chevron).xsmall().text_color(theme.muted_foreground))
                .on_click(cx.listener(move |this, _, _, cx| this.open_section(which, cx)));
            accordion = accordion.child(header);
            let leaving = self.sidebar.leaving == Some(which) && self.sidebar.tab.is_some();
            if open || leaving {
                let content = self.section_content(which, cx);
                // The padding inside: on the shutting one it would stay 4 px to the animation's end, then the rows below jump.
                let content = v_flex().flex_1().min_h_0().overflow_hidden().child(v_flex().size_full().pb_1().when(mac, |el| el.px_1()).child(content));
                accordion = match self.sidebar.leaving.is_some() {
                    true => {
                        let id = (if open { "sidebar-opening" } else { "sidebar-closing" }, self.sidebar.switches);
                        let slide = Animation::new(SLIDE).with_easing(ease_in_out);
                        accordion.child(content.with_animation(id, slide, move |mut el, delta| {
                            el.style().flex_grow = Some(if open { delta } else { 1. - delta });
                            el
                        }))
                    }
                    false => accordion.child(content),
                };
            }
        }
        accordion
    }

    /// The sections as a row of icons, as Xcode's navigators, the open one's on a pill.
    fn render_section_tabs(&mut self, tab: SidebarTab, look: TabsLook, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let (muted, hover, progress) = (theme.muted_foreground, theme.foreground.opacity(0.06), theme.progress_bar);
        let sections = sections();
        let summary = self.operations_summary();
        let count = sections.len() as f32;
        let to = section_at(tab);
        let pill = div().absolute().top_0().bottom_0().w(relative(1. / count)).rounded(look.radius).bg(look.pill);
        let pill = match self.sidebar.leaving.map(section_at) {
            Some(from) => {
                let slide = Animation::new(SLIDE).with_easing(ease_in_out);
                pill.with_animation(("sidebar-pill", self.sidebar.switches), slide, move |el, delta| {
                    el.left(relative((from + (to - from) * delta) / count))
                })
                .into_any_element()
            }
            None => pill.left(relative(to / count)).into_any_element(),
        };
        let icons = sections.into_iter().enumerate().map(|(at, (which, icon, name))| {
            let open = tab == which;
            // The queue's progress shows also while its section is shut.
            let (tip, line) = match (which, &summary) {
                (SidebarTab::Operations, Some((text, done))) => (format!("{name}: {text}"), *done),
                _ => (name.to_string(), None),
            };
            let line = line.map(|done| div().absolute().left(px(8.)).bottom(px(3.)).h(px(2.)).w(relative(done * 0.7)).rounded_full().bg(progress));
            div()
                .id(("sidebar-tab", at))
                .relative()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded(look.radius)
                .cursor_pointer()
                .map(|el| match open {
                    true => el.text_color(look.on_pill),
                    false => el.text_color(muted).hover(|style| style.bg(hover)),
                })
                .child(Icon::new(icon).size(px(15.)))
                .children(line)
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
                .on_click(cx.listener(move |this, _, _, cx| this.open_section(which, cx)))
        });
        h_flex().relative().flex_none().h(px(28.)).rounded(look.radius).bg(look.track).child(pill).children(icons)
    }

    /// The open section's content; switched to, it comes in from the side of its icon.
    fn render_open_section(&mut self, tab: SidebarTab, cx: &mut Context<Self>) -> AnyElement {
        let content = v_flex().relative().size_full().overflow_hidden().child(self.section_content(tab, cx));
        let Some(from) = self.sidebar.leaving.map(section_at) else {
            return content.into_any_element();
        };
        let to = section_at(tab);
        let side = if to > from { 12. } else { -12. };
        let slide = Animation::new(SLIDE).with_easing(ease_in_out);
        content
            .with_animation(("sidebar-content", self.sidebar.switches), slide, move |el, delta| {
                el.opacity(delta).left(px(side * (1. - delta)))
            })
            .into_any_element()
    }

    /// Shown or hidden, its edge slides; as the window opens, it is there already.
    fn slide_sidebar(&self, sidebar: Div) -> AnyElement {
        if self.sidebar.slides == 0 {
            return sidebar.into_any_element();
        }
        let width = self.sidebar.width;
        let out = self.sidebar.tab.is_none();
        let slide = Animation::new(SLIDE).with_easing(ease_in_out);
        let sliding = sidebar.with_animation(("sidebar-slide", self.sidebar.slides), slide, move |el, delta| {
            let shown = match out {
                true => 1. - delta,
                false => delta,
            };
            el.w(px(width * shown))
        });
        sliding.into_any_element()
    }

    /// macOS: a panel inset from the window's edges, the window's height, as
    /// Finder's, Notes' and Xcode's: the window's buttons and the sidebar's at its
    /// top, then the sections: a row of icons (Xcode's navigators), the open one's
    /// content under it; or their names one under another.
    fn render_mac_sidebar(&mut self, tab: SidebarTab, background: Hsla, horizontal: bool, mac: MacControls, cx: &mut Context<Self>) -> Div {
        let rim = match cx.theme().mode.is_dark() {
            true => hsla(0., 0., 1., 0.12),
            false => hsla(0., 0., 0., 0.12),
        };
        // Under the window's buttons: it moves the window, as a title bar.
        let top = TitleBar::new()
            .h(px(36.))
            .pl(px(header::MAC_LIGHTS_END) - INSET)
            .pr_2()
            .bg(transparent_black())
            .border_0()
            // At the right, as Notes'.
            .child(div().flex_1())
            .child(self.sidebar_toggle());
        let sections = match horizontal {
            true => {
                let look = TabsLook { track: mac.tab_track, pill: mac.accent, on_pill: mac.on_accent, radius: px(14.) };
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_section_tabs(tab, look, cx).mx_2())
                    // Kept off the rounded corners.
                    .child(v_flex().flex_1().min_h_0().pt_2().px_1().pb(px(12.)).child(self.render_open_section(tab, cx)))
            }
            false => self.render_accordion(tab, true, cx).pb(px(8.)),
        };
        let panel = v_flex()
            .size_full()
            .rounded(px(18.))
            .border_1()
            .border_color(rim)
            .child(top)
            .child(sections);
        div()
            .relative()
            .h_full()
            .flex_none()
            .w(px(self.sidebar.width))
            .pl(INSET)
            .py(INSET)
            .bg(background)
            .child(div().size_full().overflow_hidden().child(panel))
            .child(
                div()
                    .id("sidebar-edge")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(-3.))
                    .w(px(6.))
                    .occlude()
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_drag(SidebarDrag, |_, _, _, cx| cx.new(|_| EmptyView)),
            )
    }

    /// The sidebar's width, from where its edge is dragged in `row`.
    pub(crate) fn drag_sidebar(&mut self, row: Bounds<Pixels>, x: Pixels, cx: &mut Context<Self>) {
        self.sidebar.width = f32::from(x - row.left()).clamp(140., 640.);
        cx.notify();
    }

    fn render_tree(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let count = self.sidebar.tree.rows().len();
        let list = uniform_list(
            "sidebar-tree",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let rows = this.sidebar.tree.rows();
                range.map(|ix| this.render_tree_row(ix, rows[ix], cx)).collect()
            }),
        )
        .track_scroll(&self.sidebar.tree_scroll)
        .size_full()
        .pr(px(12.));
        let rows = div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(list)
            .child(div().absolute().inset_0().child(Scrollbar::vertical(&self.sidebar.tree_scroll)));
        v_flex().size_full().child(rows)
    }

    fn render_tree_row(&self, ix: usize, row: sidebar::Row, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        let selected = self.sidebar.followed.as_deref() == Some(row.path);
        let light = !theme.mode.is_dark();
        let icon = match (row.depth, row.path == Path::new("/")) {
            (0, true) => "folder-root.svg",
            (0, false) => "folder-home.svg",
            _ => icons::material().icon(row.name, true, light),
        };
        let icon = img(format!("material/{icon}")).size(px(16.)).flex_none();
        let path = row.path.to_owned();
        let toggled = path.clone();
        let chevron = match row.expanded {
            true => IconName::ChevronDown,
            false => IconName::ChevronRight,
        };
        h_flex()
            .id(("tree-row", ix))
            .w_full()
            .h(ROW)
            .pl(px(4. + 14. * row.depth as f32))
            .pr_1()
            .gap_1()
            .rounded(theme.radius)
            .when(selected, |el| selected_row(el, &self.colors, cx))
            .when(!selected, |el| el.hover(|style| style.bg(theme.foreground.opacity(0.06))))
            .child(
                div()
                    .id(("tree-toggle", ix))
                    .size(px(16.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(!row.leaf, |el| {
                        el.child(Icon::new(chevron).xsmall().text_color(theme.muted_foreground))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.sidebar.tree.toggle(&toggled);
                                    cx.notify();
                                }),
                            )
                    }),
            )
            .child(icon)
            .child(div().flex_1().min_w_0().truncate().child(row.name.to_owned()))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                match event.click_count() {
                    2 => this.sidebar.tree.toggle(&path),
                    _ => this.open_folder(path.clone(), window, cx),
                }
                cx.notify();
            }))
    }

    /// Asks KDE Connect for its devices now and every few seconds after, for
    /// as long as the window is there.
    pub(crate) fn watch_devices(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                let found = cx.background_executor().spawn(async { (devices::paired(), devices::volumes()) });
                let (paired, volumes) = found.await;
                if this.update(cx, |this, cx| this.set_devices(paired, volumes, cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(5)).await;
            }
        })
        .detach();
    }

    fn set_devices(&mut self, devices: Vec<Device>, volumes: Vec<Volume>, cx: &mut Context<Self>) {
        if devices == self.sidebar.devices && volumes == self.sidebar.volumes {
            return;
        }
        // KDE Connect unmounts what it can no longer reach.
        let browsable = |id: &String| devices.iter().any(|device| device.id == *id && device.browsable);
        self.sidebar.device_folders.retain(|id, _| browsable(id));
        self.sidebar.devices = devices;
        self.sidebar.volumes = volumes;
        cx.notify();
    }

    /// Shows the volume, mounting it first if it is not.
    pub(crate) fn open_volume(&mut self, volume: Volume, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = volume.path.clone() {
            return self.open_folder(path, window, cx);
        }
        let name = volume.name.clone();
        let note = Notification::info(t!("sidebar.volume.mounting", name = name)).id1::<Mounting>(name.clone()).autohide(false);
        window.push_notification(note, cx);
        // A thread of its own: an encrypted one waits on its passphrase.
        let (ask, kept) = self.dialog_ask(window, cx);
        let (done, mount) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let mounted = devices::mount_volume(&volume, &ask);
            // The passphrase asked to be kept, once it opened it.
            let keep = kept.lock().unwrap().take().filter(|_| mounted.is_ok());
            let _ = done.send((mounted.map(|path| (volume, path)), keep));
        });
        cx.spawn_in(window, async move |this, cx| {
            let (mounted, keep) = mount.await.unwrap_or_else(|_| (Err(std::io::Error::other("stopped")), None));
            let _ = this.update_in(cx, |this, window, cx| {
                window.remove_notification1::<Mounting>(name.clone(), cx);
                if let Some((key, passphrase)) = keep {
                    crate::dialogs::passwords_dialog::keep(key, passphrase, window, cx);
                }
                match mounted {
                    Ok((volume, path)) => {
                        let listed = this.sidebar.volumes.iter_mut().find(|listed| **listed == volume);
                        if let Some(listed) = listed {
                            listed.path = Some(path.clone());
                        }
                        this.open_folder(path, window, cx);
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(err) => window.push_notification(Notification::error(format!("{name}: {err}")).autohide(false), cx),
                }
            });
        })
        .detach();
    }

    /// Panels in it go home first, so nothing holds it.
    fn unmount_volume(&mut self, volume: Volume, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mount) = volume.path.clone() else {
            return;
        };
        let home = std::env::home_dir().unwrap_or_else(|| "/".into());
        for ix in 0..2 {
            let panel = self.panels[ix].tabs.current();
            if panel.remote().is_none() && panel.folder().starts_with(&mount) {
                let home = home.clone();
                self.navigate_on(ix, false, move |panel| panel.go_to_folder(home), window, cx);
            }
        }
        let unmount = cx.background_executor().spawn(async move { devices::unmount_volume(&volume).map(|()| volume) });
        cx.spawn_in(window, async move |this, cx| {
            let unmounted = unmount.await;
            let _ = this.update_in(cx, |this, window, cx| match unmounted {
                Ok(volume) => {
                    if let Some(listed) = this.sidebar.volumes.iter_mut().find(|listed| **listed == volume) {
                        listed.path = None;
                        // Locked again: its file system is gone.
                        if listed.encrypted.is_some() {
                            listed.udisks = None;
                        }
                    }
                    cx.notify();
                }
                Err(err) => window.push_notification(Notification::error(t!("sidebar.volume.unmount_failed", err = err)), cx),
            });
        })
        .detach();
    }

    fn render_volume(&self, ix: usize, volume: &Volume, inside: Option<&Path>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let selected = volume.path.is_some() && volume.path.as_deref() == inside;
        let muted = secondary(selected, cx);
        let icon = volume_icon(volume.kind);
        let detail = match (&volume.path, &volume.udisks) {
            (Some(_), _) => String::new(),
            (None, None) if volume.encrypted.is_some() => t!("sidebar.volume.locked"),
            (None, _) => t!("sidebar.volume.not_mounted"),
        };
        // A disk's size and free space, under its name; not a share's, which may hang.
        let disk = matches!(volume.kind, VolumeKind::System | VolumeKind::Drive | VolumeKind::Removable);
        let space = (volume.path.as_deref().filter(|_| disk))
            .and_then(|path| devices::disk_space(path).ok())
            .map(|(size, free)| t!("sidebar.volume.space", free = sc_core::human_size(free), total = sc_core::human_size(size)));
        let (opened, menu_volume) = (volume.clone(), volume.clone());
        let commander = cx.entity().downgrade();
        let name = v_flex()
            .map(|el| match space.is_some() {
                true => el.flex_1().min_w_0(),
                false => el.flex_none().max_w(relative(0.7)),
            })
            .child(div().truncate().child(volume.name.clone()))
            .when_some(space.clone(), |el, space| el.child(div().truncate().text_xs().text_color(muted).child(space)));
        h_flex()
            .id(("volume", ix))
            .map(|el| match space.is_some() {
                true => el.py_0p5(),
                false => el.h(ROW + px(2.)),
            })
            .px_1()
            .gap_1p5()
            .rounded(theme.radius)
            .cursor_pointer()
            .when(selected, |el| selected_row(el, &self.colors, cx))
            .when(!selected, |el| el.hover(|style| style.bg(theme.foreground.opacity(0.06))))
            .child(icon.small().text_color(muted))
            .child(name)
            .when(!detail.is_empty(), |el| el.child(div().flex_1().min_w_0().truncate().text_xs().text_color(muted).child(detail)))
            .on_click(cx.listener(move |this, _, window, cx| this.open_volume(opened.clone(), window, cx)))
            .context_menu(move |menu, _, _| {
                let act = |label: String, unmount: bool| {
                    let (commander, volume) = (commander.clone(), menu_volume.clone());
                    PopupMenuItem::new(label).on_click(move |_, window, cx| {
                        let volume = volume.clone();
                        let _ = commander.update(cx, |this, cx| match unmount {
                            true => this.unmount_volume(volume, window, cx),
                            false => this.open_volume(volume, window, cx),
                        });
                    })
                };
                let mounted = menu_volume.path.is_some() && menu_volume.udisks.is_some();
                let menu = menu.item(act(t!("sidebar.volume.open"), false));
                match mounted {
                    true => menu.item(act(t!("sidebar.volume.unmount"), true)),
                    false => menu,
                }
            })
    }

    /// Mounts the device in the background, afresh where its mount went stale,
    /// then shows `folder`, else its widest one.
    fn open_device(&mut self, device: &Device, folder: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let (id, name) = (device.id.clone(), device.name.clone());
        // Mounted, it is quick.
        if !self.sidebar.device_folders.contains_key(&id) {
            let note = Notification::info(t!("sidebar.device.opening", name = name)).id1::<Mounting>(id.clone()).autohide(false);
            window.push_notification(note, cx);
        }
        let mount = {
            let id = id.clone();
            cx.background_executor().spawn(async move { devices::mount(&id) })
        };
        cx.spawn_in(window, async move |this, cx| {
            let mounted = mount.await;
            let _ = this.update_in(cx, |this, window, cx| {
                window.remove_notification1::<Mounting>(id.clone(), cx);
                match mounted {
                    Ok(folders) => {
                        let shown = folder.unwrap_or_else(|| folders[0].0.clone());
                        this.sidebar.device_folders.insert(id, folders);
                        this.open_folder(shown, window, cx);
                    }
                    Err(err) => {
                        let note = Notification::error(format!("{name}: {err}")).autohide(false);
                        window.push_notification(note, cx);
                    }
                }
            });
        })
        .detach();
    }

    /// Mounted phones whose files stopped answering, connected again (KDE
    /// Connect's app stops serving them when the phone sleeps, the mount here
    /// staying): what showed them reads them again. Once in 30 s each; a
    /// phone that still does not answer is said to.
    pub(crate) fn reconnect_devices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mounted: Vec<(Device, Vec<PathBuf>)> =
            self.sidebar.devices.iter().map(|device| (device.clone(), device_roots(&self.sidebar, &device.id))).filter(|(_, roots)| !roots.is_empty()).collect();
        // A dead mount may hang as well as fail: looked at in the background.
        let dead = cx.background_executor().spawn(async move {
            let lost = |root: &PathBuf| std::fs::metadata(root).err().is_some_and(|err| lost_device(&err));
            mounted.into_iter().filter(|(_, roots)| roots.iter().any(lost)).map(|(device, _)| device).collect::<Vec<_>>()
        });
        cx.spawn_in(window, async move |this, cx| {
            let dead = dead.await;
            let _ = this.update_in(cx, |this, window, cx| dead.into_iter().for_each(|device| this.reconnect_device(device, window, cx)));
        })
        .detach();
    }

    fn reconnect_device(&mut self, device: Device, window: &mut Window, cx: &mut Context<Self>) {
        let (id, name) = (device.id.clone(), device.name.clone());
        if self.sidebar.reconnected.get(&id).is_some_and(|at| at.elapsed() < Duration::from_secs(30)) {
            let text = t!("sidebar.device.not_answering", name = name);
            return window.push_notification(Notification::error(text).id1::<Mounting>(id), cx);
        }
        self.sidebar.reconnected.insert(id.clone(), std::time::Instant::now());
        let note = Notification::info(t!("sidebar.device.reconnecting", name = name)).id1::<Mounting>(id.clone()).autohide(false);
        window.push_notification(note, cx);
        let mount = cx.background_executor().spawn({
            let id = id.clone();
            async move {
                // Mounted, it would not mount again.
                let _ = devices::unmount(&id);
                devices::mount(&id)
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let mounted = mount.await;
            let _ = this.update_in(cx, |this, window, cx| {
                window.remove_notification1::<Mounting>(id.clone(), cx);
                match mounted {
                    Ok(folders) => {
                        this.sidebar.device_folders.insert(id.clone(), folders);
                        let roots = device_roots(&this.sidebar, &id);
                        for ix in 0..2 {
                            let panel = this.panels[ix].tabs.current();
                            if panel.remote().is_none() && roots.iter().any(|root| panel.folder().starts_with(root)) {
                                this.navigate_on(ix, false, sc_core::Panel::reload, window, cx);
                            }
                        }
                        this.reload_quick_view();
                        cx.notify();
                    }
                    Err(err) => {
                        let text = t!("sidebar.device.not_answering_error", name = name, err = err);
                        window.push_notification(Notification::error(text).autohide(false), cx);
                    }
                }
            });
        })
        .detach();
    }

    /// The mounted device whose files `path` is in.
    pub(crate) fn device_of(&self, path: &Path) -> Option<Device> {
        let holds = |device: &&Device| device_roots(&self.sidebar, &device.id).iter().any(|root| path.starts_with(root));
        self.sidebar.devices.iter().find(holds).cloned()
    }

    /// The path bar's ×, or Disconnect, on a device: panels in its files go
    /// home first, so nothing holds them; it stays paired.
    pub(crate) fn unmount_device(&mut self, device: &Device, window: &mut Window, cx: &mut Context<Self>) {
        let roots = device_roots(&self.sidebar, &device.id);
        if roots.is_empty() {
            return;
        }
        let home = std::env::home_dir().unwrap_or_else(|| "/".into());
        for ix in 0..2 {
            let panel = self.panels[ix].tabs.current();
            if panel.remote().is_none() && roots.iter().any(|root| panel.folder().starts_with(root)) {
                let home = home.clone();
                self.navigate_on(ix, false, move |panel| panel.go_to_folder(home), window, cx);
            }
        }
        let (id, name) = (device.id.clone(), device.name.clone());
        let unmount = cx.background_executor().spawn({
            let id = id.clone();
            async move { devices::unmount(&id) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let unmounted = unmount.await;
            let _ = this.update_in(cx, |this, window, cx| match unmounted {
                Ok(()) => {
                    this.sidebar.device_folders.remove(&id);
                    cx.notify();
                }
                Err(err) => window.push_notification(Notification::error(t!("sidebar.device.unmount_failed", name = name, err = err)), cx),
            });
        })
        .detach();
    }

    fn render_places(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let panel = self.panels[self.active].tabs.current();
        let here = panel.remote().is_none().then(|| panel.folder().to_owned());
        let mut list = v_flex().id("places").flex_1().min_h_0().overflow_y_scroll().px_1();
        list = list.child(self.render_bookmarks(here.as_deref(), cx));
        // The deepest volume the active panel is in.
        let inside: Option<PathBuf> = here.as_ref().and_then(|here| {
            let mounted = self.sidebar.volumes.iter().filter_map(|volume| volume.path.as_ref());
            mounted.filter(|path| here.starts_with(path)).max_by_key(|path| path.components().count()).cloned()
        });
        let of = |kinds: &[VolumeKind]| -> Vec<(usize, Volume)> {
            let volumes = self.sidebar.volumes.iter().cloned().enumerate();
            volumes.filter(|(_, volume)| kinds.contains(&volume.kind)).collect()
        };
        let drives = of(&[VolumeKind::System, VolumeKind::Drive, VolumeKind::Removable]);
        let (phones, network) = (of(&[VolumeKind::Phone]), of(&[VolumeKind::Network]));
        let cloud = of(&[VolumeKind::Cloud]);
        if !drives.is_empty() {
            list = list.child(section(t!("sidebar.group.drives"), cx));
        }
        for (ix, volume) in &drives {
            list = list.child(self.render_volume(*ix, volume, inside.as_deref(), cx));
        }
        if !cloud.is_empty() {
            list = list.child(section(t!("sidebar.group.cloud"), cx));
        }
        for (ix, volume) in &cloud {
            list = list.child(self.render_volume(*ix, volume, inside.as_deref(), cx));
        }
        if !phones.is_empty() || !self.sidebar.devices.is_empty() {
            list = list.child(section(t!("sidebar.group.devices"), cx));
        }
        for (ix, volume) in &phones {
            list = list.child(self.render_volume(*ix, volume, inside.as_deref(), cx));
        }
        for device in &self.sidebar.devices {
            let theme = cx.theme();
            let icon = match device.kind.as_str() {
                "tablet" => Lucide::Tablet,
                "laptop" => Lucide::Laptop,
                "desktop" => Lucide::Monitor,
                "tv" => Lucide::Tv,
                _ => Lucide::Smartphone,
            };
            let detail = match (device.reachable, device.browsable) {
                (false, _) => t!("sidebar.device.not_connected"),
                (true, false) => t!("sidebar.device.no_files"),
                (true, true) => String::new(),
            };
            let opened = device.clone();
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("device-{}", device.id)))
                    .h(ROW + px(2.))
                    .px_1()
                    .gap_1p5()
                    .rounded(theme.radius)
                    .map(|el| match device.browsable {
                        true => el
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.foreground.opacity(0.06)))
                            .on_click(cx.listener(move |this, _, window, cx| this.open_device(&opened, None, window, cx))),
                        false => el.text_color(theme.muted_foreground),
                    })
                    .child(Icon::new(icon).small().text_color(theme.muted_foreground))
                    .child(div().flex_none().max_w(relative(0.7)).truncate().child(device.name.clone()))
                    .child(div().flex_1().min_w_0().truncate().text_xs().text_color(theme.muted_foreground).child(detail)),
            );
            let Some(folders) = self.sidebar.device_folders.get(&device.id) else {
                continue;
            };
            // The deepest of its folders the active panel is in.
            let inside = here.as_ref().and_then(|here| {
                let holding = folders.iter().filter(|(path, _)| here.starts_with(path));
                holding.max_by_key(|(path, _)| path.components().count()).map(|(path, _)| path.clone())
            });
            for (ix, (path, name)) in folders.iter().enumerate() {
                let theme = cx.theme();
                let selected = inside.as_ref() == Some(path);
                let icon = icons::material().icon(name, true, !theme.mode.is_dark());
                let (path, opened) = (path.clone(), device.clone());
                list = list.child(
                    h_flex()
                        .id(SharedString::from(format!("device-{}-{ix}", device.id)))
                        .h(ROW)
                        .pl(px(26.))
                        .pr_1()
                        .gap_1()
                        .rounded(theme.radius)
                        .cursor_pointer()
                        .when(selected, |el| selected_row(el, &self.colors, cx))
                        .when(!selected, |el| el.hover(|style| style.bg(theme.foreground.opacity(0.06))))
                        .child(img(format!("material/{icon}")).size(px(16.)).flex_none())
                        .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_device(&opened, Some(path.clone()), window, cx)
                        })),
                );
            }
        }
        if !network.is_empty() {
            list = list.child(section(t!("sidebar.group.network"), cx));
        }
        for (ix, volume) in &network {
            list = list.child(self.render_volume(*ix, volume, inside.as_deref(), cx));
        }
        v_flex().size_full().child(list)
    }

    /// Places > Bookmarks, shown also when empty: where folders are dropped.
    fn render_bookmarks(&self, here: Option<&Path>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let target = theme.accent.opacity(0.35);
        let manage = Button::new("bookmarks-manage")
            .ghost()
            .xsmall()
            .icon(IconName::Ellipsis)
            .tooltip(t!("command.manage_bookmarks"))
            .on_click(cx.listener(|this, _, window, cx| this.manage_bookmarks(window, cx)));
        let header = h_flex()
            .child(section(t!("sidebar.group.bookmarks"), cx).flex_1())
            .child(div().invisible().group_hover("bookmarks", |style| style.visible()).child(manage));
        let mut block = v_flex()
            .id("bookmarks")
            .group("bookmarks")
            .rounded(theme.radius)
            .drag_over::<Dragged>(move |style, _, _, _| style.bg(target))
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(target))
            .on_drop(cx.listener(|this, dragged: &Dragged, window, cx| {
                let folders = this.dragged_folders(*dragged);
                this.add_bookmarks(folders, window, cx)
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.add_bookmarks(paths.paths().to_vec(), window, cx)
            }))
            .child(header);
        if self.settings.bookmarks.is_empty() {
            let hint = div().px_1().pb_1().text_xs().text_color(theme.muted_foreground);
            block = block.child(hint.child(t!("bookmarks.drop_here")));
        }
        let home = std::env::home_dir();
        for (ix, path) in self.settings.bookmarks.iter().enumerate() {
            let theme = cx.theme();
            let folder = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
            let name = self.settings.bookmark_name(path);
            // Home's would be /home: nothing to tell.
            let parent = path.parent().filter(|_| home.as_deref() != Some(path.as_path())).map(|parent| tilde(parent, home.as_deref()));
            let selected = here == Some(path.as_path());
            let icon = icons::material().icon(&folder, true, !theme.mode.is_dark());
            let (opened, commander) = (path.clone(), cx.entity().downgrade());
            block = block.child(
                h_flex()
                    .id(("bookmark", ix))
                    .h(ROW + px(2.))
                    .px_1()
                    .gap_1p5()
                    .rounded(theme.radius)
                    .cursor_pointer()
                    .when(selected, |el| selected_row(el, &self.colors, cx))
                    .when(!selected, |el| el.hover(|style| style.bg(theme.foreground.opacity(0.06))))
                    .child(img(format!("material/{icon}")).size(px(16.)).flex_none())
                    .child(div().flex_none().max_w(relative(0.6)).truncate().child(name))
                    .child(div().flex_1().min_w_0().truncate().text_xs().text_color(secondary(selected, cx)).children(parent))
                    .on_click(cx.listener(move |this, _, window, cx| this.open_folder(opened.clone(), window, cx)))
                    .context_menu(move |menu, _, _| {
                        let act = |label: String, run: fn(&mut Commander, usize, &mut Window, &mut Context<Commander>)| {
                            let commander = commander.clone();
                            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                                let _ = commander.update(cx, |this, cx| run(this, ix, window, cx));
                            })
                        };
                        menu.item(act(t!("bookmarks.rename"), Commander::rename_bookmark))
                            .item(act(t!("bookmarks.move_up"), |this, ix, window, cx| this.move_bookmark(ix, ix.wrapping_sub(1), window, cx)))
                            .item(act(t!("bookmarks.move_down"), |this, ix, window, cx| this.move_bookmark(ix, ix + 1, window, cx)))
                            .separator()
                            .item(act(t!("bookmarks.copy_name"), |this, ix, _, cx| {
                                let name = this.settings.bookmarks.get(ix).map(|path| this.settings.bookmark_name(path));
                                cx.write_to_clipboard(ClipboardItem::new_string(name.unwrap_or_default()));
                            }))
                            .item(act(t!("bookmarks.copy_path"), |this, ix, _, cx| {
                                let path = this.settings.bookmarks.get(ix).map(|path| path.display().to_string());
                                cx.write_to_clipboard(ClipboardItem::new_string(path.unwrap_or_default()));
                            }))
                            .separator()
                            .item(act(t!("bookmarks.remove_from"), Commander::remove_bookmark))
                            .item(act(t!("command.manage_bookmarks"), |this, _, window, cx| this.manage_bookmarks(window, cx)))
                    }),
            );
        }
        block
    }

    /// Go > Bookmark This Folder (Ctrl+B): the active panel's folder.
    pub(crate) fn bookmark_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        let folder = panel.folder().to_owned();
        if panel.remote().is_some() || panel.in_archive() || panel.is_found() || !sc_core::settings::bookmarkable(&folder) {
            return window.push_notification(Notification::warning(t!("bookmarks.only_local")), cx);
        }
        let name = folder.file_name().map_or_else(|| folder.display().to_string(), |name| name.to_string_lossy().into_owned());
        let note = match self.settings.bookmarks.contains(&folder) {
            true => t!("bookmarks.already", name = name),
            false => t!("bookmarks.added", name = name),
        };
        self.add_bookmarks(vec![folder], window, cx);
        window.push_notification(Notification::info(note), cx);
    }

    /// A bookmark's own name; an empty one gives it its folder's again.
    pub(crate) fn rename_bookmark(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.settings.bookmarks.get(ix).cloned() else {
            return;
        };
        let name = self.settings.bookmark_name(&path);
        let commander = cx.entity().downgrade();
        crate::dialogs::name_dialog::open(t!("bookmarks.rename_title"), t!("bookmarks.rename_label"), name, window, cx, move |name, window, cx| {
            let _ = commander.update(cx, |this, cx| {
                match name.trim() {
                    "" => this.settings.bookmark_names.remove(&path),
                    name => this.settings.bookmark_names.insert(path.clone(), name.to_owned()),
                };
                this.save_settings(window, cx);
                cx.notify();
            });
        });
    }

    /// The `ix`th bookmark to place `to` in the list.
    pub(crate) fn move_bookmark(&mut self, ix: usize, to: usize, window: &mut Window, cx: &mut Context<Self>) {
        let bookmarks = &mut self.settings.bookmarks;
        if ix == to || ix >= bookmarks.len() || to >= bookmarks.len() {
            return;
        }
        let path = bookmarks.remove(ix);
        bookmarks.insert(to, path);
        self.save_settings(window, cx);
        cx.notify();
    }

    pub(crate) fn remove_bookmark(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix < self.settings.bookmarks.len() {
            let gone = self.settings.bookmarks.remove(ix);
            self.settings.bookmark_names.remove(&gone);
            self.save_settings(window, cx);
            cx.notify();
        }
    }

    /// Bookmarks A to Z.
    pub(crate) fn sort_bookmarks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.sort_bookmarks();
        self.save_settings(window, cx);
        cx.notify();
    }

    /// Back to the bookmarks it starts with; asked first, as the user's go.
    pub(crate) fn confirm_reset_bookmarks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let commander = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let commander = commander.clone();
            alert.title(t!("bookmarks.reset_title")).description(t!("bookmarks.reset_confirm")).ok_text(t!("bookmarks.reset_button")).show_cancel(true).on_ok(move |_, window, cx| {
                let _ = commander.update(cx, |this, cx| {
                    this.settings.reset_bookmarks();
                    this.save_settings(window, cx);
                    cx.notify();
                });
                true
            })
        });
    }

    /// A panel's dragged folders, this computer's only.
    pub(crate) fn dragged_folders(&self, dragged: Dragged) -> Vec<PathBuf> {
        let panel = self.panels[dragged.side].tabs.current();
        if panel.remote().is_some() || panel.in_archive() {
            return Vec::new();
        }
        let entries = panel.dragged(dragged.row);
        entries.iter().filter(|e| e.is_dir && !e.is_parent()).map(|e| panel.folder().join(&e.name)).collect()
    }

    /// This computer's folders among `paths` bookmarked, once each; files and servers' folders are not.
    pub(crate) fn add_bookmarks(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let count = paths.len();
        let folders: Vec<PathBuf> = paths.into_iter().filter(|path| sc_core::settings::bookmarkable(path)).collect();
        if folders.len() < count {
            window.push_notification(Notification::warning(t!("bookmarks.only_local")), cx);
        }
        let bookmarks = &mut self.settings.bookmarks;
        let before = bookmarks.len();
        for folder in folders {
            if !bookmarks.contains(&folder) {
                bookmarks.push(folder);
            }
        }
        if bookmarks.len() != before {
            self.save_settings(window, cx);
        }
        cx.notify();
    }

    /// Go > Bookmarks: Total Commander's Directory Hotlist over the active
    /// panel, the sidebar's bookmarks listed; the folder shown is checked.
    pub(crate) fn show_bookmarks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        let here = panel.remote().is_none().then(|| panel.folder().to_owned());
        let home = std::env::home_dir();
        let bookmarks: Vec<(String, PathBuf)> = self.settings.bookmarks.iter().map(|path| {
            (format!("{}   {}", self.settings.bookmark_name(path), tilde(path, home.as_deref())), path.clone())
        }).collect();
        let (focus, commander) = (self.focus.clone(), cx.entity().downgrade());
        let list = PopupMenu::build(window, cx, move |mut menu, _, _| {
            for (label, path) in &bookmarks {
                let (commander, path) = (commander.clone(), path.clone());
                let item = PopupMenuItem::new(label.clone()).checked(here.as_ref() == Some(&path)).on_click(move |_, window, cx| {
                    let _ = commander.update(cx, |this, cx| this.open_folder(path.clone(), window, cx));
                });
                menu = menu.item(item);
            }
            if !bookmarks.is_empty() {
                menu = menu.separator();
            }
            menu.command(t!("bookmarks.add_current"), Command::BookmarkFolder)
                .command(t!("command.manage_bookmarks"), Command::ManageBookmarks)
                .action_context(focus)
        });
        self.show_panel_list(list, window, cx);
    }

    /// Total Commander's Alt+F1 and Alt+F2: `side`'s drives in a list over it,
    /// the one it is on checked; that panel takes the keys. This computer's
    /// disks only: no cloud folder, share or phone.
    pub(crate) fn show_drives(&mut self, side: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = side;
        let panel = self.panels[side].tabs.current();
        let here = panel.remote().is_none().then(|| panel.folder().to_owned());
        let paths = self.sidebar.volumes.iter().filter_map(|volume| volume.path.clone());
        // The deepest it is under: /home before /.
        let on = paths.filter(|path| here.as_ref().is_some_and(|here| here.starts_with(path))).max_by_key(|path| path.components().count());
        let home = std::env::home_dir();
        let local = self.sidebar.volumes.iter().filter(|volume| matches!(volume.kind, VolumeKind::System | VolumeKind::Drive | VolumeKind::Removable));
        let drives: Vec<(Volume, String)> = local.map(|volume| {
            let label = match &volume.path {
                // Windows' names have the letter already.
                Some(path) if !(cfg!(windows) && path.parent().is_none()) => format!("{}   {}", volume.name, tilde(path, home.as_deref())),
                _ => volume.name.clone(),
            };
            (volume.clone(), label)
        }).collect();
        // Windows' drive letter opens it; elsewhere the name's first letter highlights it.
        let letters: Vec<(char, usize, bool)> = drives.iter().enumerate().filter_map(|(ix, (volume, _))| {
            let root = volume.path.as_ref().filter(|path| cfg!(windows) && path.parent().is_none());
            let letter = match root {
                Some(path) => path.to_string_lossy().chars().next(),
                None => volume.name.chars().find(|c| c.is_alphanumeric()),
            }?;
            Some((letter.to_ascii_lowercase(), ix, root.is_some()))
        }).collect();
        let commander = cx.entity().downgrade();
        let list = PopupMenu::build(window, cx, move |mut menu, _, _| {
            menu = menu.check_side(Side::Right);
            for (volume, label) in &drives {
                let (commander, opened) = (commander.clone(), volume.clone());
                let current = volume.path.is_some() && volume.path == on;
                let item = PopupMenuItem::new(label.clone()).icon(volume_icon(volume.kind)).checked(current).on_click(move |_, window, cx| {
                    let _ = commander.update(cx, |this, cx| this.open_volume(opened.clone(), window, cx));
                });
                menu = menu.item(item);
            }
            menu
        });
        self.show_panel_list(list, window, cx);
        self.panel_letters = letters;
    }

    /// Over the Connections list: new folder and connection; the chosen one
    /// edited, deleted, moved up or down, or into another folder.
    fn render_connections_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let all = &self.sidebar.connections;
        let picked = self.sidebar.selected.clone();
        let saved = match picked {
            Some(Picked::Saved(ix)) if ix < all.saved.len() => Some(ix),
            _ => None,
        };
        let (up, down) = match &picked {
            Some(Picked::Saved(ix)) => (all.clone().step_connection(*ix, true).is_some(), all.clone().step_connection(*ix, false).is_some()),
            Some(Picked::Folder(name)) => (all.clone().step_folder(name, true), all.clone().step_folder(name, false)),
            _ => (false, false),
        };
        // ~/.ssh/config's hosts are its own: never changed here.
        let changeable = matches!(picked, Some(Picked::Saved(_) | Picked::Folder(_)));
        let button = |id: &'static str, icon: Lucide, tip: String| Button::new(id).ghost().small().icon(Icon::new(icon)).tooltip(tip);
        let edit = picked.clone();
        let delete = picked.clone();
        let folders = all.folders.clone();
        let commander = cx.entity().downgrade();
        let into = move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
            let Some(ix) = saved else {
                return menu;
            };
            let places = std::iter::once((None, t!("connections.no_folder"))).chain(folders.iter().map(|folder| (Some(folder.clone()), folder.clone())));
            places.fold(menu, |menu, (folder, label)| {
                let commander = commander.clone();
                menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                    let folder = folder.clone();
                    let _ = commander.update(cx, |this, cx| this.move_server(ix, folder, None, window, cx));
                }))
            })
        };
        h_flex()
            .flex_none()
            .px_1()
            .gap_0p5()
            .child(button("server-new-folder", Lucide::FolderPlus, t!("connections.new_folder")).on_click(cx.listener(|this, _, window, cx| this.name_server_folder(None, window, cx))))
            .child(button("server-new", Lucide::Plus, t!("connections.new")).on_click(cx.listener(|this, _, window, cx| this.edit_server(None, false, window, cx))))
            .child(button("server-edit", Lucide::Pencil, t!("connections.edit")).disabled(!changeable).on_click(cx.listener(move |this, _, window, cx| match edit.clone() {
                Some(Picked::Saved(ix)) => this.edit_server(Some(ix), false, window, cx),
                Some(Picked::Folder(name)) => this.name_server_folder(Some(name), window, cx),
                _ => {}
            })))
            .child(button("server-delete", Lucide::Trash, t!("connections.delete")).disabled(!changeable).on_click(cx.listener(move |this, _, window, cx| match delete.clone() {
                Some(Picked::Saved(ix)) => this.delete_server(ix, window, cx),
                Some(Picked::Folder(name)) => this.delete_server_folder(name, window, cx),
                _ => {}
            })))
            .child(button("server-up", Lucide::ArrowUp, t!("connections.move_up")).disabled(!up).on_click(cx.listener(|this, _, window, cx| this.step_server(true, window, cx))))
            .child(button("server-down", Lucide::ArrowDown, t!("connections.move_down")).disabled(!down).on_click(cx.listener(|this, _, window, cx| this.step_server(false, window, cx))))
            .child(button("server-into", Lucide::FolderInput, t!("connections.move_to_folder")).disabled(saved.is_none()).dropdown_menu(into))
    }

    fn render_connections(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let groups = sidebar::servers(&self.sidebar.connections, &self.sidebar.hosts);
        // A drop on no row: a connection last at the top, a folder last.
        let mut list = v_flex()
            .id("connections")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_1()
            .on_drop(cx.listener(|this, ServerDrag(ix): &ServerDrag, window, cx| this.move_server(*ix, None, None, window, cx)))
            .on_drop(cx.listener(|this, ServerFolderDrag(name): &ServerFolderDrag, window, cx| this.move_server_folder(name.clone(), None, window, cx)));
        if groups.iter().all(|(group, servers)| *group == Group::Top && servers.is_empty()) {
            let theme = cx.theme();
            list = list.child(div().p_2().whitespace_normal().text_color(theme.muted_foreground).child(t!("connections.empty")));
        }
        for (group, servers) in groups {
            let open = !self.sidebar.closed.contains(&group);
            let count = servers.len();
            let folder = match &group {
                Group::Top => None,
                Group::Folder(name) => {
                    list = list.child(self.render_server_folder(name.clone(), count, open, cx));
                    Some(name.clone())
                }
                Group::SshConfig => {
                    list = list.child(self.render_ssh_config(count, open, cx));
                    None
                }
            };
            if !open {
                continue;
            }
            let indent = if group == Group::Top { px(6.) } else { px(22.) };
            for (at, server) in servers.into_iter().enumerate() {
                list = list.child(self.render_server(at, server, folder.clone(), indent, cx));
            }
        }
        v_flex().size_full().child(self.render_connections_toolbar(cx)).child(list)
    }

    /// A folder's heading: a click chooses it, its arrow or a double click
    /// opens or shuts it; connections dropped on it go last in it, a folder before it.
    fn render_server_folder(&self, name: String, count: usize, open: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let selected = self.sidebar.selected.as_ref() == Some(&Picked::Folder(name.clone()));
        let muted = secondary(selected, cx);
        let target = theme.drop_target;
        let group = Group::Folder(name.clone());
        let toggle = move |this: &mut Self, cx: &mut Context<Self>| {
            match this.sidebar.closed.iter().position(|shut| *shut == group) {
                Some(at) => drop(this.sidebar.closed.remove(at)),
                None => this.sidebar.closed.push(group.clone()),
            }
            cx.notify();
        };
        let chevron = Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).xsmall().text_color(muted);
        let arrow = div().id("folder-arrow").child(chevron).on_click({
            let toggle = toggle.clone();
            cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                toggle(this, cx)
            })
        });
        let commander = cx.entity().downgrade();
        let (dragged, into, before, menu_name) = (name.clone(), name.clone(), name.clone(), name.clone());
        h_flex()
            .id(SharedString::from(format!("server-folder-{name}")))
            .h(ROW)
            .px_1()
            .gap_1()
            .rounded(theme.radius)
            .cursor_pointer()
            .when(selected, |el| selected_row(el, &self.colors, cx))
            .when(!selected, |el| el.hover(|style| style.bg(theme.foreground.opacity(0.06))))
            .child(arrow)
            .child(Icon::new(if open { IconName::FolderOpen } else { IconName::FolderClosed }).xsmall().text_color(muted))
            .child(div().flex_1().min_w_0().truncate().font_semibold().child(name.clone()))
            .child(div().text_xs().text_color(muted).child(count.to_string()))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                this.sidebar.selected = Some(Picked::Folder(name.clone()));
                if event.click_count() == 2 {
                    toggle(this, cx);
                }
                cx.notify();
            }))
            .on_drag(ServerFolderDrag(dragged.clone()), move |_, grab, _, cx| cx.new(|_| DragLabel { text: dragged.clone().into(), grab }))
            .drag_over::<ServerDrag>(move |style, _, _, _| style.bg(target))
            .drag_over::<ServerFolderDrag>(move |style, _, _, _| style.bg(target))
            .on_drop(cx.listener(move |this, ServerDrag(ix): &ServerDrag, window, cx| this.move_server(*ix, Some(into.clone()), None, window, cx)))
            .on_drop(cx.listener(move |this, ServerFolderDrag(moved): &ServerFolderDrag, window, cx| {
                this.move_server_folder(moved.clone(), Some(before.clone()), window, cx)
            }))
            .context_menu(move |menu, _, _| {
                let act = |label: String, run: fn(&mut Commander, String, &mut Window, &mut Context<Commander>)| {
                    let (commander, name) = (commander.clone(), menu_name.clone());
                    PopupMenuItem::new(label).on_click(move |_, window, cx| {
                        let name = name.clone();
                        let _ = commander.update(cx, |this, cx| run(this, name, window, cx));
                    })
                };
                menu.item(act(t!("connections.new"), |this, name, window, cx| {
                    this.sidebar.selected = Some(Picked::Folder(name));
                    this.edit_server(None, false, window, cx)
                }))
                .separator()
                .item(act(t!("connections.folder.rename"), |this, name, window, cx| this.name_server_folder(Some(name), window, cx)))
                .item(act(t!("connections.delete"), |this, name, window, cx| this.delete_server_folder(name, window, cx)))
            })
    }

    /// The heading of `~/.ssh/config`'s hosts: read only, that file is the user's.
    fn render_ssh_config(&self, count: usize, open: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let chevron = Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).xsmall().text_color(theme.muted_foreground);
        h_flex()
            .id("server-ssh-config")
            .h(ROW)
            .px_1()
            .gap_1()
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|style| style.bg(theme.foreground.opacity(0.06)))
            .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new(t!("connections.ssh_config.read_only")).build(window, cx))
            .child(chevron)
            .child(Icon::new(Lucide::Lock).xsmall().text_color(theme.muted_foreground))
            .child(div().flex_1().font_semibold().child("~/.ssh/config"))
            .child(div().text_xs().text_color(theme.muted_foreground).child(count.to_string()))
            // Nothing goes into it.
            .on_drop(|_: &ServerDrag, _, _| {})
            .on_drop(|_: &ServerFolderDrag, _, _| {})
            .on_click(cx.listener(|this, _, _, cx| {
                match this.sidebar.closed.iter().position(|shut| *shut == Group::SshConfig) {
                    Some(at) => drop(this.sidebar.closed.remove(at)),
                    None => this.sidebar.closed.push(Group::SshConfig),
                }
                cx.notify();
            }))
    }

    /// A server's row, `at` in its group; a saved one is dragged, and takes
    /// a connection dropped on it before it.
    fn render_server(&self, at: usize, server: sidebar::Server, folder: Option<String>, indent: Pixels, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let picked = match server.saved {
            Some(ix) => Picked::Saved(ix),
            None => Picked::Host(at),
        };
        let selected = self.sidebar.selected.as_ref() == Some(&picked);
        let muted = secondary(selected, cx);
        let commander = cx.entity().downgrade();
        let target = theme.drop_target;
        let (address, saved, name) = (server.address.clone(), server.saved, server.name.clone());
        let id = match saved {
            Some(ix) => ElementId::from(("server", ix)),
            None => ElementId::from(("ssh-host", at)),
        };
        let row = h_flex()
            .id(id)
            .h(ROW + px(2.))
            .pl(indent)
            .pr_1()
            .gap_1p5()
            .rounded(theme.radius)
            .when(selected, |el| selected_row(el, &self.colors, cx))
            .when(!selected, |el| el.hover(|style| style.bg(theme.foreground.opacity(0.06))))
            .child(Icon::new(Lucide::Server).xsmall().text_color(muted))
            .child(div().flex_none().max_w(relative(0.6)).truncate().child(server.name.clone()))
            .child(div().flex_1().min_w_0().truncate().text_xs().text_color(muted).child(server.detail))
            .on_click(cx.listener({
                let address = address.clone();
                move |this, event: &ClickEvent, window, cx| {
                    this.sidebar.selected = Some(picked.clone());
                    if event.click_count() == 2 {
                        this.connect(this.active, address.clone(), window, cx);
                    }
                    cx.notify();
                }
            }));
        let row = match saved {
            Some(ix) => row
                .on_drag(ServerDrag(ix), move |_, grab, _, cx| cx.new(|_| DragLabel { text: server.name.clone().into(), grab }))
                .drag_over::<ServerDrag>(move |style, _, _, _| style.bg(target))
                .on_drop(cx.listener(move |this, ServerDrag(from): &ServerDrag, window, cx| this.move_server(*from, folder.clone(), Some(ix), window, cx))),
            // ~/.ssh/config's: nothing goes there.
            None => row.on_drop(|_: &ServerDrag, _, _| {}).on_drop(|_: &ServerFolderDrag, _, _| {}),
        };
        let connected = self.connected_to(&address);
        row.context_menu(move |menu, _, _| server_menu(menu, commander.clone(), address.clone(), name.clone(), saved, connected))
    }
}

/// `path` with `~` for the home folder.
pub(crate) fn tilde(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(inside) if inside.as_os_str().is_empty() => "~".to_owned(),
        Some(inside) => format!("~/{}", inside.display()),
        None => path.display().to_string(),
    }
}

/// A tab's title line; buttons go after it.
/// A group's name in a tab: Drives, Phones, Network.
/// A row's secondary text and icons: on macOS's accent selection white, a
/// little dimmed, as AppKit's; elsewhere the theme's muted colour.
fn secondary(selected: bool, cx: &App) -> Hsla {
    match crate::appearance::mac_tabs(cx).filter(|_| selected) {
        Some(mac) => mac.on_accent.opacity(0.75),
        None => cx.theme().muted_foreground,
    }
}

/// A selected row: on macOS in the accent, as Notes' and Finder's sidebars.
fn selected_row<E: Styled>(el: E, colors: &crate::appearance::PanelColors, cx: &App) -> E {
    match crate::appearance::mac_tabs(cx) {
        Some(mac) => el.bg(mac.accent).text_color(mac.on_accent),
        None => el.bg(colors.sidebar_selection),
    }
}

fn section(title: String, cx: &App) -> Div {
    let label = div().text_xs().font_semibold().text_color(cx.theme().muted_foreground);
    label.px_1().pt_2().pb_0p5().child(title)
}

/// A server's right-click menu; the saved ones can be changed; Disconnect
/// while a tab is on it.
fn server_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    commander: WeakEntity<Commander>,
    address: Address,
    name: String,
    saved: Option<usize>,
    connected: bool,
) -> gpui_kit::component::menu::PopupMenu {
    let copy = |label: String, text: String| PopupMenuItem::new(label).on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone())));
    // Its address as typed to connect: no password is kept in it.
    let (copy_name, copy_path) = (copy(t!("connections.copy_name"), name), copy(t!("connections.copy_path"), address.url()));
    let (commander_for_connect, ssh, server) = (commander.clone(), address.clone(), address.clone());
    let sftp = address.scheme == Scheme::Sftp;
    let connect = PopupMenuItem::new(t!("connections.connect")).on_click(move |_, window, cx| {
        let _ = commander_for_connect.update(cx, |this, cx| this.connect(this.active, address.clone(), window, cx));
    });
    let mut menu = menu.item(connect);
    if sftp {
        let commander = commander.clone();
        menu = menu.item(PopupMenuItem::new(t!("connections.open_in_terminal")).on_click(move |_, window, cx| {
            let _ = commander.update(cx, |this, cx| this.ssh_in_terminal(&ssh, window, cx));
        }));
    }
    if let Some(ix) = saved {
        let edit = |label: String, run: fn(&mut Commander, usize, &mut Window, &mut Context<Commander>)| {
            let commander = commander.clone();
            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                let _ = commander.update(cx, |this, cx| run(this, ix, window, cx));
            })
        };
        menu = menu
            .item(edit(t!("connections.edit"), |this, ix, window, cx| this.edit_server(Some(ix), false, window, cx)))
            .item(edit(t!("connections.duplicate"), |this, ix, window, cx| this.edit_server(Some(ix), true, window, cx)))
            .item(edit(t!("connections.delete"), |this, ix, window, cx| this.delete_server(ix, window, cx)));
    }
    let disconnect = PopupMenuItem::new(t!("connections.disconnect")).disabled(!connected).on_click(move |_, window, cx| {
        let _ = commander.update(cx, |this, cx| this.disconnect_server(&server, window, cx));
    });
    menu.separator().item(copy_name).item(copy_path).separator().item(disconnect)
}

/// An error of a mount whose connection is gone (sshfs's to a phone).
pub(crate) fn lost_device(err: &std::io::Error) -> bool {
    #[cfg(unix)]
    return matches!(err.raw_os_error(), Some(libc::EIO | libc::ENOTCONN));
    #[cfg(not(unix))]
    {
        let _ = err;
        false
    }
}

fn volume_icon(kind: VolumeKind) -> Icon {
    match kind {
        VolumeKind::System | VolumeKind::Drive => Icon::new(IconName::HardDrive),
        VolumeKind::Removable => Icon::new(Lucide::Usb),
        VolumeKind::Cloud => Icon::new(Lucide::Cloud),
        VolumeKind::Network => Icon::new(IconName::Network),
        VolumeKind::Phone => Icon::new(Lucide::Smartphone),
    }
}
