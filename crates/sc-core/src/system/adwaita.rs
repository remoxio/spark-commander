//! libadwaita's own look, as GNOME's apps draw it, read from the installed
//! library's stylesheet: it is compiled into the library (a GLib resource),
//! not in GNOME's settings, which only name the themes and the buttons' order.

use std::io::Read;
use std::path::PathBuf;
use std::sync::OnceLock;

/// The window buttons' round fill behind their picture: a share of the text's
/// colour normal, hovered and pressed, its corners' radius (none: round), and
/// the padding round the 16 px picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowButtons {
    pub fill: [f32; 3],
    pub radius: Option<f32>,
    pub padding: f32,
}

/// The installed libadwaita's window buttons; none without it, or a stylesheet not read.
pub fn window_buttons() -> Option<WindowButtons> {
    static LOOK: OnceLock<Option<WindowButtons>> = OnceLock::new();
    *LOOK.get_or_init(|| buttons_of(&stylesheet()?))
}

/// The installed libadwaita's windows' corners (12 px in 1.5, 15 in later ones); none without it.
pub fn window_radius() -> Option<f32> {
    static RADIUS: OnceLock<Option<f32>> = OnceLock::new();
    *RADIUS.get_or_init(|| radius_of(&stylesheet()?))
}

/// `window.csd`'s border-radius, a `var(--name)` taken from `:root`.
fn radius_of(css: &str) -> Option<f32> {
    let value = declarations(css, "window.csd").into_iter().rev().find(|(name, _)| name == "border-radius")?.1;
    let value = match value.strip_prefix("var(").and_then(|rest| rest.strip_suffix(')')) {
        Some(variable) => declarations(css, ":root").into_iter().rev().find(|(name, _)| name == variable.trim())?.1,
        None => value,
    };
    value.strip_suffix("px")?.trim().parse().ok()
}

/// The stylesheet in the library: 1.5's base.css, later ones' gtk.css.
fn stylesheet() -> Option<String> {
    let library = std::fs::read(library()?).ok()?;
    ["/org/gnome/Adwaita/styles/base.css", "/org/gnome/Adwaita/styles/gtk.css"]
        .iter()
        .find_map(|path| resource(&library, path))
        .and_then(|css| String::from_utf8(css).ok())
}

fn library() -> Option<PathBuf> {
    const NAME: &str = "libadwaita-1.so.0";
    let mut dirs: Vec<PathBuf> = ["/usr/lib64", "/usr/lib", "/lib64", "/lib"].iter().map(PathBuf::from).collect();
    // Debian's and Ubuntu's multiarch folders: /usr/lib/x86_64-linux-gnu.
    if let Ok(entries) = std::fs::read_dir("/usr/lib") {
        dirs.extend(entries.flatten().map(|entry| entry.path()).filter(|path| path.to_string_lossy().ends_with("-linux-gnu")));
    }
    dirs.into_iter().map(|dir| dir.join(NAME)).find(|path| path.exists())
}

/// A file of the GLib resource bundles in `binary` (GVDB tables, each starting
/// "GVariant"), by its path; unpacked if stored compressed.
fn resource(binary: &[u8], path: &str) -> Option<Vec<u8>> {
    const MAGIC: &[u8] = b"GVariant";
    let mut from = 0;
    while let Some(at) = binary[from..].windows(MAGIC.len()).position(|window| window == MAGIC) {
        let start = from + at;
        if let Some(found) = gvdb_lookup(&binary[start..], path) {
            return Some(found);
        }
        from = start + MAGIC.len();
    }
    None
}

fn gvdb_lookup(table: &[u8], path: &str) -> Option<Vec<u8>> {
    let u32_at = |at: usize| table.get(at..at + 4).map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap_or_default()));
    let u16_at = |at: usize| table.get(at..at + 2).map(|bytes| u16::from_le_bytes(bytes.try_into().unwrap_or_default()));
    let (root_start, root_end) = (u32_at(16)? as usize, u32_at(20)? as usize);
    let bloom_words = (u32_at(root_start)? & 0x07ff_ffff) as usize;
    let buckets = u32_at(root_start + 4)? as usize;
    let items = root_start + 8 + 4 * (bloom_words + buckets);
    let count = root_end.checked_sub(items)? / 24;
    // An item's key is its own part after its parent's, the path's folders being items too.
    let key = |item: usize| -> Option<String> {
        let mut parts = Vec::new();
        let mut at = item;
        for _ in 0..64 {
            let base = items + 24 * at;
            let (start, size) = (u32_at(base + 8)? as usize, u16_at(base + 12)? as usize);
            parts.push(String::from_utf8_lossy(table.get(start..start + size)?).into_owned());
            match u32_at(base + 4)? {
                u32::MAX => break,
                parent => at = parent as usize,
            }
        }
        parts.reverse();
        Some(parts.concat())
    };
    let item = (0..count).find(|&item| key(item).as_deref() == Some(path))?;
    let base = items + 24 * item;
    if *table.get(base + 14)? != b'v' {
        return None;
    }
    let (start, end) = (u32_at(base + 16)? as usize, u32_at(base + 20)? as usize);
    // A variant: "(uuay)" after its value and a nul; the value its size, flags, then its bytes.
    let variant = table.get(start..end)?;
    let value = variant.strip_suffix(b"\0(uuay)")?;
    let size = u32::from_le_bytes(value.get(0..4)?.try_into().ok()?) as usize;
    let flags = u32::from_le_bytes(value.get(4..8)?.try_into().ok()?);
    let data = value.get(8..)?;
    match flags & 1 {
        0 => data.get(..size).map(<[u8]>::to_vec),
        _ => {
            let mut unpacked = Vec::with_capacity(size);
            flate2::read::ZlibDecoder::new(data).read_to_end(&mut unpacked).ok()?;
            Some(unpacked)
        }
    }
}

/// From a stylesheet: `windowcontrols > button > image`'s fill and padding, and the hovered and pressed fill.
fn buttons_of(css: &str) -> Option<WindowButtons> {
    let image = declarations(css, "windowcontrols > button > image");
    let fill = |selector: &str| declarations(css, selector).iter().find(|(name, _)| name == "background-color").and_then(|(_, value)| share(value));
    let normal = image.iter().find(|(name, _)| name == "background-color").and_then(|(_, value)| share(value))?;
    let padding = image.iter().find(|(name, _)| name == "padding").and_then(|(_, value)| value.strip_suffix("px")?.trim().parse().ok())?;
    let radius = match image.iter().find(|(name, _)| name == "border-radius").map(|(_, value)| value.as_str()) {
        Some("100%") | Some("50%") => None,
        Some(value) => Some(value.strip_suffix("px")?.trim().parse().ok()?),
        None => Some(0.),
    };
    let hover = fill("windowcontrols > button:hover > image").unwrap_or(normal);
    let active = fill("windowcontrols > button:active > image").unwrap_or(hover);
    Some(WindowButtons { fill: [normal, hover, active], radius, padding })
}

/// The declarations of the top-level rules naming `selector` (not inside @media), in order.
fn declarations(css: &str, selector: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut rest = css;
    while let Some(open) = rest.find('{') {
        // After a statement (`@define-color name #fff;`), the selector.
        let head = rest[..open].rsplit(';').next().unwrap_or_default().trim();
        let Some(close) = rest[open..].find('}').map(|close| open + close) else { break };
        // An @media block's rules lie inside it: skipped to its end.
        if head.starts_with('@') {
            let mut depth = 0;
            let mut end = rest.len();
            for (at, c) in rest[open..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + at + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            rest = &rest[end..];
            continue;
        }
        if head.split(',').any(|one| one.trim() == selector) {
            for declaration in rest[open + 1..close].split(';') {
                if let Some((name, value)) = declaration.split_once(':') {
                    found.push((name.trim().to_owned(), value.trim().to_owned()));
                }
            }
        }
        rest = &rest[close + 1..];
    }
    found
}

/// The text colour's share in a fill: `alpha(currentColor,0.1)`, or
/// `color-mix(in srgb, currentColor 10%, transparent)`.
fn share(value: &str) -> Option<f32> {
    let compact: String = value.chars().filter(|c| !c.is_whitespace()).collect();
    if let Some(inner) = compact.strip_prefix("alpha(currentColor,").and_then(|rest| rest.strip_suffix(')')) {
        return inner.parse().ok();
    }
    let inner = compact.strip_prefix("color-mix(insrgb,currentColor")?.strip_suffix(",transparent)")?;
    inner.strip_suffix('%')?.parse::<f32>().ok().map(|percent| percent / 100.)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_buttons_from_both_stylesheets() {
        // libadwaita 1.5's (Ubuntu 24.04) and a later one's (Fedora), with a
        // header bar's own padding and a high-contrast rim that do not count.
        let old = "headerbar.default-decoration windowcontrols > button { min-height: 22px; }\n\
            windowcontrols > button > image { background-color: alpha(currentColor,0.1); border-radius: 100%; padding: 2px; }\n\
            windowcontrols > button:hover > image { background-color: alpha(currentColor,0.15); }\n\
            windowcontrols > button:active > image { background-color: alpha(currentColor,0.3); }";
        let new = "@define-color accent #3584e4; headerbar.default-decoration windowcontrols > button > image { padding: 3px; }\n\
            windowcontrols > button > image { background-color: color-mix(in srgb, currentColor 10%, transparent); border-radius: 100%; padding: 4px; }\n\
            @media (prefers-contrast: more) { windowcontrols > button > image { box-shadow: inset 0 0 0 1px red; padding: 9px; } }\n\
            windowcontrols > button:hover > image { background-color: color-mix(in srgb, currentColor 15%, transparent); }\n\
            windowcontrols > button:active > image { background-color: color-mix(in srgb, currentColor 30%, transparent); }";
        assert_eq!(buttons_of(old), Some(WindowButtons { fill: [0.1, 0.15, 0.3], radius: None, padding: 2. }));
        assert_eq!(buttons_of(new), Some(WindowButtons { fill: [0.1, 0.15, 0.3], radius: None, padding: 4. }));
        assert_eq!(buttons_of("windowcontrols > button { padding: 5px; }"), None);
    }

    #[test]
    fn window_radius_from_both_stylesheets() {
        assert_eq!(radius_of("window.csd { margin: 0px; border-radius: 12px; outline: 1px solid red; }"), Some(12.));
        let new = ":root { --window-radius: 15px; } window.csd { border-radius: var(--window-radius); } window.csd.tiled { --window-radius: 0px; }";
        assert_eq!(radius_of(new), Some(15.));
    }

    /// The installed libadwaita, where there is one (the test machine's).
    #[test]
    fn installed_library_read() {
        if library().is_none() {
            return;
        }
        let look = window_buttons().expect("libadwaita's window buttons");
        assert!(look.fill[0] > 0. && look.fill[1] >= look.fill[0] && look.padding > 0.);
        assert!(window_radius().is_some_and(|radius| radius > 0.));
    }
}
