use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::Filter;
use serde::{Deserialize, Serialize};

/// How much the sidebar covers the desktop behind it, in percent, where it
/// can be see-through (macOS's blur, Windows' acrylic): 100 is solid.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SidebarOpacity(pub u8);

/// About ForkLift's: the desktop shows through, dimmed.
impl Default for SidebarOpacity {
    fn default() -> Self {
        SidebarOpacity(80)
    }
}

/// The terminal's, as the sidebar's: solid until chosen.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TerminalOpacity(pub u8);

impl Default for TerminalOpacity {
    fn default() -> Self {
        TerminalOpacity(100)
    }
}

/// Whether the desktop shows through blurred (else sharp), as Konsole's "Blur background".
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Blur(pub bool);

impl Default for Blur {
    fn default() -> Self {
        Blur(true)
    }
}

/// How the sidebar's sections are chosen: their icons in a row at its top, as
/// macOS's apps have them, or their names one under another, each opening
/// under its own; each system's own until chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidebarLayout {
    Horizontal,
    Vertical,
}

/// Windows: where the sidebar's button is, WinUI's navigation button (☰) in the title bar or the toolbar's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidebarButton {
    #[default]
    TitleBar,
    Toolbar,
}

impl Default for SidebarLayout {
    fn default() -> Self {
        match cfg!(target_os = "macos") {
            true => SidebarLayout::Horizontal,
            false => SidebarLayout::Vertical,
        }
    }
}

/// The folder tabs' header shown also with one tab, its + at hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TabHeaderAlways(pub bool);

impl Default for TabHeaderAlways {
    fn default() -> Self {
        TabHeaderAlways(true)
    }
}

/// Look for add-ons' updates at start and daily.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddonUpdates(pub bool);

impl Default for AddonUpdates {
    fn default() -> Self {
        AddonUpdates(true)
    }
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: crate::system::theme::Appearance,
    pub menu: MenuStyle,
    /// Show the tab header also when there is only one tab.
    pub tab_header_always: TabHeaderAlways,
    pub show_hidden: bool,
    pub show_system: bool,
    pub sidebar_layout: SidebarLayout,
    pub sidebar_button: SidebarButton,
    pub sidebar_opacity: SidebarOpacity,
    pub sidebar_blur: Blur,
    pub terminal_opacity: TerminalOpacity,
    pub terminal_blur: Blur,
    /// The program of the terminal's shell (one of `shells::others`); the default one when none.
    pub terminal_shell: Option<String>,
    pub viewer: ViewerSettings,
    pub editor: EditorSettings,
    /// Files' own programs for Enter, F3 and F4.
    pub associations: Vec<crate::associations::Association>,
    pub keys: crate::keymap::KeySettings,
    pub addon_updates: AddonUpdates,
    /// Folders sorted as files are, also by time; else always by name.
    pub folders_like_files: bool,
    pub date: crate::dates::DateSettings,
    /// The main window's font; the desktop's when none.
    pub font: Option<FontChoice>,
    /// The viewer's, the editor's and comparison's fixed-width font; the desktop's when none.
    pub fixed_font: Option<FontChoice>,
    /// The Show menu's last custom filter, which it offers again ("*.*" before one).
    pub last_filter: String,
    /// The sidebar's Places > Bookmarks: folders dropped there or bookmarked, in their order.
    pub bookmarks: Vec<PathBuf>,
    /// Bookmarks' own names, where given; else their folder's.
    pub bookmark_names: std::collections::BTreeMap<PathBuf, String>,
    /// The Tools menu's own command lines, in its order.
    pub tools: Vec<crate::tools::Tool>,
    /// No dialog offers a missing add-on when its kind of file is opened.
    pub never_offer_addons: bool,
    /// Saved only when changed, so an unchanged one follows the default.
    #[serde(skip_serializing_if = "Toolbar::is_default")]
    pub toolbar: Toolbar,
    pub queue: QueueSettings,
    /// The app's language, a tag of `l10n/`; the system's when none.
    pub language: Option<String>,
    /// scmd's colours: a Midnight Commander skin's name (its own pastel-night and
    /// pastel-gray, or mc's, "default" its own look), or a skin file; Pastel Night when none.
    pub scmd_skin: Option<String>,
    /// scmd's F4 in `$VISUAL` or `$EDITOR` (else nano or vi), not in sedit, its
    /// own; as Midnight Commander's use_internal_edit off.
    pub scmd_external_editor: bool,
}

/// The operation queue's: how long done operations stay listed, how fast links download.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QueueSettings {
    /// Minutes a done operation stays listed; 0: none. Failed and cancelled ones stay until cleared.
    pub clear_after_minutes: u32,
    /// Download from Link's speed limit in KB/s; 0: none.
    pub download_limit_kb: u64,
}

/// The toolbar's buttons in their order, separators between their groups; none hides it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Toolbar(#[serde(deserialize_with = "crate::menu::known_toolbar")] pub Vec<crate::menu::ToolbarItem>);

impl Default for Toolbar {
    fn default() -> Self {
        Toolbar(crate::menu::TOOLBAR.to_vec())
    }
}

impl Toolbar {
    fn is_default(&self) -> bool {
        self.0 == crate::menu::TOOLBAR
    }
}

/// A font family and its size in points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FontChoice {
    pub family: String,
    pub size: f32,
}

/// Where the menus are: a bar under the title, or behind a ☰ button in the
/// window's own title bar, as GNOME's apps have them. Linux only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MenuStyle {
    /// The ☰ button on GNOME, the bar elsewhere.
    #[default]
    Automatic,
    Bar,
    Button,
}

impl MenuStyle {
    /// Whether the menus go behind the ☰ button.
    pub fn button(self) -> bool {
        let linux = cfg!(any(target_os = "linux", target_os = "freebsd"));
        linux && (self == MenuStyle::Button || self == MenuStyle::Automatic && crate::system::theme::on_gnome())
    }
}

/// F4 edits in the built-in editor, Ctrl+Shift+F4 in another program.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    /// The largest file the built-in editor opens, in MB: it takes about
    /// eight times as much memory.
    pub limit_mb: u64,
    /// Ctrl+Shift+F4's command line: `%1` is the file, else the file goes last.
    pub external: String,
    /// The View menu's, kept for the next file.
    pub wrap: bool,
    pub line_numbers: bool,
    pub whitespace: bool,
    pub indent_guides: bool,
    pub folding: bool,
    /// Where the file shows none of its own (`text_edit::detect_indent`).
    pub tab_size: usize,
    pub hard_tabs: bool,
}

impl Default for EditorSettings {
    fn default() -> Self {
        let external = match () {
            _ if cfg!(windows) => "notepad.exe",
            _ if cfg!(target_os = "macos") => "open -t",
            _ => "xdg-open",
        };
        Self {
            limit_mb: 32,
            external: external.into(),
            wrap: false,
            line_numbers: true,
            whitespace: false,
            indent_guides: true,
            folding: true,
            tab_size: 4,
            hard_tabs: false,
        }
    }
}

/// Lister's: what F3 shows a file as at first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StartMode {
    Text,
    Binary,
    Hex,
    #[default]
    Auto,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StartFont {
    Ansi,
    Ascii,
    Variable,
    #[default]
    Auto,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewerSettings {
    pub start: StartMode,
    pub font: StartFont,
    pub wrap: bool,
    /// Characters a row of wrapped text, and of Binary.
    pub text_width: usize,
    pub binary_width: usize,
    /// Beside the Viewer and the Editor.
    pub minimap: bool,
    /// Code in its language's colours (`viewer::source`).
    pub syntax: bool,
}

/// Lister's defaults.
impl Default for ViewerSettings {
    fn default() -> Self {
        Self {
            start: StartMode::Auto,
            font: StartFont::Auto,
            wrap: true,
            text_width: 81,
            binary_width: 75,
            minimap: true,
            syntax: true,
        }
    }
}

/// The settings dialog's pages.
pub const PAGES: &[&str] = &["Layout", "Toolbar", "Display", "Font", "Date", "Language", "Edit/View", "Terminal", "Operations", "Passwords", "Add-ons"];

/// A folder of this computer's own: not a server's, as kio-fuse, gvfs or
/// Windows' `\\server\share` show them.
pub fn bookmarkable(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    let unc = matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..)));
    path.is_absolute() && !unc && path.is_dir() && crate::remote::mounts::url_of(path).is_none()
}

pub fn config_dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("sparkcommander"))
}

impl Settings {
    pub fn filter(&self) -> Filter {
        Filter {
            hidden: self.show_hidden,
            system: self.show_system,
        }
    }

    /// A bookmark's name: its own, else its folder's.
    pub fn bookmark_name(&self, path: &Path) -> String {
        match self.bookmark_names.get(path) {
            Some(name) => name.clone(),
            None => path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned()),
        }
    }

    /// Bookmarks A to Z by their names, case aside.
    pub fn sort_bookmarks(&mut self) {
        let mut bookmarks = std::mem::take(&mut self.bookmarks);
        bookmarks.sort_by_cached_key(|path| self.bookmark_name(path).to_lowercase());
        self.bookmarks = bookmarks;
    }

    /// What the core shows by them everywhere: dates, folders' order.
    pub fn apply_globally(&self) {
        crate::dates::set(&self.date);
        crate::set_folders_like_files(self.folders_like_files);
        crate::ops::set_download_limit(self.queue.download_limit_kb.saturating_mul(1024));
        crate::l10n::set_language(self.language.as_deref());
    }

    /// On the first start, the bookmarks it starts with.
    pub fn load() -> io::Result<Self> {
        let first = config_dir().is_none_or(|dir| !dir.join("settings.json").exists());
        let mut settings: Self = read_json("settings.json")?;
        if first {
            settings.reset_bookmarks();
        }
        Ok(settings)
    }

    /// The bookmarks it starts with, their own names gone: Home, Desktop,
    /// Documents and Downloads; on macOS Applications first, as Finder's.
    pub fn reset_bookmarks(&mut self) {
        let home = std::env::home_dir().unwrap_or_default();
        // Without the desktop's user-dirs file, the English names.
        let named = |dir: Option<PathBuf>, english| dir.unwrap_or_else(|| home.join(english));
        let mut folders = vec![
            home.clone(),
            named(dirs::desktop_dir(), "Desktop"),
            named(dirs::document_dir(), "Documents"),
            named(dirs::download_dir(), "Downloads"),
        ];
        if cfg!(target_os = "macos") {
            folders.insert(0, "/Applications".into());
        }
        self.bookmarks.clear();
        self.bookmark_names.clear();
        for dir in folders {
            // A desktop set to Home itself comes once.
            if dir.is_dir() && !self.bookmarks.contains(&dir) {
                self.bookmarks.push(dir);
            }
        }
        // Its folder's name is the user's.
        if self.bookmarks.contains(&home) {
            self.bookmark_names.insert(home, "Home".into());
        }
    }

    pub fn save(&self) -> io::Result<()> {
        write_json("settings.json", self)
    }
}

/// The defaults when the file is not there yet.
pub(crate) fn read_json<T: Default + DeserializeOwned>(file: &str) -> io::Result<T> {
    let Some(dir) = config_dir() else {
        return Ok(T::default());
    };
    match fs::read_to_string(dir.join(file)) {
        Ok(text) => Ok(serde_json::from_str(&text)?),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(err),
    }
}

/// Write then rename, so a failed save keeps the old file.
pub(crate) fn write_json(file: &str, value: &impl Serialize) -> io::Result<()> {
    let dir = config_dir().ok_or_else(|| io::Error::other(crate::t!("settings.error.no_folder")))?;
    fs::create_dir_all(&dir)?;
    // Each save its own, beside another window's or app's; the last one in place wins.
    static SAVES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let save = SAVES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = dir.join(format!("{file}.{}-{save}.new", std::process::id()));
    let written = (|| {
        // Only the user's, as the saved passwords' file is too.
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut out = options.open(&temp)?;
        io::Write::write_all(&mut out, serde_json::to_string_pretty(value)?.as_bytes())?;
        out.sync_all()?;
        fs::rename(&temp, dir.join(file))
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_saved_only_when_changed() {
        let mut settings = Settings::default();
        assert!(!serde_json::to_string(&settings).unwrap().contains("toolbar"));
        settings.toolbar.0.pop();
        let saved = serde_json::to_string(&settings).unwrap();
        let read: Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(read.toolbar, settings.toolbar);
        settings.toolbar.0.clear();
        let read: Settings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(read.toolbar.0.is_empty(), "an emptied toolbar stays empty");
    }

    #[test]
    fn toolbar_keeps_its_separators() {
        use crate::menu::{Command, ToolbarItem};
        let read: Settings = serde_json::from_str(r#"{"toolbar": ["GoBack", "Separator", "NoSuchCommand", "Search"]}"#).unwrap();
        let expected = [ToolbarItem::Button(Command::GoBack), ToolbarItem::Separator, ToolbarItem::Button(Command::Search)];
        assert_eq!(read.toolbar.0, expected);
        let saved = serde_json::to_string(&read).unwrap();
        assert!(saved.contains(r#""toolbar":["GoBack","Separator","Search"]"#), "{saved}");
    }

    #[test]
    fn resets_bookmarks_to_the_first_ones() {
        let mut settings = Settings { bookmarks: vec!["/x/a".into()], ..Settings::default() };
        settings.bookmark_names.insert("/x/a".into(), "Mine".into());
        settings.reset_bookmarks();
        assert!(!settings.bookmarks.is_empty() && !settings.bookmarks.contains(&"/x/a".into()));
        assert!(settings.bookmarks.iter().all(|dir| dir.is_dir()));
        assert!(settings.bookmark_names.values().all(|name| name == "Home"));
    }

    #[test]
    fn sorts_bookmarks_and_takes_only_folders() {
        let mut settings = Settings { bookmarks: vec!["/x/b".into(), "/x/C".into(), "/x/a".into()], ..Settings::default() };
        settings.bookmark_names.insert("/x/a".into(), "Zed".into());
        settings.sort_bookmarks();
        assert_eq!(settings.bookmarks, [PathBuf::from("/x/b"), "/x/C".into(), "/x/a".into()]);
        let dir = std::env::temp_dir();
        assert!(bookmarkable(&dir));
        assert!(!bookmarkable(Path::new("relative")));
        let file = dir.join(format!("sc-bookmark-{}", std::process::id()));
        fs::write(&file, "").unwrap();
        assert!(!bookmarkable(&file), "a file");
        fs::remove_file(&file).unwrap();
    }
}
