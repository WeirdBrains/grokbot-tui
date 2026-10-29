//! A playback thread per media item: paces frames from a [`FrameSource`] against a clock
//! (the audio position when there is sound), plays audio with rodio, and reports frames
//! and progress to the app as events.
use super::source::{Frame, FrameSource};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmd {
    Play,
    Pause,
    Toggle,
    /// Seek to a fraction (0..=1) of the length.
    SeekFrac(f64),
    /// Seek by a signed number of seconds.
    SeekBy(f64),
    Stop,
}

#[derive(Debug, Clone)]
pub enum Out {
    Frame(std::sync::Arc<image::DynamicImage>),
    Progress {
        pos: Duration,
        dur: Option<Duration>,
        playing: bool,
        /// GIFs: (frame index, frame count).
        frame: Option<(usize, usize)>,
    },
    Info {
        decoder: &'static str,
        audio: bool,
    },
    Error(String),
}

pub struct Spec {
    pub video: Option<Box<dyn FrameSource>>,
    pub audio: Option<std::sync::Arc<Vec<u8>>>,
    pub fps: u32,
    pub looping: bool,
    pub autoplay: bool,
    pub volume: f32,
    /// Removed when the thread ends (temp copy for ffmpeg).
    pub temp: Option<std::path::PathBuf>,
}

struct Audio {
    _sink: rodio::MixerDeviceSink,
    player: rodio::Player,
}

fn open_audio(bytes: std::sync::Arc<Vec<u8>>, volume: f32) -> Result<Audio, String> {
    let dec = rodio::Decoder::new(std::io::Cursor::new(super::source::ArcBytes(bytes)))
        .map_err(|e| format!("audio: {e}"))?;
    // Quiet error callback and no drop message: stderr would scribble over the TUI.
    let quiet: fn(rodio::cpal::Error) = |_| {};
    let mut sink = rodio::DeviceSinkBuilder::from_default_device()
        .map_err(|e| format!("audio device: {e}"))?
        .with_error_callback(quiet)
        .open_sink_or_fallback()
        .map_err(|e| format!("audio device: {e}"))?;
    sink.log_on_drop(false);
    let player = rodio::Player::connect_new(sink.mixer());
    player.pause();
    player.set_volume(volume);
    player.append(dec);
    Ok(Audio {
        _sink: sink,
        player,
    })
}

/// Run until `Stop` or the app goes away. `send` returns false once the app is gone.
pub fn run(mut spec: Spec, rx: Receiver<Cmd>, send: impl Fn(Out) -> bool) {
    // Sound opens on the first Play (no audio stream per item just for showing it).
    let mut audio_bytes = spec.audio.take();
    let probe_dur = audio_bytes.as_ref().and_then(|b| {
        use rodio::Source;
        match rodio::Decoder::new(std::io::Cursor::new(super::source::ArcBytes(b.clone()))) {
            Ok(d) => Some(d.total_duration()),
            Err(e) => {
                if spec.video.is_none() {
                    send(Out::Error(format!("audio: {e}")));
                }
                None
            }
        }
    });
    if spec.video.is_none() && probe_dur.is_none() {
        return;
    }
    if probe_dur.is_none() {
        audio_bytes = None; // no decodable sound track: silent video
    }
    let mut audio: Option<Audio> = None;
    let mut video = spec.video.take();
    let frames_total = video.as_ref().and_then(|v| v.frames());
    let dur = video
        .as_ref()
        .and_then(|v| v.duration())
        .or(probe_dur.flatten());
    send(Out::Info {
        decoder: video.as_ref().map(|v| v.name()).unwrap_or("rodio"),
        audio: audio_bytes.is_some(),
    });
    let mut frame_idx = 0usize;
    let mut ahead: Option<Frame> = None;
    // Poster frame.
    if let Some(v) = video.as_mut()
        && let Some(f) = v.next()
        && !send(Out::Frame(std::sync::Arc::new(f.img)))
    {
        return;
    }
    let mut playing = false;
    let mut base = Duration::ZERO; // position at `since` (no-audio clock)
    let mut since = Instant::now();
    let interval = Duration::from_millis(1000 / spec.fps.clamp(1, 60) as u64);
    let mut last_progress = Instant::now() - Duration::from_secs(1);
    let clock = |playing: bool, base: Duration, since: Instant, audio: &Option<Audio>| {
        if let Some(a) = audio {
            a.player.get_pos()
        } else if playing {
            base + since.elapsed()
        } else {
            base
        }
    };
    let mut want_play = spec.autoplay;
    loop {
        let cmd = if want_play {
            want_play = false;
            Some(Cmd::Play)
        } else {
            match rx.recv_timeout(if playing {
                interval
            } else {
                Duration::from_millis(250)
            }) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        };
        let mut seek_to: Option<Duration> = None;
        let pos_now = clock(playing, base, since, &audio);
        match cmd {
            Some(Cmd::Stop) => break,
            Some(Cmd::Play) | Some(Cmd::Toggle) if !playing => {
                if dur.is_some_and(|d| pos_now >= d) {
                    seek_to = Some(Duration::ZERO);
                }
                playing = true;
                base = seek_to.unwrap_or(pos_now);
                since = Instant::now();
                if audio.is_none()
                    && let Some(b) = audio_bytes.take()
                {
                    match open_audio(b, spec.volume) {
                        Ok(a) => {
                            if !base.is_zero() {
                                let _ = a.player.try_seek(base);
                            }
                            audio = Some(a);
                        }
                        Err(e) if video.is_none() => {
                            send(Out::Error(e));
                            break;
                        }
                        Err(_) => {} // silent video
                    }
                }
                if let Some(a) = &audio {
                    a.player.play();
                }
            }
            Some(Cmd::Pause) | Some(Cmd::Toggle) if playing => {
                base = pos_now;
                playing = false;
                if let Some(a) = &audio {
                    a.player.pause();
                }
            }
            Some(Cmd::SeekFrac(f)) => {
                if let Some(d) = dur {
                    seek_to = Some(d.mul_f64(f.clamp(0.0, 1.0)));
                }
            }
            Some(Cmd::SeekBy(s)) => {
                let t = (pos_now.as_secs_f64() + s).max(0.0);
                let t = dur.map(|d| t.min(d.as_secs_f64())).unwrap_or(t);
                seek_to = Some(Duration::from_secs_f64(t));
            }
            _ => {}
        }
        if let Some(t) = seek_to {
            if let Some(a) = &audio {
                let _ = a.player.try_seek(t);
            }
            base = t;
            since = Instant::now();
            ahead = None;
            if let Some(v) = video.as_mut() {
                v.seek(t);
                // Show the frame at the new position even when paused.
                if let Some(f) = v.next() {
                    frame_idx = frames_total.map(|n| index_at(t, dur, n)).unwrap_or(0);
                    if !send(Out::Frame(std::sync::Arc::new(f.img))) {
                        return;
                    }
                }
            }
        }
        let mut pos = clock(playing, base, since, &audio);
        let mut ended = false;
        if playing {
            if let Some(v) = video.as_mut() {
                let mut show: Option<Frame> = None;
                loop {
                    let f = match ahead.take() {
                        Some(f) => f,
                        None => match v.next() {
                            Some(f) => f,
                            None => {
                                ended = true;
                                break;
                            }
                        },
                    };
                    if f.pts <= pos {
                        frame_idx += 1;
                        show = Some(f);
                    } else {
                        ahead = Some(f);
                        break;
                    }
                }
                if let Some(f) = show
                    && !send(Out::Frame(std::sync::Arc::new(f.img)))
                {
                    return;
                }
            } else if audio.as_ref().is_some_and(|a| a.player.empty()) {
                ended = true;
            }
            if ended && spec.looping {
                if let Some(v) = video.as_mut() {
                    v.seek(Duration::ZERO);
                }
                base = Duration::ZERO;
                since = Instant::now();
                frame_idx = 0;
                ended = false;
                pos = Duration::ZERO;
            } else if ended {
                playing = false;
                base = dur.unwrap_or(pos);
                pos = base;
                if let Some(a) = &audio {
                    a.player.pause();
                }
            }
        }
        if cmd.is_some() || ended || last_progress.elapsed() >= Duration::from_millis(250) {
            last_progress = Instant::now();
            let frame = frames_total.map(|n| (frame_idx.min(n.saturating_sub(1)), n));
            if !send(Out::Progress {
                pos,
                dur,
                playing,
                frame,
            }) {
                break;
            }
        }
    }
    if let Some(a) = audio.take() {
        a.player.stop();
    }
    if let Some(t) = spec.temp.take() {
        let _ = std::fs::remove_file(t);
    }
}

fn index_at(t: Duration, dur: Option<Duration>, n: usize) -> usize {
    match dur {
        Some(d) if !d.is_zero() => ((t.as_secs_f64() / d.as_secs_f64()) * n as f64) as usize,
        _ => 0,
    }
}
