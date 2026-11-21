//! A sound's spectrum as bars, for the players to draw: a second, silent mpv
//! writes the sound as plain samples, as fast as it decodes, into a pipe read
//! here, its bands' levels 50 times for each second of sound.

use std::ffi::{c_char, c_int};
use std::fs::File;
use std::io::{self, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::{EVENT_END_FILE, EVENT_SHUTDOWN, Mpv, Raw, error, library, text};

pub const BANDS: usize = 32;
/// Levels for each second of sound.
const STEPS: usize = 50;
/// Samples a second: bands up to 16 kHz.
const RATE: usize = 32_000;
const HOP: usize = RATE / STEPS;
/// Samples in each look, 64 ms.
const SIZE: usize = 2048;
const LOWEST: f32 = 50.;
const HIGHEST: f32 = 16_000.;
/// The quietest drawn, in decibels under a full-scale tone.
const FLOOR: f32 = 60.;

/// The levels read so far; reading stops when it is dropped.
pub struct Spectrum {
    levels: Arc<Mutex<Vec<[u8; BANDS]>>>,
    stop: Arc<AtomicBool>,
}

impl Spectrum {
    /// Starts reading the sound of `path` on a thread of its own.
    pub fn open(path: &Path) -> Spectrum {
        let (levels, stop) = (Arc::default(), Arc::new(AtomicBool::new(false)));
        let (path, into, stopped) = (path.to_owned(), Arc::clone(&levels), Arc::clone(&stop));
        std::thread::spawn(move || {
            let _ = read(&path, &into, &stopped);
        });
        Spectrum { levels, stop }
    }

    /// Each band's level, 0 to 1, `seconds` in; none where not read yet.
    pub fn at(&self, seconds: f64) -> Option<[f32; BANDS]> {
        let levels = self.levels.lock().unwrap();
        let step = seconds.max(0.) * STEPS as f64;
        let (first, share) = (step as usize, step.fract() as f32);
        let a = levels.get(first)?;
        let b = levels.get(first + 1).unwrap_or(a);
        Some(std::array::from_fn(|band| (f32::from(a[band]) * (1. - share) + f32::from(b[band]) * share) / 255.))
    }
}

impl Drop for Spectrum {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What is drawn: bars that rise at once and fall slowly, and each one's
/// peak, held a moment.
#[derive(Clone, Default)]
pub struct Meter {
    pub bars: [f32; BANDS],
    pub peaks: [f32; BANDS],
    held: [f32; BANDS],
}

/// Heights a second.
const FALL: f32 = 1.2;
const PEAK_FALL: f32 = 0.5;
/// Seconds.
const HOLD: f32 = 0.6;

impl Meter {
    /// To `levels` (none: silence), `seconds` after the last; whether anything moved.
    pub fn update(&mut self, levels: Option<[f32; BANDS]>, seconds: f32) -> bool {
        let before = (self.bars, self.peaks);
        let levels = levels.unwrap_or_default();
        for (band, level) in levels.into_iter().enumerate() {
            let bar = level.max(self.bars[band] - FALL * seconds).max(0.);
            self.bars[band] = bar;
            if bar >= self.peaks[band] {
                (self.peaks[band], self.held[band]) = (bar, HOLD);
            } else if self.held[band] > 0. {
                self.held[band] -= seconds;
            } else {
                self.peaks[band] = (self.peaks[band] - PEAK_FALL * seconds).max(bar);
            }
        }
        // A peak above its bar has yet to fall.
        (self.bars, self.peaks) != before || (0..BANDS).any(|band| self.peaks[band] > self.bars[band])
    }
}

/// Reads the sound into `levels` until it ends or `stop`.
fn read(path: &Path, levels: &Mutex<Vec<[u8; BANDS]>>, stop: &AtomicBool) -> Result<(), String> {
    let mpv = library()?;
    let pipe = Pipe::create().map_err(|err| err.to_string())?;
    // Open before mpv is, so its opening does not wait.
    let mut input = pipe.reader().map_err(|err| err.to_string())?;
    let mut silent = Some(Silent::start(mpv, path, &pipe.0)?);
    let (mut analysis, mut chunk) = (Analysis::new(), vec![0; 1 << 16]);
    let (mut ended, mut closing) = (false, None);
    loop {
        let stopped = stop.load(Ordering::Relaxed);
        if (ended || stopped)
            && let Some(player) = silent.take()
        {
            // Closed elsewhere, as mpv may wait to write its last samples till they are read here.
            closing = Some(std::thread::spawn(move || drop(player)));
        }
        // Asked before reading, so what mpv wrote while closing is read first.
        let closed = closing.as_ref().is_some_and(std::thread::JoinHandle::is_finished);
        let nothing = match input.read(&mut chunk) {
            Ok(0) => true,
            Ok(_) if stopped => false,
            Ok(read) => {
                analysis.feed(&chunk[..read], |step| levels.lock().unwrap().push(step));
                false
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => true,
            Err(err) if err.kind() == ErrorKind::Interrupted => false,
            Err(err) => {
                // Closed first: an mpv in mid-write gets an error rather than waiting.
                drop(input);
                return Err(err.to_string());
            }
        };
        if nothing {
            match &silent {
                Some(player) => ended = player.ended(0.005),
                None if closed => return Ok(()),
                None => std::thread::sleep(std::time::Duration::from_millis(2)),
            }
        }
    }
}

/// Where mpv writes the samples: a pipe, which keeps nothing.
struct Pipe(PathBuf);

impl Pipe {
    fn create() -> io::Result<Pipe> {
        static MADE: AtomicUsize = AtomicUsize::new(0);
        let name = format!("sc-spectrum-{}-{}", std::process::id(), MADE.fetch_add(1, Ordering::Relaxed));
        let path = std::env::temp_dir().join(name);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            use std::os::unix::ffi::OsStrExt;
            let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
            // SAFETY: a valid path.
            if unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        // shortcut: elsewhere a file, holding the whole sound's samples (4 MB a minute) till read; a named pipe if long ones matter.
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        File::create(&path)?;
        Ok(Pipe(path))
    }

    /// Reads what is there without waiting; empty before mpv opens it, and after it closes it.
    fn reader(&self) -> io::Result<File> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(&self.0)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        File::open(&self.0)
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// mpv writing a file's sound as 16-bit mono samples, as fast as it decodes.
struct Silent {
    mpv: &'static Mpv,
    handle: Raw,
}

impl Silent {
    fn start(mpv: &'static Mpv, path: &Path, into: &Path) -> Result<Silent, String> {
        // SAFETY: a plain constructor; null when it fails.
        let handle = Raw(unsafe { (mpv.create)() });
        if handle.0.is_null() {
            return Err(crate::t!("media.error.not_started"));
        }
        let silent = Silent { mpv, handle };
        let check = |code: c_int| if code >= 0 { Ok(()) } else { Err(error(mpv, code)) };
        let into = into.to_string_lossy();
        for (name, value) in [
            ("vid", "no"),
            ("sid", "no"),
            ("vo", "null"),
            ("ao", "pcm"),
            ("ao-pcm-file", &*into),
            ("ao-pcm-waveheader", "no"),
            ("audio-format", "s16"),
            ("audio-channels", "mono"),
            ("audio-samplerate", "32000"),
            ("untimed", "yes"),
            ("terminal", "no"),
            ("idle", "no"),
        ] {
            let (name, value) = (text(name)?, text(value)?);
            // SAFETY: valid strings, a live handle.
            check(unsafe { (mpv.set_option_string)(handle.0, name.as_ptr(), value.as_ptr()) })?;
        }
        // SAFETY: a live handle.
        check(unsafe { (mpv.initialize)(handle.0) })?;
        let (command, file) = (text("loadfile")?, text(&path.to_string_lossy())?);
        let mut args: [*const c_char; 3] = [command.as_ptr(), file.as_ptr(), std::ptr::null()];
        // SAFETY: a null-ended list of strings that outlive the call.
        check(unsafe { (mpv.command)(handle.0, args.as_mut_ptr()) })?;
        Ok(silent)
    }

    /// The sound has ended, or could not be read; waits up to `seconds` for news.
    fn ended(&self, seconds: f64) -> bool {
        // SAFETY: a live handle, waited on by this thread alone.
        let event = unsafe { &*(self.mpv.wait_event)(self.handle.0, seconds) };
        matches!(event.id, EVENT_END_FILE | EVENT_SHUTDOWN)
    }
}

impl Drop for Silent {
    fn drop(&mut self) {
        // SAFETY: no one else uses the handle.
        unsafe { (self.mpv.terminate_destroy)(self.handle.0) };
    }
}

/// The samples read so far, looked at HOP apart, each look's bands.
struct Analysis {
    /// From the next look's first.
    samples: Vec<f32>,
    /// A sample's first byte, its second in the next read.
    odd: Option<u8>,
    window: Vec<f32>,
    /// Each band's bins, from and to.
    bins: [(usize, usize); BANDS],
    fft: Fft,
}

impl Analysis {
    fn new() -> Self {
        let width = RATE as f32 / SIZE as f32;
        let edge = |band: usize| LOWEST * (HIGHEST / LOWEST).powf(band as f32 / BANDS as f32) / width;
        Analysis {
            // Half a look of silence first, so each look is centred on its time.
            samples: vec![0.; SIZE / 2],
            odd: None,
            // Hann's.
            window: (0..SIZE).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / SIZE as f32).cos()).collect(),
            bins: std::array::from_fn(|band| {
                let from = edge(band).round() as usize;
                (from, (edge(band + 1).round() as usize).max(from + 1))
            }),
            fft: Fft::new(SIZE),
        }
    }

    fn feed(&mut self, mut bytes: &[u8], mut step: impl FnMut([u8; BANDS])) {
        if let Some(first) = self.odd.take()
            && let Some((&second, rest)) = bytes.split_first()
        {
            self.samples.push(f32::from(i16::from_le_bytes([first, second])) / 32768.);
            bytes = rest;
        }
        let (pairs, odd) = bytes.as_chunks::<2>();
        self.odd = odd.first().copied();
        self.samples.extend(pairs.iter().map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32768.));
        let mut taken = 0;
        while self.samples.len() - taken >= SIZE {
            step(self.bands(taken));
            taken += HOP;
        }
        self.samples.drain(..taken);
    }

    /// The look from sample `from`.
    fn bands(&mut self, from: usize) -> [u8; BANDS] {
        let look = &self.samples[from..from + SIZE];
        let (re, im) = (&mut self.fft.re, &mut self.fft.im);
        for i in 0..SIZE {
            (re[i], im[i]) = (look[i] * self.window[i], 0.);
        }
        self.fft.run();
        // A full-scale tone's bin, through the window: SIZE / 4.
        let full = (SIZE as f32 / 4.).powi(2);
        std::array::from_fn(|band| {
            let (from, to) = self.bins[band];
            let power: f32 = (from..to).map(|bin| self.fft.re[bin].powi(2) + self.fft.im[bin].powi(2)).sum();
            let decibels = 10. * (power / full).max(1e-12).log10();
            (((decibels + FLOOR) / FLOOR).clamp(0., 1.) * 255.).round() as u8
        })
    }
}

/// Fourier's transform, in place, of a power of two's samples.
struct Fft {
    re: Vec<f32>,
    im: Vec<f32>,
    /// e^(-2πik/n), k below n/2.
    turns: Vec<(f32, f32)>,
}

impl Fft {
    fn new(size: usize) -> Self {
        let turn = |k: usize| -std::f32::consts::TAU * k as f32 / size as f32;
        Fft {
            re: vec![0.; size],
            im: vec![0.; size],
            turns: (0..size / 2).map(|k| (turn(k).cos(), turn(k).sin())).collect(),
        }
    }

    fn run(&mut self) {
        let n = self.re.len();
        let (re, im) = (&mut self.re, &mut self.im);
        let mut j = 0;
        for i in 1..n {
            let mut bit = n >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j |= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let (half, stride) = (len / 2, n / len);
            for start in (0..n).step_by(len) {
                for k in 0..half {
                    let (w_re, w_im) = self.turns[k * stride];
                    let (a, b) = (start + k, start + k + half);
                    let (t_re, t_im) = (re[b] * w_re - im[b] * w_im, re[b] * w_im + im[b] * w_re);
                    (re[b], im[b]) = (re[a] - t_re, im[a] - t_im);
                    (re[a], im[a]) = (re[a] + t_re, im[a] + t_im);
                }
            }
            len <<= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tone's 16-bit samples, `seconds` long.
    fn tone(hertz: f32, amplitude: f32, seconds: f32) -> Vec<u8> {
        let count = (RATE as f32 * seconds) as usize;
        let sample = |i: usize| (amplitude * (std::f32::consts::TAU * hertz * i as f32 / RATE as f32).sin() * 32767.) as i16;
        (0..count).flat_map(|i| sample(i).to_le_bytes()).collect()
    }

    fn loudest(levels: &[u8; BANDS]) -> usize {
        (0..BANDS).max_by_key(|band| levels[*band]).unwrap()
    }

    /// The band 1 kHz is in.
    fn band_of(hertz: f32) -> usize {
        (BANDS as f32 * (hertz / LOWEST).ln() / (HIGHEST / LOWEST).ln()) as usize
    }

    #[test]
    fn a_tone_lights_its_band() {
        let mut analysis = Analysis::new();
        let mut steps = Vec::new();
        // In odd pieces, as a pipe gives them.
        for piece in tone(1000., 0.5, 1.).chunks(777) {
            analysis.feed(piece, |step| steps.push(step));
        }
        assert_eq!(steps.len(), (RATE + SIZE / 2 - SIZE) / HOP + 1);
        let middle = &steps[steps.len() / 2];
        assert_eq!(loudest(middle), band_of(1000.));
        // Half of full scale: about 6 dB under the top.
        let top = f32::from(middle[band_of(1000.)]) / 255. * FLOOR - FLOOR;
        assert!((-8.0..-3.0).contains(&top), "{top} dB");
        assert!(middle[band_of(100.)] < 40 && middle[band_of(10_000.)] < 40, "{middle:?}");
        let low = tone(100., 0.5, 0.5);
        let mut analysis = Analysis::new();
        let mut last = None;
        analysis.feed(&low, |step| last = Some(step));
        assert_eq!(loudest(&last.unwrap()), band_of(100.));
    }

    #[test]
    fn bars_fall_slowly_and_peaks_wait() {
        let mut meter = Meter::default();
        let mut loud = [0.; BANDS];
        loud[3] = 1.;
        assert!(meter.update(Some(loud), 0.02));
        assert_eq!((meter.bars[3], meter.peaks[3]), (1., 1.));
        meter.update(None, 0.5);
        assert!((meter.bars[3] - (1. - FALL * 0.5)).abs() < 1e-5);
        assert_eq!(meter.peaks[3], 1., "held");
        meter.update(None, 0.5);
        meter.update(None, 0.5);
        assert_eq!(meter.bars[3], 0.);
        assert!(meter.peaks[3] < 1. && meter.peaks[3] > 0., "falling: {}", meter.peaks[3]);
        for _ in 0..10 {
            meter.update(None, 0.5);
        }
        assert!(!meter.update(None, 0.5), "still");
    }

    /// Through libmpv: the shipped one, `cargo test -p sc-core media -- --ignored`.
    #[test]
    #[ignore]
    fn reads_a_file_through_mpv() {
        let dir = std::env::temp_dir().join(format!("sc-spectrum-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        let samples = tone(1000., 0.5, 3.);
        let mut wav = Vec::new();
        for part in [&b"RIFF"[..], &(36 + samples.len() as u32).to_le_bytes(), b"WAVEfmt ", &16u32.to_le_bytes()] {
            wav.extend_from_slice(part);
        }
        // PCM, mono, 32 kHz, 64000 bytes a second, 2 a sample, 16 bits.
        for value in [1u16, 1] {
            wav.extend_from_slice(&value.to_le_bytes());
        }
        wav.extend_from_slice(&(RATE as u32).to_le_bytes());
        wav.extend_from_slice(&(RATE as u32 * 2).to_le_bytes());
        for value in [2u16, 16] {
            wav.extend_from_slice(&value.to_le_bytes());
        }
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        wav.extend_from_slice(&samples);
        std::fs::write(&path, wav).unwrap();
        let spectrum = Spectrum::open(&path);
        let started = std::time::Instant::now();
        while spectrum.at(2.9).is_none() {
            assert!(started.elapsed().as_secs() < 20, "not read");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(loudest(&spectrum.at(1.5).unwrap().map(|level| (level * 255.) as u8)), band_of(1000.));
        drop(spectrum);
        std::fs::remove_dir_all(&dir).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let ours = format!("sc-spectrum-{}-", std::process::id());
        let mut left = std::fs::read_dir(std::env::temp_dir()).unwrap().flatten();
        assert!(!left.any(|entry| entry.file_name().to_string_lossy().starts_with(&ours)), "the pipe is removed");
    }
}
