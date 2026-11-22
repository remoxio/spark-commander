//! Video and sound in the viewer, played by libmpv (sc_core::viewer::media): the
//! frames mpv draws at this view's size, and the controls below them. A song:
//! its cover, its title, artist, album and format beside it over its words
//! (the line sung lit), its spectrum's bars under them.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Icon, IconNamed, Sizable, StyledExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::viewer::media::lyrics::Lyrics;
use sc_core::viewer::media::spectrum::{BANDS, Meter, Spectrum};
use sc_core::viewer::media::{Player, State, clock, is_video};
use sc_core::viewer::resume;

/// The player's icons, Lucide's (assets/media).
#[derive(Clone, Copy)]
pub enum MediaIcon {
    Play,
    Pause,
    Volume,
    Muted,
    FullScreen,
    Window,
}

impl IconNamed for MediaIcon {
    fn path(self) -> SharedString {
        match self {
            MediaIcon::Play => "media/play.svg",
            MediaIcon::Pause => "media/pause.svg",
            MediaIcon::Volume => "media/volume-2.svg",
            MediaIcon::Muted => "media/volume-x.svg",
            MediaIcon::FullScreen => "media/maximize.svg",
            MediaIcon::Window => "media/minimize.svg",
        }
        .into()
    }
}

/// The icons' files, by name.
pub const ICONS: [(&str, &[u8]); 6] = [
    ("play.svg", include_bytes!("../../assets/media/play.svg")),
    ("pause.svg", include_bytes!("../../assets/media/pause.svg")),
    ("volume-2.svg", include_bytes!("../../assets/media/volume-2.svg")),
    ("volume-x.svg", include_bytes!("../../assets/media/volume-x.svg")),
    ("maximize.svg", include_bytes!("../../assets/media/maximize.svg")),
    ("minimize.svg", include_bytes!("../../assets/media/minimize.svg")),
];

pub struct MediaView {
    path: PathBuf,
    player: Result<Rc<Player>, String>,
    image: Option<Arc<RenderImage>>,
    /// Frames shown before, their textures freed at the next draw.
    stale: Vec<Arc<RenderImage>>,
    /// Where the seek bar is, for the mouse.
    bar: Rc<Cell<Bounds<Pixels>>>,
    seeking: bool,
    /// The file's fingerprint once made, to go on where it was left
    /// (sc_core::viewer::resume); none in Quick View, which only shows it.
    fingerprint: Option<String>,
    /// Gone to where it was left, or found there was nowhere to go.
    resumed: bool,
    /// "Continuing at 12:34", for a moment.
    continued: Option<f64>,
    /// When the place was last saved while playing.
    saved: Instant,
    /// Sound alone, drawn as bars; read once it plays.
    music: bool,
    spectrum: Option<Spectrum>,
    meter: Meter,
    metered: Instant,
    /// A song's tags and words, read once it is loaded.
    song: Option<Rc<Song>>,
    reading: bool,
    /// Its words' list, and the line last lit.
    lyrics_scroll: ScrollHandle,
    sung: Option<usize>,
    _frames: Task<()>,
}

/// What a song's tags say, its format, its words.
struct Song {
    title: String,
    artist: Option<String>,
    album: Option<String>,
    format: Vec<String>,
    lyrics: Option<Lyrics>,
}

/// While playing, the place is saved this often.
const SAVE_EVERY: Duration = Duration::from_secs(15);

impl MediaView {
    /// `paused`: Quick View's, which shows what the cursor passes.
    pub fn new(path: PathBuf, paused: bool, cx: &mut Context<Self>) -> Self {
        let (sent, mut received) = futures::channel::mpsc::unbounded::<()>();
        let player = Player::open(&path, paused, move || {
            let _ = sent.unbounded_send(());
        });
        if !paused {
            let print = path.clone();
            let printing = cx.background_executor().spawn(async move { resume::fingerprint(&print).ok() });
            cx.spawn(async move |this, cx| {
                let fingerprint = printing.await;
                let _ = this.update(cx, |this, cx| {
                    this.fingerprint = fingerprint;
                    this.resume(cx);
                });
            })
            .detach();
        }
        let frames = cx.spawn(async move |this, cx| {
            while received.next().await.is_some() {
                // Notices come in bursts; one redraw takes them all.
                while received.try_recv().is_ok() {}
                if this.update(cx, |this, cx| this.take_frame(cx)).is_err() {
                    break;
                }
            }
        });
        MediaView {
            player: player.map(Rc::new),
            image: None,
            stale: Vec::new(),
            bar: Rc::default(),
            seeking: false,
            fingerprint: None,
            resumed: false,
            continued: None,
            saved: Instant::now(),
            music: !is_video(&path),
            path,
            spectrum: None,
            meter: Meter::default(),
            metered: Instant::now(),
            song: None,
            reading: false,
            lyrics_scroll: ScrollHandle::new(),
            sung: None,
            _frames: frames,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn player(&self) -> Option<Rc<Player>> {
        self.player.as_ref().ok().cloned()
    }

    fn take_frame(&mut self, cx: &mut Context<Self>) {
        let frame = self.player().and_then(|player| player.take_frame());
        if let Some(frame) = frame
            && let Some(buffer) = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra)
        {
            let image = Arc::new(RenderImage::new([image::Frame::new(buffer)]));
            self.stale.extend(self.image.replace(image));
        }
        self.resume(cx);
        self.read_song(cx);
        if self.saved.elapsed() > SAVE_EVERY && self.player().is_some_and(|player| !player.state().paused) {
            self.save();
        }
        cx.notify();
    }

    /// Once its length is known: a long one goes on where it was left.
    fn resume(&mut self, cx: &mut Context<Self>) {
        let (Some(fingerprint), Some(player)) = (&self.fingerprint, self.player()) else {
            return;
        };
        let duration = player.state().duration;
        if self.resumed || duration <= 0. {
            return;
        }
        self.resumed = true;
        let Some(at) = resume::left_at(fingerprint).filter(|at| duration >= resume::MEDIA_MIN && *at < duration) else {
            return;
        };
        player.seek_seconds(at);
        self.continued = Some(at);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(4)).await;
            let _ = this.update(cx, |this, cx| {
                this.continued = None;
                cx.notify();
            });
        })
        .detach();
    }

    /// A song's tags once it is loaded; its words (an .lrc beside it read) meanwhile.
    fn read_song(&mut self, cx: &mut Context<Self>) {
        let Some(player) = self.player().filter(|player| self.music && !self.reading && player.state().duration > 0.) else {
            return;
        };
        self.reading = true;
        let tags = player.tags();
        let tag = |name: &str| tags.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.trim().to_owned()).filter(|value| !value.is_empty());
        let stem = self.path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let (title, artist, album, format) = (tag("title").unwrap_or(stem), tag("artist"), tag("album"), player.sound_format());
        let path = self.path.clone();
        let reading = cx.background_executor().spawn(async move { Lyrics::of(&path, &tags) });
        cx.spawn(async move |this, cx| {
            let lyrics = reading.await;
            let _ = this.update(cx, |this, cx| {
                this.song = Some(Rc::new(Song { title, artist, album, format, lyrics }));
                cx.notify();
            });
        })
        .detach();
    }

    /// The line sung in the middle of the words' list, as it changes.
    fn follow_lyrics(&mut self, line: Option<usize>) {
        if line == self.sung {
            return;
        }
        self.sung = line;
        let scroll = &self.lyrics_scroll;
        let Some(item) = line.and_then(|line| scroll.bounds_for_item(line)) else {
            return;
        };
        // A line's bounds are where it is unscrolled.
        let (view, offset) = (scroll.bounds(), scroll.offset());
        let top = item.origin.y - view.origin.y;
        let wanted = (top - (view.size.height - item.size.height) / 2.).clamp(px(0.), scroll.max_offset().y.max(px(0.)));
        scroll.set_offset(point(offset.x, -wanted));
    }

    /// Where a long one is now, to go on from next time; not before it went back there.
    fn save(&mut self) {
        self.saved = Instant::now();
        let (Some(fingerprint), Some(player)) = (&self.fingerprint, self.player()) else {
            return;
        };
        let state = player.state();
        if !self.resumed || state.duration < resume::MEDIA_MIN {
            return;
        }
        let name = self.path.file_name().unwrap_or_default().to_string_lossy();
        let _ = resume::leave(fingerprint, &name, state.position, state.duration);
    }

    /// The bars at the place playing, moved on since the last frame; drawn again while they move.
    fn meter(&mut self, state: &State, window: &mut Window) -> Meter {
        if self.spectrum.is_none() && !state.paused {
            self.spectrum = Some(Spectrum::open(&self.path));
        }
        let levels = self.spectrum.as_ref().and_then(|spectrum| spectrum.at(state.position));
        let seconds = self.metered.elapsed().as_secs_f32().min(0.1);
        self.metered = Instant::now();
        if self.meter.update(levels, seconds) || !state.paused {
            window.request_animation_frame();
        }
        self.meter.clone()
    }

    /// To where the mouse is on the seek bar.
    fn seek_at(&self, x: Pixels) {
        let bar = self.bar.get();
        if let Some(player) = self.player().filter(|_| bar.size.width > px(0.)) {
            player.seek_to(((x - bar.origin.x) / bar.size.width) as f64);
        }
    }
}

impl Drop for MediaView {
    fn drop(&mut self) {
        self.save();
    }
}

impl Render for MediaView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for old in self.stale.drain(..) {
            let _ = window.drop_image(old);
        }
        let player = match &self.player {
            Ok(player) => player.clone(),
            Err(err) => {
                let text = sc_core::t!("media.cannot_play_as_text", err = err);
                return div().p_4().child(text).into_any_element();
            }
        };
        let state = player.state();
        let meter = self.music.then(|| self.meter(&state, window));
        let image = self.image.clone();
        let sizing = player.clone();
        // mpv draws at the picture's size in pixels; the frame fills it.
        let picture = canvas(
            move |bounds, window, _| {
                let pixels = |side: Pixels| (f32::from(side) * window.scale_factor()).round() as u32;
                sizing.set_size(pixels(bounds.size.width), pixels(bounds.size.height));
            },
            move |bounds, (), window, _| {
                if let Some(image) = image {
                    let _ = window.paint_image(bounds, bounds, Corners::default(), image, 0, false);
                }
            },
        )
        .size_full();
        let controls = self.render_controls(&state, window, cx);
        let continued = self.continued.map(|at| {
            div()
                .absolute()
                .top_3()
                .left_3()
                .px_3()
                .py_1()
                .rounded_md()
                .bg(gpui_kit::black().opacity(0.6))
                .text_color(gpui_kit::white())
                .text_sm()
                .child(sc_core::t!("media.continuing", time = clock(at)))
        });
        let body = match (meter, &state.error) {
            (Some(meter), None) => self.render_song(&state, picture, meter, cx).into_any_element(),
            _ => {
                let notice = match (&state.error, state.video) {
                    (Some(err), _) => Some(sc_core::t!("media.cannot_play", err = err)),
                    (None, false) => Some(self.path.file_name().unwrap_or_default().display().to_string()),
                    (None, true) => None,
                };
                div()
                    .id("media-picture")
                    .size_full()
                    .relative()
                    .bg(gpui_kit::black())
                    .on_click(|event, window, _| {
                        if event.click_count() == 2 {
                            window.toggle_fullscreen();
                        }
                    })
                    .child(picture)
                    .when_some(notice, |el, notice| {
                        el.child(div().absolute().inset_0().flex().items_center().justify_center().text_color(gpui_kit::white()).child(notice))
                    })
                    .into_any_element()
            }
        };
        v_flex()
            .size_full()
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, _| {
                if this.seeking && event.pressed_button == Some(MouseButton::Left) {
                    this.seek_at(event.position.x);
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.seeking = false))
            .child(div().flex_1().min_h_0().relative().child(body).children(continued))
            .child(controls)
            .into_any_element()
    }
}

impl MediaView {
    /// The cover, the song's names and format over its words, the bars under them.
    fn render_song(&mut self, state: &State, picture: Canvas<()>, meter: Meter, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let song = self.song.clone();
        let frame = |el: Div| el.rounded(theme.radius).border_1().border_color(theme.border).overflow_hidden();
        let cover = frame(div().h_full().flex_none().aspect_ratio(1.).max_w(relative(0.45)).bg(gpui_kit::black())).map(|el| match state.video {
            true => el.child(picture),
            false => el.flex().items_center().justify_center().bg(theme.muted).child(Icon::new(Lucide::Music).size(px(64.)).text_color(theme.muted_foreground)),
        });
        let names = song.as_ref().map(|song| {
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().text_2xl().font_bold().truncate().child(song.title.clone()))
                .children(song.artist.clone().map(|artist| div().text_lg().truncate().child(artist)))
                .children(song.album.clone().map(|album| div().text_color(theme.muted_foreground).truncate().child(album)))
        });
        let format = song.as_ref().map(|song| {
            let mut parts = song.format.clone();
            if state.bitrate > 0. {
                parts.push(format!("{:.0} kb/s", state.bitrate / 1000.));
            }
            let line = |icon: Lucide, text: String| h_flex().gap_2().child(Icon::new(icon).size_4().text_color(theme.muted_foreground)).child(text);
            v_flex()
                .flex_none()
                .gap_2()
                .pl_4()
                .border_l_1()
                .border_color(theme.border)
                .text_sm()
                .child(line(Lucide::Clock, clock(state.duration)))
                .when(!parts.is_empty(), |el| el.child(line(Lucide::AudioLines, parts.join(" · "))))
        });
        let lyrics = song.as_ref().and_then(|song| song.lyrics.as_ref()).map(|lyrics| self.render_lyrics(lyrics, state.position, cx));
        let header = h_flex().flex_none().items_stretch().gap_4().children(names).children(format);
        let side = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .gap_4()
            .map(|el| match lyrics {
                Some(lyrics) => el.child(header).child(frame(div().flex_1().min_h_0().flex().flex_col()).child(lyrics)),
                None => el.justify_center().child(header),
            });
        let bars = frame(div().flex_none().h(relative(0.32)).px_4().pt_3().pb_3().bg(gpui_kit::black()))
            .child(canvas(|_, _, _| {}, move |bounds, (), window, _| paint_bars(&meter, bounds, window)).size_full());
        v_flex()
            .size_full()
            .p_4()
            .gap_4()
            .child(h_flex().flex_1().min_h_0().gap_4().child(cover).child(side))
            .child(bars)
    }

    /// The words, the line sung lit and kept in the middle; a synced line clicked goes there.
    fn render_lyrics(&mut self, lyrics: &Lyrics, position: f64, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme().clone();
        let sung = lyrics.line_at(position);
        self.follow_lyrics(sung);
        let lines = lyrics.lines.iter().enumerate().map(|(ix, line)| {
            let now = sung == Some(ix);
            let past = sung.is_some_and(|sung| ix < sung);
            let text = match line.text.is_empty() {
                true => "♪".to_owned(),
                false => line.text.clone(),
            };
            let marker = div().w(px(14.)).flex_none().when(now, |el| el.child(Icon::new(MediaIcon::Play).size_3().text_color(theme.primary)));
            h_flex()
                .id(("lyric", ix))
                .flex_none()
                .gap_2()
                .px_2()
                .py_1()
                .rounded(theme.radius)
                .map(|el| match (now, past) {
                    (true, _) => el.bg(theme.primary.opacity(0.18)).text_lg().font_semibold(),
                    (false, true) => el.text_color(theme.muted_foreground),
                    (false, false) => el.text_color(theme.foreground.opacity(0.8)),
                })
                .child(marker)
                .child(div().min_w_0().child(text))
                .when_some(line.at, |el, at| {
                    el.cursor_pointer().hover(|style| style.bg(theme.foreground.opacity(0.06))).on_click(cx.listener(move |this, _, _, _| {
                        this.player().inspect(|player| player.seek_seconds(at));
                    }))
                })
        });
        v_flex().id("lyrics").size_full().p_2().overflow_y_scroll().track_scroll(&self.lyrics_scroll).children(lines)
    }

    /// Play, the place, the seek bar, mute and the volume, full screen.
    fn render_controls(&self, state: &State, window: &Window, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let share = match state.duration > 0. {
            true => (state.position / state.duration).clamp(0., 1.) as f32,
            false => 0.,
        };
        let bar = self.bar.clone();
        let full_screen = window.is_fullscreen();
        let button = |id: &'static str, icon: MediaIcon| Button::new(id).icon(icon).small().ghost();
        h_flex()
            .flex_none()
            .gap_2()
            .px_2()
            .py_1()
            .items_center()
            .border_t_1()
            .border_color(theme.border)
            .child(
                button("media-play", if state.paused { MediaIcon::Play } else { MediaIcon::Pause }).on_click(cx.listener(|this, _, _, _| {
                    this.player().inspect(|player| player.toggle_pause());
                })),
            )
            .child(div().flex_none().text_sm().child(format!("{} / {}", clock(state.position), clock(state.duration))))
            .child(
                div()
                    .id("media-seek")
                    .flex_1()
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.seeking = true;
                            this.seek_at(event.position.x);
                        }),
                    )
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(6.))
                            .rounded_full()
                            .bg(theme.muted)
                            .child(div().absolute().left_0().top_0().h_full().w(relative(share)).rounded_full().bg(theme.primary))
                            .child(canvas(move |bounds, _, _| bar.set(bounds), |_, _, _, _| {}).absolute().size_full()),
                    ),
            )
            .child(
                button("media-mute", if state.muted { MediaIcon::Muted } else { MediaIcon::Volume }).on_click(cx.listener(|this, _, _, _| {
                    this.player().inspect(|player| player.toggle_mute());
                })),
            )
            .child(div().flex_none().w(px(40.)).text_sm().child(format!("{:.0}%", state.volume)))
            .child(
                button("media-full-screen", if full_screen { MediaIcon::Window } else { MediaIcon::FullScreen })
                    .on_click(|_, window, _| window.toggle_fullscreen()),
            )
    }
}

/// LED columns, green at the foot to red at the top: unlit segments faint, each peak lit.
fn paint_bars(meter: &Meter, bounds: Bounds<Pixels>, window: &mut Window) {
    let rows = (f32::from(bounds.size.height) / 9.).clamp(6., 40.) as usize;
    let (column, row) = (bounds.size.width / BANDS as f32, bounds.size.height / rows as f32);
    for band in 0..BANDS {
        let lit = (meter.bars[band] * rows as f32).round() as usize;
        let peak = (meter.peaks[band] * rows as f32).ceil() as usize;
        for at in 0..rows {
            let share = (at as f32 + 0.5) / rows as f32;
            // Yellow at 60%.
            let hue = match share < 0.6 {
                true => 120. - share / 0.6 * 60.,
                false => 60. - (share - 0.6) / 0.4 * 60.,
            };
            let on = at < lit || (peak > lit && at + 1 == peak);
            let x = bounds.origin.x + column * (band as f32 + 0.12);
            let y = bounds.origin.y + bounds.size.height - row * (at + 1) as f32;
            let segment = Bounds::new(point(x.round(), y.round()), size((column * 0.76).round(), (row * 0.72).round().max(px(1.))));
            window.paint_quad(fill(segment, hsla(hue / 360., 0.9, 0.5, if on { 1. } else { 0.1 })));
        }
    }
}
