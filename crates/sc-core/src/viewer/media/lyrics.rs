//! A song's words: an .lrc file beside it (`Song.lrc` for `Song.flac`), else
//! its tags'; synced where they carry LRC's times (`[01:23.45]`), else plain.

use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Seconds into the song; none in plain words.
    pub at: Option<f64>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lyrics {
    pub lines: Vec<Line>,
}

impl Lyrics {
    /// LRC's lines in time order (a line with several times once for each), else
    /// the text's lines; none without a word.
    pub fn parse(text: &str) -> Option<Lyrics> {
        let mut offset = 0.;
        let mut timed = Vec::new();
        for raw in text.lines() {
            let (times, rest) = stamps(raw.trim(), &mut offset);
            let text = words(rest);
            timed.extend(times.into_iter().map(|at| Line { at: Some(at), text: text.clone() }));
        }
        let mut lines = match timed.is_empty() {
            true => text.lines().map(|line| Line { at: None, text: line.trim().to_owned() }).collect(),
            false => {
                // LRC's offset is in milliseconds; a positive one shows the words sooner.
                timed.iter_mut().for_each(|line| line.at = line.at.map(|at| (at - offset / 1000.).max(0.)));
                timed.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal));
                timed
            }
        };
        while lines.last().is_some_and(|line| line.text.is_empty()) {
            lines.pop();
        }
        let start = lines.iter().position(|line| !line.text.is_empty())?;
        lines.drain(..start);
        Some(Lyrics { lines })
    }

    pub fn synced(&self) -> bool {
        self.lines.first().is_some_and(|line| line.at.is_some())
    }

    /// The line sung at `seconds`; none before the first.
    pub fn line_at(&self, seconds: f64) -> Option<usize> {
        let after = self.lines.partition_point(|line| line.at.is_some_and(|at| at <= seconds));
        after.checked_sub(1).filter(|_| self.synced())
    }

    /// `song`'s: its .lrc, else the tags' (`tags`, as mpv names them), the synced first.
    pub fn of(song: &Path, tags: &[(String, String)]) -> Option<Lyrics> {
        let beside = song.with_extension("lrc");
        // shortcut: a .lrc not in UTF-8 is read as Latin-1; other code pages would need the viewer's guess.
        let file = std::fs::read(&beside).ok().map(|bytes| match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(err) => err.into_bytes().iter().map(|&byte| byte as char).collect(),
        });
        let tagged = tags.iter().filter(|(key, _)| {
            let key = key.to_ascii_lowercase();
            key == "lyrics" || key.starts_with("lyrics-") || key == "unsyncedlyrics" || key == "syncedlyrics"
        });
        let mut found: Vec<Lyrics> = file.iter().chain(tagged.map(|(_, value)| value)).filter_map(|text| Lyrics::parse(text)).collect();
        let synced = found.iter().position(Lyrics::synced).unwrap_or(0);
        (!found.is_empty()).then(|| found.swap_remove(synced))
    }
}

/// A line's leading `[…]` tags: its times in seconds (an `[offset:…]` kept in
/// `offset`), and what follows them; none for `[ar:…]` and the like.
fn stamps<'a>(mut line: &'a str, offset: &mut f64) -> (Vec<f64>, &'a str) {
    let mut times = Vec::new();
    while let Some(rest) = line.strip_prefix('[') {
        let Some((tag, after)) = rest.split_once(']') else { break };
        match time(tag) {
            Some(at) => times.push(at),
            None => {
                if let Some(value) = tag.strip_prefix("offset:") {
                    *offset = value.trim().parse().unwrap_or(0.);
                }
                return (Vec::new(), "");
            }
        }
        line = after;
    }
    (times, line)
}

/// `mm:ss`, `mm:ss.xx` or `mm:ss:xx` in seconds.
fn time(tag: &str) -> Option<f64> {
    let (minutes, rest) = tag.trim().split_once(':')?;
    let seconds = rest.replacen(':', ".", 1);
    let (minutes, seconds): (f64, f64) = (minutes.parse().ok()?, seconds.parse().ok()?);
    (minutes >= 0. && (0. ..60.).contains(&seconds)).then_some(minutes * 60. + seconds)
}

/// The words without enhanced LRC's `<mm:ss.xx>` times of each word.
fn words(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        let Some(close) = rest[open..].find('>') else { break };
        if time(&rest[open + 1..open + close]).is_none() {
            out.push_str(&rest[..open + close + 1]);
        } else {
            out.push_str(&rest[..open]);
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lrc() {
        let text = "[ar:Someone]\n[offset:+500]\n[00:12.00]First <00:12.50>line\n\n[00:20.5][01:02:30]Chorus\n[00:30.00]\n[00:31.00]Last";
        let lyrics = Lyrics::parse(text).unwrap();
        let shown: Vec<(f64, &str)> = lyrics.lines.iter().map(|line| (line.at.unwrap(), line.text.as_str())).collect();
        assert_eq!(shown, [(11.5, "First line"), (20., "Chorus"), (29.5, ""), (30.5, "Last"), (61.8, "Chorus")]);
        assert_eq!(lyrics.line_at(5.), None);
        assert_eq!(lyrics.line_at(11.5), Some(0));
        assert_eq!(lyrics.line_at(25.), Some(1));
        assert_eq!(lyrics.line_at(500.), Some(4));
    }

    #[test]
    fn plain_words_and_tags() {
        let plain = Lyrics::parse("\n  Verse one\nline two\n\nVerse two\n\n").unwrap();
        assert!(!plain.synced() && plain.line_at(10.).is_none());
        assert_eq!(plain.lines.len(), 4);
        assert_eq!(Lyrics::parse(" \n\n"), None);
        let tags = [("LYRICS".to_owned(), "plain words".to_owned()), ("lyrics-eng".to_owned(), "[00:01.00]timed".to_owned())];
        let found = Lyrics::of(Path::new("/nowhere/song.flac"), &tags).unwrap();
        assert_eq!(found.lines, [Line { at: Some(1.), text: "timed".into() }]);
    }
}
