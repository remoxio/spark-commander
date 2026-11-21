# Spark Commander

A twin-panel file manager for Linux, Windows and macOS, in the tradition of
Total Commander, Krusader and Midnight Commander. It comes in two forms that
share one core: **Spark Commander**, the desktop app, and **scmd**, the same
file manager in a text terminal. Written in Rust; free software under the
GPL.

![Spark Commander: a project folder on the left, photos as thumbnails on the right](docs/screenshots/linux/kde/main-dark.png)

## What it does

**Panels**
- Two panels with tabs; full, brief or thumbnail view; sorting by name,
  extension, size or time.
- A sidebar with the folder tree, Places (bookmarks, drives, network and
  cloud places, phones through KDE Connect), saved connections and the
  operation queue, chosen by a row of icons or by their names one under
  another.
- Workspaces (sets of tabs to switch between), bookmarks, folder history,
  synchronized browsing, a quick filter and a Quick View panel.

**Files**
- Copy and move in the background, queued, with undo and redo; delete to the
  trash or for good.
- Multi-Rename Tool, Synchronize Directories, Compare By Content, search
  (also in its own window), checksums (CRC32, MD5, SHA-1, SHA-256, SHA-512)
  made and verified, split and combine, attributes, links, occupied space.
- Archives open like folders: 7z, zip, rar, tar and others, through the 7-Zip
  that comes with the app, which also packs, unpacks and tests them.

**Viewing and editing**
- A viewer (F3) for text with syntax colouring and other character sets,
  pictures, PDF, HTML and e-books; with the add-ons below also video and
  sound, HEIC and AVIF photos, and Word, Excel and PowerPoint files.
- A built-in editor (F4) with search and a minimap, or the editor of your
  choice.
- In a terminal, `sview FILE` (or `scmd -v FILE`) is scmd's viewer alone, as
  Midnight Commander's mcview, and `sedit FILE` (`scmd -e FILE`) its editor,
  as mcedit: mcedit's keys (F3 marks a block, F5 copies it, F6 moves it) with
  the common ones beside them, syntax colours (in scmd's viewer too), word
  wrap, F9 menus, find and replace asking at each match, undo. scmd's F4
  opens it too; `"scmd_external_editor": true` in the settings uses `$EDITOR`
  instead.

**Servers and the terminal**
- SFTP and FTP/FTPS; SMB, WebDAV and NFS shares through the system's own
  bridge (kio-fuse on KDE, gvfs on GNOME, the system's clients on Windows and
  macOS).
- With the rclone add-on, S3-compatible storage, Backblaze B2 and OpenStack
  Swift. Google Drive, OneDrive, Dropbox and other services are on the way:
  each needs the app registered with the provider first.
- Passwords are kept in the system's keychain.
- A terminal inside the window, with tabs, and a command line under the
  panels.
- Your own commands in the Tools menu, with placeholders for the panel's
  folder and the selected files.

**Fitting in**
- Key presets of Total Commander, Krusader, Midnight Commander and ForkLift;
  every shortcut can be changed (Configuration › Shortcuts).
- Every menu and menu item has a letter to press (underlined): Alt and the
  letter opens a menu, the letter alone picks in it, on Windows and Linux and
  in scmd, whose F9 opens the same menus.
- scmd wears Midnight Commander's skins, and two of its own, Pastel Night (the
  default) and Pastel Gray (`scmd -S pastel-gray`, or `"scmd_skin"` in the
  settings; `default` is mc's classic blue); they are mc skins too
  (`crates/sc-tui/skins/`).
- It follows the desktop's look: light or dark, the KDE, GNOME, Windows 11 or
  macOS style of controls, and the sidebar's blur on KDE Plasma and GNOME.
- 36 languages ([the list](#translations)).

## Screenshots

| | |
|---|---|
| ![Inside a zip archive, with the terminal open, in the light look](docs/screenshots/linux/kde/main-light.png) | ![The viewer showing Rust code with syntax colouring](docs/screenshots/linux/kde/viewer.png) |
| Inside a zip archive, with the terminal open | The viewer |
| ![The Multi-Rename Tool previewing new names for eight photos](docs/screenshots/linux/kde/multi-rename.png) | ![scmd, the terminal version, in Midnight Commander's colours](docs/screenshots/linux/scmd.png) |
| The Multi-Rename Tool | scmd, in a text terminal |

More, on [Linux, macOS and Windows](docs/screenshots.md).

## Download

There is no packaged release yet: build it from source as below. The
packaging scripts for each system are in [`packaging/`](packaging).

## Building from source

You need a recent stable [Rust](https://rustup.rs) (edition 2024) and, on
Linux, the development files of the usual desktop libraries. On Debian and
Ubuntu:

```sh
sudo apt install build-essential pkg-config clang cmake \
    libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev \
    libfontconfig1-dev libfreetype-dev libx11-dev libx11-xcb-dev libxcb1-dev \
    libxcb-xkb-dev libxi-dev libdbus-1-dev libssl-dev libzstd-dev libasound2-dev
```

7-Zip and PDFium are not kept in git. Fetch them once after cloning (they are
checked against their published checksums), then build and run:

```sh
git clone https://github.com/remoxio/spark-commander.git
cd spark-commander
vendor/7zip/fetch.sh && vendor/pdfium/fetch.sh   # Windows: vendor\7zip\fetch.bat, vendor\pdfium\fetch.bat

cargo run -p sc-gui    # Spark Commander
cargo run -p sc-tui    # scmd
```

`cargo build --release -p sc-gui -p sc-tui` builds both programs,
`sparkcommander` and `scmd`. For a release folder with everything that goes
with them, see [Linux](packaging/linux/README.md),
[macOS](packaging/macos/README.md) and [Windows](packaging/windows/README.md).

| Crate | What it is |
|---|---|
| `sc-core` | everything the file manager does: panels, operations, archives, servers, viewers' content, settings |
| `sc-gui` | the desktop app, on [GPUI](https://www.gpui.rs) and GPUI Kit |
| `sc-tui` | scmd, on [ratatui](https://ratatui.rs) |
| `sc-qtstyle`, `sc-gtkstyle` | small helpers that let KDE's Qt style or a GTK 3 theme draw the controls |

## Add-ons

The app works without them; Configuration › Settings › Add-ons downloads the
ones for your system, and the app offers them when a file needs one.

| Add-on | For |
|---|---|
| rclone | online storage |
| libmpv | video and sound, HEIC and AVIF photos |
| Craft tools | Word, Excel and PowerPoint files, shown as PDF |

7-Zip (archives) and PDFium (PDF) come with the app itself.

## Translations

Both programs speak 36 languages; Configuration › Settings › Language
chooses one, else the system's is used.

|   |   |   |   |
|---|---|---|---|
| Български<br>Bulgarian | Català<br>Catalan | 简体中文<br>Chinese (Simplified) | 繁體中文<br>Chinese (Traditional) |
| Hrvatski<br>Croatian | Čeština<br>Czech | Dansk<br>Danish | Nederlands<br>Dutch |
| English | Eesti<br>Estonian | Suomi<br>Finnish | Français<br>French |
| Deutsch<br>German | Ελληνικά<br>Greek | हिन्दी<br>Hindi | Magyar<br>Hungarian |
| Bahasa Indonesia<br>Indonesian | Italiano<br>Italian | 日本語<br>Japanese | 한국어<br>Korean |
| Latviešu<br>Latvian | Lietuvių<br>Lithuanian | Norsk bokmål<br>Norwegian Bokmål | Polski<br>Polish |
| Português (Brasil)<br>Portuguese (Brazil) | Português (Portugal)<br>Portuguese (Portugal) | Română<br>Romanian | Српски<br>Serbian |
| Slovenčina<br>Slovak | Slovenščina<br>Slovenian | Español<br>Spanish | Svenska<br>Swedish |
| ไทย<br>Thai | Türkçe<br>Turkish | Українська<br>Ukrainian | Tiếng Việt<br>Vietnamese |

The words of both programs are in [`l10n/`](l10n), one file per language,
with English as the source; [`l10n/README.md`](l10n/README.md) explains the
format.

## License

Spark Commander is free software: you can redistribute it and/or modify it
under the terms of the [GNU General Public License](LICENSE), version 3 or
(at your option) any later version.

The programs and libraries that come with it keep their own licenses: Help ›
About lists them, and a release folder carries their license files. The
patched GPUI Kit crates in [`vendor/`](vendor) are Apache-2.0.
