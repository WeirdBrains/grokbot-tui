//! Full-size player modal: the current frame large, the bar, and keys
//! (Space play/pause, ←/→ seek 5 s, Home start, Esc/q close).
use super::{Cmd, bar_parts};
use crate::app::App;
use crate::theme;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};
use ratatui_image::protocol::Protocol;

pub struct Modal {
    pub key: String,
    pub proto: Option<(Protocol, u16, u16)>,
    pub area: Rect,
    pub bar: Rect,
    pub button: Rect,
}

impl App {
    pub fn open_media_modal(&mut self, key: &str) {
        self.media.modal = Some(Modal {
            key: key.to_string(),
            proto: None,
            area: Rect::default(),
            bar: Rect::default(),
            button: Rect::default(),
        });
        self.needs_clear = true;
    }

    pub fn close_media_modal(&mut self) {
        if self.media.modal.take().is_some() {
            self.needs_clear = true;
        }
    }

    pub fn media_modal_key(&mut self, k: KeyEvent) {
        let Some(key) = self.media.modal.as_ref().map(|m| m.key.clone()) else {
            return;
        };
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => self.close_media_modal(),
            KeyCode::Char(' ') | KeyCode::Char('k') | KeyCode::Enter => {
                self.media_cmd(&key, Cmd::Toggle)
            }
            KeyCode::Left | KeyCode::Char('h') => self.media_cmd(&key, Cmd::SeekBy(-5.0)),
            KeyCode::Right | KeyCode::Char('l') => self.media_cmd(&key, Cmd::SeekBy(5.0)),
            KeyCode::Home | KeyCode::Char('0') => self.media_cmd(&key, Cmd::SeekFrac(0.0)),
            _ => {}
        }
    }

    /// Mouse click inside the modal (outside closes it).
    pub fn media_modal_click(&mut self, col: u16, row: u16) {
        let at = ratatui::layout::Position::new(col, row);
        let Some(m) = self.media.modal.as_ref() else {
            return;
        };
        let key = m.key.clone();
        if m.button.contains(at) {
            self.media_cmd(&key, Cmd::Toggle);
        } else if m.bar.contains(at) {
            let f = super::frac_at(m.bar.x, m.bar.width, col);
            self.media_cmd(&key, Cmd::SeekFrac(f));
        } else if !m.area.contains(at) {
            self.close_media_modal();
        } else {
            self.media_cmd(&key, Cmd::Toggle);
        }
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let screen = f.area();
    let Some(key) = app.media.modal.as_ref().map(|m| m.key.clone()) else {
        return;
    };
    let area = Rect::new(
        screen.x + 1,
        screen.y + 1,
        screen.width.saturating_sub(2),
        screen.height.saturating_sub(2),
    );
    f.render_widget(Clear, area);
    let (label, decoder, audio) = app
        .media
        .items
        .get(&key)
        .map(|i| (i.label.clone(), i.decoder, i.audio))
        .unwrap_or_default();
    let block = theme::block(&theme::help_frame())
        .title(theme::help_title().span(format!(" ▶ {label} ")))
        .title(
            Line::from(theme::image_viewer_info().span(format!(
                " {decoder}{} ",
                if audio { " · sound" } else { "" }
            )))
            .right_aligned(),
        )
        .title_bottom(
            " Space play/pause · ←/→ seek 5s · Home start · click bar to seek · Esc/q close ",
        );
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 3 {
        return;
    }
    let pic = Rect {
        height: inner.height - 1,
        ..inner
    };
    let bar_row = Rect {
        y: inner.y + inner.height - 1,
        height: 1,
        ..inner
    };
    // Picture.
    let img = match app.inline.states.get(&key) {
        Some(crate::inline::State::Ready(i)) => Some(i.clone()),
        _ => None,
    };
    let picker = app.image_picker.clone();
    let m = app.media.modal.as_mut().expect("checked");
    m.area = area;
    if let Some(img) = img {
        let (cols, rows) = crate::inline::fit(img.width(), img.height(), pic.width, pic.height, {
            let fs = picker.font_size();
            (fs.width.max(1), fs.height.max(1))
        });
        let r = Rect::new(
            pic.x + (pic.width - cols) / 2,
            pic.y + (pic.height - rows) / 2,
            cols,
            rows,
        );
        if m.proto
            .as_ref()
            .is_none_or(|(_, c, rr)| (*c, *rr) != (cols, rows))
        {
            m.proto = picker
                .new_protocol(
                    (*img).clone(),
                    r.as_size(),
                    ratatui_image::Resize::Fit(Some(ratatui_image::FilterType::Triangle)),
                )
                .ok()
                .map(|p| (p, cols, rows));
        }
        if let Some((p, _, _)) = &m.proto {
            f.render_widget(ratatui_image::Image::new(p), r);
        }
    } else {
        f.render_widget(
            Paragraph::new(theme::muted().span(if label.is_empty() {
                "loading…"
            } else {
                "♪"
            })),
            pic,
        );
    }
    // Bar.
    if let Some(it) = app.media.items.get(&key) {
        let (b, bar, l) = bar_parts(it, bar_row.width as usize);
        let bw = b.chars().count() as u16;
        let barw = bar.chars().count() as u16;
        let line = Line::from(vec![
            theme::download_button().span(b),
            theme::media_bar().span(bar),
            theme::muted().span(l),
        ]);
        f.render_widget(Paragraph::new(line), bar_row);
        let m = app.media.modal.as_mut().expect("checked");
        m.button = Rect {
            width: bw,
            ..bar_row
        };
        m.bar = Rect {
            x: bar_row.x + bw + 1,
            width: barw.saturating_sub(1),
            ..bar_row
        };
    }
}
