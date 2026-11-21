//! Sound in the viewer: mpv plays it, its cover above the spectrum's bars,
//! the place and the volume under them. Space pauses, Left and Right seek,
//! + and - change the volume, M mutes.

use std::path::Path;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui_image::picker::Picker;
use sc_core::t;
use sc_core::viewer::Mode;
use sc_core::viewer::media::spectrum::{BANDS, Meter, Spectrum};
use sc_core::viewer::media::{Player, clock, is_video};
use unicode_width::UnicodeWidthStr;

use crate::App;
use crate::picture::{self, Key, Picture};
use crate::skin::skin;
use crate::viewer::Viewer;

/// While it plays, the screen is drawn this often.
pub const FRAME: Duration = Duration::from_millis(33);
/// Left and Right seek this far.
const SEEK: f64 = 5.;

pub struct Music {
    player: Result<Player, String>,
    spectrum: Spectrum,
    meter: Meter,
    metered: Instant,
    /// Playing, or the bars still move.
    pub moving: bool,
}

impl Music {
    fn open(path: &Path) -> Self {
        Music {
            player: Player::open(path, false, || {}),
            spectrum: Spectrum::open(path),
            meter: Meter::default(),
            metered: Instant::now(),
            moving: true,
        }
    }

    /// Space, Left and Right, + and -, M; whether it was one.
    pub fn key(&self, key: KeyCode) -> bool {
        let Ok(player) = &self.player else {
            return false;
        };
        match key {
            KeyCode::Char(' ') => player.toggle_pause(),
            KeyCode::Left => player.seek_by(-SEEK),
            KeyCode::Right => player.seek_by(SEEK),
            KeyCode::Char('+' | '=') => player.change_volume(5.),
            KeyCode::Char('-') => player.change_volume(-5.),
            KeyCode::Char('m' | 'M') => player.toggle_mute(),
            _ => return false,
        }
        true
    }
}

/// The cover's rows, the bars' and the line under them, in the viewer's body.
fn split(body: Rect) -> (Rect, Rect, Rect) {
    let bars = (body.height.saturating_sub(1) * 2 / 5).max(3).min(body.height.saturating_sub(1));
    let cover = body.height.saturating_sub(bars + 1);
    (
        Rect::new(body.x, body.y, body.width, cover),
        Rect::new(body.x + 1, body.y + cover, body.width.saturating_sub(2), bars),
        Rect::new(body.x + 1, body.y + cover + bars, body.width.saturating_sub(2), body.height.min(1)),
    )
}

impl App {
    /// Plays the viewer's sound while it is in sight; its cover once mpv has drawn it.
    pub fn want_music(&mut self) {
        let Some(viewer) = self.viewer.as_mut() else {
            return;
        };
        let path = viewer.shown.path().to_owned();
        if viewer.shown.mode != Mode::Picture || !viewer.shown.media || is_video(&path) {
            viewer.music = None;
            return;
        }
        let music = viewer.music.get_or_insert_with(|| Music::open(&path));
        let Ok(player) = &music.player else {
            return;
        };
        let picker = self.picker.get_or_insert_with(Picker::halfblocks);
        let font = picker.font_size();
        let (cover, _, _) = split(Rect::new(0, 0, viewer.width as u16, viewer.height as u16));
        let room = (u32::from(cover.width) * u32::from(font.width), u32::from(cover.height) * u32::from(font.height));
        // At the cover's own shape, so mpv adds no black bars.
        if let Some((width, height)) = player.video_size() {
            let scale = (f64::from(room.0) / width).min(f64::from(room.1) / height);
            player.set_size((width * scale).max(1.) as u32, (height * scale).max(1.) as u32);
        }
        let Some(frame) = player.take_frame() else {
            return;
        };
        let size = (frame.width, frame.height);
        let Some(image) = picture::image_of(frame) else {
            return;
        };
        let key = Key {
            path,
            page: None,
            room,
            office: None,
        };
        let shown = Ok((picker.new_resize_protocol(image), String::new()));
        viewer.picture = Some(Picture { key, shown, size });
    }
}

/// The cover or the file's name, the bars, the place and the volume.
pub fn draw(frame: &mut Frame, body: Rect, viewer: &mut Viewer) {
    let name = viewer.shown.path().file_name().unwrap_or_default().to_string_lossy().into_owned();
    let Some(music) = viewer.music.as_mut() else {
        return;
    };
    let style = skin().viewer;
    let player = match &music.player {
        Ok(player) => player,
        Err(err) => return middle(frame, body, t!("media.cannot_play", err = err), style),
    };
    let state = player.state();
    let levels = music.spectrum.at(state.position);
    let seconds = music.metered.elapsed().as_secs_f32().min(0.1);
    music.metered = Instant::now();
    music.moving = music.meter.update(levels, seconds) || !state.paused;
    let meter = music.meter.clone();
    let (cover, bars, line) = split(body);
    paint_bars(frame.buffer_mut(), bars, &meter);
    place(frame.buffer_mut(), line, &state, style);
    match (&state.error, state.video) {
        (Some(err), _) => middle(frame, cover, t!("media.cannot_play", err = err), style),
        (None, true) if viewer.picture.is_some() => picture::draw(frame, cover, viewer),
        _ => middle(frame, cover, name, style),
    }
}

fn middle(frame: &mut Frame, area: Rect, text: String, style: Style) {
    let at = Rect::new(area.x, area.y + area.height / 2, area.width, 2.min(area.height));
    frame.render_widget(Paragraph::new(text).style(style).centered().wrap(Wrap { trim: true }), at);
}

/// LED columns, a segment a row, green at the foot to red at the top, each peak lit.
fn paint_bars(buf: &mut Buffer, area: Rect, meter: &Meter) {
    let rows = area.height as usize;
    for band in 0..BANDS {
        let from = area.x + (band * area.width as usize / BANDS) as u16;
        let to = area.x + ((band + 1) * area.width as usize / BANDS) as u16;
        // A column apart, where there is room.
        let width = if to - from >= 2 { to - from - 1 } else { to - from };
        let lit = (meter.bars[band] * rows as f32).round() as usize;
        let peak = (meter.peaks[band] * rows as f32).ceil() as usize;
        for at in 0..rows {
            if at >= lit && !(peak > lit && at + 1 == peak) {
                continue;
            }
            let share = (at as f32 + 0.5) / rows as f32;
            let colour = led(share);
            let y = area.bottom() - 1 - at as u16;
            for x in from..from + width {
                buf[(x, y)].set_symbol("▆").set_fg(colour);
            }
        }
    }
}

/// Green at the foot, yellow at 60%, red at the top.
fn led(share: f32) -> Color {
    match share < 0.6 {
        true => Color::Rgb((share / 0.6 * 255.) as u8, 220, 40),
        false => Color::Rgb(255, (220. - (share - 0.6) / 0.4 * 220.) as u8, 40),
    }
}

/// ▶ 1:05 / 3:20 ━━━━━━─────── 100%, and the keys where there is room; the volume struck through when muted.
fn place(buf: &mut Buffer, area: Rect, state: &sc_core::viewer::media::State, style: Style) {
    let mark = if state.paused { "❚❚" } else { "▶" };
    let left = format!("{mark} {} / {} ", clock(state.position), clock(state.duration));
    let volume = format!(" {:.0}%", state.volume);
    let keys = format!("   {}", t!("scmd.music.keys"));
    let width = area.width as usize;
    let keys = match width >= left.width() + 20 + volume.width() + keys.width() {
        true => keys,
        false => String::new(),
    };
    let room = width.saturating_sub(left.width() + volume.width() + keys.width());
    let share = match state.duration > 0. {
        true => (state.position / state.duration).clamp(0., 1.),
        false => 0.,
    };
    let done = (share * room as f64).round() as usize;
    let bar = "━".repeat(done) + &"─".repeat(room - done);
    let muted = if state.muted { style.add_modifier(Modifier::CROSSED_OUT) } else { style };
    let mut x = area.x;
    for (text, style) in [(&left, style), (&bar, style), (&volume, muted), (&keys, style.add_modifier(Modifier::DIM))] {
        x = buf.set_stringn(x, area.y, text, area.right().saturating_sub(x) as usize, style).0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_stand_in_their_columns() {
        let area = Rect::new(0, 0, 64, 10);
        let mut buf = Buffer::empty(area);
        let mut meter = Meter::default();
        let mut levels = [0.; BANDS];
        (levels[0], levels[31]) = (1., 0.5);
        meter.update(Some(levels), 0.);
        paint_bars(&mut buf, area, &meter);
        let lit = |x: u16| (0..10).filter(|&y| buf[(x, y)].symbol() == "▆").count();
        assert_eq!((lit(0), lit(1)), (10, 0), "a column, then a gap");
        assert_eq!((lit(62), lit(63)), (5, 0));
        assert_eq!(lit(30), 0);
        assert_eq!(buf[(0, 9)].fg, led(0.05), "green at the foot");
    }

    #[test]
    fn splits_the_body() {
        let (cover, bars, line) = split(Rect::new(0, 1, 80, 22));
        assert_eq!((cover.y, cover.height), (1, 13));
        assert_eq!((bars.y, bars.height), (14, 8));
        assert_eq!((line.y, line.height), (22, 1));
    }
}
