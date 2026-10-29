//! Inline images in chat bubbles (1.22.0): received images and image attachments are drawn
//! in the feed, scaled to the bubble width (aspect kept, at most `images.inline_max_rows`
//! rows). Loaded lazily; the `[🖼 …]` placeholder stays until the image is ready and
//! whenever it is only partly on screen (graphics can't be clipped without artifacts).
//! Clicking an image opens the full-size viewer.
use crate::app::{App, Event};
use ratatui::layout::Rect;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use std::collections::HashMap;
use std::sync::Arc;

pub enum State {
    Loading,
    Ready(Arc<image::DynamicImage>),
    /// Failed to load: the text placeholder stays (the viewer shows why).
    Failed,
}

#[derive(Default)]
pub struct Inline {
    pub states: HashMap<String, State>,
    /// Resized protocols per (key, cols, rows).
    pub protos: HashMap<(String, u16, u16), Protocol>,
    /// What was drawn last frame (graphics protocols need a clear when it changes).
    pub shown_sig: u64,
    /// Bumped whenever a state changes (part of the feed cache key).
    pub generation: u64,
}

impl Inline {
    pub fn fail(&mut self, key: &str, _why: String) {
        self.states.insert(key.to_string(), State::Failed);
        self.generation += 1;
    }
    /// Rows and columns an image takes at `max_cols` wide, capped at `max_rows`.
    pub fn size_of(
        &self,
        key: &str,
        max_cols: u16,
        max_rows: u16,
        picker: &Picker,
    ) -> Option<(u16, u16)> {
        let State::Ready(img) = self.states.get(key)? else {
            return None;
        };
        Some(fit(
            img.width(),
            img.height(),
            max_cols,
            max_rows,
            font(picker),
        ))
    }
}

fn font(p: &Picker) -> (u16, u16) {
    let f = p.font_size();
    let (w, h) = (f.width, f.height);
    if w == 0 || h == 0 { (10, 20) } else { (w, h) }
}

/// (cols, rows) for a `w`×`h` px image at most `max_cols` × `max_rows` cells.
pub fn fit(w: u32, h: u32, max_cols: u16, max_rows: u16, font: (u16, u16)) -> (u16, u16) {
    let (fw, fh) = (font.0.max(1) as f64, font.1.max(1) as f64);
    let (w, h) = (w.max(1) as f64, h.max(1) as f64);
    // Natural size in cells; never upscale beyond it.
    let nat_c = (w / fw).ceil();
    let mut cols = nat_c.min(max_cols.max(1) as f64);
    let mut rows = (cols * fw * h / w / fh).ceil();
    if rows > max_rows.max(1) as f64 {
        rows = max_rows.max(1) as f64;
        cols = (rows * fh * w / h / fw).floor().max(1.0);
    }
    (cols as u16, rows.max(1.0) as u16)
}

/// Slot of an inline image in the feed: lines `line..line+rows`, `cols` wide.
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub key: String,
    /// What a click opens in the viewer.
    pub url: String,
    pub line: usize,
    pub rows: u16,
    pub cols: u16,
    /// Right-aligned (your own messages).
    pub right: bool,
    /// Video/GIF/music: a one-row player bar follows the picture rows.
    pub media: Option<crate::media::Kind>,
}

impl App {
    /// Start loading `key` (a URL, or `gbattach:<path>` for an attachment) if not yet.
    pub fn inline_want(&mut self, key: &str) {
        if self.inline.states.contains_key(key) {
            return;
        }
        let url = if let Some(path) = key.strip_prefix(crate::attach::SCHEME) {
            match self.fresh_read(path) {
                Some(r) => r.url.clone(),
                None => {
                    // Resolve the read URL once; the resolved read rebuilds the feed and
                    // this runs again.
                    let marker = format!("{key}#resolving");
                    if let std::collections::hash_map::Entry::Vacant(v) =
                        self.inline.states.entry(marker)
                    {
                        v.insert(State::Loading);
                        let p = path.to_string();
                        self.open_attachment(&p, crate::attach::Then::Inline);
                    }
                    return;
                }
            }
        } else {
            key.to_string()
        };
        self.inline.states.insert(key.to_string(), State::Loading);
        let key = key.to_string();
        self.spawn(async move {
            let result = crate::imageview::load(url).await.map(|(img, _)| img);
            Event::InlineLoaded { key, result }
        });
    }

    pub fn on_inline_loaded(&mut self, key: String, result: Result<image::DynamicImage, String>) {
        self.inline.states.insert(
            key,
            match result {
                Ok(img) => State::Ready(Arc::new(img)),
                Err(_) => State::Failed,
            },
        );
        self.inline.generation += 1;
        self.feed_cache = None;
    }

    /// The one-row player bar under a media slot: [▶] ━━━──── 0:12/1:03.
    fn draw_media_bar(&mut self, buf: &mut ratatui::buffer::Buffer, inner: Rect, s: &Slot, y: u16) {
        use ratatui::widgets::Widget;
        let Some(it) = self.media.items.get(&s.key) else {
            return;
        };
        let w = s.cols.max(24).min(inner.width);
        let x = if s.right {
            inner.x + inner.width.saturating_sub(w)
        } else {
            inner.x
        };
        let (b, bar, l) = crate::media::bar_parts(it, w as usize);
        let bw = b.chars().count() as u16;
        let barw = bar.chars().count() as u16;
        let line = ratatui::text::Line::from(vec![
            crate::theme::download_button().span(b),
            crate::theme::media_bar().span(bar),
            crate::theme::muted().span(l),
        ]);
        let r = Rect::new(x, y, w, 1);
        for xx in r.x..r.x + r.width {
            if let Some(c) = buf.cell_mut((xx, y)) {
                c.set_symbol(" ");
            }
        }
        ratatui::widgets::Paragraph::new(line).render(r, buf);
        self.media.hits.push((
            Rect::new(x, y, bw, 1),
            s.key.clone(),
            crate::media::Part::Button,
        ));
        if barw > 1 {
            self.media.hits.push((
                Rect::new(x + bw + 1, y, barw - 1, 1),
                s.key.clone(),
                crate::media::Part::Bar,
            ));
        }
        self.media.shown.push(s.key.clone());
    }

    /// Draw the visible, fully on-screen inline images and their click areas.
    pub fn draw_inline(
        &mut self,
        buf: &mut ratatui::buffer::Buffer,
        inner: Rect,
        row_of_line: &[usize],
        off: usize,
    ) {
        use ratatui::widgets::Widget;
        let slots = std::mem::take(&mut self.inline_slots);
        let mut sig: u64 = 0;
        self.media.hits.clear();
        self.media.pics.clear();
        self.media.shown.clear();
        for s in &slots {
            let Some(&start) = row_of_line.get(s.line) else {
                continue;
            };
            let y = start as isize - off as isize;
            if s.media.is_some() {
                let by = y + s.rows as isize;
                if by >= 0 && by < inner.height as isize {
                    self.draw_media_bar(buf, inner, s, inner.y + by as u16);
                }
                if s.rows == 0 {
                    continue;
                }
            }
            if y < 0 || y + s.rows as isize > inner.height as isize {
                continue; // partly visible: keep the placeholder text
            }
            let x = if s.right {
                inner.x + inner.width.saturating_sub(s.cols)
            } else {
                inner.x
            };
            let r = Rect::new(x, inner.y + y as u16, s.cols.min(inner.width), s.rows);
            let k = (s.key.clone(), r.width, r.height);
            if !self.inline.protos.contains_key(&k)
                && let Some(State::Ready(img)) = self.inline.states.get(&s.key)
                && let Ok(p) = self.image_picker.new_protocol(
                    (**img).clone(),
                    r.as_size(),
                    ratatui_image::Resize::Fit(Some(ratatui_image::FilterType::Triangle)),
                )
            {
                self.inline.protos.insert(k.clone(), p);
            }
            if let Some(p) = self.inline.protos.get(&k) {
                // Blank the placeholder text under the picture first.
                for yy in r.y..r.y + r.height {
                    for xx in r.x..r.x + r.width {
                        if let Some(c) = buf.cell_mut((xx, yy)) {
                            c.set_symbol(" ");
                        }
                    }
                }
                ratatui_image::Image::new(p).render(r, buf);
                sig = sig
                    .wrapping_mul(31)
                    .wrapping_add((r.x as u64) << 32 | (r.y as u64) << 16 | s.line as u64);
                if s.media.is_some() {
                    self.media.pics.push((r, s.key.clone()));
                    continue;
                }
                for yy in r.y..r.y + r.height {
                    self.link_hits.push(crate::ui::LinkHit {
                        x: r.x,
                        y: yy,
                        w: r.width,
                        url: s.url.clone(),
                        image: true,
                    });
                }
            }
        }
        self.inline_slots = slots;
        self.media_after_draw();
        if sig != self.inline.shown_sig {
            self.inline.shown_sig = sig;
            if self.image_picker.protocol_type() != ratatui_image::picker::ProtocolType::Halfblocks
            {
                self.needs_clear = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_aspect_caps_rows_and_never_upscales() {
        // 400x200 px, 10x20 font: natural 40 cols x 10 rows.
        assert_eq!(fit(400, 200, 60, 12, (10, 20)), (40, 10));
        // Narrow bubble: 20 cols -> 5 rows.
        assert_eq!(fit(400, 200, 20, 12, (10, 20)), (20, 5));
        // Tall image capped at 12 rows: width shrinks to keep the aspect.
        assert_eq!(fit(200, 2000, 60, 12, (10, 20)), (2, 12));
        // Tiny image stays tiny.
        assert_eq!(fit(16, 16, 60, 12, (10, 20)), (2, 1));
    }
}
