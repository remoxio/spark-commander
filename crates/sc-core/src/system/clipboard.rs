//! Files on the clipboard as text: the paths this program puts there, and
//! the `file://` addresses or paths other programs do.

use std::ffi::OsString;
use std::path::PathBuf;

/// The files' text: their paths, a line each.
pub fn text_of(paths: &[PathBuf]) -> String {
    let lines: Vec<String> = paths.iter().map(|path| path.display().to_string()).collect();
    lines.join("\n")
}

/// Windows: the files added to what is on the clipboard as a file list, as
/// Explorer puts it (CF_HDROP), copied or cut; other programs and remote
/// desktops paste that.
#[cfg(windows)]
pub fn add_files(paths: &[PathBuf], cut: bool) -> windows::core::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{CloseClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_HDROP;
    use windows::Win32::UI::Shell::DROPFILES;

    let mut list: Vec<u16> = Vec::new();
    for path in paths {
        list.extend(path.as_os_str().encode_wide());
        list.push(0);
    }
    list.push(0);
    let head = DROPFILES { pFiles: size_of::<DROPFILES>() as u32, fWide: true.into(), ..Default::default() };
    // DROPEFFECT_MOVE or DROPEFFECT_COPY.
    let effect: u32 = if cut { 2 } else { 1 };
    unsafe fn global(bytes: &[&[u8]]) -> windows::core::Result<HGLOBAL> {
        let size = bytes.iter().map(|part| part.len()).sum();
        let global = unsafe { GlobalAlloc(GMEM_MOVEABLE, size)? };
        let mut at = unsafe { GlobalLock(global) } as *mut u8;
        for part in bytes {
            unsafe {
                std::ptr::copy_nonoverlapping(part.as_ptr(), at, part.len());
                at = at.add(part.len());
            }
        }
        let _ = unsafe { GlobalUnlock(global) };
        Ok(global)
    }
    unsafe {
        let head = std::slice::from_raw_parts((&head as *const DROPFILES).cast::<u8>(), size_of::<DROPFILES>());
        let files = std::slice::from_raw_parts(list.as_ptr().cast::<u8>(), list.len() * 2);
        let drop = global(&[head, files])?;
        let effect = global(&[&effect.to_le_bytes()])?;
        let effect_format = RegisterClipboardFormatW(windows::core::w!("Preferred DropEffect"));
        if let Err(error) = OpenClipboard(None) {
            let _ = (GlobalFree(Some(drop)), GlobalFree(Some(effect)));
            return Err(error);
        }
        // Once set, the system owns the memory.
        let mut result = Ok(());
        for (format, data) in [(CF_HDROP.0 as u32, drop), (effect_format, effect)] {
            if result.is_ok() {
                result = SetClipboardData(format, Some(HANDLE(data.0))).map(|_| ());
            }
            if result.is_err() {
                let _ = GlobalFree(Some(data));
            }
        }
        let _ = CloseClipboard();
        result
    }
}

/// Windows: files another program offers with no path of their own (an
/// Outlook attachment, a remote desktop's, an archive's), written into `dir`
/// as Explorer pastes them; the ones at the top. None on the clipboard: empty.
/// Runs OLE on the calling thread: one of its own, as the source's may take a while.
#[cfg(windows)]
pub fn save_virtual_files(dir: &std::path::Path) -> std::io::Result<Vec<PathBuf>> {
    use std::io::Write;
    use std::path::Component;
    use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL, TYMED_ISTREAM};
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::{OleGetClipboard, OleInitialize, ReleaseStgMedium};
    use windows::Win32::UI::Shell::{FD_ATTRIBUTES, FD_FILESIZE, FILEDESCRIPTORW, FILEGROUPDESCRIPTORW};

    unsafe {
        let _ = OleInitialize(None);
        let data = OleGetClipboard()?;
        let format = |name, lindex, tymed: i32| FORMATETC {
            cfFormat: RegisterClipboardFormatW(name) as u16,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex,
            tymed: tymed as u32,
        };
        let Ok(mut medium) = data.GetData(&format(windows::core::w!("FileGroupDescriptorW"), -1, TYMED_HGLOBAL.0)) else {
            return Ok(Vec::new());
        };
        let group = GlobalLock(medium.u.hGlobal) as *const FILEGROUPDESCRIPTORW;
        let items: Vec<FILEDESCRIPTORW> = match group.is_null() {
            true => Vec::new(),
            false => (0..std::ptr::addr_of!((*group).cItems).read_unaligned() as usize)
                .map(|at| std::ptr::addr_of!((*group).fgd).cast::<FILEDESCRIPTORW>().add(at).read_unaligned())
                .collect(),
        };
        let _ = GlobalUnlock(medium.u.hGlobal);
        ReleaseStgMedium(&mut medium);
        let mut top = Vec::new();
        for (at, item) in items.iter().enumerate() {
            let name = item.cFileName;
            let name = String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())]);
            // Its parts only: a name with .. or a drive would write outside `dir`.
            let parts: Vec<_> = std::path::Path::new(&name).components().filter_map(|part| match part {
                Component::Normal(part) => Some(part.to_owned()),
                _ => None,
            }).collect();
            let Some(first) = parts.first() else { continue };
            let path = parts.iter().fold(dir.to_path_buf(), |path, part| path.join(part));
            if !top.contains(&dir.join(first)) {
                top.push(dir.join(first));
            }
            let folder = item.dwFlags & FD_ATTRIBUTES.0 as u32 != 0 && item.dwFileAttributes & 0x10 != 0;
            if folder {
                std::fs::create_dir_all(&path)?;
                continue;
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut medium = data.GetData(&format(windows::core::w!("FileContents"), at as i32, TYMED_ISTREAM.0 | TYMED_HGLOBAL.0))?;
            let mut file = std::fs::File::create(&path)?;
            let written = match medium.tymed as i32 {
                tymed if tymed == TYMED_ISTREAM.0 => match &*medium.u.pstm {
                    Some(stream) => {
                        let mut buffer = vec![0u8; 1 << 16];
                        loop {
                            let mut read = 0;
                            stream.Read(buffer.as_mut_ptr().cast(), buffer.len() as u32, Some(&mut read)).ok()?;
                            if read == 0 {
                                break Ok(());
                            }
                            file.write_all(&buffer[..read as usize])?;
                        }
                    }
                    None => Ok(()),
                },
                _ => {
                    let size = (u64::from(item.nFileSizeHigh) << 32 | u64::from(item.nFileSizeLow)) as usize;
                    let bytes = GlobalLock(medium.u.hGlobal) as *const u8;
                    // A global is often longer than its file: its size where the descriptor gives it.
                    let whole = windows::Win32::System::Memory::GlobalSize(medium.u.hGlobal);
                    let size = if item.dwFlags & FD_FILESIZE.0 as u32 != 0 { size.min(whole) } else { whole };
                    let written = match bytes.is_null() {
                        true => Ok(()),
                        false => file.write_all(std::slice::from_raw_parts(bytes, size)),
                    };
                    let _ = GlobalUnlock(medium.u.hGlobal);
                    written
                }
            };
            ReleaseStgMedium(&mut medium);
            written?;
        }
        Ok(top)
    }
}

/// The tools a terminal reaches the clipboard through, which has no window to
/// ask: wl-clipboard's on Wayland, xclip's or xsel's on X11, macOS's own.
#[cfg(unix)]
fn tools(paste: bool) -> Vec<(&'static str, &'static [&'static str])> {
    if cfg!(target_os = "macos") {
        return vec![if paste { ("pbpaste", &[]) } else { ("pbcopy", &[]) }];
    }
    let mut tools: Vec<(&str, &[&str])> = Vec::new();
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        tools.push(if paste { ("wl-paste", &["--no-newline"]) } else { ("wl-copy", &[]) });
    }
    if std::env::var_os("DISPLAY").is_some() {
        tools.push(("xclip", if paste { &["-selection", "clipboard", "-o"] } else { &["-selection", "clipboard"] }));
        tools.push(("xsel", if paste { &["--clipboard", "--output"] } else { &["--clipboard", "--input"] }));
    }
    tools
}

/// The clipboard's text, for the terminal's programs; none where no tool
/// answers (a server's terminal).
#[cfg(unix)]
pub fn text() -> Option<String> {
    use std::process::{Command, Stdio};
    tools(true).into_iter().find_map(|(tool, args)| {
        let out = Command::new(tool).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    })
}

/// `text` on the clipboard, for the terminal's programs; false where no tool took it.
#[cfg(unix)]
pub fn set_text(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};
    tools(false).into_iter().any(|(tool, args)| {
        let started = Command::new(tool).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        let Ok(mut child) = started else { return false };
        let written = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
        // The tool forks one that holds the text; the one started ends.
        written && child.wait().is_ok_and(|status| status.success())
    })
}

#[cfg(windows)]
pub fn text() -> Option<String> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_UNICODETEXT;
    unsafe {
        OpenClipboard(None).ok()?;
        let text = GetClipboardData(CF_UNICODETEXT.0 as u32).ok().and_then(|data| {
            let global = HGLOBAL(data.0);
            let at = GlobalLock(global) as *const u16;
            if at.is_null() {
                return None;
            }
            let length = (0..).take_while(|&ix| *at.add(ix) != 0).count();
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(at, length));
            let _ = GlobalUnlock(global);
            Some(text)
        });
        let _ = CloseClipboard();
        text
    }
}

#[cfg(windows)]
pub fn set_text(text: &str) -> bool {
    use windows::Win32::Foundation::{GlobalFree, HANDLE};
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_UNICODETEXT;
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        let Ok(global) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) else { return false };
        let at = GlobalLock(global) as *mut u16;
        if at.is_null() || OpenClipboard(None).is_err() {
            let _ = GlobalFree(Some(global));
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), at, wide.len());
        let _ = GlobalUnlock(global);
        let _ = EmptyClipboard();
        // Once set, the system owns the memory.
        let set = SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(global.0))).is_ok();
        if !set {
            let _ = GlobalFree(Some(global));
        }
        let _ = CloseClipboard();
        set
    }
}

/// A path as a `file://` address, other bytes than letters, digits and
/// `-._~/` written `%XX`.
pub fn file_uri(path: &std::path::Path) -> String {
    #[cfg(unix)]
    let bytes = std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str()).to_vec();
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().replace('\\', "/").into_bytes();
    let mut uri = String::from("file://");
    for byte in bytes {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// The files a text names, a line each, as `file://` addresses or
/// absolute paths, all of them there; none for any other text.
pub fn paths_in(text: &str) -> Option<Vec<PathBuf>> {
    let lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let paths = lines.map(|line| {
        let path = match line.strip_prefix("file://") {
            Some(rest) => PathBuf::from(decode(rest.strip_prefix("localhost").unwrap_or(rest))?),
            None => PathBuf::from(line),
        };
        (path.is_absolute() && path.exists()).then_some(path)
    });
    paths.collect::<Option<Vec<_>>>().filter(|paths| !paths.is_empty())
}

/// `%20` and the like, as addresses have them.
fn decode(text: &str) -> Option<OsString> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(at + 1..at + 3)?).ok()?;
                decoded.push(u8::from_str_radix(hex, 16).ok()?);
                at += 3;
            }
            byte => {
                decoded.push(byte);
                at += 1;
            }
        }
    }
    #[cfg(unix)]
    return Some(std::os::unix::ffi::OsStringExt::from_vec(decoded));
    #[cfg(not(unix))]
    return String::from_utf8(decoded).ok().map(OsString::from);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    #[ignore = "replaces what is on the clipboard"]
    fn puts_a_file_list() {
        use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
        use windows::Win32::System::Ole::CF_HDROP;
        use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
        let paths = [std::env::temp_dir(), std::env::current_dir().unwrap()];
        add_files(&paths, false).unwrap();
        let mut read = Vec::new();
        unsafe {
            OpenClipboard(None).unwrap();
            let drop = HDROP(GetClipboardData(CF_HDROP.0 as u32).unwrap().0);
            for at in 0..DragQueryFileW(drop, u32::MAX, None) {
                let mut name = vec![0u16; 1024];
                let len = DragQueryFileW(drop, at, Some(&mut name)) as usize;
                read.push(PathBuf::from(String::from_utf16_lossy(&name[..len])));
            }
            let _ = CloseClipboard();
        }
        assert_eq!(read, paths);
    }

    #[test]
    #[cfg(windows)]
    #[ignore = "reads what is on the clipboard: copy an Outlook attachment first"]
    fn saves_virtual_files() {
        let dir = std::env::temp_dir().join(format!("sc-virtual-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let saved = save_virtual_files(&dir).unwrap();
        println!("{saved:?}");
        assert!(!saved.is_empty() && saved.iter().all(|path| path.exists()));
    }

    #[test]
    fn reads_files_from_text() {
        let dir = std::env::temp_dir().join(format!("sc-clip-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("a b")).unwrap();
        std::fs::write(dir.join("é.txt"), "x").unwrap();
        let (folder, file) = (dir.join("a b"), dir.join("é.txt"));
        let text = text_of(&[folder.clone(), file.clone()]);
        assert_eq!(paths_in(&text), Some(vec![folder.clone(), file.clone()]));
        let uri = format!("file://{}\r\n", dir.display()).replace(' ', "%20") + "a%20b";
        assert_eq!(paths_in(&uri.replace("\r\na%20b", "/a%20b")), Some(vec![folder]));
        let encoded = format!("file://{}/%C3%A9.txt", dir.display());
        assert_eq!(paths_in(&encoded), Some(vec![file.clone()]));
        let uri = file_uri(&dir.join("a b"));
        assert!(uri.ends_with("/a%20b"), "{uri}");
        assert_eq!(paths_in(&file_uri(&file)), Some(vec![file]));
        assert_eq!(paths_in("just some words"), None);
        assert_eq!(paths_in("/no/such/file"), None);
        assert_eq!(paths_in(""), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
