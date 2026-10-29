//! Frame sources: animated GIFs (image crate), H.264 MP4 (bundled OpenH264), the
//! linked FFmpeg (`-ffmpeg` builds) and a system `ffmpeg` process (fallback).
use image::{DynamicImage, RgbaImage};
use std::time::Duration;

/// Longest edge frames are scaled to before display (terminal cells are coarse).
pub const MAX_EDGE: u32 = 640;

pub struct Frame {
    pub pts: Duration,
    pub img: DynamicImage,
}

pub trait FrameSource: Send {
    /// Total length, if known.
    fn duration(&self) -> Option<Duration>;
    /// Frame count for GIFs (their bar shows frame position).
    fn frames(&self) -> Option<usize> {
        None
    }
    /// The next frame in presentation order; None at the end.
    fn next(&mut self) -> Option<Frame>;
    /// Jump so the next frame is at or just after `t`.
    fn seek(&mut self, t: Duration);
    /// Which decoder this is (for /status and the modal).
    fn name(&self) -> &'static str;
}

pub fn shrink(img: RgbaImage) -> DynamicImage {
    let (w, h) = img.dimensions();
    let d = DynamicImage::ImageRgba8(img);
    if w.max(h) <= MAX_EDGE {
        return d;
    }
    d.resize(MAX_EDGE, MAX_EDGE, image::imageops::FilterType::Triangle)
}

// ---------- GIF ----------

pub struct Gif {
    frames: Vec<(Duration, DynamicImage)>,
    total: Duration,
    at: usize,
}

/// GIFs longer than this many frames are cut (memory).
pub const GIF_MAX_FRAMES: usize = 1000;

impl Gif {
    pub fn open(bytes: &[u8]) -> Result<Self, String> {
        use image::AnimationDecoder;
        let d = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes))
            .map_err(|e| format!("gif: {e}"))?;
        let mut frames = vec![];
        let mut t = Duration::ZERO;
        for f in d.into_frames().take(GIF_MAX_FRAMES) {
            let f = f.map_err(|e| format!("gif: {e}"))?;
            let (n, dd) = f.delay().numer_denom_ms();
            let mut delay = Duration::from_millis((n as u64) / (dd.max(1) as u64));
            if delay < Duration::from_millis(20) {
                delay = Duration::from_millis(100); // browsers' rule for 0/1-tick delays
            }
            frames.push((t, shrink(f.into_buffer())));
            t += delay;
        }
        if frames.is_empty() {
            return Err("gif has no frames".into());
        }
        Ok(Gif {
            frames,
            total: t,
            at: 0,
        })
    }
}

impl FrameSource for Gif {
    fn duration(&self) -> Option<Duration> {
        Some(self.total)
    }
    fn frames(&self) -> Option<usize> {
        Some(self.frames.len())
    }
    fn next(&mut self) -> Option<Frame> {
        let (pts, img) = self.frames.get(self.at)?.clone();
        self.at += 1;
        Some(Frame { pts, img })
    }
    fn seek(&mut self, t: Duration) {
        self.at = self.frames.iter().rposition(|(p, _)| *p <= t).unwrap_or(0);
    }
    fn name(&self) -> &'static str {
        "gif"
    }
}

// ---------- H.264 in MP4 (OpenH264, compiled in) ----------

pub struct H264 {
    mp4: mp4::Mp4Reader<std::io::Cursor<ArcBytes>>,
    track: u32,
    timescale: u32,
    count: u32,
    next_sample: u32,
    decoder: openh264::decoder::Decoder,
    sps_pps: Vec<u8>,
    nal_len: usize,
    duration: Duration,
    /// Frames decoded but not yet returned (decoder output may lag).
    skip_before: Duration,
    w: usize,
    h: usize,
}

pub struct ArcBytes(pub std::sync::Arc<Vec<u8>>);

impl AsRef<[u8]> for ArcBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

pub fn is_mp4(bytes: &[u8]) -> bool {
    bytes.len() > 12 && &bytes[4..8] == b"ftyp"
}

impl H264 {
    pub fn open(bytes: std::sync::Arc<Vec<u8>>) -> Result<Self, String> {
        let size = bytes.len() as u64;
        let mp4 = mp4::Mp4Reader::read_header(std::io::Cursor::new(ArcBytes(bytes)), size)
            .map_err(|e| format!("mp4: {e}"))?;
        let (id, t) = mp4
            .tracks()
            .iter()
            .find(|(_, t)| matches!(t.media_type(), Ok(mp4::MediaType::H264)))
            .ok_or("no H.264 video track")?;
        let track = *id;
        let mut sps_pps = vec![];
        for ps in [t.sequence_parameter_set(), t.picture_parameter_set()] {
            let ps = ps.map_err(|e| format!("mp4: {e}"))?;
            sps_pps.extend_from_slice(&[0, 0, 0, 1]);
            sps_pps.extend_from_slice(ps);
        }
        let nal_len = t
            .trak
            .mdia
            .minf
            .stbl
            .stsd
            .avc1
            .as_ref()
            .map(|a| (a.avcc.length_size_minus_one & 3) as usize + 1)
            .unwrap_or(4);
        let decoder = openh264::decoder::Decoder::new().map_err(|e| format!("openh264: {e}"))?;
        Ok(H264 {
            timescale: t.timescale().max(1),
            count: t.sample_count(),
            duration: t.duration(),
            w: t.width() as usize,
            h: t.height() as usize,
            mp4,
            track,
            next_sample: 1,
            decoder,
            sps_pps,
            nal_len,
            skip_before: Duration::ZERO,
        })
    }
}

/// AVCC (length-prefixed) → Annex B (start codes).
pub fn annexb(nal_len: usize, data: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i + nal_len <= data.len() {
        let mut n = 0usize;
        for b in &data[i..i + nal_len] {
            n = (n << 8) | *b as usize;
        }
        i += nal_len;
        let end = (i + n).min(data.len());
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&data[i..end]);
        i = end;
    }
}

impl FrameSource for H264 {
    fn duration(&self) -> Option<Duration> {
        Some(self.duration)
    }
    fn next(&mut self) -> Option<Frame> {
        use openh264::formats::YUVSource;
        let mut buf = vec![];
        while self.next_sample <= self.count {
            let id = self.next_sample;
            self.next_sample += 1;
            let Ok(Some(s)) = self.mp4.read_sample(self.track, id) else {
                continue;
            };
            buf.clear();
            if s.is_sync {
                buf.extend_from_slice(&self.sps_pps);
            }
            annexb(self.nal_len, &s.bytes, &mut buf);
            let pts = Duration::from_secs_f64(
                (s.start_time as i64 + s.rendering_offset as i64).max(0) as f64
                    / self.timescale as f64,
            );
            if let Ok(Some(yuv)) = self.decoder.decode(&buf) {
                if pts < self.skip_before {
                    continue; // decoding up to the seek point
                }
                let (w, h) = yuv.dimensions();
                let (w, h) = if w * h == 0 { (self.w, self.h) } else { (w, h) };
                let mut rgba = vec![0u8; w * h * 4];
                yuv.write_rgba8(&mut rgba);
                let img = RgbaImage::from_raw(w as u32, h as u32, rgba)?;
                return Some(Frame {
                    pts,
                    img: shrink(img),
                });
            }
        }
        None
    }
    fn seek(&mut self, t: Duration) {
        // Restart at the last sync sample at or before t, then decode forward.
        let target = (t.as_secs_f64() * self.timescale as f64) as u64;
        let mut sync = 1;
        for id in 1..=self.count {
            match self.mp4.read_sample(self.track, id) {
                Ok(Some(s)) if s.start_time > target => break,
                Ok(Some(s)) if s.is_sync => sync = id,
                _ => {}
            }
        }
        self.next_sample = sync;
        self.skip_before = t;
        if let Ok(d) = openh264::decoder::Decoder::new() {
            self.decoder = d;
        }
    }
    fn name(&self) -> &'static str {
        "openh264"
    }
}

// ---------- system ffmpeg (fallback for other codecs) ----------

/// `ffmpeg` on PATH, if any.
pub fn system_ffmpeg() -> Option<std::path::PathBuf> {
    let exe = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join(exe))
            .find(|f| f.is_file())
    })
}

pub struct SystemFfmpeg {
    exe: std::path::PathBuf,
    file: std::path::PathBuf,
    fps: u32,
    w: u32,
    h: u32,
    duration: Option<Duration>,
    child: Option<std::process::Child>,
    start: Duration,
    n: u64,
}

impl SystemFfmpeg {
    /// `file` is a local copy of the media (ffmpeg seeks it with -ss).
    pub fn open(
        exe: std::path::PathBuf,
        file: std::path::PathBuf,
        fps: u32,
    ) -> Result<Self, String> {
        let out = std::process::Command::new(&exe)
            .args(["-hide_banner", "-i"])
            .arg(&file)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("ffmpeg: {e}"))?;
        let info = String::from_utf8_lossy(&out.stderr).to_string();
        let (w, h) = probe_size(&info).ok_or("ffmpeg found no video stream")?;
        let (w, h) = scaled(w, h);
        Ok(SystemFfmpeg {
            exe,
            file,
            fps: fps.max(1),
            w,
            h,
            duration: probe_duration(&info),
            child: None,
            start: Duration::ZERO,
            n: 0,
        })
    }

    fn spawn(&mut self) -> Option<()> {
        if self.child.is_none() {
            let c = std::process::Command::new(&self.exe)
                .args(["-hide_banner", "-loglevel", "error", "-ss"])
                .arg(format!("{:.3}", self.start.as_secs_f64()))
                .arg("-i")
                .arg(&self.file)
                .args(["-an", "-vf"])
                .arg(format!("fps={},scale={}:{}", self.fps, self.w, self.h))
                .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok()?;
            self.child = Some(c);
            self.n = 0;
        }
        Some(())
    }
}

/// Fit within MAX_EDGE with even dimensions.
pub fn scaled(w: u32, h: u32) -> (u32, u32) {
    let s = (MAX_EDGE as f64 / w.max(h).max(1) as f64).min(1.0);
    let e = |v: u32| (((v as f64 * s) as u32) & !1).max(2);
    (e(w), e(h))
}

/// "Stream #0:0...: Video: h264 ..., 1920x1080 ..." → (1920, 1080).
pub fn probe_size(info: &str) -> Option<(u32, u32)> {
    let line = info.lines().find(|l| l.contains("Video:"))?;
    line.split([',', ' '])
        .filter_map(|t| {
            let (a, b) = t.split_once('x')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        })
        .find(|(w, h): &(u32, u32)| *w > 0 && *h > 0)
}

/// "Duration: 00:01:02.50," → 62.5 s.
pub fn probe_duration(info: &str) -> Option<Duration> {
    let i = info.find("Duration: ")? + 10;
    let t = info[i..].split(',').next()?;
    let mut secs = 0.0;
    for p in t.trim().split(':') {
        secs = secs * 60.0 + p.parse::<f64>().ok()?;
    }
    Some(Duration::from_secs_f64(secs))
}

impl FrameSource for SystemFfmpeg {
    fn duration(&self) -> Option<Duration> {
        self.duration
    }
    fn next(&mut self) -> Option<Frame> {
        use std::io::Read;
        self.spawn()?;
        let mut buf = vec![0u8; (self.w * self.h * 4) as usize];
        let out = self.child.as_mut()?.stdout.as_mut()?;
        if out.read_exact(&mut buf).is_err() {
            if let Some(mut c) = self.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            return None;
        }
        let pts = self.start + Duration::from_secs_f64(self.n as f64 / self.fps as f64);
        self.n += 1;
        Some(Frame {
            pts,
            img: DynamicImage::ImageRgba8(RgbaImage::from_raw(self.w, self.h, buf)?),
        })
    }
    fn seek(&mut self, t: Duration) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.start = t;
    }
    fn name(&self) -> &'static str {
        "system ffmpeg"
    }
}

impl Drop for SystemFfmpeg {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

// ---------- linked FFmpeg (the -ffmpeg builds) ----------

#[cfg(feature = "ffmpeg")]
pub struct LibFfmpeg {
    input: ffmpeg_next::format::context::Input,
    stream: usize,
    decoder: ffmpeg_next::decoder::Video,
    scaler: ffmpeg_next::software::scaling::Context,
    tb: f64,
    duration: Option<Duration>,
    eof: bool,
    skip_before: Duration,
}

// SAFETY: a LibFfmpeg is owned and used by one thread at a time (built on a blocking
// task, then moved into its player thread). FFmpeg's contexts may move between threads;
// only concurrent use is unsafe, and nothing here is shared.
#[cfg(feature = "ffmpeg")]
unsafe impl Send for LibFfmpeg {}

#[cfg(feature = "ffmpeg")]
impl LibFfmpeg {
    pub fn open(file: &std::path::Path) -> Result<Self, String> {
        use ffmpeg_next as ff;
        ff::init().map_err(|e| e.to_string())?;
        ff::util::log::set_level(ff::util::log::Level::Quiet);
        let input = ff::format::input(file).map_err(|e| e.to_string())?;
        let st = input
            .streams()
            .best(ff::media::Type::Video)
            .ok_or("no video stream")?;
        let stream = st.index();
        let tb = f64::from(st.time_base());
        let ctx = ff::codec::context::Context::from_parameters(st.parameters())
            .map_err(|e| e.to_string())?;
        let decoder = ctx.decoder().video().map_err(|e| e.to_string())?;
        let (w, h) = scaled(decoder.width(), decoder.height());
        let scaler = ff::software::scaling::Context::get(
            decoder.format(),
            decoder.width(),
            decoder.height(),
            ff::format::Pixel::RGBA,
            w,
            h,
            ff::software::scaling::Flags::BILINEAR,
        )
        .map_err(|e| e.to_string())?;
        let d = input.duration();
        Ok(LibFfmpeg {
            duration: (d > 0).then(|| Duration::from_micros(d as u64)),
            input,
            stream,
            decoder,
            scaler,
            tb,
            eof: false,
            skip_before: Duration::ZERO,
        })
    }
}

#[cfg(feature = "ffmpeg")]
impl FrameSource for LibFfmpeg {
    fn duration(&self) -> Option<Duration> {
        self.duration
    }
    fn next(&mut self) -> Option<Frame> {
        use ffmpeg_next as ff;
        let mut frame = ff::frame::Video::empty();
        loop {
            if self.decoder.receive_frame(&mut frame).is_ok() {
                let pts = Duration::from_secs_f64(
                    (frame.timestamp().unwrap_or(0).max(0) as f64) * self.tb,
                );
                if pts < self.skip_before {
                    continue;
                }
                let mut rgba = ff::frame::Video::empty();
                self.scaler.run(&frame, &mut rgba).ok()?;
                let (w, h) = (rgba.width(), rgba.height());
                let stride = rgba.stride(0);
                let data = rgba.data(0);
                let mut buf = Vec::with_capacity((w * h * 4) as usize);
                for y in 0..h as usize {
                    buf.extend_from_slice(&data[y * stride..y * stride + w as usize * 4]);
                }
                return Some(Frame {
                    pts,
                    img: DynamicImage::ImageRgba8(RgbaImage::from_raw(w, h, buf)?),
                });
            }
            if self.eof {
                return None;
            }
            let mut sent = false;
            for (s, p) in self.input.packets() {
                if s.index() == self.stream {
                    let _ = self.decoder.send_packet(&p);
                    sent = true;
                    break;
                }
            }
            if !sent {
                self.eof = true;
                let _ = self.decoder.send_eof();
            }
        }
    }
    fn seek(&mut self, t: Duration) {
        let ts = (t.as_micros() as i64).max(0);
        let _ = self.input.seek(ts, ..ts);
        self.decoder.flush();
        self.eof = false;
        self.skip_before = t;
    }
    fn name(&self) -> &'static str {
        "ffmpeg (linked)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_probe_parsing() {
        let info = "  Duration: 00:01:02.50, start: 0.000000, bitrate: 1 kb/s\n    Stream #0:0(und): Video: vp9 (Profile 0), yuv420p(tv), 1920x1080, SAR 1:1 DAR 16:9, 30 fps\n";
        assert_eq!(probe_duration(info), Some(Duration::from_millis(62_500)));
        assert_eq!(probe_size(info), Some((1920, 1080)));
        assert_eq!(scaled(1920, 1080), (640, 360));
        assert_eq!(scaled(320, 240), (320, 240));
    }

    #[test]
    fn gif_frames_and_seek() {
        use image::codecs::gif::{GifEncoder, Repeat};
        use image::{Delay, Frame as IFrame, Rgba};
        let mut out = vec![];
        {
            let mut e = GifEncoder::new(&mut out);
            e.set_repeat(Repeat::Infinite).unwrap();
            for i in 0..3u8 {
                let img = RgbaImage::from_pixel(4, 4, Rgba([i * 80, 0, 0, 255]));
                e.encode_frame(IFrame::from_parts(
                    img,
                    0,
                    0,
                    Delay::from_numer_denom_ms(200, 1),
                ))
                .unwrap();
            }
        }
        let mut g = Gif::open(&out).unwrap();
        assert_eq!(g.frames(), Some(3));
        assert_eq!(g.duration(), Some(Duration::from_millis(600)));
        assert_eq!(g.next().unwrap().pts, Duration::ZERO);
        g.seek(Duration::from_millis(450));
        assert_eq!(g.next().unwrap().pts, Duration::from_millis(400));
        assert!(g.next().is_none());
    }

    #[test]
    fn annexb_conversion() {
        let mut h = vec![];
        annexb(4, &[0, 0, 0, 2, 0x65, 1, 0, 0, 0, 1, 0x41], &mut h);
        assert_eq!(h, [0, 0, 0, 1, 0x65, 1, 0, 0, 0, 1, 0x41]);
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn openh264_decodes_the_mp4_fixture_and_seeks() {
        let b = fixture("tiny.mp4");
        assert!(is_mp4(&b));
        let mut v = H264::open(std::sync::Arc::new(b)).unwrap();
        let d = v.duration().unwrap();
        assert!(
            d >= Duration::from_millis(900) && d <= Duration::from_millis(1100),
            "{d:?}"
        );
        let f = v.next().unwrap();
        assert_eq!((f.img.width(), f.img.height()), (64, 48));
        let mut n = 1;
        while v.next().is_some() {
            n += 1;
        }
        assert!((9..=11).contains(&n), "{n} frames");
        v.seek(Duration::from_millis(500));
        let f = v.next().unwrap();
        assert!(f.pts >= Duration::from_millis(500), "{:?}", f.pts);
    }

    #[test]
    fn system_ffmpeg_plays_other_codecs_when_installed() {
        let Some(exe) = system_ffmpeg() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.webm");
        std::fs::write(&p, fixture("tiny.webm")).unwrap();
        let mut v = SystemFfmpeg::open(exe, p, 10).unwrap();
        let f = v.next().unwrap();
        assert_eq!((f.img.width(), f.img.height()), (64, 48));
        v.seek(Duration::from_millis(500));
        assert!(v.next().unwrap().pts >= Duration::from_millis(500));
    }

    #[cfg(feature = "ffmpeg")]
    #[test]
    fn linked_ffmpeg_decodes_vp9_webm_and_seeks() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.webm");
        std::fs::write(&p, fixture("tiny.webm")).unwrap();
        let mut v = LibFfmpeg::open(&p).unwrap();
        let f = v.next().unwrap();
        assert_eq!((f.img.width(), f.img.height()), (64, 48));
        v.seek(Duration::from_millis(500));
        assert!(v.next().unwrap().pts >= Duration::from_millis(500));
    }
}
