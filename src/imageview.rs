//! Image viewer modal: shows one image from the chat as large as the terminal allows
//! (aspect ratio kept) with ratatui-image (kitty / sixel / iTerm2, halfblocks fallback).

use crate::app::{App, Event};
use crate::markdown::{Openable, human_size};
use crate::theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};

/// Largest image we download or read (bytes).
pub const MAX_BYTES: u64 = 25 * 1024 * 1024;

pub enum ViewState {
    Loading,
    Ready {
        proto: Box<StatefulProtocol>,
        /// The pixels, for Fuzzball placement.
        img: std::sync::Arc<image::DynamicImage>,
        w: u32,
        h: u32,
        bytes: u64,
    },
    Failed(String),
}

pub struct ImageView {
    /// Images in the chat, newest first.
    pub items: Vec<Openable>,
    pub idx: usize,
    pub state: ViewState,
    /// The modal's rect last frame (clicks outside close it).
    pub area: Rect,
    /// Matches load results to the image still shown.
    pub load_id: u64,
}

/// The decoded image (or why not), sent back from the loader task.
pub type Loaded = Result<(image::DynamicImage, u64), String>;

static KITTY_COMPRESSION: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Send kitty images zlib-compressed (`o=z`): set at startup when the terminal answered
/// the compression probe ([`crate::probe::KITTY_COMPRESSION_QUERY`]) with OK.
pub fn set_kitty_compression(on: bool) {
    KITTY_COMPRESSION.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn kitty_compression() -> bool {
    KITTY_COMPRESSION.load(std::sync::atomic::Ordering::Relaxed)
}

/// A fresh kitty image id (ratatui-image picks a random one; ids only need to differ).
fn kitty_id() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() ^ (d.as_secs() as u32))
        .unwrap_or(0)
        ^ std::process::id().rotate_left(16);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    seed.wrapping_add(n.wrapping_mul(0x9E37_79B9)) | 1
}

/// `Picker::new_resize_protocol`, but kitty images go zlib-compressed when the terminal
/// can inflate them. ratatui-image only sets that from its own stdio query, which the
/// startup burst replaces, so the kitty protocol is built here with the flag.
pub fn new_resize_protocol(picker: &Picker, image: image::DynamicImage) -> StatefulProtocol {
    resize_protocol_with(picker, image, kitty_compression())
}

fn resize_protocol_with(
    picker: &Picker,
    image: image::DynamicImage,
    compress: bool,
) -> StatefulProtocol {
    if picker.protocol_type() != ProtocolType::Kitty || !compress {
        return picker.new_resize_protocol(image);
    }
    use ratatui_image::protocol::{StatefulProtocolType, kitty::StatefulKitty};
    StatefulProtocol::new(
        image,
        picker.font_size(),
        None,
        StatefulProtocolType::Kitty(StatefulKitty::new(kitty_id(), picker.tmux_detected(), true)),
    )
}

/// `Picker::new_protocol` with the same kitty compression as [`new_resize_protocol`]
/// (the resize step mirrors ratatui-image's, which has no background colour here).
pub fn new_protocol(
    picker: &Picker,
    image: image::DynamicImage,
    size: ratatui::layout::Size,
    resize: Resize,
) -> Result<ratatui_image::protocol::Protocol, ratatui_image::errors::Errors> {
    protocol_with(picker, image, size, resize, kitty_compression())
}

fn protocol_with(
    picker: &Picker,
    image: image::DynamicImage,
    size: ratatui::layout::Size,
    resize: Resize,
    compress: bool,
) -> Result<ratatui_image::protocol::Protocol, ratatui_image::errors::Errors> {
    if picker.protocol_type() != ProtocolType::Kitty || !compress {
        return picker.new_protocol(image, size, resize);
    }
    let fs = picker.font_size();
    let desired = Resize::natural_size(&image, fs);
    let fits = !matches!(resize, Resize::Scale(_))
        && desired.width <= size.width
        && desired.height <= size.height
        && (image.width() == u32::from(desired.width) * u32::from(fs.width)
            || image.height() == u32::from(desired.height) * u32::from(fs.height));
    let (image, area) = if fits {
        (image, desired)
    } else {
        let area = resize.size_for(&image, fs, size);
        (resize.resize(&image, fs, area, None), area)
    };
    Ok(ratatui_image::protocol::Protocol::Kitty(
        ratatui_image::protocol::kitty::Kitty::new(
            image,
            area,
            kitty_id(),
            picker.tmux_detected(),
            true,
        )?,
    ))
}

/// Build the picker from the startup burst's answers ([`crate::probe`]): kitty graphics,
/// else sixel (from DA1), else iTerm2 by environment, else halfblocks. Images need a cell
/// size: from `CSI 16 t`, else the window size in pixels; without one, halfblocks.
pub fn picker_from(a: &crate::probe::Answers) -> Picker {
    use ratatui_image::picker::ProtocolType;
    if !crate::probe::Want::from_env().images {
        return Picker::halfblocks();
    }
    let Some(cell) = a.cell.or_else(window_cell_size) else {
        return Picker::halfblocks();
    };
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    // WezTerm and Konsole: their kitty placeholders / sixel are unreliable (as upstream).
    let blacklisted = env("WEZTERM_EXECUTABLE").is_some() || env("KONSOLE_VERSION").is_some();
    let iterm2 = env("TERM_PROGRAM").is_some_and(|t| {
        [
            "iTerm",
            "WezTerm",
            "mintty",
            "vscode",
            "Tabby",
            "Hyper",
            "rio",
            "Bobcat",
            "WarpTerminal",
        ]
        .iter()
        .any(|n| t.contains(n))
    }) || env("LC_TERMINAL").is_some_and(|t| t.contains("iTerm"));
    #[allow(deprecated)]
    let mut p = Picker::from_fontsize(ratatui_image::FontSize::new(cell.0, cell.1));
    let proto = if a.kitty_graphics && !blacklisted {
        Some(ProtocolType::Kitty)
    } else if a.sixel && !blacklisted {
        Some(ProtocolType::Sixel)
    } else if iterm2 {
        Some(ProtocolType::Iterm2)
    } else {
        None
    };
    if let Some(proto) = proto {
        p.set_protocol_type(proto);
    }
    p
}

/// Cell size from the window size in pixels (`TIOCGWINSZ`), when the terminal reports it.
fn window_cell_size() -> Option<(u16, u16)> {
    #[cfg(unix)]
    {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        // SAFETY: TIOCGWINSZ fills a winsize we own.
        if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) } != 0
            || ws.ws_col == 0
            || ws.ws_row == 0
            || ws.ws_xpixel == 0
            || ws.ws_ypixel == 0
        {
            return None;
        }
        Some((ws.ws_xpixel / ws.ws_col, ws.ws_ypixel / ws.ws_row))
    }
    #[cfg(not(unix))]
    None
}

/// Pick the graphics path per `images.protocol`: Fuzzball (if it answered), then kitty,
/// then native (sixel / iTerm2 / halfblocks). Returns a note when a forced choice wasn't
/// available.
pub fn choose(
    pref: crate::config::ImagesProtocol,
    fuzz: Option<&crate::fuzzball::Caps>,
    detected: ProtocolType,
) -> (bool, ProtocolType, Option<String>) {
    use crate::config::ImagesProtocol as P;
    let fz = fuzz.is_some_and(|c| c.graphics());
    let native = |t| {
        if t == ProtocolType::Kitty {
            ProtocolType::Halfblocks
        } else {
            t
        }
    };
    match pref {
        P::Auto => (fz, detected, None),
        P::Fuzzball if fz => (true, detected, None),
        P::Fuzzball => (
            false,
            detected,
            Some("images.protocol = fuzzball, but the terminal didn't answer the Fuzzball query; using the next protocol.".into()),
        ),
        P::Kitty if detected == ProtocolType::Kitty => (false, detected, None),
        P::Kitty => (
            false,
            detected,
            Some("images.protocol = kitty, but the terminal doesn't speak kitty graphics; using native.".into()),
        ),
        P::Native => (false, native(detected), None),
    }
}

/// Apply [`choose`] to the app (after detection, or when the pref changes).
pub fn apply_images_protocol(app: &mut crate::app::App) {
    let detected = app
        .image_detected
        .get_or_insert(app.image_picker.protocol_type())
        .to_owned();
    let (fz, t, note) = choose(app.config.images_protocol, app.fuzz_caps.as_ref(), detected);
    app.image_picker.set_protocol_type(t);
    app.fuzz = if fz {
        app.fuzz_caps.clone().map(crate::fuzzball::Gfx::new)
    } else {
        None
    };
    app.inline.protos.clear();
    app.needs_clear = true;
    if let Some(n) = note {
        app.info(n);
    }
}

/// Whether the image path leaves pixels a full clear must remove (sixel, iTerm2).
/// kitty uses Unicode placeholders (gone when their cells are redrawn), Fuzzball is
/// diffed per placement, halfblocks are text.
pub fn leaves_ghosts(app: &crate::app::App) -> bool {
    app.fuzz.is_none()
        && matches!(
            app.image_picker.protocol_type(),
            ProtocolType::Sixel | ProtocolType::Iterm2
        )
}

/// The active image path, for the viewer title and /graphics.
pub fn active_name(app: &crate::app::App) -> &'static str {
    if app.fuzz.is_some() {
        "fuzzball"
    } else {
        protocol_name(&app.image_picker)
    }
}

pub fn protocol_name(p: &Picker) -> &'static str {
    match p.protocol_type() {
        ProtocolType::Halfblocks => "halfblocks",
        ProtocolType::Sixel => "sixel",
        ProtocolType::Kitty => "kitty",
        ProtocolType::Iterm2 => "iTerm2",
    }
}

pub fn local_path(url: &str) -> Option<std::path::PathBuf> {
    let u = url.strip_prefix("file://").unwrap_or(url);
    if u.contains("://") {
        return None;
    }
    if let Some(rest) = u.strip_prefix("~/") {
        return dirs::home_dir().map(|h| h.join(rest));
    }
    Some(u.into())
}

/// Read (local) or download (http/https) and decode an image.
pub async fn load(url: String) -> Loaded {
    let bytes: Vec<u8> = if let Some(p) = local_path(&url) {
        let m = std::fs::metadata(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        if m.len() > MAX_BYTES {
            return Err(format!("{} is larger than 25 MB", human_size(m.len())));
        }
        std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?
    } else if url.starts_with("http://") || url.starts_with("https://") {
        let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        if resp.content_length().is_some_and(|n| n > MAX_BYTES) {
            return Err("larger than 25 MB".into());
        }
        let b = resp.bytes().await.map_err(|e| e.to_string())?;
        if b.len() as u64 > MAX_BYTES {
            return Err("larger than 25 MB".into());
        }
        b.to_vec()
    } else {
        return Err("unsupported image location".into());
    };
    let n = bytes.len() as u64;
    tokio::task::spawn_blocking(move || image::load_from_memory(&bytes))
        .await
        .map_err(|e| e.to_string())?
        .map(|img| (img, n))
        .map_err(|e| format!("can't decode: {e}"))
}

impl App {
    /// Images in the active chat's bot replies, newest first.
    pub fn chat_images(&self) -> Vec<Openable> {
        let mut out: Vec<Openable> = vec![];
        for e in self
            .feed()
            .iter()
            .rev()
            .filter(|e| e.kind == crate::app::EntryKind::Bot)
        {
            for o in crate::markdown::openables(&e.text).into_iter().rev() {
                if o.image && !out.iter().any(|x| x.url == o.url) {
                    out.push(o);
                }
            }
        }
        for o in self.attachment_openables() {
            if o.image && !out.iter().any(|x| x.url == o.url) {
                out.push(o);
            }
        }
        out
    }

    /// Preview a local image (a staged attachment) in the viewer.
    pub fn view_local_image(&mut self, path: &str) {
        self.view_image_items(
            vec![Openable {
                url: path.to_string(),
                label: path.to_string(),
                image: true,
            }],
            0,
        );
    }

    /// Open the viewer on arbitrary images (browser `[🖼]` placeholders).
    pub fn view_image_items(&mut self, items: Vec<Openable>, idx: usize) {
        self.image_view = Some(ImageView {
            items,
            idx,
            state: ViewState::Loading,
            area: Rect::default(),
            load_id: 0,
        });
        self.start_image_load();
    }

    /// Open the viewer on `url` (or the newest image). False when the chat has none.
    pub fn view_image(&mut self, url: Option<&str>) -> bool {
        let items = self.chat_images();
        if items.is_empty() {
            return false;
        }
        let idx = url
            .and_then(|u| items.iter().position(|o| o.url == u))
            .unwrap_or(0);
        self.image_view = Some(ImageView {
            items,
            idx,
            state: ViewState::Loading,
            area: Rect::default(),
            load_id: 0,
        });
        self.start_image_load();
        true
    }

    /// Open the image nearest the bottom of the screen, else the newest.
    pub fn view_nearest_image(&mut self) -> bool {
        let url = self
            .link_hits
            .iter()
            .rev()
            .find(|h| h.image)
            .map(|h| h.url.clone());
        self.view_image(url.as_deref())
    }

    fn start_image_load(&mut self) {
        self.image_load_seq += 1;
        let id = self.image_load_seq;
        let Some(v) = self.image_view.as_mut() else {
            return;
        };
        v.load_id = id;
        v.state = ViewState::Loading;
        let url = v.items[v.idx].url.clone();
        self.spawn(async move {
            Event::ImageLoaded {
                id,
                result: load(url).await,
            }
        });
    }

    pub fn on_image_loaded(&mut self, id: u64, result: Loaded) {
        let Some(v) = self.image_view.as_mut() else {
            return;
        };
        if v.load_id != id {
            return;
        }
        v.state = match result {
            Ok((img, bytes)) => ViewState::Ready {
                w: img.width(),
                h: img.height(),
                bytes,
                img: std::sync::Arc::new(img.clone()),
                proto: Box::new(new_resize_protocol(&self.image_picker, img)),
            },
            Err(e) => ViewState::Failed(e),
        };
        self.needs_clear = true;
    }

    /// Step to the next (+1, older) or previous (-1, newer) image.
    pub fn image_step(&mut self, d: i32) {
        let Some(v) = self.image_view.as_mut() else {
            return;
        };
        let n = v.items.len() as i32;
        let next = (v.idx as i32 + d).rem_euclid(n.max(1)) as usize;
        if next != v.idx {
            v.idx = next;
            self.needs_clear = true;
            self.start_image_load();
        }
    }

    pub fn close_image_view(&mut self) {
        if self.image_view.take().is_some() {
            // Graphics live outside the cell grid: wipe them (kitty delete-all, and a full
            // clear + redraw for sixel/iTerm2) so no ghost stays behind.
            self.needs_clear = true;
        }
    }

    pub fn image_open_external(&mut self) {
        let Some(url) = self.image_view.as_ref().map(|v| v.items[v.idx].url.clone()) else {
            return;
        };
        // Presigned URLs show the host only; in SSH this asks first.
        self.open_system(&url);
    }

    /// Mouse click while the viewer is open: outside closes it.
    pub fn image_view_click(&mut self, col: u16, row: u16) {
        if let Some(v) = &self.image_view
            && !v.area.contains(ratatui::layout::Position::new(col, row))
        {
            self.close_image_view();
        }
    }
}

/// Cells for a `w × h` px image scaled (up or down, aspect kept) into `cols × rows`.
pub fn scale_fit(w: u32, h: u32, cols: u16, rows: u16, cell: (u16, u16)) -> (u16, u16) {
    let (cw, ch) = (cell.0.max(1) as f64, cell.1.max(1) as f64);
    let (pw, ph) = (cols as f64 * cw, rows as f64 * ch);
    let s = (pw / w.max(1) as f64).min(ph / h.max(1) as f64);
    let c = ((w as f64 * s / cw).round() as u16).clamp(1, cols.max(1));
    let r = ((h as f64 * s / ch).round() as u16).clamp(1, rows.max(1));
    (c, r)
}

/// Draw the viewer over everything.
pub fn draw(f: &mut Frame, app: &mut App) {
    let screen = f.area();
    let proto = active_name(app);
    let Some(v) = app.image_view.as_mut() else {
        return;
    };
    let area = Rect::new(
        screen.x + 1,
        screen.y + 1,
        screen.width.saturating_sub(2),
        screen.height.saturating_sub(2),
    );
    v.area = area;
    f.render_widget(Clear, area);
    let item = &v.items[v.idx];
    let name = if item.label.trim().is_empty() {
        crate::markdown::file_name(&item.url).to_string()
    } else {
        item.label.trim().to_string()
    };
    let info = match &v.state {
        ViewState::Ready { w, h, bytes, .. } => format!("{w}×{h} · {}", human_size(*bytes)),
        ViewState::Loading => "loading…".into(),
        ViewState::Failed(_) => "failed".into(),
    };
    let block = theme::block(&theme::help_frame())
        .title(theme::help_title().span(format!(" 🖼 {name} ")))
        .title(
            Line::from(theme::image_viewer_info().span(format!(
                " {info} · {}/{} · {proto} ",
                v.idx + 1,
                v.items.len()
            )))
            .right_aligned(),
        )
        .title_bottom(" Esc/q close · o open externally · ←/→ n/p prev/next ");
    let inner = block.inner(area);
    f.render_widget(block, area);
    match &mut v.state {
        ViewState::Ready { img, w, h, .. } if app.fuzz.is_some() => {
            let cell = app
                .fuzz
                .as_ref()
                .map(|g| g.cell(crate::inline::font(&app.image_picker)))
                .unwrap_or((10, 20));
            let (cols, rows) = scale_fit(*w, *h, inner.width, inner.height, cell);
            let r = Rect::new(
                inner.x + inner.width.saturating_sub(cols) / 2,
                inner.y + inner.height.saturating_sub(rows) / 2,
                cols,
                rows,
            );
            crate::fuzzball::reserve(f.buffer_mut(), r);
            let key = format!("viewer:{}", item.url);
            let ident = std::sync::Arc::as_ptr(img) as usize;
            let img = img.clone();
            if let Some(g) = app.fuzz.as_mut() {
                g.show(&key, ident, &img, r, cell);
            }
        }
        ViewState::Ready { proto, .. } => {
            let img = StatefulImage::default()
                .resize(Resize::Scale(Some(ratatui_image::FilterType::Triangle)));
            // Center the fitted image in the modal.
            let fit = proto.size_for(Resize::Scale(None), inner.as_size());
            let r = Rect::new(
                inner.x + inner.width.saturating_sub(fit.width) / 2,
                inner.y + inner.height.saturating_sub(fit.height) / 2,
                fit.width.min(inner.width),
                fit.height.min(inner.height),
            );
            f.render_stateful_widget(img, r, proto.as_mut());
        }
        ViewState::Loading => f.render_widget(
            Paragraph::new(theme::modal_text().span(format!("Loading {}…", item.url))),
            inner,
        ),
        ViewState::Failed(e) => f.render_widget(
            Paragraph::new(theme::modal_text().span(format!(
                "Couldn't show {}: {e}. Press o to open it externally.",
                item.url
            )))
            .wrap(Wrap { trim: false }),
            inner,
        ),
    }
}

/// Kitty: delete every image we placed (APC `a=d,d=A`). Other protocols ignore nothing
/// here; the caller's full clear repaints over sixel / iTerm2 pixels.
pub fn kitty_delete_all(p: &Picker) -> Option<&'static str> {
    (p.protocol_type() == ProtocolType::Kitty).then_some("\x1b_Ga=d,d=A\x1b\\")
}

#[cfg(test)]
mod tests {
    use crate::app::{Entry, EntryKind};

    fn kitty_picker() -> super::Picker {
        #[allow(deprecated)]
        let mut p = super::Picker::from_fontsize(ratatui_image::FontSize::new(10, 20));
        p.set_protocol_type(super::ProtocolType::Kitty);
        p
    }

    fn noisy_image() -> image::DynamicImage {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(37, 53, |x, y| {
            image::Rgb([(x * 7) as u8, (y * 5) as u8, (x ^ y) as u8])
        }))
    }

    /// Everything a widget wrote into the buffer (kitty puts its escapes in cell symbols).
    fn rendered(f: impl FnOnce(ratatui::layout::Rect, &mut ratatui::buffer::Buffer)) -> String {
        let area = ratatui::layout::Rect::new(0, 0, 20, 10);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        f(area, &mut buf);
        buf.content.iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn kitty_images_are_compressed_only_when_the_terminal_can_inflate() {
        use ratatui::widgets::{StatefulWidget, Widget};
        let p = kitty_picker();
        let fit = || ratatui_image::Resize::Fit(None);
        for compress in [true, false] {
            let proto = super::protocol_with(
                &p,
                noisy_image(),
                ratatui::layout::Size::new(20, 10),
                fit(),
                compress,
            )
            .unwrap();
            let out = rendered(|a, b| ratatui_image::Image::new(&proto).render(a, b));
            assert!(out.contains("\x1b_G"), "kitty transmit drawn");
            assert_eq!(out.contains("o=z"), compress, "static, compress={compress}");

            let mut sp = super::resize_protocol_with(&p, noisy_image(), compress);
            let out =
                rendered(|a, b| ratatui_image::StatefulImage::default().render(a, b, &mut sp));
            assert!(out.contains("\x1b_G"), "kitty transmit drawn");
            assert_eq!(
                out.contains("o=z"),
                compress,
                "stateful, compress={compress}"
            );
        }
    }

    #[test]
    fn compressed_static_protocol_sizes_like_ratatui_image() {
        let p = kitty_picker();
        for (size, resize) in [
            ((20, 10), ratatui_image::Resize::Fit(None)),
            ((2, 1), ratatui_image::Resize::Fit(None)),
            ((20, 10), ratatui_image::Resize::Scale(None)),
            ((3, 2), ratatui_image::Resize::Crop(None)),
        ] {
            let size = ratatui::layout::Size::new(size.0, size.1);
            let ours = super::protocol_with(&p, noisy_image(), size, resize.clone(), true).unwrap();
            let theirs = p.new_protocol(noisy_image(), size, resize).unwrap();
            assert_eq!(ours.size(), theirs.size(), "{size:?}");
        }
    }

    #[test]
    fn other_protocols_ignore_the_compression_flag() {
        let mut p = kitty_picker();
        p.set_protocol_type(super::ProtocolType::Halfblocks);
        let proto = super::protocol_with(
            &p,
            noisy_image(),
            ratatui::layout::Size::new(20, 10),
            ratatui_image::Resize::Fit(None),
            true,
        )
        .unwrap();
        assert!(matches!(
            proto,
            ratatui_image::protocol::Protocol::Halfblocks(_)
        ));
    }

    #[test]
    fn only_sixel_and_iterm2_need_a_full_clear() {
        use super::ProtocolType as T;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        for (t, ghosts) in [
            (T::Kitty, false),
            (T::Halfblocks, false),
            (T::Sixel, true),
            (T::Iterm2, true),
        ] {
            app.image_picker.set_protocol_type(t);
            assert_eq!(super::leaves_ghosts(&app), ghosts, "{t:?}");
        }
        app.image_picker.set_protocol_type(T::Sixel);
        app.fuzz = Some(crate::fuzzball::Gfx::new(Default::default()));
        assert!(!super::leaves_ghosts(&app), "Fuzzball is diffed");
    }

    #[test]
    fn protocol_chain_fuzzball_then_kitty_then_native() {
        use super::{ProtocolType as T, choose, scale_fit};
        use crate::config::ImagesProtocol as P;
        let fz = crate::fuzzball::parse_caps(
            b"\x1bP=Fzcr;id=1,v=0,sub=gk\x1b\\",
            crate::fuzzball::Mux::None,
        )
        .unwrap();
        let nog = crate::fuzzball::parse_caps(
            b"\x1bP=Fzcr;id=1,v=0,sub=k\x1b\\",
            crate::fuzzball::Mux::None,
        )
        .unwrap();
        assert!(choose(P::Auto, Some(&fz), T::Kitty).0);
        assert_eq!(choose(P::Auto, None, T::Kitty), (false, T::Kitty, None));
        assert_eq!(
            choose(P::Auto, Some(&nog), T::Sixel),
            (false, T::Sixel, None)
        );
        assert_eq!(
            choose(P::Auto, None, T::Halfblocks),
            (false, T::Halfblocks, None)
        );
        let (f, t, n) = choose(P::Fuzzball, None, T::Kitty);
        assert!(!f && t == T::Kitty && n.is_some());
        assert_eq!(
            choose(P::Kitty, Some(&fz), T::Kitty),
            (false, T::Kitty, None)
        );
        assert!(choose(P::Kitty, None, T::Sixel).2.is_some());
        assert_eq!(
            choose(P::Native, Some(&fz), T::Kitty),
            (false, T::Halfblocks, None)
        );
        assert_eq!(choose(P::Native, None, T::Sixel), (false, T::Sixel, None));
        // Viewer fit scales up and keeps the aspect.
        assert_eq!(scale_fit(100, 50, 40, 40, (10, 20)), (40, 10));
        assert_eq!(scale_fit(100, 400, 40, 10, (10, 20)), (5, 10));
    }

    #[test]
    fn fuzzball_images_are_placed_not_drawn_and_hidden_under_overlays() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        app.fuzz_caps = crate::fuzzball::parse_caps(
            b"\x1bP=Fzcr;id=1,v=0,sub=g,max_chunk=4096\x1b\\\x1bP=Fzgr;id=2,ok=1,cell_w=10,cell_h=20\x1b\\",
            crate::fuzzball::Mux::None,
        );
        super::apply_images_protocol(&mut app);
        assert_eq!(super::active_name(&app), "fuzzball");
        assert!(crate::commands::graphics_report(&app).contains("Images: fuzzball"));
        let img = image::DynamicImage::new_rgba8(40, 20);
        app.image_view = Some(super::ImageView {
            items: vec![crate::markdown::Openable {
                url: "/tmp/x.png".into(),
                label: "x".into(),
                image: true,
            }],
            idx: 0,
            state: super::ViewState::Ready {
                proto: Box::new(super::new_resize_protocol(&app.image_picker, img.clone())),
                img: std::sync::Arc::new(img),
                w: 40,
                h: 20,
                bytes: 10,
            },
            area: Default::default(),
            load_id: 0,
        });
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 30)).unwrap();
        let frame = term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let buf = frame.buffer.clone();
        let g = app.fuzz.as_mut().unwrap();
        assert_eq!(g.frame.len(), 1);
        g.retain_visible(|r| crate::fuzzball::reserved(&buf, r));
        let out = g.flush();
        assert!(out.contains("\x1bP=Fzgu;id=1,fmt=rgba"), "{out:?}");
        assert!(out.contains("\x1bP=Fzgp;id=1,pid=1"), "{out:?}");
        // Something drawn over the picture: not placed (and removed).
        let g = app.fuzz.as_mut().unwrap();
        g.show(
            "k",
            1,
            &image::DynamicImage::new_rgba8(4, 4),
            ratatui::layout::Rect::new(1, 1, 2, 1),
            (10, 20),
        );
        let blank = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 80, 30));
        g.retain_visible(|r| crate::fuzzball::reserved(&blank, r));
        let out = g.flush();
        assert!(
            !out.contains("Fzgp;") && out.contains("Fzgd;what=image,id=1,pid=1"),
            "{out:?}"
        );
    }

    #[test]
    fn viewer_loads_steps_and_closes_with_a_clear() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, d) = crate::app::tests::test_app(&rt);
        crate::app::tests::mock_login(&mut app, "tok");
        let a = d.path().join("a.png");
        image::RgbImage::from_pixel(40, 20, image::Rgb([200, 10, 10]))
            .save(&a)
            .unwrap();
        let bot = app.active_bot().unwrap().id.clone();
        app.logs.insert(
            bot,
            vec![Entry {
                kind: EntryKind::Bot,
                text: format!("![red](/nope/b.png) and ![red box]({})", a.display()),
            }],
        );
        let draw = |app: &mut crate::app::App| {
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
            term.draw(|f| crate::ui::draw(f, app)).unwrap();
            crate::ui::tests::screen_rows(&term).join("\n")
        };
        // Clicking the placeholder opens the viewer on that image.
        let screen = draw(&mut app);
        let h = app
            .link_hits
            .iter()
            .find(|h| h.image && h.url.ends_with("a.png"))
            .cloned()
            .unwrap_or_else(|| panic!("{:?}\n{screen}", app.link_hits));
        app.click(h.x, h.y);
        assert!(app.image_view.is_some());
        let t = std::time::Instant::now();
        while matches!(
            app.image_view.as_ref().unwrap().state,
            super::ViewState::Loading
        ) && t.elapsed().as_secs() < 5
        {
            app.pump();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let s = draw(&mut app);
        assert!(s.contains("🖼 red box"), "{s}");
        assert!(s.contains("40×20 · "), "{s}");
        assert!(s.contains("1/2 · halfblocks"), "{s}");
        // Next: the missing file fails gracefully.
        app.image_step(1);
        let t = std::time::Instant::now();
        while matches!(
            app.image_view.as_ref().unwrap().state,
            super::ViewState::Loading
        ) && t.elapsed().as_secs() < 5
        {
            app.pump();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(draw(&mut app).contains("Couldn't show /nope/b.png"));
        app.needs_clear = false;
        // Click outside closes and asks for a full clear (no image ghosts).
        app.image_view_click(0, 0);
        assert!(app.image_view.is_none() && app.needs_clear);
    }
}
