//! Spark Commander in the terminal: the window's panels, keys and core,
//! drawn as Midnight Commander does.

mod dialog;
mod editor;
mod input;
mod keys;
mod menu;
mod music;
mod picture;
mod skin;
#[cfg(unix)]
mod subshell;
mod ui;
mod viewer;

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use sc_core::{keymap, t};
use sc_core::menu::Command;
use sc_core::viewer::origin::{self, Upload};
use sc_core::session::{self, OpenTabs, Session, Workspaces};
use sc_core::settings::Settings;
use sc_core::viewer::Searches;
use sc_core::{Panel, Sort, SortKey, Tabs, archive, command, edit};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::{execute, queue};
use ratatui::crossterm::terminal::{BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate, SetTitle};
use ratatui_image::picker::Picker;

use crate::dialog::{Answer, Choice, Chooser, Form, List};
use sc_core::remote::connections::{Connection, Connections};
use sc_core::remote::mounts::{self, Login};
use sc_core::remote::{self, Address, Ask, Prompt, Reply, Scheme, secrets};
use std::sync::{Arc, Mutex};
use unicode_width::UnicodeWidthStr;
use crate::input::Input;
use crate::ui::Places;
use crate::viewer::Viewer;

pub struct Side {
    pub tabs: Tabs,
    /// The first entry in sight.
    pub top: usize,
}

pub struct Message {
    pub text: String,
    pub error: bool,
}

/// What work done in the background does to the app, back in its loop.
pub type Done = Box<dyn FnOnce(&mut App) + Send>;

/// What a dialog's answer is for.
pub enum Purpose {
    Find,
    /// Select (true) or Unselect Group.
    Group(bool),
    /// sedit's.
    Edit(editor::Asked),
    /// The name taken when none is typed.
    NewWorkspace(String),
    /// A copy of this one, and the name taken when none is typed.
    DuplicateWorkspace(usize, String),
    RenameWorkspace(usize),
    DeleteWorkspace(usize),
    /// A server not known yet: trusted, and whether remembered.
    Trust(mpsc::Sender<Reply>),
    /// A password; kept once logged in, when asked to be.
    Secret { reply: mpsc::Sender<Reply>, keyring: Option<String>, keep: Keep },
}

/// A password to keep under its keyring key once the login worked.
type Keep = Arc<Mutex<Option<(String, String)>>>;

/// A bookmark's folder, or a saved connection.
#[derive(Clone)]
pub enum Place {
    Folder(PathBuf),
    Server(Connection),
}

/// Where connecting took a panel.
enum Went {
    Server(Arc<dyn sc_core::vfs::Remote>, PathBuf),
    /// A share the desktop mounted.
    Mounted(PathBuf),
}

/// Programs that take the terminal over until they end; the loop runs them.
enum Foreground {
    /// A command line for the shell, and its folder.
    Shell(String, PathBuf),
    /// The file, and for a copy how saves go back.
    Editor(PathBuf, Option<Upload>),
    /// Ctrl+O's shell, as Midnight Commander's.
    Subshell,
    /// sedit's text, staged, written over a file as administrator; then whether sedit closes.
    Admin(PathBuf, PathBuf, bool),
}

/// The window's own: scmd has one shell, Ctrl+O's.
const NOT_HERE: [Command; 1] = [Command::NewTerminal];

/// What scmd cannot do yet: its message's `what` (see `not_yet`); F9's menus dim these.
fn lacks(command: Command) -> Option<&'static str> {
    use Command::*;
    match command {
        Copy | Move | NewFolder | Delete | DeletePermanently | CopyToClipboard | CutToClipboard | PasteFromClipboard => Some("file_operations"),
        ConnectToServer => Some("servers"),
        Quit | ToggleMark | MarkInPlace | SelectAll | UnselectAll | InvertSelection | SortByName | SortByExtension | SortByTime
        | SortBySize | Unsorted | ReversedOrder | RereadSource | SyncBrowsing | SelectGroup | UnselectGroup | SelectAllWithSameExtension
        | SaveSelection | RestoreSelection | CompareDirectories | MarkNewerHideSame | GoBack | GoForward | BranchView | BranchViewSelected
        | GoHome | DuplicateTab | LockTab | CloseTab | NextTab | PreviousTab | CloseAllTabs | CloseDuplicateTabs | TerminalPanel | View
        | Edit | EditExternally | Disconnect | Bookmarks | ManageBookmarks | SidebarConnections | BookmarkFolder | ManageWorkspaces
        | NewWorkspace | DuplicateWorkspace => None,
        command if command.workspace().is_some() => None,
        _ => Some("command"),
    }
}

pub struct App {
    pub sides: [Side; 2],
    pub active: usize,
    pub settings: Settings,
    pub command: Input,
    pub message: Option<Message>,
    pub places: Places,
    pub viewer: Option<Viewer>,
    /// sedit, or F4's.
    pub editor: Option<editor::Editor>,
    /// What sedit copied or cut.
    clipboard: String,
    pub form: Option<(Form, Purpose)>,
    /// Alt+W's list of the workspaces, Ctrl+D's of the bookmarks and connections.
    pub chooser: Option<Chooser>,
    /// What runs meanwhile, as connecting: told on the command line's row.
    pub busy: Option<String>,
    /// F9's.
    pub menus: menu::Menus,
    /// Named sets of both sides' tabs, scmd's own; the open one is the tabs shown.
    workspaces: Workspaces,
    /// Keys as the terminal names them; Cmd never reaches it, so `secondary` is Ctrl.
    bindings: HashMap<String, Command>,
    /// For double clicks: when, and on which side's row.
    clicked: Option<(Instant, usize, usize)>,
    foreground: Option<Foreground>,
    /// Read at the first Find.
    searches: Option<Searches>,
    /// Save Selection's names, for Restore Selection.
    saved: Vec<std::ffi::OsString>,
    /// Ctrl+O's, from its first Ctrl+O until it ends.
    #[cfg(unix)]
    subshell: Option<subshell::Subshell>,
    /// Synchronized browsing: the other panel makes the active one's moves too.
    sync_browsing: bool,
    /// How the terminal draws pictures; asked at the start.
    picker: Option<Picker>,
    done: (mpsc::Sender<Done>, mpsc::Receiver<Done>),
    /// sview's and sedit's: no panels, closing the viewer or the editor quits.
    alone: bool,
    quit: bool,
}

/// The terminal scmd runs in, told to the programs it starts (Ctrl+O's shell
/// its own), so none starts another scmd in it, as Midnight Commander's MC_SID.
pub const IN_TERMINAL: &str = "SCMD_TERMINAL";

fn this_terminal() -> Option<String> {
    #[cfg(unix)]
    {
        // SAFETY: asked before any other thread starts; the name is copied at once.
        let name = unsafe { libc::ttyname(0) };
        // SAFETY: a NUL-ended name, non-null.
        (!name.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(name) }.to_string_lossy().into_owned())
    }
    // Ctrl+O's shell is in scmd's own console there.
    #[cfg(windows)]
    Some("console".into())
}

const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// How long the loop waits for a key before it looks at background work.
const TICK: Duration = Duration::from_millis(100);

fn main() -> io::Result<()> {
    let mut problems = Vec::new();
    let settings = Settings::load().unwrap_or_else(|err| {
        problems.push(t!("scmd.error.read_settings", err = err));
        Settings::default()
    });
    settings.apply_globally();
    // Folders to open, and as mc's: -S or --skin and the skin's name; -v or
    // --view and files to view, -e or --edit and a file to edit, as when
    // started as sview or sedit (links to scmd), mc's mcview and mcedit.
    let called = std::env::args_os().next().map(PathBuf::from);
    let stem = called.as_deref().and_then(Path::file_stem);
    let (mut view, mut edit) = (stem.is_some_and(|stem| stem == "sview"), stem.is_some_and(|stem| stem == "sedit"));
    let (mut paths, mut skin) = (Vec::new(), settings.scmd_skin.clone());
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("-S" | "--skin") => skin = args.next().map(|name| name.to_string_lossy().into_owned()),
            Some(given) if given.starts_with("--skin=") => skin = Some(given["--skin=".len()..].to_owned()),
            Some("-v" | "--view") => view = true,
            Some("-e" | "--edit") => edit = true,
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if let Some(name) = skin {
        match skin::load(&name) {
            Ok(skin) => skin::set(skin),
            Err(err) => problems.push(t!("scmd.error.read_skin", name = name, err = err)),
        }
    }
    if view {
        return view_alone(settings, paths, problems);
    }
    if edit {
        return edit_alone(settings, paths.into_iter().next(), problems);
    }
    let terminal = this_terminal();
    if terminal.is_some() && std::env::var(IN_TERMINAL).ok() == terminal {
        eprintln!("{}", t!("scmd.error.running_here"));
        std::process::exit(1);
    }
    if let Some(terminal) = terminal {
        // SAFETY: no other thread yet.
        unsafe { std::env::set_var(IN_TERMINAL, terminal) };
    }
    let session = Session::load_from(session::TERMINAL).unwrap_or_else(|err| {
        problems.push(t!("scmd.error.read_tabs", err = err));
        Session::default()
    });
    std::thread::spawn(archive::remove_stale_scratch);
    let mut app = App::new(settings, &session, paths.into_iter());
    app.workspaces = Workspaces::load_from(session::WORKSPACES_TERMINAL).unwrap_or_else(|err| {
        problems.push(t!("scmd.error.read_workspaces", err = err));
        Workspaces::default()
    });
    if !problems.is_empty() {
        app.say(problems.join("\n"), true);
    }
    let mut terminal = start();
    app.picker = Some(picture::ask_terminal());
    let ran = app.run(&mut terminal);
    stop();
    app.save_session();
    archive::remove_scratch();
    ran
}

/// sview: the viewer alone, N and P going through the files given; closing it
/// ends scmd. Its tabs and workspaces stay as they were.
fn view_alone(settings: Settings, files: Vec<PathBuf>, problems: Vec<String>) -> io::Result<()> {
    let Some(first) = files.first().cloned() else {
        eprintln!("{}", t!("scmd.error.view_nothing"));
        std::process::exit(2);
    };
    let viewer = Viewer::open(files, 0, settings.viewer.clone(), None).unwrap_or_else(|err| {
        eprintln!("{}: {err}", first.display());
        std::process::exit(1);
    });
    let mut app = App::new(settings, &Session::default(), std::iter::empty());
    (app.viewer, app.alone) = (Some(viewer), true);
    if !problems.is_empty() {
        app.say(problems.join("\n"), true);
    }
    let mut terminal = start();
    app.picker = Some(picture::ask_terminal());
    let ran = app.run(&mut terminal);
    stop();
    ran
}

/// sedit: the editor alone, on the file given (a new one where there is
/// none); closing it ends scmd.
fn edit_alone(settings: Settings, file: Option<PathBuf>, problems: Vec<String>) -> io::Result<()> {
    let options = settings.editor.clone();
    let editor = editor::Editor::open(file.clone(), None, options).unwrap_or_else(|err| {
        eprintln!("{}: {err}", file.unwrap_or_default().display());
        std::process::exit(1);
    });
    let mut app = App::new(settings, &Session::default(), std::iter::empty());
    app.alone = true;
    app.show_editor(editor);
    if !problems.is_empty() {
        app.say(problems.join("\n"), true);
    }
    let mut terminal = start();
    let ran = app.run(&mut terminal);
    stop();
    ran
}

/// An SMB or NFS share, as the desktop mounts it: with the kept password
/// first; refused, the user's asked for theirs.
fn mount(address: &Address, ask: &Ask) -> io::Result<PathBuf> {
    let (url, key) = (address.url(), remote::keyring_key(address));
    let kept = address.user.clone().zip(secrets::password(&key)).map(|(user, password)| Login { user, password });
    let first = mounts::mount(&url, kept.as_ref());
    let (Err(_), true, Some(user)) = (&first, address.scheme.logs_in(), address.user.clone()) else {
        return first;
    };
    let text = t!("remote.sftp.password", user = user, host = address.host);
    match ask(Prompt::Secret { title: t!("remote.password.title"), text, keyring: Some(key) }) {
        Reply::Secret(password) => mounts::mount(&url, Some(&Login { user, password })),
        _ => first,
    }
}

/// Full screen, raw keys, the mouse.
// shortcut: no kitty keyboard protocol, so most terminals send Ctrl+Tab as Tab (Alt+1..9
// pick tabs); turn it on once it is tried in kitty, foot or WezTerm.
fn start() -> DefaultTerminal {
    let terminal = ratatui::init();
    let _ = execute!(
        io::stdout(),
        EnableMouseCapture,
        SetTitle("Spark Commander")
    );
    // Pasted text comes whole, not as keys: Enter in it does not indent in sedit.
    let _ = execute!(io::stdout(), EnableBracketedPaste);
    terminal
}

fn stop() {
    let _ = execute!(io::stdout(), DisableMouseCapture);
    let _ = execute!(io::stdout(), DisableBracketedPaste);
    ratatui::restore();
}

impl App {
    /// The saved tabs; folders given on the command line open on the left
    /// and right.
    fn new(settings: Settings, session: &Session, folders: impl Iterator<Item = PathBuf>) -> Self {
        let home = std::env::home_dir().unwrap_or_else(|| "/".into());
        let filter = settings.filter();
        let fallback = || {
            Panel::new(&home, filter)
                .or_else(|_| Panel::new("/", filter))
                .expect("list the home folder or /")
        };
        let side = |ix: usize| Side {
            tabs: match session.open.sides.get(ix) {
                Some(state) => Tabs::restore(state, filter, fallback),
                None => Tabs::new(fallback()),
            },
            top: 0,
        };
        let bindings = (settings.keys.bindings().into_iter())
            .filter(|(_, command)| !NOT_HERE.contains(command))
            .map(|(key, command)| (keymap::normalize(&key, false), command))
            .collect();
        let mut app = App {
            sides: [side(0), side(1)],
            active: session.open.active.min(1),
            settings,
            command: Input::default(),
            message: None,
            places: Places::default(),
            viewer: None,
            editor: None,
            clipboard: String::new(),
            form: None,
            chooser: None,
            busy: None,
            menus: menu::Menus::default(),
            workspaces: Workspaces::default(),
            bindings,
            clicked: None,
            foreground: None,
            searches: None,
            saved: Vec::new(),
            #[cfg(unix)]
            subshell: None,
            sync_browsing: session.sync_browsing,
            picker: None,
            done: mpsc::channel(),
            alone: false,
            quit: false,
        };
        for (ix, folder) in folders.take(2).enumerate() {
            let folder = command::resolve(
                &folder.to_string_lossy(),
                &std::env::current_dir().unwrap_or_default(),
            );
            app.navigate_on(ix, move |panel| panel.go_to_folder(folder));
        }
        app.make_menus();
        app
    }

    /// F9's menus as they are now: the workspaces', the keys'.
    fn make_menus(&mut self) {
        let alone = |rest: &str| rest.chars().next().filter(|_| rest.chars().count() == 1);
        let alt = self.bindings.keys().filter_map(|key| key.strip_prefix("alt-").and_then(alone));
        // Alt+1 to Alt+9 are the tabs'.
        let taken: Vec<char> = alt.chain('1'..='9').collect();
        let names: Vec<String> = self.workspaces.list.iter().map(|workspace| workspace.name.clone()).collect();
        let keys = &self.settings.keys;
        let key_of = |command| {
            let key = keys.keys_of(command).into_iter().find(|key| !key.is_empty());
            key.map(|key| keys::shown(&keymap::normalize(&key, false))).unwrap_or_default()
        };
        let works = |command| lacks(command).is_none();
        let making = menu::Making { works: &works, key_of: &key_of, hidden: &NOT_HERE, workspaces: &names, taken: &taken };
        self.menus.make(&making);
    }

    fn picked(&mut self, picked: menu::Picked) {
        match picked {
            menu::Picked::Command(command) => self.command(command),
            menu::Picked::Workspace(ix) => {
                self.open_workspace(ix);
                self.save_session();
            }
        }
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        let mut redraw = true;
        let mut picture = None;
        while !self.quit && !(self.alone && self.viewer.is_none() && self.editor.is_none()) {
            self.want_music();
            self.want_picture();
            // Pixels of a picture stay until the screen is cleared.
            let in_sight = self.viewer.as_ref().and_then(Viewer::picture_in_sight);
            let turned = in_sight != picture;
            if turned {
                // Not terminal.clear() (it asks where the cursor is, and waits); queued to go out with the new frame, not before its encoding.
                queue!(io::stdout(), BeginSynchronizedUpdate, Clear(ClearType::All))?;
                terminal.swap_buffers();
                (picture, redraw) = (in_sight, true);
            }
            // Asked before drawing: work that ends while it draws gets one more frame.
            let busy = self.viewer.as_ref().is_some_and(Viewer::busy);
            if redraw {
                terminal.draw(|frame| ui::draw(frame, self))?;
            }
            if turned {
                execute!(io::stdout(), EndSynchronizedUpdate)?;
            }
            redraw = busy;
            let wait = match self.viewer.as_ref().is_some_and(Viewer::playing) {
                true => music::FRAME,
                false => TICK,
            };
            if event::poll(wait)? {
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
                    Event::Mouse(mouse) => self.mouse(mouse),
                    Event::Paste(text) => self.pasted(&text),
                    _ => {}
                }
                redraw = true;
            }
            while let Ok(done) = self.done.1.try_recv() {
                done(self);
                redraw = true;
            }
            if let Some(foreground) = self.foreground.take() {
                *terminal = self.run_foreground(foreground);
                redraw = true;
            }
        }
        Ok(())
    }

    /// `work` on a thread of its own; what it gives back runs in the loop.
    fn background(&self, work: impl FnOnce() -> Done + Send + 'static) {
        let sender = self.done.0.clone();
        std::thread::spawn(move || {
            let _ = sender.send(work());
        });
    }

    fn searches(&mut self) -> &mut Searches {
        self.searches
            .get_or_insert_with(|| Searches::load().unwrap_or_default())
    }

    fn open_tabs(&self) -> OpenTabs {
        OpenTabs {
            sides: self.sides.iter().map(|side| side.tabs.state()).collect(),
            active: self.active,
            ..Default::default()
        }
    }

    /// The tabs, and the open workspace's with them.
    fn save_session(&mut self) {
        let open = self.open_tabs();
        let current = self.workspaces.current;
        self.workspaces.list[current].open = open.clone();
        let session = Session { open, sync_browsing: self.sync_browsing, ..Session::default() };
        if let Err(err) = session.save_to(session::TERMINAL).and_then(|()| self.workspaces.save_to(session::WORKSPACES_TERMINAL)) {
            eprintln!("{}", t!("scmd.error.save_tabs", err = err));
        }
    }

    /// Alt+W: the workspaces, the open one checked and chosen.
    fn choose_workspace(&mut self) {
        let items = self.workspaces.list.iter().map(|workspace| workspace.name.clone()).collect();
        let current = self.workspaces.current;
        let at = self.chooser.as_ref().map_or(current, |chooser| chooser.at);
        let keys = t!("scmd.workspaces.keys");
        self.chooser = Some(Chooser { title: t!("scmd.workspaces.title"), items, at, checked: Some(current), keys, list: List::Workspaces });
    }

    fn chose(&mut self, choice: Choice) {
        if matches!(self.chooser.as_ref().map(|chooser| &chooser.list), Some(List::Places(_))) {
            return self.chose_place(choice);
        }
        let name = |this: &Self, ix: usize| this.workspaces.list.get(ix).map(|workspace| workspace.name.clone()).unwrap_or_default();
        let ask = |title: String, value: &str, purpose| (Form::new(title).text(&t!("scmd.workspace.name"), value, Vec::new()), purpose);
        // The field cannot select a name to type over: empty, it takes the one its label gives.
        let unnamed = |title: String, name: String, purpose| (Form::new(title).text(&t!("scmd.workspace.name_or_default", name = name), "", Vec::new()), purpose);
        match choice {
            Choice::Open => {}
            Choice::Cancel => self.chooser = None,
            Choice::Pick(ix) => {
                self.chooser = None;
                self.open_workspace(ix);
                self.save_session();
            }
            Choice::New => {
                let free = (1..).map(|n| t!("scmd.workspace.default_name", number = n)).find(|name| self.workspaces.name_error(name, None).is_none()).unwrap_or_default();
                self.form = Some(unnamed(t!("scmd.workspace.new_title"), free.clone(), Purpose::NewWorkspace(free)));
            }
            Choice::Duplicate(ix) => {
                let free = self.workspaces.free_name(&t!("scmd.workspace.copy_name", name = name(self, ix)));
                self.form = Some(unnamed(t!("scmd.workspace.duplicate_title"), free.clone(), Purpose::DuplicateWorkspace(ix, free)));
            }
            Choice::Rename(ix) => self.form = Some(ask(t!("scmd.workspace.rename_title"), &name(self, ix), Purpose::RenameWorkspace(ix))),
            Choice::Delete(_) if self.workspaces.list.len() < 2 => self.say(t!("scmd.workspace.last_stays"), true),
            Choice::Delete(ix) => {
                let title = t!("scmd.workspace.delete_question", name = name(self, ix));
                self.form = Some((Form::new(title), Purpose::DeleteWorkspace(ix)));
            }
        }
    }

    fn workspace_answered(&mut self, purpose: Purpose, form: &Form) {
        let given = match &purpose {
            Purpose::NewWorkspace(name) | Purpose::DuplicateWorkspace(_, name) if form.value(0).trim().is_empty() => name.clone(),
            _ => form.value(0).trim().to_owned(),
        };
        let typed = || given.clone();
        let named = |this: &mut Self, ix: Option<usize>| match this.workspaces.name_error(&typed(), ix) {
            Some(why) => {
                this.say(why, true);
                false
            }
            None => true,
        };
        match purpose {
            Purpose::NewWorkspace(_) if named(self, None) => {
                let at = self.workspaces.list.len();
                self.workspaces.insert(at, &typed(), OpenTabs::default());
                self.open_workspace(at);
            }
            Purpose::DuplicateWorkspace(ix, _) if named(self, None) => {
                let open = match ix == self.workspaces.current {
                    true => self.open_tabs(),
                    false => self.workspaces.list[ix].open.clone(),
                };
                self.workspaces.insert(ix + 1, &typed(), open);
                self.open_workspace(ix + 1);
            }
            Purpose::RenameWorkspace(ix) if named(self, Some(ix)) => self.workspaces.list[ix].name = typed(),
            Purpose::DeleteWorkspace(ix) => {
                if ix == self.workspaces.current {
                    self.open_workspace(self.workspaces.neighbour(ix));
                }
                self.workspaces.remove(ix);
            }
            _ => {}
        }
        self.save_session();
        if self.chooser.is_some() {
            self.choose_workspace();
        }
    }

    /// Ctrl+D: the bookmarks, then the saved connections, each with where it
    /// goes; `servers`: the first connection chosen.
    fn choose_place(&mut self, servers: bool) {
        // The window may have changed them meanwhile.
        if let Ok(fresh) = Settings::load() {
            (self.settings.bookmarks, self.settings.bookmark_names) = (fresh.bookmarks, fresh.bookmark_names);
        }
        let mut places: Vec<Place> = self.settings.bookmarks.iter().cloned().map(Place::Folder).collect();
        let first_server = places.len();
        places.extend(Connections::load().unwrap_or_default().saved.into_iter().map(Place::Server));
        if places.is_empty() {
            return self.say(t!("scmd.places.empty"), false);
        }
        let rows: Vec<(String, String)> = (places.iter())
            .map(|place| match place {
                Place::Folder(path) => (self.settings.bookmark_name(path), ui::tilde(&path.to_string_lossy())),
                Place::Server(connection) => {
                    let name = if connection.name.is_empty() { &connection.host } else { &connection.name };
                    let name = match &connection.folder {
                        Some(folder) => format!("{folder} › {name}"),
                        None => name.clone(),
                    };
                    (name, connection.url())
                }
            })
            .collect();
        let names = rows.iter().map(|(name, _)| name.width()).max().unwrap_or(0).min(32);
        let items = rows.into_iter().map(|(name, goes)| format!("{}  {goes}", ui::fit(&name, names))).collect();
        let at = match servers {
            true => first_server.min(places.len() - 1),
            false => self.chooser.as_ref().map_or(0, |chooser| chooser.at.min(places.len() - 1)),
        };
        let title = format!("{} · {}", t!("bookmarks.title"), t!("sidebar.section.connections"));
        self.chooser = Some(Chooser { title, items, at, checked: None, keys: t!("scmd.places.keys"), list: List::Places(places) });
    }

    fn chose_place(&mut self, choice: Choice) {
        let Some(Chooser { list: List::Places(places), .. }) = &self.chooser else {
            return;
        };
        match choice {
            Choice::Cancel => self.chooser = None,
            Choice::Pick(ix) => {
                let place = places.get(ix).cloned();
                self.chooser = None;
                match place {
                    Some(Place::Folder(path)) => self.navigate(move |panel| match panel.remote() {
                        Some(_) => panel.disconnect(path),
                        None => panel.go_to_folder(path),
                    }),
                    Some(Place::Server(connection)) => self.connect(connection.address()),
                    None => {}
                }
            }
            Choice::Delete(ix) => {
                if let Some(Place::Folder(path)) = places.get(ix).cloned()
                    && self.change_bookmarks(|settings| {
                        settings.bookmarks.retain(|bookmark| *bookmark != path);
                        settings.bookmark_names.remove(&path);
                    })
                {
                    self.choose_place(false);
                }
            }
            _ => {}
        }
    }

    /// Ctrl+B: the active panel's folder, one of this computer's.
    fn bookmark_folder(&mut self) {
        let panel = self.panel();
        let folder = panel.folder().to_owned();
        if panel.remote().is_some() || panel.in_archive() || panel.is_found() || !sc_core::settings::bookmarkable(&folder) {
            return self.say(t!("bookmarks.only_local"), true);
        }
        let name = folder.file_name().map_or_else(|| folder.display().to_string(), |name| name.to_string_lossy().into_owned());
        let text = match self.settings.bookmarks.contains(&folder) {
            true => t!("bookmarks.already", name = name),
            false => t!("bookmarks.added", name = name),
        };
        let added = folder.clone();
        if self.change_bookmarks(move |settings| {
            if !settings.bookmarks.contains(&added) {
                settings.bookmarks.push(added);
            }
        }) {
            self.say(text, false);
        }
    }

    /// Changes the bookmarks in the settings as saved, the window's changes
    /// since kept; says why when they cannot be saved.
    fn change_bookmarks(&mut self, change: impl FnOnce(&mut Settings)) -> bool {
        let saved = Settings::load().and_then(|mut fresh| {
            change(&mut fresh);
            fresh.save().map(|()| fresh)
        });
        match saved {
            Ok(fresh) => {
                (self.settings.bookmarks, self.settings.bookmark_names) = (fresh.bookmarks, fresh.bookmark_names);
                true
            }
            Err(err) => {
                self.say(t!("main.error.save_settings", err = err), true);
                false
            }
        }
    }

    /// Connects in the background and shows the server on the active side;
    /// SMB and NFS shares the desktop mounts. What it asks comes as dialogs.
    fn connect(&mut self, address: Address) {
        let side = self.active;
        let server = address.host.clone();
        self.busy = Some(t!("remote.connecting", server = server));
        let (ask, keep) = self.asker();
        self.background(move || {
            let went = match address.scheme {
                Scheme::Smb | Scheme::Nfs => mount(&address, &ask).map(Went::Mounted),
                _ => remote::connect(&address, ask).map(|(remote, start)| Went::Server(remote, start)),
            };
            let kept = keep.lock().unwrap_or_else(|err| err.into_inner()).take().filter(|_| went.is_ok());
            let kept = kept.map(|(key, password)| secrets::remember(&key, &password));
            Box::new(move |app: &mut App| {
                app.busy = None;
                match went {
                    Ok(Went::Server(remote, start)) => app.navigate_on(side, move |panel| panel.go_remote(remote, start)),
                    Ok(Went::Mounted(path)) => app.navigate_on(side, move |panel| match panel.remote() {
                        Some(_) => panel.disconnect(path),
                        None => panel.go_to_folder(path),
                    }),
                    Err(err) => app.say(format!("{server}: {err}"), true),
                }
                if let Some(Err(err)) = kept {
                    app.say(err.to_string(), true);
                }
            })
        });
    }

    /// Questions from a connecting thread, answered in dialogs; with a password
    /// the user asked to keep, once it has logged in.
    fn asker(&self) -> (Ask, Keep) {
        let (sender, keep) = (self.done.0.clone(), Keep::default());
        let kept = keep.clone();
        let ask: Ask = Arc::new(move |prompt| {
            let (reply, answer) = mpsc::channel();
            let keep = keep.clone();
            let asked: Done = Box::new(move |app: &mut App| app.ask(prompt, reply, keep));
            match sender.send(asked) {
                Ok(()) => answer.recv().unwrap_or(Reply::Cancel),
                Err(_) => Reply::Cancel,
            }
        });
        (ask, kept)
    }

    fn ask(&mut self, prompt: Prompt, reply: mpsc::Sender<Reply>, keep: Keep) {
        self.form = Some(match prompt {
            Prompt::UnknownHost { host, key_type, fingerprint } => {
                let text = t!("remote.unknown_host.text", key_type = key_type, host = host, fingerprint = fingerprint);
                let form = Form::new(t!("remote.unknown_host.title")).note(&text).check(&t!("remote.unknown_host.remember"), true);
                (form, Purpose::Trust(reply))
            }
            Prompt::Secret { title, text, keyring } => {
                let form = Form::new(title).secret(&text);
                let form = match keyring {
                    Some(_) => form.check(&t!("remote.remember_password"), false),
                    None => form,
                };
                (form, Purpose::Secret { reply, keyring, keep })
            }
        });
    }

    /// Workspace `ix`'s tabs shown, this one's kept in it; saved by the caller.
    fn open_workspace(&mut self, ix: usize) {
        if ix >= self.workspaces.list.len() || ix == self.workspaces.current {
            return;
        }
        let current = self.workspaces.current;
        self.workspaces.list[current].open = self.open_tabs();
        self.workspaces.current = ix;
        let open = self.workspaces.list[ix].open.clone();
        let filter = self.settings.filter();
        let home = std::env::home_dir().unwrap_or_else(|| "/".into());
        let fallback = || Panel::new(&home, filter).or_else(|_| Panel::new("/", filter)).expect("list the home folder or /");
        for (side, state) in self.sides.iter_mut().zip([open.sides.first(), open.sides.get(1)]) {
            side.tabs = match state {
                Some(state) => Tabs::restore(state, filter, fallback),
                None => Tabs::new(fallback()),
            };
            side.top = 0;
        }
        self.active = open.active.min(1);
    }

    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.message = Some(Message {
            text: text.into(),
            error,
        });
    }

    fn panel(&self) -> &Panel {
        self.sides[self.active].tabs.current()
    }

    fn panel_mut(&mut self) -> &mut Panel {
        self.sides[self.active].tabs.current_mut()
    }

    /// Rows a page moves: the file rows in sight.
    fn page(&self) -> isize {
        (self.places.rows[self.active].height as isize - 1).max(1)
    }

    fn navigate(&mut self, go: impl FnOnce(&mut Panel) -> io::Result<()>) {
        self.navigate_on(self.active, go);
    }

    fn navigate_on(&mut self, ix: usize, go: impl FnOnce(&mut Panel) -> io::Result<()>) {
        let tabs = &mut self.sides[ix].tabs;
        let before = tabs.current().spot();
        if let Err(err) = tabs.navigate(go) {
            let text = format!("{}: {err}", tabs.current().location());
            return self.say(text, true);
        }
        // Synchronized browsing: the other panel makes the move too.
        let after = self.sides[ix].tabs.current().spot();
        if self.sync_browsing
            && ix == self.active
            && self.sides[1 - ix].tabs.current().spot().is_some()
            && let (Some((_, from)), Some((_, to))) = (before, after)
            && from != to
        {
            let other = &mut self.sides[1 - ix].tabs;
            if let Err(err) = other.navigate(|panel| panel.follow(&from, &to)) {
                self.sync_browsing = false;
                let text = t!("scmd.sync_browsing.stopped", err = err);
                self.say(text, true);
            }
        }
    }

    fn key(&mut self, key: KeyEvent) {
        if self.message.take().is_some() {
            return;
        }
        if let Some((form, _)) = self.form.as_mut() {
            let answer = form.key(&key);
            if answer != Answer::Open
                && let Some((form, purpose)) = self.form.take()
                && answer == Answer::Ok
            {
                match purpose {
                    Purpose::Find => self.find_answered(&form),
                    Purpose::Group(mark) => self.panel_mut().mark_group(form.value(0), mark),
                    Purpose::Trust(reply) => {
                        let _ = reply.send(Reply::Trust { remember: form.checked(1) });
                    }
                    Purpose::Secret { reply, keyring, keep } => {
                        let password = form.value(0).to_owned();
                        let remember = form.fields.len() > 1 && form.checked(1);
                        if let Some(key) = keyring.filter(|_| remember) {
                            *keep.lock().unwrap_or_else(|err| err.into_inner()) = Some((key, password.clone()));
                        }
                        let _ = reply.send(Reply::Secret(password));
                    }
                    Purpose::Edit(asked) => self.editor_answered(asked, &form),
                    workspace => self.workspace_answered(workspace, &form),
                }
            } else if answer == Answer::Cancel && matches!(self.chooser.as_ref().map(|chooser| &chooser.list), Some(List::Workspaces)) {
                self.choose_workspace();
            } else if answer == Answer::Cancel {
                self.editor_cancelled();
            }
            return;
        }
        if let Some(chooser) = self.chooser.as_mut() {
            let choice = chooser.key(&key);
            return self.chose(choice);
        }
        if self.menus.is_open() {
            if let Some(picked) = self.menus.key(&key) {
                self.picked(picked);
            }
            return;
        }
        if self.editor.is_some() {
            return self.editor_key(key);
        }
        if self.viewer.is_some() {
            return self.viewer_key(key);
        }
        let typing = !self.command.is_empty();
        let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
        match key.code {
            KeyCode::Tab if plain => return self.active = 1 - self.active,
            KeyCode::Up if key.modifiers == KeyModifiers::SHIFT => return self.panel_mut().mark_and_move(-1),
            KeyCode::Down if key.modifiers == KeyModifiers::SHIFT => return self.panel_mut().mark_and_move(1),
            KeyCode::Up if plain => return self.panel_mut().move_cursor(-1),
            KeyCode::Down if plain => return self.panel_mut().move_cursor(1),
            KeyCode::PageUp if plain => return self.move_page(-1),
            KeyCode::PageDown if plain => return self.move_page(1),
            KeyCode::Home if plain && !typing => return self.panel_mut().set_cursor(0),
            KeyCode::End if plain && !typing => return self.panel_mut().set_cursor(usize::MAX),
            KeyCode::Enter if plain && typing => return self.run_line(),
            KeyCode::Enter if plain => return self.navigate(|panel| panel.open().map(drop)),
            KeyCode::Backspace if plain && !typing => return self.navigate(Panel::up),
            KeyCode::PageUp if key.modifiers == KeyModifiers::CONTROL => {
                return self.navigate(Panel::up);
            }
            KeyCode::Esc => return self.command.clear(),
            KeyCode::F(10) if plain => return self.quit = true,
            KeyCode::F(9) if plain => {
                self.make_menus();
                return self.menus.open(0);
            }
            // As in Midnight Commander, with nothing typed: groups and inverting.
            KeyCode::Char('+') if !typing && plain => return self.command(Command::SelectGroup),
            KeyCode::Char('-') if !typing && plain => return self.command(Command::UnselectGroup),
            KeyCode::Char('*') if !typing && plain => {
                return self.command(Command::InvertSelection);
            }
            // Alt+1 to Alt+9: that tab.
            KeyCode::Char(c @ '1'..='9') if key.modifiers == KeyModifiers::ALT => {
                let tabs = &mut self.sides[self.active].tabs;
                return tabs.select(c as usize - '1' as usize);
            }
            _ => {}
        }
        // The line's own editing keys come first while there is a line.
        let editing = matches!(
            key.code,
            KeyCode::Left
                | KeyCode::Right
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Backspace
                | KeyCode::Delete
        );
        if typing && editing && self.command.handle(&key) {
            return;
        }
        let space = key.code == KeyCode::Char(' ');
        if let Some(name) = keys::name(&key)
            && (!space || !typing)
            && let Some(&command) = self.bindings.get(name.as_str())
        {
            return self.command(command);
        }
        // An Alt+letter no command has: the menu with that letter.
        if key.modifiers == KeyModifiers::ALT
            && let KeyCode::Char(letter) = key.code
        {
            self.make_menus();
            if self.menus.open_by_letter(letter) {
                return;
            }
        }
        self.command.handle(&key);
    }

    fn move_page(&mut self, direction: isize) {
        let rows = self.page() * direction;
        self.panel_mut().move_cursor(rows);
    }

    fn mouse(&mut self, mouse: MouseEvent) {
        if self.message.is_some() {
            if let MouseEventKind::Down(_) = mouse.kind {
                self.message = None;
            }
            return;
        }
        if self.form.is_some() || self.chooser.is_some() {
            return;
        }
        let at = ratatui::layout::Position::new(mouse.column, mouse.row);
        if self.menus.is_open() || self.menus.on_bar(at) {
            if !self.menus.is_open() && matches!(mouse.kind, MouseEventKind::Down(_)) {
                self.make_menus();
            }
            if let Some(picked) = self.menus.mouse(&mouse) {
                self.picked(picked);
            }
            return;
        }
        // A click on the key bar presses that key.
        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind
            && self.places.keys.contains(at)
        {
            let slot = (mouse.column - self.places.keys.x) / (self.places.keys.width / 10).max(1);
            let key = KeyCode::F(slot.min(9) as u8 + 1);
            return self.key(KeyEvent::new(key, KeyModifiers::NONE));
        }
        if self.editor.is_some() {
            return self.editor_mouse(mouse);
        }
        if self.viewer.is_some() {
            match mouse.kind {
                MouseEventKind::ScrollUp => self.viewer_scroll(-3),
                MouseEventKind::ScrollDown => self.viewer_scroll(3),
                _ => {}
            }
            return;
        }
        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
            for ix in 0..2 {
                if let Some(tab) = self.places.tabs[ix].iter().position(|r| r.contains(at)) {
                    self.active = ix;
                    return self.sides[ix].tabs.select(tab);
                }
            }
        }
        let side = (0..2).find(|&ix| self.places.rows[ix].contains(at));
        match (mouse.kind, side) {
            (MouseEventKind::ScrollUp, Some(ix)) => {
                self.sides[ix].tabs.current_mut().move_cursor(-3)
            }
            (MouseEventKind::ScrollDown, Some(ix)) => {
                self.sides[ix].tabs.current_mut().move_cursor(3)
            }
            (MouseEventKind::Down(MouseButton::Left), Some(ix)) => {
                self.active = ix;
                let row = self.sides[ix].top + (mouse.row - self.places.rows[ix].y) as usize;
                if row >= self.panel().entries().len() {
                    return;
                }
                self.panel_mut().set_cursor(row);
                let double = self.clicked.is_some_and(|(when, side, at)| {
                    side == ix && at == row && when.elapsed() < DOUBLE_CLICK
                });
                self.clicked = Some((Instant::now(), ix, row));
                if double {
                    self.clicked = None;
                    self.navigate(|panel| panel.open().map(drop));
                }
            }
            _ => {}
        }
    }

    fn command(&mut self, command: Command) {
        if let Some(what) = lacks(command) {
            return self.not_yet(what);
        }
        let side = self.active;
        match command {
            Command::Quit => self.quit = true,
            Command::ToggleMark => self.panel_mut().toggle_mark(),
            Command::MarkInPlace => self.panel_mut().mark_in_place(),
            Command::SelectAll => self.panel_mut().mark_all(true),
            Command::UnselectAll => self.panel_mut().mark_all(false),
            Command::InvertSelection => self.panel_mut().invert_marks(),
            Command::SortByName => self.sort(SortKey::Name),
            Command::SortByExtension => self.sort(SortKey::Extension),
            Command::SortByTime => self.sort(SortKey::Modified),
            Command::SortBySize => self.sort(SortKey::Size),
            Command::Unsorted => self.sort(SortKey::Unsorted),
            Command::ReversedOrder => {
                let sort = self.panel().sort();
                self.panel_mut().set_sort(Sort {
                    descending: !sort.descending,
                    ..sort
                });
            }
            Command::RereadSource => self.navigate(Panel::reload),
            Command::SyncBrowsing => {
                self.sync_browsing = !self.sync_browsing;
                let text = if self.sync_browsing { t!("scmd.sync_browsing.on") } else { t!("scmd.sync_browsing.off") };
                self.say(text, false);
            }
            Command::SelectGroup | Command::UnselectGroup => {
                let mark = command == Command::SelectGroup;
                let title = if mark {
                    t!("scmd.group.select_title")
                } else {
                    t!("scmd.group.unselect_title")
                };
                let form =
                    Form::new(title).text(&t!("scmd.group.files"), "*", Vec::new());
                self.form = Some((form, Purpose::Group(mark)));
            }
            Command::SelectAllWithSameExtension => self.panel_mut().mark_same_extension(),
            Command::SaveSelection => self.saved = self.panel().marked_names(),
            Command::RestoreSelection => {
                let saved = std::mem::take(&mut self.saved);
                self.panel_mut().mark_names(&saved);
                self.saved = saved;
            }
            Command::CompareDirectories | Command::MarkNewerHideSame => {
                let hide = command == Command::MarkNewerHideSame;
                let [left, right] = &mut self.sides;
                let count = left
                    .tabs
                    .current_mut()
                    .compare_with(right.tabs.current_mut(), hide);
                let text = match count {
                    0 => t!("scmd.compare.same"),
                    n => t!("scmd.compare.marked", count = n),
                };
                self.say(text, false);
            }
            // This computer's folders only: scmd opens no servers.
            Command::GoBack | Command::GoForward => {
                let by = if command == Command::GoBack { -1 } else { 1 };
                if let Some(place) = self.sides[side].tabs.current_mut().history.step(by) {
                    self.navigate(move |panel| panel.go_to_folder(PathBuf::from(place)));
                }
            }
            Command::BranchView | Command::BranchViewSelected => {
                let selected = command == Command::BranchViewSelected;
                self.navigate(move |panel| panel.toggle_branch(selected));
            }
            Command::GoHome => {
                let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                self.navigate(move |panel| panel.go_to_folder(home));
            }
            Command::DuplicateTab => self.sides[side].tabs.duplicate(),
            Command::LockTab => self.sides[side].tabs.toggle_lock(),
            Command::CloseTab => self.sides[side].tabs.close(),
            Command::NextTab => self.sides[side].tabs.step(1),
            Command::PreviousTab => self.sides[side].tabs.step(-1),
            Command::CloseAllTabs => self.sides[side].tabs.close_all(),
            Command::CloseDuplicateTabs => self.sides[side].tabs.close_duplicates(),
            Command::TerminalPanel => self.foreground = Some(Foreground::Subshell),
            Command::View => self.view(),
            Command::Edit => self.edit(),
            Command::EditExternally => self.edit_externally(),
            Command::Disconnect => {
                if self.panel().remote().is_some() {
                    let home = std::env::home_dir().unwrap_or_else(|| "/".into());
                    self.navigate(move |panel| panel.disconnect(home));
                }
            }
            Command::Bookmarks | Command::ManageBookmarks => self.choose_place(false),
            Command::SidebarConnections => self.choose_place(true),
            Command::BookmarkFolder => self.bookmark_folder(),
            Command::ManageWorkspaces => self.choose_workspace(),
            Command::NewWorkspace => self.chose(Choice::New),
            Command::DuplicateWorkspace => self.chose(Choice::Duplicate(self.workspaces.current)),
            command if command.workspace().is_some() => {
                self.open_workspace(command.workspace().unwrap_or(0));
                self.save_session();
            }
            _ => {}
        }
    }

    /// The same key on the same column turns the order around, as a
    /// column title click does in the window.
    fn sort(&mut self, key: SortKey) {
        let sort = self.panel().sort();
        let sort = match sort.key == key {
            true => Sort {
                descending: !sort.descending,
                ..sort
            },
            false => Sort::by(key),
        };
        self.panel_mut().set_sort(sort);
    }

    // shortcut: the commands the terminal lacks say so; they come step by step.
    /// `what`: file_operations, servers or command.
    fn not_yet(&mut self, what: &str) {
        self.say(t!("scmd.not_yet", what = what), false);
    }

    /// `cd` moves the panel; anything else goes to the shell.
    fn run_line(&mut self) {
        let line = self.command.text().trim().to_owned();
        self.command.clear();
        let folder = self.panel().folder().to_owned();
        if let Some(target) = command::change_dir(&line, &folder) {
            return self.navigate(move |panel| panel.go_to_folder(target));
        }
        if self.panel().remote().is_some() {
            return self.say(t!("scmd.command_line.not_on_server"), true);
        }
        self.foreground = Some(Foreground::Shell(line, folder));
    }

    /// The terminal is the program's until it ends.
    fn run_foreground(&mut self, foreground: Foreground) -> DefaultTerminal {
        stop();
        let folder = match foreground {
            Foreground::Shell(line, folder) => {
                println!("{}> {line}", folder.display());
                let ran = command::shell(&line).current_dir(&folder).status();
                if let Err(err) = ran {
                    println!("{err}");
                }
                print!("\n{}", t!("scmd.shell.press_enter"));
                let _ = io::stdout().flush();
                let _ = io::stdin().read_line(&mut String::new());
                folder
            }
            Foreground::Editor(file, upload) => {
                let before = origin::modified(&file).ok().flatten();
                if let Err(err) = edit::edit_in_terminal(&file) {
                    self.say(format!("{}: {err}", file.display()), true);
                }
                if let Some(upload) = upload {
                    self.send_back(file.clone(), before, upload);
                }
                file.parent().map(Path::to_owned).unwrap_or_default()
            }
            Foreground::Subshell => self.subshell(),
            Foreground::Admin(staged, file, then_close) => self.save_as_admin(staged, file, then_close),
        };
        for side in &mut self.sides {
            side.tabs.reload_folder(&folder);
        }
        start()
    }

    /// Ctrl+O: the shell, in the panel's folder (a server's panel leaves it
    /// where it is); the panel goes where it was left. Gives the folder to read again.
    #[cfg(unix)]
    fn subshell(&mut self) -> PathBuf {
        let folder = self.panel().folder().to_owned();
        let local = self.panel().remote().is_none();
        let alive = self.subshell.take().and_then(|mut shell| (!shell.ended()).then_some(shell));
        let started = match alive {
            Some(mut shell) => {
                let moved = local && shell.folder().as_deref() != Some(folder.as_path());
                if moved {
                    shell.cd(&folder);
                }
                Ok((shell, moved))
            }
            None => {
                let at = if local { folder.clone() } else { std::env::home_dir().unwrap_or_else(|| "/".into()) };
                let program = sc_core::system::shells::command(self.settings.terminal_shell.as_deref());
                subshell::Subshell::start(&at, program).map(|shell| (shell, local))
            }
        };
        let (mut shell, moved) = match started {
            Ok(started) => started,
            Err(err) => {
                self.say(t!("context_menu.terminal_failed", err = err), true);
                return folder;
            }
        };
        let before = if moved { Some(folder.clone()) } else { shell.folder() };
        let _ = ratatui::crossterm::terminal::enable_raw_mode();
        let ended = shell.show().unwrap_or(true);
        let after = shell.folder();
        if !ended {
            self.subshell = Some(shell);
        }
        if let Some(after) = after.filter(|after| Some(after) != before.as_ref() && *after != folder) {
            self.navigate(move |panel| panel.go_to_folder(after));
        }
        self.panel().folder().to_owned()
    }

    /// Windows: the shell of the settings, else Windows PowerShell, until it ends.
    #[cfg(windows)]
    fn subshell(&mut self) -> PathBuf {
        let folder = self.panel().folder().to_owned();
        if self.panel().remote().is_some() {
            self.say(t!("scmd.command_line.not_on_server"), true);
            return folder;
        }
        let chosen = self.settings.terminal_shell.as_deref();
        let shell = sc_core::system::shells::others().into_iter().find(|shell| Some(shell.program.as_str()) == chosen);
        let (program, args) = shell.map_or(("powershell.exe".into(), Vec::new()), |shell| (shell.program, shell.args));
        if let Err(err) = std::process::Command::new(program).args(args).current_dir(&folder).status() {
            self.say(t!("context_menu.terminal_failed", err = err), true);
        }
        folder
    }

    /// A changed copy goes back to its server or archive, in the background.
    fn send_back(&self, file: PathBuf, before: Option<std::time::SystemTime>, upload: Upload) {
        self.background(move || {
            let sent = origin::send_back(&file, before, &upload);
            Box::new(move |app: &mut App| match sent {
                Some(Ok(text)) => app.say(text, false),
                Some(Err(err)) => app.say(t!("scmd.error.save_back", err = err), true),
                None => {}
            })
        });
    }

    /// F3: a server's or an archive's file is copied out first.
    fn view(&mut self) {
        let panel = self.panel();
        let Some(origin) = panel.origin() else {
            return;
        };
        let settings = self.settings.viewer.clone();
        if origin.is_local() {
            let (files, at) = panel.files_around();
            match Viewer::open(files, at, settings, None) {
                Ok(viewer) => self.viewer = Some(viewer),
                Err(err) => self.say(format!("{}: {err}", origin.name()), true),
            }
            return;
        }
        let name = origin.name();
        self.say(t!("scmd.copying", name = name), false);
        self.background(move || {
            let opened = origin
                .view()
                .and_then(|file| Viewer::open(vec![file], 0, settings, Some(name.clone())));
            Box::new(move |app: &mut App| {
                app.message = None;
                match opened {
                    Ok(viewer) => app.viewer = Some(viewer),
                    Err(err) => app.say(format!("{name}: {err}"), true),
                }
            })
        });
    }

    /// F4 in sedit, or in the terminal's editor (Settings' scmd_external_editor);
    /// a server's or an archive's file is copied out first, and what the editor
    /// saves goes back.
    fn edit(&mut self) {
        let Some(origin) = self.panel().origin() else {
            return;
        };
        let external = self.settings.scmd_external_editor;
        if let origin::Origin::Local(path) = origin {
            return match external {
                true => self.foreground = Some(Foreground::Editor(path, None)),
                false => self.open_editor(Some(path), None, None),
            };
        }
        let name = origin.name();
        self.say(t!("scmd.copying", name = name), false);
        self.background(move || {
            let prepared = origin.edit();
            Box::new(move |app: &mut App| {
                app.message = None;
                match prepared {
                    Ok((file, upload)) if external => app.foreground = Some(Foreground::Editor(file, upload)),
                    Ok((file, upload)) => app.open_editor(Some(file), Some(name), upload),
                    Err(err) => app.say(format!("{name}: {err}"), true),
                }
            })
        });
    }

    /// Pasted into a dialog's line, sedit, or the command line.
    fn pasted(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if let Some((form, _)) = self.form.as_mut() {
            return form.paste(&text);
        }
        match self.editor.as_mut() {
            Some(editor) => editor.type_text(&text, false),
            None if self.viewer.is_none() && self.chooser.is_none() && !self.menus.is_open() => self.command.insert(&text.replace('\n', " ")),
            None => {}
        }
    }

    /// The external editor of Settings, a window of its own; a copy goes
    /// back once it ends.
    fn edit_externally(&mut self) {
        let Some(origin) = self.panel().origin() else {
            return;
        };
        let (line, name) = (self.settings.editor.external.clone(), origin.name());
        self.background(move || {
            let edited = origin.edit().and_then(|(file, upload)| {
                let Some(upload) = upload else {
                    return edit::edit_externally(&line, &file).map(|()| None);
                };
                let before = origin::modified(&file)?;
                edit::start_editor(&line, &file)?.wait()?;
                Ok(origin::send_back(&file, before, &upload))
            });
            Box::new(move |app: &mut App| match edited {
                Ok(Some(Ok(text))) => app.say(text, false),
                Ok(Some(Err(err))) => app.say(t!("scmd.error.save_back", err = err), true),
                Ok(None) => {}
                Err(err) => app.say(format!("{name}: {err}"), true),
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn workspaces_keep_their_tabs_and_the_list_shows_them() {
        let root = std::env::temp_dir().join(format!("sc-tui-workspaces-{}", std::process::id()));
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        let folders = [root.join("a"), root.join("b")].into_iter();
        let mut app = App::new(Settings::default(), &Session::default(), folders);
        app.workspaces.insert(1, "Site", OpenTabs::default());
        app.open_workspace(1);
        assert_ne!(app.sides[0].tabs.current().folder(), root.join("a"), "a new one: home");
        app.open_workspace(0);
        assert_eq!(app.sides[0].tabs.current().folder(), root.join("a"), "its own tabs back");
        assert_eq!(app.sides[1].tabs.current().folder(), root.join("b"));
        app.choose_workspace();
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        let screen: String = terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect();
        assert!(screen.contains("1 ✓ Default") && screen.contains("2   Site"), "{screen}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn draws_both_panels() {
        let root = std::env::temp_dir().join(format!("sc-tui-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("notes.txt"), "hello").unwrap();
        let folders = [root.clone(), root.join("sub")].into_iter();
        let mut app = App::new(Settings::default(), &Session::default(), folders);
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            screen.contains("/sub") && screen.contains("notes.txt"),
            "{screen}"
        );
        assert!(screen.contains("UP--DIR"));
        assert!(screen.contains("Mkdir") && screen.contains("Quit"));

        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        app.key(key(KeyCode::Down));
        app.key(key(KeyCode::Char(' ')));
        assert!(
            app.panel()
                .entries()
                .iter()
                .any(|e| app.panel().is_marked(e)),
            "space marks"
        );
        for c in "cd sub".chars() {
            app.key(key(KeyCode::Char(c)));
        }
        assert_eq!(
            app.command.text(),
            "cd sub",
            "typing goes to the command line"
        );
        app.key(key(KeyCode::Enter));
        assert_eq!(app.panel().path(), root.join("sub"));
        app.key(key(KeyCode::Backspace));
        assert_eq!(app.panel().path(), root, "back up, the line being empty");
        app.key(key(KeyCode::Char('l')));
        app.key(key(KeyCode::Enter));
        assert!(
            matches!(app.foreground.take(), Some(Foreground::Shell(line, at)) if line == "l" && at == root),
            "for the shell"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn other_panel_follows_with_synchronized_browsing() {
        let root = std::env::temp_dir().join(format!("sc-tui-sync-{}", std::process::id()));
        for dir in ["a/x", "a/only", "b/x"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        let folders = [root.join("a"), root.join("b")].into_iter();
        let mut app = App::new(Settings::default(), &Session::default(), folders);
        app.key(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::ALT | KeyModifiers::SHIFT));
        assert!(app.sync_browsing, "Alt+Shift+Y");
        let other = |app: &App| app.sides[1].tabs.current().path();
        app.navigate(|panel| panel.go_to_folder(root.join("a/x")));
        assert_eq!(other(&app), root.join("b/x"));
        app.navigate(Panel::up);
        assert_eq!(other(&app), root.join("b"));
        app.navigate(|panel| panel.go_to_folder(root.join("a/only")));
        assert_eq!(other(&app), root.join("b"));
        assert!(!app.sync_browsing, "off where the other panel cannot follow");
        let saved = Session { sync_browsing: true, ..Session::default() };
        assert!(App::new(Settings::default(), &saved, std::iter::empty()).sync_browsing, "on again after a restart");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn changes_tabs_by_keys_and_clicks() {
        let root = std::env::temp_dir().join(format!("sc-tui-tabs-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let folders = [root.clone(), root.clone()].into_iter();
        let mut app = App::new(Settings::default(), &Session::default(), folders);
        app.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert_eq!(app.sides[0].tabs.active(), 1, "a second tab");
        let alt_shift = KeyModifiers::ALT | KeyModifiers::SHIFT;
        app.key(KeyEvent::new(KeyCode::Left, alt_shift));
        assert_eq!(app.sides[0].tabs.active(), 0, "Alt+Shift+Left");
        app.key(KeyEvent::new(KeyCode::Right, alt_shift));
        assert_eq!(app.sides[0].tabs.active(), 1, "Alt+Shift+Right");
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        let first = app.places.tabs[0][0];
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: first.x + 1,
            row: first.y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.sides[0].tabs.active(), 0, "a click on the first tab");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn selects_groups_as_midnight_commander() {
        let root = std::env::temp_dir().join(format!("sc-tui-group-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for name in ["a.txt", "b.md", "c.txt"] {
            std::fs::write(root.join(name), name).unwrap();
        }
        let mut app = App::new(
            Settings::default(),
            &Session::default(),
            [root.clone()].into_iter(),
        );
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        app.key(key(KeyCode::Char('+')));
        assert!(
            app.form.is_some(),
            "+ with nothing typed asks for the group"
        );
        app.key(key(KeyCode::Backspace));
        for c in "*.txt".chars() {
            app.key(key(KeyCode::Char(c)));
        }
        app.key(key(KeyCode::Enter));
        let marked: Vec<String> = app
            .panel()
            .marked_names()
            .iter()
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        assert_eq!(marked, ["a.txt", "c.txt"]);
        app.key(key(KeyCode::Char('*')));
        assert_eq!(app.panel().marked_names(), ["b.md"]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
