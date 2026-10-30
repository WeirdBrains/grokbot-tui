//! The card modal (1.29.0): answer a question widget, approve or deny a blocked action,
//! provide a secret, fill in a form, or send/edit/discard a draft. Opened by clicking a
//! card's button in the chat, `/card`, F11 or keymap `card.open`.
use crate::app::{App, EntryKind, Event};
use crate::cards::{self, Body, Call, Card, State};
use crate::theme;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

/// (start column, width, url) of a clickable span on a line.
pub type ColHit = (usize, usize, String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Opt(usize),
    Submit,
    Dismiss,
    Approve,
    Deny,
    Send,
    Edit,
    Custom,
    Close,
}

#[derive(Default)]
pub struct CardModal {
    pub key: String,
    pub bot: String,
    pub session: Option<String>,
    pub card: Option<Card>,
    pub step: usize,
    pub cursor: usize,
    pub picks: Vec<Vec<usize>>,
    pub custom: String,
    /// Typing into the custom answer / draft body.
    pub typing: bool,
    /// A secret being typed: masked, never logged, cleared after sending or closing.
    secret: String,
    /// Form values by field index (secret fields masked on screen).
    form: Vec<String>,
    pub form_cursor: usize,
    pub draft_body: Option<String>,
    pub busy: bool,
    pub error: Option<String>,
    pub hits: Vec<(Rect, Act)>,
    pub area: Rect,
}

impl Drop for CardModal {
    fn drop(&mut self) {
        wipe(&mut self.secret);
        for v in &mut self.form {
            wipe(v);
        }
    }
}

/// Overwrite then clear a string that may hold a secret.
fn wipe(s: &mut String) {
    // SAFETY-free overwrite: replace every byte's char with '\0' of the same length.
    let n = s.len();
    s.clear();
    s.extend(std::iter::repeat_n('\0', n));
    s.clear();
}

impl std::fmt::Debug for CardModal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CardModal({})", self.key)
    }
}

impl CardModal {
    fn card(&self) -> &Card {
        self.card.as_ref().expect("modal has a card")
    }
    /// Length of the secret being typed (tests).
    #[cfg(test)]
    pub fn secret_len(&self) -> usize {
        self.secret.chars().count()
    }
}

impl App {
    /// Message key of a card's bubble.
    pub fn card_key(card: &Card) -> String {
        crate::emoji::message_key(EntryKind::Bot as u8, &card.summary())
    }

    /// Keep the cards the last transcript call saw (`bot`). A card we just answered keeps
    /// its local state until the server reports one.
    pub fn absorb_cards(&mut self, bot: &str, cards: Vec<Card>) {
        for c in cards {
            let k = Self::card_key(&c);
            let keep_local = self
                .cards
                .get(&k)
                .is_some_and(|(_, old)| !old.state.is_pending() && c.state.is_pending());
            if keep_local {
                continue;
            }
            if self.cards.get(&k).map(|(_, o)| o) != Some(&c) {
                self.cards.insert(k, (bot.to_string(), c));
                self.cards_gen += 1;
            }
        }
    }

    /// Open the card behind bubble `key`, or the newest pending one in this chat.
    pub fn open_card(&mut self, key: Option<&str>) -> bool {
        let bot = self.active_bot().map(|b| b.id.clone());
        let key = match key {
            Some(k) => k.to_string(),
            None => {
                let feed = self.feed();
                let found = feed.iter().rev().find_map(|e| {
                    if e.kind != EntryKind::Bot {
                        return None;
                    }
                    let k = crate::emoji::message_key(e.kind as u8, &e.text);
                    self.cards
                        .get(&k)
                        .filter(|(b, c)| Some(b) == bot.as_ref() && c.state.is_pending())
                        .map(|_| k)
                });
                match found {
                    Some(k) => k,
                    None => {
                        self.info("No pending card in this chat.".into());
                        return false;
                    }
                }
            }
        };
        let Some((bot, card)) = self.cards.get(&key).cloned() else {
            self.info(
                "That card's details haven't loaded yet; try again after the next poll.".into(),
            );
            return false;
        };
        let session = self
            .bots
            .iter()
            .find(|b| b.id == bot)
            .and_then(|b| b.viewer_session_id.clone());
        let steps = match &card.body {
            Body::Widget { steps, .. } => steps.len(),
            _ => 1,
        };
        let fields = match &card.body {
            Body::Form { fields, .. } => fields.len(),
            _ => 0,
        };
        let typing = matches!(card.body, Body::Secret { .. } | Body::Form { .. });
        let mut m = CardModal::default();
        m.key = key;
        m.bot = bot;
        m.session = session;
        m.picks = vec![vec![]; steps];
        m.form = vec![String::new(); fields];
        m.typing = typing && card.state.is_pending();
        m.card = Some(card);
        self.card_modal = Some(m);
        true
    }

    fn card_send(&mut self, call: Call) {
        let Some(m) = self.card_modal.as_mut() else {
            return;
        };
        let Some(api) = self.api.clone() else {
            m.error = Some("Not logged in.".into());
            return;
        };
        m.busy = true;
        m.error = None;
        let (bot, session, key) = (m.bot.clone(), m.session.clone(), m.key.clone());
        let dismiss = matches!(
            call.method,
            "dismissWidget" | "dismissUserForm" | "discardDraft"
        );
        let method = call.method;
        let done = call.done.clone();
        let epoch = self.epoch;
        self.spawn(async move {
            let result = api
                .card_call(&bot, session.as_deref(), call)
                .await
                .map(|()| if dismiss { None } else { Some(done) })
                // The error text comes from the gateway (method, status, message);
                // the request body (which may hold a secret) is never part of it.
                .map_err(|e| e.to_string());
            Event::CardDone {
                epoch,
                key,
                method,
                result,
            }
        });
    }

    pub fn card_done(
        &mut self,
        key: String,
        method: &'static str,
        result: Result<Option<String>, String>,
    ) {
        match result {
            Ok(done) => {
                let state = match done {
                    Some(d) => State::Done(d),
                    None => State::Dismissed,
                };
                let label = state.label();
                if let Some((_, c)) = self.cards.get_mut(&key) {
                    c.state = state;
                    self.cards_gen += 1;
                }
                if self.card_modal.as_ref().is_some_and(|m| m.key == key) {
                    self.card_modal = None;
                }
                self.info(format!("Card {label}."));
                self.poll_now();
            }
            Err(e) => {
                let msg = format!("{method} failed: {e}");
                match self.card_modal.as_mut().filter(|m| m.key == key) {
                    Some(m) => {
                        m.busy = false;
                        m.error = Some(msg);
                    }
                    None => self.error(msg),
                }
            }
        }
    }

    /// Run a modal action (keys and clicks both end here).
    pub fn card_act(&mut self, act: Act) {
        let Some(m) = self.card_modal.as_mut() else {
            return;
        };
        if act == Act::Close {
            self.card_modal = None;
            return;
        }
        if m.busy || !m.card().state.is_pending() {
            return;
        }
        let card = m.card().clone();
        let call = match (&card.body, act) {
            (Body::Widget { steps, .. }, Act::Opt(i)) => {
                let st = &steps[m.step];
                if i >= st.options.len() {
                    return;
                }
                m.cursor = i;
                if st.multi {
                    let p = &mut m.picks[m.step];
                    if let Some(j) = p.iter().position(|&x| x == i) {
                        p.remove(j);
                    } else {
                        p.push(i);
                    }
                    return;
                }
                m.picks[m.step] = vec![i];
                if m.step + 1 < steps.len() {
                    m.step += 1;
                    m.cursor = 0;
                    return;
                }
                cards::answer_widget(&card, cards::widget_value(steps, &m.picks, ""))
            }
            (Body::Widget { steps, .. }, Act::Submit) => {
                let st = &steps[m.step];
                if !st.multi && m.picks[m.step].is_empty() && m.custom.trim().is_empty() {
                    m.picks[m.step] = vec![m.cursor];
                }
                if m.step + 1 < steps.len() {
                    m.step += 1;
                    m.cursor = 0;
                    return;
                }
                let custom = if m.typing || !m.custom.is_empty() {
                    m.custom.clone()
                } else {
                    String::new()
                };
                // Typing a custom answer to a single-choice question replaces the pick.
                if !st.multi && !custom.trim().is_empty() {
                    m.picks[m.step].clear();
                }
                let v = cards::widget_value(steps, &m.picks, &custom);
                if v.trim().is_empty() {
                    m.error = Some("Pick an option first.".into());
                    return;
                }
                cards::answer_widget(&card, v)
            }
            (Body::Widget { allow_custom, .. }, Act::Custom) => {
                if *allow_custom {
                    m.typing = true;
                }
                return;
            }
            (Body::Approval { .. }, Act::Approve | Act::Deny) => {
                match cards::approve(&card, act == Act::Approve) {
                    Some(c) => c,
                    None => return,
                }
            }
            (Body::Secret { .. }, Act::Submit) => {
                if m.secret.is_empty() {
                    m.error = Some("Type the secret first.".into());
                    return;
                }
                let v = std::mem::take(&mut m.secret);
                cards::submit_secret(&card, v)
            }
            (Body::Form { fields, .. }, Act::Submit) => {
                if let Some(f) = fields
                    .iter()
                    .zip(&m.form)
                    .find(|(f, v)| f.required && v.trim().is_empty())
                {
                    m.error = Some(format!("{} is required.", f.0.label));
                    return;
                }
                let values = fields
                    .iter()
                    .zip(m.form.iter_mut())
                    .filter(|(_, v)| !v.is_empty())
                    .map(|(f, v)| (f.id.clone(), std::mem::take(v)))
                    .collect();
                cards::submit_form(&card, values)
            }
            (Body::Draft { body, .. }, Act::Edit) => {
                m.draft_body.get_or_insert_with(|| body.clone());
                m.typing = true;
                return;
            }
            (Body::Draft { .. }, Act::Send) => {
                match cards::send_draft(&card, m.draft_body.as_deref()) {
                    Some(c) => c,
                    None => return,
                }
            }
            (_, Act::Dismiss) => match cards::dismiss(&card) {
                Some(c) => c,
                None => return,
            },
            _ => return,
        };
        self.card_send(call);
    }

    /// Keys while the modal is open.
    pub fn card_modal_key(&mut self, k: KeyEvent) {
        let Some(m) = self.card_modal.as_mut() else {
            return;
        };
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let pending = m.card().state.is_pending();
        let body = m.card().body.clone();
        // Typing: custom answer, secret, form field or draft body.
        if m.typing && pending {
            match (&body, k.code) {
                (_, KeyCode::Esc) => {
                    if matches!(body, Body::Secret { .. } | Body::Form { .. }) {
                        self.card_modal = None;
                    } else {
                        m.typing = false;
                    }
                }
                (_, KeyCode::Char('d')) if ctrl => self.card_act(Act::Dismiss),
                (Body::Draft { .. }, KeyCode::Char('s')) if ctrl => m.typing = false,
                (Body::Form { .. }, KeyCode::Char('s')) if ctrl => self.card_act(Act::Submit),
                (Body::Draft { .. }, KeyCode::Enter) => {
                    m.draft_body.get_or_insert_default().push('\n')
                }
                (Body::Form { fields, .. }, KeyCode::Enter | KeyCode::Tab | KeyCode::Down) => {
                    if k.code == KeyCode::Enter && m.form_cursor + 1 >= fields.len() {
                        self.card_act(Act::Submit);
                    } else {
                        m.form_cursor = (m.form_cursor + 1) % fields.len().max(1);
                    }
                }
                (Body::Form { fields, .. }, KeyCode::BackTab | KeyCode::Up) => {
                    m.form_cursor = (m.form_cursor + fields.len().max(1) - 1) % fields.len().max(1)
                }
                (_, KeyCode::Enter) => self.card_act(Act::Submit),
                (_, KeyCode::Backspace) => {
                    let s = match &body {
                        Body::Secret { .. } => &mut m.secret,
                        Body::Form { .. } => match m.form.get_mut(m.form_cursor) {
                            Some(s) => s,
                            None => return,
                        },
                        Body::Draft { .. } => m.draft_body.get_or_insert_default(),
                        _ => &mut m.custom,
                    };
                    s.pop();
                }
                (_, KeyCode::Char(c)) if !ctrl => {
                    let s = match &body {
                        Body::Secret { .. } => &mut m.secret,
                        Body::Form { .. } => match m.form.get_mut(m.form_cursor) {
                            Some(s) => s,
                            None => return,
                        },
                        Body::Draft { .. } => m.draft_body.get_or_insert_default(),
                        _ => &mut m.custom,
                    };
                    s.push(c);
                }
                _ => {}
            }
            return;
        }
        let n_opts = match &body {
            Body::Widget { steps, .. } => steps[m.step.min(steps.len() - 1)].options.len(),
            _ => 0,
        };
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => self.card_modal = None,
            KeyCode::Up | KeyCode::Char('k') if n_opts > 0 => m.cursor = m.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') if n_opts > 0 => {
                m.cursor = (m.cursor + 1).min(n_opts - 1)
            }
            KeyCode::Char(c @ '1'..='9') if n_opts > 0 => {
                self.card_act(Act::Opt(c as usize - '1' as usize))
            }
            KeyCode::Char(' ') if n_opts > 0 => {
                let i = m.cursor;
                self.card_act(Act::Opt(i))
            }
            KeyCode::Enter => self.card_act(Act::Submit),
            KeyCode::Tab | KeyCode::Char('c') => self.card_act(Act::Custom),
            KeyCode::Char('x') => self.card_act(Act::Dismiss),
            KeyCode::Char('a' | 'y') => self.card_act(Act::Approve),
            KeyCode::Char('d' | 'n') => self.card_act(Act::Deny),
            KeyCode::Char('s') => self.card_act(Act::Send),
            KeyCode::Char('e') => self.card_act(Act::Edit),
            _ => {}
        }
    }

    pub fn card_modal_click(&mut self, col: u16, row: u16) -> bool {
        let Some(m) = self.card_modal.as_ref() else {
            return false;
        };
        let at = Position::new(col, row);
        if let Some(&(_, a)) = m.hits.iter().find(|(r, _)| r.contains(at)) {
            self.card_act(a);
        } else if !m.area.contains(at) {
            self.card_modal = None;
        }
        true
    }
}

/// Bubble lines for a card: state, details, and its button (a [`crate::ui::ColHit`]).
pub fn bubble_lines(card: &Card, key: &str) -> Vec<(Line<'static>, Vec<ColHit>)> {
    let st = if card.state.is_pending() {
        theme::card_pending()
    } else {
        theme::card_done()
    };
    let mut out = vec![(
        Line::from(sp(st, format!("● {}", card.state.label()))),
        vec![],
    )];
    for l in card.bubble_lines() {
        out.push((Line::from(sp(theme::card_frame(), l)), vec![]));
    }
    let b = card.button();
    out.push((
        Line::from(sp(theme::card_button(), b.to_string())),
        vec![(
            0,
            crate::input::str_width(b),
            format!("{}{key}", cards::SCHEME),
        )],
    ));
    out
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let Some(m) = app.card_modal.as_mut() else {
        return;
    };
    let card = m.card().clone();
    let w = (area.width * 7 / 10).clamp(30.min(area.width), 90.min(area.width));
    let mut lines: Vec<Line<'static>> = vec![];
    let mut acts: Vec<(usize, u16, u16, Act)> = vec![]; // (line, x, w, act)
    let pending = card.state.is_pending();
    let stl = if pending {
        theme::card_pending()
    } else {
        theme::card_done()
    };
    lines.push(Line::from(sp(
        stl,
        format!("{} · {}", card.kind, card.state.label()),
    )));
    lines.push(Line::default());
    let buttons = |lines: &mut Vec<Line<'static>>,
                   acts: &mut Vec<(usize, u16, u16, Act)>,
                   bs: &[(&str, Act)]| {
        let mut spans = vec![];
        let mut x = 0u16;
        for (label, a) in bs {
            let t = format!("[{label}]");
            let tw = crate::input::str_width(&t) as u16;
            acts.push((lines.len(), x, tw, *a));
            spans.push(sp(theme::card_button(), t));
            spans.push(sp(theme::muted(), "  ".to_string()));
            x += tw + 2;
        }
        lines.push(Line::from(spans));
    };
    let hint: String = match &card.body {
        Body::Widget {
            help,
            steps,
            allow_custom,
        } => {
            let si = m.step.min(steps.len() - 1);
            let st = &steps[si];
            if steps.len() > 1 {
                lines.push(Line::from(sp(
                    theme::muted(),
                    format!("Step {} of {}", si + 1, steps.len()),
                )));
            }
            lines.push(Line::from(sp(theme::bot_header_name(), st.prompt.clone())));
            if !help.is_empty() {
                lines.push(Line::from(sp(theme::muted(), help.clone())));
            }
            for (i, o) in st.options.iter().enumerate() {
                let picked = m.picks[si].contains(&i);
                let mark = match (st.multi, picked) {
                    (true, true) => "[x]",
                    (true, false) => "[ ]",
                    (false, true) => "(•)",
                    (false, false) => "( )",
                };
                let num = if i < 9 {
                    format!("{}", i + 1)
                } else {
                    " ".into()
                };
                let t = format!("{num} {mark} {}", o.label);
                acts.push((
                    lines.len(),
                    0,
                    crate::input::str_width(&t) as u16,
                    Act::Opt(i),
                ));
                let style = if i == m.cursor && !m.typing {
                    theme::cursor_row()
                } else {
                    theme::card_frame()
                };
                lines.push(Line::from(sp(style, t)));
                if !o.description.is_empty() {
                    lines.push(Line::from(sp(
                        theme::muted(),
                        format!("      {}", o.description),
                    )));
                }
            }
            if *allow_custom {
                let t = if m.typing {
                    format!("✎ {}▏", m.custom)
                } else if m.custom.is_empty() {
                    "✎ type your own (Tab)".into()
                } else {
                    format!("✎ {}", m.custom)
                };
                acts.push((
                    lines.len(),
                    0,
                    crate::input::str_width(&t) as u16,
                    Act::Custom,
                ));
                lines.push(Line::from(sp(theme::card_frame(), t)));
            }
            lines.push(Line::default());
            if pending {
                let last = si + 1 == steps.len();
                buttons(
                    &mut lines,
                    &mut acts,
                    &[
                        (if last { "Send" } else { "Next" }, Act::Submit),
                        ("Dismiss", Act::Dismiss),
                        ("Close", Act::Close),
                    ],
                );
            }
            if st.multi {
                "1-9/Space toggle · Enter send · Tab type your own · x dismiss · Esc close".into()
            } else {
                "1-9 or ↑↓ Enter choose · Tab type your own · x dismiss · Esc close".into()
            }
        }
        Body::Approval {
            local,
            summary,
            command,
            reason,
            ..
        } => {
            lines.push(Line::from(sp(
                theme::bot_header_name(),
                if *local {
                    "A tool on your computer wants permission".to_string()
                } else {
                    "Auto-review blocked an action".to_string()
                },
            )));
            if !summary.is_empty() {
                lines.push(Line::from(sp(theme::card_frame(), summary.clone())));
            }
            if !command.is_empty() {
                lines.push(Line::from(sp(theme::md_code(), format!("$ {command}"))));
            }
            if !reason.is_empty() {
                lines.push(Line::from(sp(theme::muted(), format!("Why: {reason}"))));
            }
            lines.push(Line::default());
            if pending {
                buttons(
                    &mut lines,
                    &mut acts,
                    &[
                        (if *local { "Allow once" } else { "Approve" }, Act::Approve),
                        ("Deny", Act::Deny),
                        ("Close", Act::Close),
                    ],
                );
            }
            "a approve · d deny · Esc close (nothing is approved automatically)".into()
        }
        Body::Secret { label, description } => {
            lines.push(Line::from(sp(
                theme::bot_header_name(),
                format!("Secret: {label}"),
            )));
            if !description.is_empty() {
                lines.push(Line::from(sp(theme::muted(), description.clone())));
            }
            lines.push(Line::default());
            if pending {
                let n = m.secret.chars().count();
                lines.push(Line::from(sp(
                    theme::card_frame(),
                    format!("› {}▏", "•".repeat(n)),
                )));
                lines.push(Line::default());
                buttons(
                    &mut lines,
                    &mut acts,
                    &[
                        ("Save", Act::Submit),
                        ("Dismiss", Act::Dismiss),
                        ("Close", Act::Close),
                    ],
                );
            }
            "Type the secret (masked; sent only to Grok Bot, never stored here) · Enter save · Ctrl-D dismiss · Esc close".into()
        }
        Body::Form { title, fields } => {
            lines.push(Line::from(sp(theme::bot_header_name(), title.clone())));
            lines.push(Line::default());
            for (i, fd) in fields.iter().enumerate() {
                let v = m.form.get(i).cloned().unwrap_or_default();
                let shown = if fd.secret {
                    "•".repeat(v.chars().count())
                } else {
                    v
                };
                let cur = if i == m.form_cursor && pending {
                    "▏"
                } else {
                    ""
                };
                let req = if fd.required { " *" } else { "" };
                let style = if i == m.form_cursor {
                    theme::cursor_row()
                } else {
                    theme::card_frame()
                };
                let ph = if shown.is_empty() && cur.is_empty() {
                    fd.placeholder.clone()
                } else {
                    String::new()
                };
                lines.push(Line::from(sp(
                    style,
                    format!("{}{req}: {shown}{cur}{ph}", fd.label),
                )));
            }
            lines.push(Line::default());
            if pending {
                buttons(
                    &mut lines,
                    &mut acts,
                    &[
                        ("Submit", Act::Submit),
                        ("Dismiss", Act::Dismiss),
                        ("Close", Act::Close),
                    ],
                );
            }
            "Tab/↑↓ field · Enter next/submit · Ctrl-S submit · Ctrl-D dismiss · Esc close (• = masked)".into()
        }
        Body::Draft {
            from,
            to,
            subject,
            body,
            ..
        } => {
            if !from.is_empty() {
                lines.push(Line::from(sp(theme::muted(), format!("From: {from}"))));
            }
            lines.push(Line::from(sp(
                theme::card_frame(),
                format!("To: {}", to.join(", ")),
            )));
            if !subject.is_empty() {
                lines.push(Line::from(sp(
                    theme::bot_header_name(),
                    format!("Subject: {subject}"),
                )));
            }
            lines.push(Line::default());
            let shown = m.draft_body.clone().unwrap_or_else(|| body.clone());
            for l in shown.lines() {
                lines.push(Line::from(sp(theme::card_frame(), l.to_string())));
            }
            if m.typing {
                lines.push(Line::from(sp(theme::card_frame(), "▏".to_string())));
            }
            lines.push(Line::default());
            if pending {
                buttons(
                    &mut lines,
                    &mut acts,
                    &[
                        ("Send", Act::Send),
                        ("Edit", Act::Edit),
                        ("Discard", Act::Dismiss),
                        ("Close", Act::Close),
                    ],
                );
            }
            if m.typing {
                "Editing the body · Enter newline · Ctrl-S done · Esc stop".into()
            } else {
                "s send · e edit body · x discard · Esc close".into()
            }
        }
        Body::Info {
            title, lines: ls, ..
        } => {
            if !title.is_empty() {
                lines.push(Line::from(sp(theme::bot_header_name(), title.clone())));
            }
            for l in ls {
                lines.push(Line::from(sp(theme::card_frame(), l.clone())));
            }
            lines.push(Line::default());
            lines.push(Line::from(sp(
                theme::muted(),
                "grokbot-tui can't act on this card yet; use the desktop app.".to_string(),
            )));
            "Esc close".into()
        }
    };
    if m.busy {
        lines.push(Line::from(sp(theme::muted(), "Sending…".to_string())));
    }
    if let Some(e) = &m.error {
        lines.push(Line::from(sp(theme::entry(EntryKind::Error), e.clone())));
    }
    let h = (lines.len() as u16 + 4).min(area.height);
    let r = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + area.height.saturating_sub(h) / 2,
        w,
        h,
    );
    m.area = r;
    f.render_widget(Clear, r);
    let block = theme::block(&theme::panel(true))
        .title(format!(
            " {} ",
            card.summary()
                .chars()
                .take(w as usize - 6)
                .collect::<String>()
        ))
        .title_bottom(Line::from(sp(theme::muted(), format!(" {hint} "))));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let inner = Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(2),
        ..inner
    };
    m.hits = acts
        .into_iter()
        .filter(|(l, ..)| (*l as u16) < inner.height)
        .map(|(l, x, w, a)| (Rect::new(inner.x + x, inner.y + l as u16, w, 1), a))
        .collect();
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn sp(s: ratatui_lipgloss::Style, t: impl Into<String>) -> Span<'static> {
    Span::styled(t.into(), ratatui::style::Style::from(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{mock_login, settle, test_app, type_line};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(app: &mut App, c: KeyCode) {
        app.card_modal_key(KeyEvent::new(c, KeyModifiers::NONE));
    }
    fn ctrl(app: &mut App, c: char) {
        app.card_modal_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }
    fn typ(app: &mut App, s: &str) {
        for c in s.chars() {
            key(app, KeyCode::Char(c));
        }
    }
    fn screen(term: &mut Terminal<TestBackend>, app: &mut App) -> String {
        term.draw(|f| crate::ui::draw(f, app)).unwrap();
        let b = term.backend().buffer();
        (0..b.area.height)
            .map(|y| {
                (0..b.area.width)
                    .map(|x| b[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    /// Post `card <kind>` in mock mode and poll until its card is known.
    fn post(kind: &str) -> (tokio::runtime::Runtime, App, tempfile::TempDir) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, d) = test_app(&rt);
        mock_login(&mut app, "t");
        type_line(&mut app, &format!("card {kind}"));
        settle(&mut app);
        app.poll_now();
        settle(&mut app);
        assert!(!app.cards.is_empty(), "card {kind} loaded");
        (rt, app, d)
    }
    fn state(app: &App) -> State {
        app.cards.values().next().unwrap().1.state.clone()
    }
    fn answered(app: &mut App) -> State {
        settle(app);
        app.poll_now();
        settle(app);
        state(app)
    }

    #[test]
    fn single_choice_widget_from_bubble_button_to_answered() {
        let (_rt, mut app, _d) = post("widget");
        let mut term = Terminal::new(TestBackend::new(110, 34)).unwrap();
        let s = screen(&mut term, &mut app);
        assert!(s.contains("● pending"), "{s}");
        assert!(s.contains("Pick a color"), "{s}");
        // Click the bubble's button.
        let h = app
            .download_hits
            .iter()
            .find(|h| h.url.starts_with(cards::SCHEME))
            .cloned()
            .expect("card button hit");
        app.click(h.x, h.y);
        assert!(app.card_modal.is_some());
        let s = screen(&mut term, &mut app);
        assert!(s.contains("( ) Green"), "{s}");
        key(&mut app, KeyCode::Char('2'));
        assert_eq!(answered(&mut app), State::Done("Green".into()));
        assert!(app.card_modal.is_none());
        let s = screen(&mut term, &mut app);
        assert!(s.contains("answered · Green"), "{s}");
        // An answered card opens read-only.
        assert!(!app.open_card(None));
    }

    #[test]
    fn multi_select_with_custom_answer() {
        let (_rt, mut app, _d) = post("multi");
        assert!(app.open_card(None));
        key(&mut app, KeyCode::Char('1'));
        key(&mut app, KeyCode::Char('3'));
        key(&mut app, KeyCode::Char('3')); // toggled off again
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Char(' '));
        key(&mut app, KeyCode::Tab);
        typ(&mut app, "Pineapple");
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            answered(&mut app),
            State::Done("Cheese Basil Pineapple".into())
        );
    }

    #[test]
    fn multi_step_widget_answers_every_step() {
        let (_rt, mut app, _d) = post("steps");
        assert!(app.open_card(None));
        key(&mut app, KeyCode::Char('2'));
        assert_eq!(app.card_modal.as_ref().unwrap().step, 1);
        key(&mut app, KeyCode::Char('1'));
        match answered(&mut app) {
            State::Done(s) => assert!(s.contains("Large") && s.contains("Tea"), "{s}"),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn dismissing_a_widget() {
        let (_rt, mut app, _d) = post("widget");
        assert!(app.open_card(None));
        key(&mut app, KeyCode::Char('x'));
        assert_eq!(answered(&mut app), State::Dismissed);
    }

    #[test]
    fn approval_and_local_tool_cards_approve_and_deny() {
        let (_rt, mut app, _d) = post("approval");
        let mut term = Terminal::new(TestBackend::new(110, 34)).unwrap();
        assert!(app.open_card(None));
        let s = screen(&mut term, &mut app);
        assert!(
            s.contains("$ ls -la ~/Downloads") && s.contains("[Approve]"),
            "{s}"
        );
        key(&mut app, KeyCode::Char('a'));
        assert!(matches!(answered(&mut app), State::Done(s) if s.contains("approved")));
        let (_rt, mut app, _d) = post("tool");
        assert!(app.open_card(None));
        key(&mut app, KeyCode::Char('d'));
        assert!(matches!(answered(&mut app), State::Done(s) if s.contains("denied")));
    }

    #[test]
    fn secrets_are_masked_and_never_shown_or_stored() {
        const SECRET: &str = "hunter2-very-secret";
        let (_rt, mut app, d) = post("secret");
        let mut term = Terminal::new(TestBackend::new(110, 34)).unwrap();
        assert!(app.open_card(None));
        typ(&mut app, SECRET);
        assert_eq!(app.card_modal.as_ref().unwrap().secret_len(), SECRET.len());
        let s = screen(&mut term, &mut app);
        assert!(
            !s.contains(SECRET) && s.contains(&"•".repeat(SECRET.len())),
            "{s}"
        );
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.card_modal.as_ref().map(|m| m.secret_len()), Some(0));
        let st = answered(&mut app);
        assert!(
            matches!(st, State::Done(ref s) if !s.contains(SECRET)),
            "{st:?}"
        );
        let s = screen(&mut term, &mut app);
        assert!(!s.contains(SECRET));
        for e in app.feed().iter().chain(app.notices()) {
            assert!(!e.text.contains(SECRET));
        }
        // Not in the store either.
        for f in walk(d.path()) {
            let bytes = std::fs::read(&f).unwrap_or_default();
            assert!(
                !bytes.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()),
                "{f:?}"
            );
        }
        // Esc while typing drops it.
        let (_rt, mut app, _d) = post("secret");
        assert!(app.open_card(None));
        typ(&mut app, "abc");
        key(&mut app, KeyCode::Esc);
        assert!(app.card_modal.is_none());
        assert_eq!(state(&app), State::Pending);
    }

    fn walk(p: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = vec![];
        for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
        out
    }

    #[test]
    fn form_masks_password_fields_and_requires_fields() {
        let (_rt, mut app, _d) = post("form");
        let mut term = Terminal::new(TestBackend::new(110, 34)).unwrap();
        assert!(app.open_card(None));
        ctrl(&mut app, 's');
        assert!(
            app.card_modal
                .as_ref()
                .unwrap()
                .error
                .as_deref()
                .unwrap()
                .contains("required")
        );
        typ(&mut app, "mike");
        key(&mut app, KeyCode::Tab);
        typ(&mut app, "pw-xyz");
        let s = screen(&mut term, &mut app);
        assert!(
            s.contains("Username *: mike") && !s.contains("pw-xyz") && s.contains("••••••"),
            "{s}"
        );
        key(&mut app, KeyCode::Enter);
        assert!(matches!(answered(&mut app), State::Done(_)));
    }

    #[test]
    fn draft_edit_then_send_and_discard() {
        let (_rt, mut app, _d) = post("draft");
        let mut term = Terminal::new(TestBackend::new(110, 34)).unwrap();
        assert!(app.open_card(None));
        let s = screen(&mut term, &mut app);
        assert!(
            s.contains("To: you@example.com") && s.contains("[Send]") && s.contains("[Discard]"),
            "{s}"
        );
        key(&mut app, KeyCode::Char('e'));
        typ(&mut app, " Thanks!");
        ctrl(&mut app, 's');
        assert!(
            app.card_modal
                .as_ref()
                .unwrap()
                .draft_body
                .as_deref()
                .unwrap()
                .ends_with("soon. Thanks!")
        );
        key(&mut app, KeyCode::Char('s'));
        assert!(matches!(answered(&mut app), State::Done(_)));
        let (_rt, mut app, _d) = post("draft");
        assert!(app.open_card(None));
        key(&mut app, KeyCode::Char('x'));
        assert_eq!(answered(&mut app), State::Dismissed);
    }

    #[test]
    fn unsupported_cards_are_read_only() {
        let (_rt, mut app, _d) = post("purchase");
        let mut term = Terminal::new(TestBackend::new(110, 34)).unwrap();
        assert!(app.open_card(None));
        let s = screen(&mut term, &mut app);
        assert!(s.contains("use the desktop app"), "{s}");
        for c in ['a', 'x', 's', '1'] {
            key(&mut app, KeyCode::Char(c));
        }
        settle(&mut app);
        assert_eq!(state(&app), State::Pending);
    }
}
