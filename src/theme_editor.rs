//! Theme switching (header `‹name› ▾`, F5 picker, `/theme`) and the full-screen theme
//! editor (Shift-F5, `/theme edit`).
//!
//! The editor works on a copy of a theme file. The left side lists the palette colors and
//! every styled part; the right side shows a live preview (the real drawing code rendered
//! off-screen with the edited theme) and the selected part's details. Nothing touches the
//! file until you save.
use crate::app::App;
use crate::picker::{PickItem, Picker};
use crate::theme::{
    self, COLOR_HELP, COLOR_SLOTS, ColorRef, N_COLORS, N_STYLES, STYLE_SLOTS, Theme, Variant,
    VariantSpec, color,
};
use crate::themes::{self, LoadError};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use std::path::PathBuf;
use std::sync::Arc;
use unicode_width::UnicodeWidthStr;

/// Why a theme is being downloaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadFor {
    /// No theme files at startup / `/theme reload`: the default theme.
    Startup,
    /// `/theme init [name|all]`: these shipped themes, one after another (the first is
    /// the one being downloaded).
    Init(Vec<String>),
}

const EDIT_ITEM: &str = "\u{1}edit";
const INIT_ITEM: &str = "\u{1}init";

// ================================================================ app: switching themes

impl App {
    /// `<data dir>/themes`.
    pub fn themes_dir(&self) -> PathBuf {
        themes::user_dir(&self.store.dir)
    }

    /// Startup: download the default theme when the user has no themes, then load `ui.theme`.
    pub fn init_themes(&mut self) {
        let dir = self.themes_dir();
        // Themes live only in the user's themes dir. When it has none (first run), fetch
        // default.toml from this version's git tag in the background. Until it arrives (or
        // if it fails) the terminal's own colors are used.
        if themes::list(&dir).is_empty() {
            if self.theme_downloading {
                return;
            }
            if let Some(url) = self.download_url(themes::DEFAULT_ID) {
                self.theme = Arc::new(Theme::unstyled(themes::DEFAULT_ID));
                self.theme_problem = Some("downloading");
                self.info(format!(
                    "No theme files found. Downloading the default theme from {url} (plain \
                     terminal colors until then; turn this off with {}=1 or /prefs set \
                     ui.theme_download false).",
                    themes::NO_DOWNLOAD_ENV
                ));
                self.start_theme_download(DownloadFor::Startup, url);
                return;
            }
        }
        self.load_theme();
    }

    /// Theme `name`'s download URL, unless downloading is off (env, pref, mock mode).
    pub fn download_url(&self, name: &str) -> Option<String> {
        self.theme_download_base
            .as_deref()
            .filter(|_| self.config.theme_download)
            .map(|b| themes::theme_url(b, name))
    }

    fn start_theme_download(&mut self, purpose: DownloadFor, url: String) {
        self.theme_downloading = true;
        self.spawn(async move {
            let result = themes::download(&url).await;
            crate::app::Event::ThemeDownload {
                purpose,
                url,
                result,
            }
        });
    }

    /// A default-theme download finished: write it (never over an existing file) and use it.
    pub fn on_theme_download(
        &mut self,
        purpose: DownloadFor,
        url: &str,
        result: Result<String, String>,
    ) {
        self.theme_downloading = false;
        let dir = self.themes_dir();
        match purpose {
            DownloadFor::Startup => {
                match result {
                    Ok(text) => match themes::write_new(&dir, themes::DEFAULT_ID, &text) {
                        Ok(p) => self.info(format!(
                            "Downloaded the default theme from {url} into {}.",
                            p.display()
                        )),
                        // Someone put a file there meanwhile: keep theirs.
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(e) => self.error(format!(
                            "Downloaded the default theme but couldn't write it to {}: {e}",
                            themes::path(&dir, themes::DEFAULT_ID).display()
                        )),
                    },
                    Err(e) => self.error(format!(
                        "Couldn't download the default theme from {url}: {e}."
                    )),
                }
                self.load_theme();
            }
            DownloadFor::Init(mut queue) => {
                let id = queue.remove(0);
                match result {
                    Ok(text) => match themes::write_new(&dir, &id, &text) {
                        Ok(p) => {
                            self.info(format!(
                                "Downloaded the {id} theme from {url} into {}.",
                                p.display()
                            ));
                            self.after_theme_written(&id);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                            self.info(format!(
                                "{} already exists; it was kept.",
                                themes::path(&dir, &id).display()
                            ))
                        }
                        Err(e) => self.error(format!(
                            "Couldn't write {}: {e}",
                            themes::path(&dir, &id).display()
                        )),
                    },
                    Err(e) => self.error(format!(
                        "Couldn't download the {id} theme from {url}: {e}. Shipped themes: {}. \
                         /theme starter {id} writes a minimal starter instead.",
                        themes::SHIPPED.join(", ")
                    )),
                }
                // Next in line (`/theme init all`).
                if let Some(next) = queue.first()
                    && let Some(url) = self.download_url(next)
                {
                    self.start_theme_download(DownloadFor::Init(queue), url);
                }
            }
        }
    }

    /// Load the selected theme (`ui.theme`). On failure, draw with plain terminal colors and
    /// say what's wrong and how to fix it. Returns whether the theme loaded.
    pub fn load_theme(&mut self) -> bool {
        let id = self.config.theme.clone();
        match themes::load(&self.themes_dir(), &id) {
            Ok((t, warnings)) => {
                self.apply_theme(t);
                self.theme_warnings(&id, &warnings);
                true
            }
            Err(e) => {
                self.theme = Arc::new(Theme::unstyled(&id));
                self.theme_problem = Some(match e {
                    LoadError::Missing(_) => "missing",
                    LoadError::Broken(..) => "broken",
                });
                let msg = self.fallback_notice(&id, &e);
                self.error(msg);
                false
            }
        }
    }

    fn apply_theme(&mut self, t: Theme) {
        self.theme = Arc::new(t);
        self.theme_problem = None;
    }

    fn theme_warnings(&mut self, id: &str, w: &[String]) {
        if w.is_empty() {
            return;
        }
        let more = if w.len() > 5 {
            format!(" (and {} more)", w.len() - 5)
        } else {
            String::new()
        };
        let shown: Vec<&str> = w.iter().take(5).map(String::as_str).collect();
        self.info(format!(
            "Theme '{id}' loaded with warnings: {}{more}",
            shown.join("; ")
        ));
    }

    /// The notice shown when the selected theme can't be used.
    pub fn fallback_notice(&self, id: &str, e: &LoadError) -> String {
        let others: Vec<String> = themes::list(&self.themes_dir())
            .into_iter()
            .filter(|t| t != id)
            .collect();
        let use_hint = if others.is_empty() {
            String::new()
        } else {
            format!(" · /theme use <name> (installed: {})", others.join(", "))
        };
        match e {
            LoadError::Missing(p) => format!(
                "Theme '{id}' not found: {} is missing, so plain terminal colors are used. \
                 Fix: /theme init (downloads the default theme), or save {} there and run \
                 /theme reload{use_hint} · /theme starter writes a minimal starter theme to \
                 edit (Shift-F5).",
                p.display(),
                themes::default_url()
            ),
            LoadError::Broken(p, err) => format!(
                "Theme '{id}' can't be used: {}: {err}. Plain terminal colors are used and the \
                 file was left as is. Fix it and run /theme reload, or open it with Shift-F5{use_hint}.",
                p.display()
            ),
        }
    }

    /// Switch to a theme file and save the choice (global pref `ui.theme`). A theme that
    /// doesn't load isn't selected.
    pub fn set_theme(&mut self, id: &str) -> bool {
        match themes::load(&self.themes_dir(), id) {
            Ok((t, w)) => {
                if let Err(e) = self.store.set_pref(crate::config::PREF_THEME, id) {
                    self.error(format!("{e:#}"));
                    return false;
                }
                // An explicit choice is saved and ends a --theme override.
                self.theme_override = None;
                self.config.theme = id.to_string();
                let name = t.name.clone();
                self.apply_theme(t);
                self.info(format!(
                    "Theme: {name} ({id}). F5 picks another, Shift-F5 edits it."
                ));
                self.theme_warnings(id, &w);
                true
            }
            Err(e) => {
                self.error(format!(
                    "Can't switch to theme '{id}': {e}. Kept the current theme."
                ));
                false
            }
        }
    }

    /// The theme pick list (F5, header `‹name› ▾`): every theme file, then "Edit themes…".
    pub fn open_theme_picker(&mut self) {
        let dir = self.themes_dir();
        let ids = themes::list(&dir);
        let mut items: Vec<PickItem> = ids
            .iter()
            .map(|id| {
                let (label, broken) = match themes::load(&dir, id) {
                    Ok((t, _)) if t.name != *id => (format!("{} ({id})", t.name), false),
                    Ok(_) => (id.clone(), false),
                    Err(_) => (id.clone(), true),
                };
                let mut it = PickItem::new(label, id.clone());
                if broken {
                    it = it.detail("broken");
                } else if *id == self.config.theme {
                    it = it.detail("current");
                }
                it
            })
            .collect();
        if ids.is_empty() {
            let label = if self.download_url(themes::DEFAULT_ID).is_some() {
                "Download the default theme (/theme init)"
            } else {
                "Write a starter theme (/theme init)"
            };
            items.push(PickItem::new(label, INIT_ITEM));
        }
        items.push(PickItem::new("Edit themes…", EDIT_ITEM));
        let current = self.config.theme.clone();
        // Moving the highlight previews a theme; Esc (or a non-theme item) puts this back.
        self.theme_preview = Some((self.theme.clone(), self.theme_problem));
        self.open_picker(
            Picker::new(
                "Theme · ↑↓ preview · Enter uses · Esc keeps",
                items,
                on_theme_pick,
            )
            .with_selected(&current)
            .on_move(preview_theme_pick)
            .on_cancel(cancel_theme_pick),
        );
    }

    /// Put back the theme that was in use when the picker opened.
    fn end_theme_preview(&mut self) {
        if let Some((t, problem)) = self.theme_preview.take() {
            self.theme = t;
            self.theme_problem = problem;
        }
    }

    /// `/theme list`.
    pub fn theme_list_text(&self) -> String {
        let dir = self.themes_dir();
        let mut out = themes::list_text(&dir, &self.config.theme);
        if themes::list(&dir).is_empty() {
            out += "\n/theme init downloads the default theme (see /help theme-files).";
        } else {
            out += "\n/theme use <name> · F5 pick · Shift-F5 edit · /theme export <name> [path]";
        }
        out
    }

    /// `/theme export <name> [path]`: copy a theme file out of the themes folder.
    pub fn export_theme(&mut self, id: &str, dest: Option<&str>) {
        let dest = dest.map(expand_tilde);
        match themes::export(&self.themes_dir(), id, dest.as_deref(), &self.store.dir) {
            Ok(p) => self.info(format!("Exported theme '{id}' to {}.", p.display())),
            Err(e) => self.error(format!("Couldn't export theme '{id}': {e}.")),
        }
    }

    /// `/theme init [name|all]`: download a shipped theme (default `default`; `all` = every
    /// one in [`themes::SHIPPED`]) from the release's `themes/` folder as `<name>.toml`.
    /// Existing files are kept. With downloading off, the minimal starter is written instead.
    pub fn theme_init(&mut self, name: Option<&str>) {
        let dir = self.themes_dir();
        let wanted: Vec<String> = match name.map(str::trim) {
            Some("all") => themes::SHIPPED.iter().map(|s| s.to_string()).collect(),
            Some(n) => vec![n.to_string()],
            None => vec![themes::DEFAULT_ID.to_string()],
        };
        if let Some(bad) = wanted.iter().find(|n| !themes::valid_id(n)) {
            self.error(format!(
                "'{bad}' isn't a valid theme name (a-z, 0-9, _ and -, up to 32)."
            ));
            return;
        }
        let mut queue = vec![];
        for n in wanted {
            let p = themes::path(&dir, &n);
            if p.exists() {
                self.info(format!(
                    "{} already exists; it was kept (nothing downloaded).",
                    p.display()
                ));
            } else {
                queue.push(n);
            }
        }
        let Some(first) = queue.first().cloned() else {
            return;
        };
        match self.download_url(&first) {
            Some(url) if !self.theme_downloading => {
                self.info(format!(
                    "Downloading {} from {}…",
                    queue.join(", "),
                    self.theme_download_base.clone().unwrap_or_default()
                ));
                self.start_theme_download(DownloadFor::Init(queue), url);
            }
            Some(_) => {
                self.error("A theme download is already running; try again in a moment.".into())
            }
            None => {
                self.info(format!(
                    "Theme downloads are off ({}=1 or ui.theme_download = false); writing the \
                     minimal starter instead.",
                    themes::NO_DOWNLOAD_ENV
                ));
                for id in queue {
                    self.write_starter(&id);
                }
            }
        }
    }

    /// `/theme starter [name]`: always the minimal starter (no download).
    pub fn theme_starter(&mut self, name: Option<&str>) {
        if let Some(id) = self.new_theme_id(name) {
            self.write_starter(&id);
        }
    }

    /// The id for a new theme file: `name`, or `default` if that doesn't exist yet.
    fn new_theme_id(&mut self, name: Option<&str>) -> Option<String> {
        let dir = self.themes_dir();
        let id = match name {
            Some(n) => n.trim().to_string(),
            None if !themes::path(&dir, themes::DEFAULT_ID).exists() => {
                themes::DEFAULT_ID.to_string()
            }
            None => {
                self.error(format!(
                    "{} already exists; use /theme init <name> for another one.",
                    themes::path(&dir, themes::DEFAULT_ID).display()
                ));
                return None;
            }
        };
        if !themes::valid_id(&id) {
            self.error(format!(
                "'{id}' isn't a valid theme name (a-z, 0-9, _ and -, up to 32)."
            ));
            return None;
        }
        if themes::path(&dir, &id).exists() {
            self.error(format!(
                "{} already exists; not overwritten.",
                themes::path(&dir, &id).display()
            ));
            return None;
        }
        Some(id)
    }

    fn write_starter(&mut self, id: &str) {
        let dir = self.themes_dir();
        let t = starter(id);
        if themes::path(&dir, id).exists() {
            self.error(format!(
                "{} already exists; not overwritten.",
                themes::path(&dir, id).display()
            ));
            return;
        }
        match themes::save(&dir, &t) {
            Ok(p) => {
                self.info(format!(
                    "Wrote the starter theme {}: every color is 'reset' (plain terminal colors). \
                     Set colors with Shift-F5.",
                    p.display()
                ));
                self.after_theme_written(id);
            }
            Err(e) => self.error(format!(
                "Couldn't write {}: {e}",
                themes::path(&dir, id).display()
            )),
        }
    }

    /// A new theme file exists: use it if it's the selected one, or if nothing loads.
    fn after_theme_written(&mut self, id: &str) {
        if id == self.config.theme {
            self.load_theme();
        } else if self.theme_problem.is_some() {
            self.set_theme(id);
        }
    }

    /// `/theme reload`: re-read the selected file (and download the default if there are none).
    pub fn reload_themes(&mut self) {
        let before = self.theme_problem;
        self.init_themes();
        if self.theme_problem.is_none() {
            self.info(format!(
                "Reloaded theme '{}'{}.",
                self.config.theme,
                if before.is_some() { " (fixed)" } else { "" }
            ));
        }
    }

    /// Ask before deleting a theme file (Cancel is the default).
    pub fn confirm_delete_theme(&mut self, id: &str) {
        let p = themes::path(&self.themes_dir(), id);
        if !themes::valid_id(id) || !p.exists() {
            self.error(format!("No theme '{id}' ({} doesn't exist).", p.display()));
            return;
        }
        let items = vec![
            PickItem::new("Cancel", "cancel"),
            PickItem::new(format!("Delete {}", p.display()), format!("delete:{id}")),
        ];
        self.open_picker(Picker::new(
            format!("Delete theme '{id}'?"),
            items,
            on_delete_pick,
        ));
    }

    /// Delete a theme file. Deleting the selected theme switches to `default` (or the first
    /// remaining theme), else falls back to plain colors with a notice.
    pub fn delete_theme_now(&mut self, id: &str) {
        let dir = self.themes_dir();
        if let Err(e) = themes::delete(&dir, id) {
            self.error(format!(
                "Couldn't delete {}: {e}",
                themes::path(&dir, id).display()
            ));
            return;
        }
        self.info(format!("Deleted {}.", themes::path(&dir, id).display()));
        let rest = themes::list(&dir);
        if id == self.config.theme {
            let next = rest
                .iter()
                .find(|t| t.as_str() == themes::DEFAULT_ID)
                .or(rest.first())
                .cloned();
            match next {
                Some(n) => {
                    self.set_theme(&n);
                }
                None => {
                    self.load_theme();
                }
            }
        }
        if self.editor.as_ref().is_some_and(|e| e.theme.id == id) {
            if rest.is_empty() {
                self.editor = None;
            } else {
                let next = if rest.contains(&self.config.theme) {
                    self.config.theme.clone()
                } else {
                    rest[0].clone()
                };
                self.open_theme_editor(Some(&next));
            }
        }
    }

    // ------------------------------------------------------------ editor (app side)

    /// Open the editor on a theme (default: the selected one). A missing file opens a
    /// starter to save; a broken one opens a starter and saving asks before replacing it.
    pub fn open_theme_editor(&mut self, id: Option<&str>) {
        let id = id
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| self.config.theme.clone());
        if !themes::valid_id(&id) {
            self.error(format!("'{id}' isn't a valid theme name."));
            return;
        }
        let dir = self.themes_dir();
        let variant = self.theme_variant();
        let ed = match themes::load(&dir, &id) {
            Ok((t, w)) => {
                let mut e = ThemeEditor::new(
                    t.clone(),
                    Some(t),
                    themes::read_text(&dir, &id),
                    None,
                    variant,
                );
                if let Some(first) = w.first() {
                    e.status = Some((
                        format!(
                            "{} warning(s) on load (saving rewrites the file cleanly): {first}",
                            w.len()
                        ),
                        true,
                    ));
                }
                e
            }
            Err(LoadError::Missing(p)) => {
                let mut e = ThemeEditor::new(starter(&id), None, None, None, variant);
                e.status = Some((
                    format!(
                        "No {} yet: this is a starter (every color reset). s saves it.",
                        p.display()
                    ),
                    false,
                ));
                e
            }
            Err(LoadError::Broken(_, err)) => {
                let mut e = ThemeEditor::new(
                    starter(&id),
                    None,
                    themes::read_text(&dir, &id),
                    Some(err.clone()),
                    variant,
                );
                e.status = Some((
                    format!(
                        "The file is broken ({err}); this is a fresh starter. Saving asks before replacing it."
                    ),
                    true,
                ));
                e
            }
        };
        self.editor = Some(ed);
    }

    /// Keys while the editor is open.
    pub fn editor_key(&mut self, k: KeyEvent) {
        let Some(ed) = self.editor.as_mut() else {
            return;
        };
        let action = ed.key(k);
        self.editor_action(action);
    }

    fn editor_action(&mut self, action: Action) {
        let dir = self.themes_dir();
        let Some(ed) = self.editor.as_mut() else {
            return;
        };
        match action {
            Action::None => {}
            Action::Save => self.editor_save(),
            Action::Close => {
                if ed.dirty() {
                    let items = vec![
                        PickItem::new("Cancel (keep editing)", "cancel"),
                        PickItem::new("Save and close", "save"),
                        PickItem::new("Discard changes and close", "discard"),
                    ];
                    self.open_picker(Picker::new("Unsaved changes", items, on_close_pick));
                } else {
                    self.editor = None;
                }
            }
            Action::Create { name, duplicate } => self.editor_create(&name, duplicate),
            Action::Delete => {
                let id = ed.theme.id.clone();
                if themes::path(&dir, &id).exists() {
                    self.confirm_delete_theme(&id);
                } else {
                    ed.status = Some((
                        "Not saved yet: nothing to delete (Esc discards).".into(),
                        true,
                    ));
                }
            }
            Action::RevertAll => {
                if ed.dirty() {
                    let items = vec![
                        PickItem::new("Cancel", "cancel"),
                        PickItem::new("Revert all changes", "revert"),
                    ];
                    self.open_picker(Picker::new("Revert all changes?", items, on_revert_pick));
                } else {
                    ed.status = Some(("Nothing to revert.".into(), false));
                }
            }
            Action::Step(by) => {
                if ed.dirty() {
                    ed.status = Some(("Save (s) or revert (R) first.".into(), true));
                    return;
                }
                let ids = themes::list(&dir);
                let here = ids.iter().position(|id| *id == ed.theme.id);
                let next = match (here, ids.len() as i32) {
                    (_, 0) => None,
                    (Some(i), n) => Some((i as i32 + by).rem_euclid(n) as usize),
                    (None, n) => Some(if by > 0 { 0 } else { n as usize - 1 }),
                };
                match next.map(|i| ids[i].clone()) {
                    Some(id) if id != ed.theme.id => {
                        let variant = ed.variant;
                        self.open_theme_editor(Some(&id));
                        if let Some(ed) = self.editor.as_mut() {
                            ed.variant = variant;
                        }
                    }
                    _ => ed.status = Some(("No other themes; n creates one.".into(), true)),
                }
            }
            Action::OpenOther => {
                if ed.dirty() {
                    ed.status = Some(("Save (s) or revert (R) first.".into(), true));
                    return;
                }
                let current = ed.theme.id.clone();
                let items: Vec<PickItem> = themes::list(&dir)
                    .into_iter()
                    .map(|id| PickItem::new(id.clone(), id))
                    .collect();
                if items.is_empty() {
                    ed.status = Some(("No other themes; n creates one.".into(), true));
                    return;
                }
                self.open_picker(
                    Picker::new("Edit theme", items, |app, it| {
                        app.open_theme_editor(Some(&it.value))
                    })
                    .with_selected(&current),
                );
            }
        }
    }

    /// Save, asking first when the file on disk is broken or changed since it was opened.
    pub fn editor_save(&mut self) {
        let dir = self.themes_dir();
        let Some(ed) = self.editor.as_ref() else {
            return;
        };
        let on_disk = themes::read_text(&dir, &ed.theme.id);
        if ed.broken.is_some() && on_disk.is_some() {
            let items = vec![
                PickItem::new("Cancel", "cancel"),
                PickItem::new("Replace the broken file", "overwrite"),
            ];
            self.open_picker(Picker::new(
                format!("{}.toml is broken", ed.theme.id),
                items,
                on_conflict_pick,
            ));
        } else if on_disk != ed.disk_text {
            let items = vec![
                PickItem::new("Cancel", "cancel"),
                PickItem::new("Overwrite the file on disk", "overwrite"),
                PickItem::new("Reload from disk (drop my changes)", "reload"),
            ];
            self.open_picker(Picker::new(
                format!("{}.toml changed on disk", ed.theme.id),
                items,
                on_conflict_pick,
            ));
        } else {
            self.editor_save_now();
        }
    }

    /// Write the edited theme; apply it right away when it's the selected theme.
    pub fn editor_save_now(&mut self) {
        let dir = self.themes_dir();
        let Some(ed) = self.editor.as_mut() else {
            return;
        };
        match themes::save(&dir, &ed.theme) {
            Ok(p) => {
                ed.saved = Some(ed.theme.clone());
                ed.disk_text = themes::read_text(&dir, &ed.theme.id);
                ed.broken = None;
                ed.status = Some((format!("Saved {}.", p.display()), false));
                if ed.theme.id == self.config.theme {
                    let t = ed.theme.clone();
                    self.apply_theme(t);
                }
            }
            Err(e) => ed.status = Some((format!("Couldn't save: {e}"), true)),
        }
    }

    fn editor_create(&mut self, name: &str, duplicate: bool) {
        let dir = self.themes_dir();
        let Some(ed) = self.editor.as_mut() else {
            return;
        };
        if themes::path(&dir, name).exists() {
            ed.edit = Edit::Name {
                duplicate,
                buf: name.to_string(),
                err: Some(format!("{name}.toml already exists")),
            };
            return;
        }
        let t = if duplicate {
            let mut t = ed.theme.clone();
            t.id = name.to_string();
            t.name = format!("{} (copy)", t.name);
            t
        } else {
            starter(name)
        };
        match themes::save(&dir, &t) {
            Ok(p) => {
                self.open_theme_editor(Some(name));
                if let Some(ed) = self.editor.as_mut() {
                    ed.status = Some((format!("Created {}.", p.display()), false));
                }
            }
            Err(e) => ed.status = Some((format!("Couldn't create {name}.toml: {e}"), true)),
        }
    }

    /// Mouse click in the editor.
    pub fn editor_click(&mut self, col: u16, row: u16) {
        if let Some(ed) = self.editor.as_mut() {
            ed.click(col, row);
        }
    }

    /// Mouse wheel in the editor.
    pub fn editor_scroll(&mut self, delta: i32) {
        if let Some(ed) = self.editor.as_mut() {
            ed.scroll(delta);
        }
    }
}

fn starter(id: &str) -> Theme {
    let mut t = Theme::unstyled(id);
    t.description =
        "Starter theme: every color is the terminal default. Set colors with Shift-F5.".into();
    t
}

/// Draw with the highlighted theme while the picker is open (nothing is saved).
fn preview_theme_pick(app: &mut App) {
    let Some((orig, problem)) = app.theme_preview.clone() else {
        return;
    };
    let id = app
        .picker
        .as_ref()
        .and_then(|p| p.selected())
        .map(|i| i.value.clone())
        .unwrap_or_default();
    match themes::load(&app.themes_dir(), &id) {
        Ok((t, _)) => app.theme = Arc::new(t),
        Err(_) => {
            app.theme = orig;
            app.theme_problem = problem;
        }
    }
}

fn cancel_theme_pick(app: &mut App) {
    app.end_theme_preview();
}

/// `~/x` → the home folder (paths typed in the app aren't expanded by a shell).
fn expand_tilde(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ if p == "~" => dirs::home_dir().unwrap_or_else(|| PathBuf::from(p)),
        _ => PathBuf::from(p),
    }
}

fn on_theme_pick(app: &mut App, it: PickItem) {
    app.end_theme_preview();
    match it.value.as_str() {
        EDIT_ITEM => app.open_theme_editor(None),
        INIT_ITEM => app.theme_init(None),
        id => {
            app.set_theme(id);
        }
    }
}

fn on_delete_pick(app: &mut App, it: PickItem) {
    if let Some(id) = it.value.strip_prefix("delete:") {
        app.delete_theme_now(id);
    }
}

fn on_close_pick(app: &mut App, it: PickItem) {
    match it.value.as_str() {
        "save" => {
            app.editor_save();
            if app.picker.is_none() && app.editor.as_ref().is_some_and(|e| !e.dirty()) {
                app.editor = None;
            }
        }
        "discard" => app.editor = None,
        _ => {}
    }
}

fn on_revert_pick(app: &mut App, it: PickItem) {
    if it.value == "revert"
        && let Some(ed) = app.editor.as_mut()
    {
        ed.revert_all();
        ed.status = Some(("Reverted all changes.".into(), false));
    }
}

fn on_conflict_pick(app: &mut App, it: PickItem) {
    match it.value.as_str() {
        "overwrite" => app.editor_save_now(),
        "reload" => {
            if let Some(id) = app.editor.as_ref().map(|e| e.theme.id.clone()) {
                app.open_theme_editor(Some(&id));
            }
        }
        _ => {}
    }
}

// ================================================================ editor state

/// Rows in the editor's list: the palette colors, then the style parts.
pub const N_ROWS: usize = N_COLORS + N_STYLES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Color(usize),
    Style(usize),
}

pub fn row_at(i: usize) -> Row {
    if i < N_COLORS {
        Row::Color(i)
    } else {
        Row::Style(i - N_COLORS)
    }
}

/// What a color edit changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Color(usize),
    Fg(usize),
    Bg(usize),
    Border(usize),
}

/// The style form's fields: three colors, then the attributes.
pub const FIELDS: [&str; 8] = [
    "fg",
    "bg",
    "border",
    "bold",
    "italic",
    "underline",
    "reverse",
    "dim",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    None,
    /// The style form for the selected part, focused on a field.
    Form {
        field: usize,
    },
    /// Typing a color.
    Text {
        target: Target,
        buf: String,
        err: Option<String>,
    },
    /// The color picker: a tab of [`PICKER_TABS`] and the selected cell in it.
    Palette {
        target: Target,
        tab: usize,
        cursor: usize,
    },
    /// Naming a new or duplicated theme.
    Name {
        duplicate: bool,
        buf: String,
        err: Option<String>,
    },
}

/// What the app has to do after an editor key.
/// The color picker's tabs.
pub const PICKER_TABS: [&str; 4] = ["16 colors", "216 cube", "24 grays", "@ palette"];

/// The 16 ANSI colors, in terminal order (the picker's first tab).
const ANSI16: [Color; 16] = [
    Color::Black,
    Color::Red,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::Gray,
    Color::DarkGray,
    Color::LightRed,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightBlue,
    Color::LightMagenta,
    Color::LightCyan,
    Color::White,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Save,
    Close,
    Create {
        name: String,
        duplicate: bool,
    },
    Delete,
    RevertAll,
    OpenOther,
    /// `[` / `]`: the previous / next theme file.
    Step(i32),
}

pub struct ThemeEditor {
    /// The working copy.
    pub theme: Theme,
    /// What's on disk (`None`: not saved yet).
    pub saved: Option<Theme>,
    /// The file's text when opened or saved (to notice changes made elsewhere).
    pub disk_text: Option<String>,
    /// The file on disk didn't parse (saving asks before replacing it).
    pub broken: Option<String>,
    pub variant: Variant,
    pub cursor: usize,
    pub offset: usize,
    pub edit: Edit,
    /// Status line: message and whether it's an error.
    pub status: Option<(String, bool)>,
    // Hit areas, set by the renderer.
    pub list_area: Rect,
    /// Row index per visible list line (`None` for group headings).
    pub list_rows: Vec<Option<usize>>,
    pub tab_areas: Vec<(Rect, Variant)>,
    pub field_areas: Vec<(Rect, usize)>,
    pub palette_area: Rect,
    pub palette_cols: u16,
    pub palette_cell_w: u16,
    /// Hit areas of the color picker's tabs.
    pub palette_tab_areas: Vec<(Rect, usize)>,
    /// Mark the selected part's cells in the preview (`h` toggles).
    pub highlight: bool,
    pub preview_area: Rect,
}

impl ThemeEditor {
    pub fn new(
        theme: Theme,
        saved: Option<Theme>,
        disk_text: Option<String>,
        broken: Option<String>,
        variant: Variant,
    ) -> Self {
        Self {
            theme,
            saved,
            disk_text,
            broken,
            variant,
            cursor: 0,
            offset: 0,
            edit: Edit::None,
            status: None,
            list_area: Rect::default(),
            list_rows: vec![],
            tab_areas: vec![],
            field_areas: vec![],
            palette_area: Rect::default(),
            palette_cols: 16,
            palette_cell_w: 3,
            palette_tab_areas: Vec::new(),
            highlight: true,
            preview_area: Rect::default(),
        }
    }

    /// Unsaved changes (a theme that was never saved always counts).
    pub fn dirty(&self) -> bool {
        self.saved.as_ref() != Some(&self.theme)
    }

    pub fn var(&self) -> &VariantSpec {
        self.theme.variant(self.variant)
    }

    fn var_mut(&mut self) -> &mut VariantSpec {
        self.theme.variant_mut(self.variant)
    }

    pub fn row(&self) -> Row {
        row_at(self.cursor)
    }

    /// The selected item's name (`accent`, `panel_focused`, …).
    pub fn row_name(&self) -> &'static str {
        match self.row() {
            Row::Color(c) => COLOR_SLOTS[c],
            Row::Style(s) => STYLE_SLOTS[s].key,
        }
    }

    /// A copy of the theme where the selected palette color or part is drawn in colors the
    /// variant doesn't use anywhere, so diffing a render of it against the normal render
    /// finds exactly the cells that item paints.
    pub fn marked_theme(&self) -> Theme {
        let mut t = self.theme.clone();
        let v = t.variant_mut(self.variant);
        let used = |c: Color, v: &VariantSpec| {
            v.colors.contains(&c)
                || v.styles
                    .iter()
                    .any(|s| [s.fg, s.bg, s.border].contains(&Some(ColorRef::Lit(c))))
        };
        let mut free = (1u8..=250)
            .step_by(3)
            .map(|n| Color::Rgb(n, 255 - n, n / 2 + 7))
            .filter(|c| !used(*c, v));
        let (a, b) = (
            free.next().unwrap_or(Color::Rgb(1, 2, 3)),
            free.next().unwrap_or(Color::Rgb(3, 2, 1)),
        );
        match self.row() {
            Row::Color(c) => v.colors[c] = a,
            Row::Style(s) => {
                let st = &mut v.styles[s];
                st.fg = Some(ColorRef::Lit(a));
                st.bg = Some(ColorRef::Lit(b));
                if STYLE_SLOTS[s].framed {
                    st.border = Some(ColorRef::Lit(a));
                }
            }
        }
        t
    }

    pub fn move_by(&mut self, d: i32) {
        self.cursor = (self.cursor as i32 + d).clamp(0, N_ROWS as i32 - 1) as usize;
    }

    pub fn flip_variant(&mut self) {
        self.variant = match self.variant {
            Variant::Dark => Variant::Light,
            Variant::Light => Variant::Dark,
        };
    }

    pub fn target_value(&self, t: Target) -> Option<ColorRef> {
        let v = self.var();
        match t {
            Target::Color(i) => Some(ColorRef::Lit(v.colors[i])),
            Target::Fg(s) => v.styles[s].fg,
            Target::Bg(s) => v.styles[s].bg,
            Target::Border(s) => v.styles[s].border,
        }
    }

    pub fn set_target(&mut self, t: Target, val: Option<ColorRef>) {
        let v = self.var_mut();
        match t {
            Target::Color(i) => {
                if let Some(ColorRef::Lit(c)) = val {
                    v.colors[i] = c;
                }
            }
            Target::Fg(s) => v.styles[s].fg = val,
            Target::Bg(s) => v.styles[s].bg = val,
            Target::Border(s) => v.styles[s].border = val,
        }
    }

    fn field_target(slot: usize, field: usize) -> Option<Target> {
        match field {
            0 => Some(Target::Fg(slot)),
            1 => Some(Target::Bg(slot)),
            2 => Some(Target::Border(slot)),
            _ => None,
        }
    }

    /// Where an edit of `t` returns to.
    fn back_from(t: Target) -> Edit {
        match t {
            Target::Color(_) => Edit::None,
            Target::Fg(_) => Edit::Form { field: 0 },
            Target::Bg(_) => Edit::Form { field: 1 },
            Target::Border(_) => Edit::Form { field: 2 },
        }
    }

    /// Enter: type a palette color, or open the style form.
    pub fn start_edit(&mut self) {
        self.edit = match self.row() {
            Row::Color(i) => Edit::Text {
                target: Target::Color(i),
                buf: themes::format_color(self.var().colors[i]),
                err: None,
            },
            Row::Style(_) => Edit::Form { field: 0 },
        };
    }

    fn start_text(&mut self, t: Target) {
        let buf = self
            .target_value(t)
            .map(themes::format_color_ref)
            .unwrap_or_default();
        self.edit = Edit::Text {
            target: t,
            buf,
            err: None,
        };
    }

    /// Apply the typed color. Palette colors take a literal; style colors also take
    /// `@palette` or nothing (none).
    pub fn apply_text(&mut self) -> bool {
        let Edit::Text { target, buf, .. } = &self.edit else {
            return false;
        };
        let (target, text) = (*target, buf.trim().to_string());
        let parsed = match target {
            Target::Color(_) if text.starts_with('@') => {
                Err("palette colors take a literal color, not @name".to_string())
            }
            Target::Color(_) => themes::parse_color(&text).map(|c| Some(ColorRef::Lit(c))),
            _ if text.is_empty() || text == "none" || text == "-" => Ok(None),
            _ => themes::parse_color_ref(&text).map(Some),
        };
        match parsed {
            Ok(v) => {
                self.set_target(target, v);
                self.edit = Self::back_from(target);
                true
            }
            Err(e) => {
                if let Edit::Text { err, .. } = &mut self.edit {
                    *err = Some(e);
                }
                false
            }
        }
    }

    /// The cells of picker tab `tab` for target `t`. The `@ palette` tab offers the
    /// palette colors (parts only; palette colors themselves take literals) and `reset`.
    pub fn grid(t: Target, tab: usize) -> Vec<ColorRef> {
        match tab {
            0 => ANSI16.iter().map(|c| ColorRef::Lit(*c)).collect(),
            1 => (16u8..=231)
                .map(|i| ColorRef::Lit(Color::Indexed(i)))
                .collect(),
            2 => (232u8..=255)
                .map(|i| ColorRef::Lit(Color::Indexed(i)))
                .collect(),
            _ => {
                let mut v: Vec<ColorRef> = match t {
                    Target::Color(_) => vec![],
                    _ => (0..N_COLORS).map(ColorRef::Slot).collect(),
                };
                v.push(ColorRef::Lit(Color::Reset));
                v
            }
        }
    }

    /// Columns of picker tab `tab` (1: a list).
    pub fn grid_cols(tab: usize) -> usize {
        match tab {
            0 => 8,
            1 => 18,
            2 => 12,
            _ => 1,
        }
    }

    /// Open the color picker on the tab and cell holding the current value.
    pub fn open_palette(&mut self, t: Target) {
        let cur = self.target_value(t).map(|r| match r {
            // 0-15 are the same colors as the named ones in the first tab.
            ColorRef::Lit(Color::Indexed(n)) if n < 16 => ColorRef::Lit(ANSI16[n as usize]),
            other => other,
        });
        let (tab, cursor) = cur
            .and_then(|c| {
                (0..PICKER_TABS.len()).find_map(|tab| {
                    Self::grid(t, tab)
                        .iter()
                        .position(|g| *g == c)
                        .map(|i| (tab, i))
                })
            })
            .unwrap_or((0, 0));
        self.edit = Edit::Palette {
            target: t,
            tab,
            cursor,
        };
    }

    /// Set the picker's target to cell `idx` of its current tab and close the picker.
    pub fn palette_pick(&mut self, idx: usize) {
        if let Edit::Palette { target, tab, .. } = self.edit
            && let Some(c) = Self::grid(target, tab).get(idx)
        {
            self.set_target(target, Some(*c));
            self.edit = Self::back_from(target);
        }
    }

    /// Move the picker's selection by `d` cells, or switch tab by `dt`.
    fn palette_move(&mut self, d: i32, dt: i32) {
        if let Edit::Palette {
            target,
            tab,
            cursor,
        } = self.edit
        {
            let n = PICKER_TABS.len() as i32;
            let tab = (tab as i32 + dt).rem_euclid(n) as usize;
            let len = Self::grid(target, tab).len() as i32;
            let cursor = if dt != 0 {
                0
            } else {
                (cursor as i32 + d).clamp(0, len - 1) as usize
            };
            self.edit = Edit::Palette {
                target,
                tab,
                cursor,
            };
        }
    }

    fn framed(&self) -> bool {
        matches!(self.row(), Row::Style(s) if STYLE_SLOTS[s].framed)
    }

    /// Move the form focus, skipping `border` for parts without a box.
    pub fn form_move(&mut self, d: i32) {
        if let Edit::Form { field } = self.edit {
            let framed = self.framed();
            let mut f = field as i32;
            loop {
                f = (f + d).rem_euclid(FIELDS.len() as i32);
                if f != 2 || framed {
                    break;
                }
            }
            self.edit = Edit::Form { field: f as usize };
        }
    }

    /// Enter / Space in the form: type a color, or toggle an attribute.
    pub fn form_activate(&mut self) {
        let (Edit::Form { field }, Row::Style(slot)) = (self.edit.clone(), self.row()) else {
            return;
        };
        match Self::field_target(slot, field) {
            Some(t) => self.start_text(t),
            None => {
                let i = field - 3;
                let on = !self.var().styles[slot].attr(i);
                self.var_mut().styles[slot].set_attr(i, on);
            }
        }
    }

    /// Clear the focused color field (none: inherit the surrounding color).
    pub fn form_clear(&mut self) {
        if let (Edit::Form { field }, Row::Style(slot)) = (self.edit.clone(), self.row())
            && let Some(t) = Self::field_target(slot, field)
        {
            self.set_target(t, None);
        }
    }

    /// Put the selected color / part back to its saved value (starter rule if unsaved).
    pub fn revert_selected(&mut self) {
        let saved = self
            .saved
            .as_ref()
            .map(|s| s.variant(self.variant).clone())
            .unwrap_or_else(VariantSpec::starter);
        match self.row() {
            Row::Color(i) => self.var_mut().colors[i] = saved.colors[i],
            Row::Style(s) => self.var_mut().styles[s] = saved.styles[s],
        }
    }

    pub fn revert_all(&mut self) {
        self.theme = self
            .saved
            .clone()
            .unwrap_or_else(|| starter(&self.theme.id));
    }

    /// Whether a row differs from the saved file (in the shown variant).
    pub fn changed(&self, i: usize) -> bool {
        let Some(saved) = &self.saved else {
            return true;
        };
        let (a, b) = (self.var(), saved.variant(self.variant));
        match row_at(i) {
            Row::Color(c) => a.colors[c] != b.colors[c],
            Row::Style(s) => a.styles[s] != b.styles[s],
        }
    }

    /// Contrast of the selected row: ratio and what it's measured against.
    pub fn contrast(&self) -> Option<(f64, String)> {
        self.contrast_of(self.cursor)
    }

    /// Row `i` is readable on its own background by less than 3:1 (⚠ in the list).
    pub fn low_contrast(&self, i: usize) -> bool {
        self.contrast_of(i).is_some_and(|(r, _)| r < 3.0)
    }

    /// Contrast of row `i`: ratio and what it's measured against.
    pub fn contrast_of(&self, i: usize) -> Option<(f64, String)> {
        let v = self.var();
        let c = |i: usize| v.colors[i];
        let (a, b, what) = match row_at(i) {
            Row::Color(i) => {
                let (other, what) = match i {
                    color::TEXT => (color::BACKGROUND, "background"),
                    color::BACKGROUND => (color::TEXT, "text"),
                    color::ON_ACCENT => (color::ACCENT, "accent"),
                    color::MODAL_TEXT => (color::MODAL_BACKGROUND, "modal_background"),
                    color::MODAL_BACKGROUND => (color::MODAL_TEXT, "modal_text"),
                    color::HEADER => (color::TEXT, "text"),
                    color::LOGO_DARK => (color::LOGO_LIGHT, "logo_light"),
                    color::LOGO_LIGHT => (color::LOGO_DARK, "logo_dark"),
                    color::ACCENT => (color::ON_ACCENT, "on_accent"),
                    _ => (color::BACKGROUND, "background"),
                };
                (c(i), c(other), what.to_string())
            }
            Row::Style(s) => {
                let r = v.resolve(s);
                let info = STYLE_SLOTS[s];
                let (behind, what) = if info.area == "Modals" {
                    (c(color::MODAL_BACKGROUND), "modal_background")
                } else if info.area == "Screen and header" && s != theme::slot::BASE {
                    (c(color::HEADER), "header")
                } else {
                    (c(color::BACKGROUND), "background")
                };
                if info.framed {
                    (r.border?, behind, what.to_string())
                } else {
                    let fg = r.fg.unwrap_or(c(color::TEXT));
                    match r.bg {
                        Some(bg) => (fg, bg, "its bg".to_string()),
                        None => (fg, behind, what.to_string()),
                    }
                }
            }
        };
        themes::contrast(a, b).map(|r| (r, what))
    }

    /// A note when the selected row uses true color the terminal may not show.
    pub fn truecolor_note(&self) -> Option<&'static str> {
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        if colorterm.contains("truecolor") || colorterm.contains("24bit") {
            return None;
        }
        let v = self.var();
        let rgb = |c: Option<Color>| matches!(c, Some(Color::Rgb(..)));
        let uses = match self.row() {
            Row::Color(i) => rgb(Some(v.colors[i])),
            Row::Style(s) => {
                let r = v.resolve(s);
                rgb(r.fg) || rgb(r.bg) || rgb(r.border)
            }
        };
        uses.then_some("#rrggbb needs a true-color terminal ($COLORTERM isn't truecolor)")
    }

    /// Handle a key; returns what the app has to do.
    pub fn key(&mut self, k: KeyEvent) -> Action {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match self.edit.clone() {
            Edit::Text { target, .. } => {
                match k.code {
                    KeyCode::Esc => self.edit = Self::back_from(target),
                    KeyCode::Enter => {
                        self.apply_text();
                    }
                    KeyCode::Backspace => {
                        if let Edit::Text { buf, err, .. } = &mut self.edit {
                            buf.pop();
                            *err = None;
                        }
                    }
                    KeyCode::Char(ch) if !ctrl => {
                        if let Edit::Text { buf, err, .. } = &mut self.edit {
                            buf.push(ch);
                            *err = None;
                        }
                    }
                    _ => {}
                }
                Action::None
            }
            Edit::Name { duplicate, buf, .. } => {
                match k.code {
                    KeyCode::Esc => self.edit = Edit::None,
                    KeyCode::Enter => {
                        let name = buf.trim().to_string();
                        if themes::valid_id(&name) {
                            self.edit = Edit::None;
                            return Action::Create { name, duplicate };
                        }
                        self.edit = Edit::Name {
                            duplicate,
                            buf,
                            err: Some(
                                "use a-z, 0-9, _ and - (up to 32, starting with a letter or digit)"
                                    .into(),
                            ),
                        };
                    }
                    KeyCode::Backspace => {
                        let mut b = buf;
                        b.pop();
                        self.edit = Edit::Name {
                            duplicate,
                            buf: b,
                            err: None,
                        };
                    }
                    KeyCode::Char(ch) if !ctrl => {
                        let mut b = buf;
                        b.push(ch);
                        self.edit = Edit::Name {
                            duplicate,
                            buf: b,
                            err: None,
                        };
                    }
                    _ => {}
                }
                Action::None
            }
            Edit::Palette {
                target,
                tab,
                cursor,
            } => {
                let cols = Self::grid_cols(tab) as i32;
                match k.code {
                    KeyCode::Esc => self.edit = Self::back_from(target),
                    KeyCode::Enter | KeyCode::Char(' ') => self.palette_pick(cursor),
                    KeyCode::Tab => self.palette_move(0, 1),
                    KeyCode::BackTab => self.palette_move(0, -1),
                    KeyCode::Left | KeyCode::Char('h') => self.palette_move(-1, 0),
                    KeyCode::Right | KeyCode::Char('l') => self.palette_move(1, 0),
                    KeyCode::Up | KeyCode::Char('k') => self.palette_move(-cols, 0),
                    KeyCode::Down | KeyCode::Char('j') => self.palette_move(cols, 0),
                    KeyCode::Home => self.palette_move(-(cursor as i32), 0),
                    KeyCode::End => self.palette_move(i32::MAX / 2, 0),
                    _ => {}
                }
                Action::None
            }
            Edit::Form { field } => {
                match k.code {
                    KeyCode::Esc => self.edit = Edit::None,
                    KeyCode::Left | KeyCode::BackTab | KeyCode::Char('h') => self.form_move(-1),
                    KeyCode::Right | KeyCode::Tab | KeyCode::Char('l') => self.form_move(1),
                    KeyCode::Enter | KeyCode::Char(' ') => self.form_activate(),
                    KeyCode::Char('p') => {
                        if let Row::Style(slot) = self.row()
                            && let Some(t) = Self::field_target(slot, field)
                        {
                            self.open_palette(t);
                        }
                    }
                    KeyCode::Backspace | KeyCode::Delete => self.form_clear(),
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.edit = Edit::None;
                        self.move_by(-1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.edit = Edit::None;
                        self.move_by(1);
                    }
                    KeyCode::Char('s') => return Action::Save,
                    _ => {}
                }
                Action::None
            }
            Edit::None => match k.code {
                KeyCode::Char('s') if ctrl => Action::Save,
                KeyCode::Char('r') if ctrl => Action::RevertAll,
                KeyCode::Char('x') if ctrl => Action::Delete,
                KeyCode::Char('n' | 'd') if ctrl => {
                    self.edit = Edit::Name {
                        duplicate: k.code == KeyCode::Char('d'),
                        buf: String::new(),
                        err: None,
                    };
                    Action::None
                }
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::F(17) => Action::Close,
                KeyCode::F(5) if k.modifiers.contains(KeyModifiers::SHIFT) => Action::Close,
                KeyCode::Char('[') => Action::Step(-1),
                KeyCode::Char(']') => Action::Step(1),
                KeyCode::Up | KeyCode::Char('k') => {
                    self.move_by(-1);
                    Action::None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.move_by(1);
                    Action::None
                }
                KeyCode::PageUp => {
                    self.move_by(-10);
                    Action::None
                }
                KeyCode::PageDown => {
                    self.move_by(10);
                    Action::None
                }
                KeyCode::Home => {
                    self.cursor = 0;
                    Action::None
                }
                KeyCode::End => {
                    self.cursor = N_ROWS - 1;
                    Action::None
                }
                KeyCode::Tab | KeyCode::BackTab => {
                    self.flip_variant();
                    Action::None
                }
                KeyCode::Enter | KeyCode::Char('e') => {
                    self.start_edit();
                    Action::None
                }
                KeyCode::Char('p') => {
                    let t = match self.row() {
                        Row::Color(i) => Target::Color(i),
                        Row::Style(s) if STYLE_SLOTS[s].framed => Target::Border(s),
                        Row::Style(s) => Target::Fg(s),
                    };
                    self.open_palette(t);
                    Action::None
                }
                KeyCode::Char('r') => {
                    self.revert_selected();
                    Action::None
                }
                KeyCode::Char('R') => Action::RevertAll,
                KeyCode::Char('s') => Action::Save,
                KeyCode::Char('d') | KeyCode::Char('n') => {
                    self.edit = Edit::Name {
                        duplicate: k.code == KeyCode::Char('d'),
                        buf: String::new(),
                        err: None,
                    };
                    Action::None
                }
                KeyCode::Char('x') | KeyCode::Delete => Action::Delete,
                KeyCode::Char('t') => Action::OpenOther,
                KeyCode::Char('h') => {
                    self.highlight = !self.highlight;
                    Action::None
                }
                _ => Action::None,
            },
        }
    }

    pub fn click(&mut self, col: u16, row: u16) {
        let at = Position::new(col, row);
        if let Some(&(_, v)) = self.tab_areas.iter().find(|(r, _)| r.contains(at)) {
            self.variant = v;
            return;
        }
        if let Edit::Palette { target, tab, .. } = self.edit {
            if let Some(&(_, t)) = self.palette_tab_areas.iter().find(|(r, _)| r.contains(at)) {
                self.edit = Edit::Palette {
                    target,
                    tab: t,
                    cursor: 0,
                };
            } else if self.palette_area.contains(at) {
                let cx = (col - self.palette_area.x) / self.palette_cell_w.max(1);
                let cy = row - self.palette_area.y;
                let idx = cy as usize * self.palette_cols as usize + cx as usize;
                if cx < self.palette_cols && idx < Self::grid(target, tab).len() {
                    self.palette_pick(idx);
                }
            }
            return;
        }
        if let Some(&(_, field)) = self.field_areas.iter().find(|(r, _)| r.contains(at)) {
            self.edit = Edit::Form { field };
            self.form_activate();
            return;
        }
        if self.list_area.contains(at) {
            let line = (row - self.list_area.y) as usize;
            if let Some(Some(i)) = self.list_rows.get(line) {
                if *i == self.cursor && self.edit == Edit::None {
                    self.start_edit();
                } else {
                    self.cursor = *i;
                    self.edit = Edit::None;
                }
            }
        }
    }

    pub fn scroll(&mut self, delta: i32) {
        if let Edit::Palette { .. } = self.edit {
            self.palette_move(delta * self.palette_cols.max(1) as i32, 0);
        } else {
            self.move_by(delta);
        }
    }
}

// ================================================================ drawing

fn swatch(c: Color) -> Span<'static> {
    if c == Color::Reset {
        Span::raw("··")
    } else {
        Span::styled("██", ratatui::style::Style::default().fg(c))
    }
}

/// The editor: full screen, drawn with the app's own theme; the preview uses the edited one.
pub fn draw(f: &mut Frame, app: &mut App) {
    let Some(mut ed) = app.editor.take() else {
        return;
    };
    let area = f.area();
    let modal = theme::modal_text();
    f.render_widget(Clear, area);
    let dirty = if ed.dirty() { " ● modified" } else { "" };
    let block = theme::block(&theme::help_frame())
        .title(theme::help_title().span(format!(
            " Theme editor: {} ({}.toml){dirty} ",
            ed.theme.name, ed.theme.id
        )))
        .style(modal.clone());
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [tabs, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(2),
    ])
    .areas(inner);
    let list_w = 46.min(body.width / 2);
    let [list, right] =
        Layout::horizontal([Constraint::Length(list_w), Constraint::Min(10)]).areas(body);
    let [preview_box, detail_box] =
        Layout::vertical([Constraint::Min(4), Constraint::Length(7)]).areas(right);

    // Tabs.
    ed.tab_areas.clear();
    let mut x = tabs.x;
    let mut spans = vec![];
    for (label, v) in [(" Dark ", Variant::Dark), (" Light ", Variant::Light)] {
        let w = label.width() as u16;
        ed.tab_areas.push((
            Rect::new(x, tabs.y, w.min(tabs.right().saturating_sub(x)), 1),
            v,
        ));
        spans.push(theme::mode_choice(ed.variant == v).span(label));
        spans.push(Span::raw(" "));
        x += w + 1;
    }
    let file = themes::path(&app.themes_dir(), &ed.theme.id);
    spans.push(theme::key_hint().span(format!(" Tab flips · {}", file.display())));
    f.render_widget(Paragraph::new(Line::from(spans)), tabs);

    draw_list(f, &mut ed, list);
    if let Edit::Palette {
        target,
        tab,
        cursor,
    } = ed.edit
    {
        draw_palette(f, &mut ed, preview_box, target, tab, cursor);
    } else {
        ed.preview_area = preview_box;
        draw_preview(f, app, &ed, preview_box);
    }
    draw_detail(f, &mut ed, detail_box);

    let keys = match &ed.edit {
        Edit::None => [
            "↑↓ select · Enter edit · p palette · Tab dark/light · r revert part · R revert all · s save",
            "d duplicate · n new · x delete · t / [ ] other theme · h highlight · ⚠ low contrast · Esc close",
        ],
        Edit::Form { .. } => [
            "←→ field · Enter/Space edit or toggle · p palette · Del clear color · ↑↓ other part",
            "s save · Esc done",
        ],
        Edit::Text { .. } => [
            "Type 0-255, #rrggbb, #rgb, a name (cyan, darkgray…) or reset; styles also take @palette or empty",
            "Enter apply · Esc cancel",
        ],
        Edit::Palette { .. } => [
            "Tab / Shift-Tab: 16 colors · 216 cube · 24 grays · @ palette · arrows move · Enter/click pick",
            "Esc cancel",
        ],
        Edit::Name { .. } => ["Theme name: a-z, 0-9, _ and -", "Enter create · Esc cancel"],
    };
    f.render_widget(
        Paragraph::new(vec![
            theme::key_hint().line(keys[0]),
            theme::key_hint().line(keys[1]),
        ]),
        footer,
    );
    app.editor = Some(ed);
}

fn draw_list(f: &mut Frame, ed: &mut ThemeEditor, area: Rect) {
    let block = theme::block(&theme::panel(true)).title(" Colors and parts ");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let v = ed.var().clone();
    let mut lines: Vec<(Option<usize>, Line)> = vec![(None, theme::help_title().line("Palette"))];
    for (i, name) in COLOR_SLOTS.iter().enumerate() {
        let sel = i == ed.cursor;
        let lead = format!(
            "{}{}{name:<17}{}",
            if sel { "›" } else { " " },
            if ed.changed(i) { "●" } else { " " },
            if ed.low_contrast(i) { "⚠" } else { " " }
        );
        let lead_style = if sel {
            theme::selected()
        } else {
            theme::modal_text()
        };
        lines.push((
            Some(i),
            Line::from(vec![
                lead_style.span(lead),
                Span::raw(" "),
                swatch(v.colors[i]),
                Span::raw(format!(" {}", themes::format_color(v.colors[i]))),
            ]),
        ));
    }
    let mut group = "";
    for (s, info) in STYLE_SLOTS.iter().enumerate() {
        if info.area != group {
            group = info.area;
            lines.push((None, theme::help_title().line(group)));
        }
        let i = N_COLORS + s;
        let sel = i == ed.cursor;
        let lead = format!(
            "{}{}{:<19}{}",
            if sel { "›" } else { " " },
            if ed.changed(i) { "●" } else { " " },
            info.key,
            if ed.low_contrast(i) { "⚠" } else { " " }
        );
        let lead_style = if sel {
            theme::selected()
        } else {
            theme::modal_text()
        };
        let sample = if info.framed {
            let b = v.resolve(s).border.unwrap_or(Color::Reset);
            Span::styled("┌──┐", ratatui::style::Style::default().fg(b))
        } else {
            v.style(s).span(" Aa ")
        };
        lines.push((Some(i), Line::from(vec![lead_style.span(lead), sample])));
    }
    let h = inner.height as usize;
    let cur_line = lines
        .iter()
        .position(|(r, _)| *r == Some(ed.cursor))
        .unwrap_or(0);
    if cur_line < ed.offset {
        ed.offset = cur_line.saturating_sub(1);
    } else if cur_line >= ed.offset + h {
        ed.offset = cur_line + 1 - h;
    }
    ed.offset = ed.offset.min(lines.len().saturating_sub(h));
    let shown: Vec<(Option<usize>, Line)> = lines.into_iter().skip(ed.offset).take(h).collect();
    ed.list_rows = shown.iter().map(|(r, _)| *r).collect();
    ed.list_area = inner;
    f.render_widget(
        Paragraph::new(shown.into_iter().map(|(_, l)| l).collect::<Vec<_>>()),
        inner,
    );
}

/// The live preview: the real screen (header, sidebar, chat, input) drawn off-screen with
/// the edited theme, plus sample modals, copied into `area`.
fn draw_preview(f: &mut Frame, app: &mut App, ed: &ThemeEditor, area: Rect) {
    let variant_name = match ed.variant {
        Variant::Dark => "dark",
        Variant::Light => "light",
    };
    let p = theme::block(&theme::panel(false)).inner(area);
    if p.width < 10 || p.height < 4 {
        f.render_widget(theme::block(&theme::panel(false)), area);
        return;
    }
    let mut shown = preview_buffer(app, &ed.theme, ed.variant, p.width, p.height);
    let mut marked_cells = 0;
    if ed.highlight {
        let marked = preview_buffer(app, &ed.marked_theme(), ed.variant, p.width, p.height);
        marked_cells = highlight_changes(&mut shown, &marked);
    }
    let note = match (ed.highlight, marked_cells) {
        (false, _) => " · h: highlight".to_string(),
        (true, 0) => format!(" · ‹{}› isn't in the preview", ed.row_name()),
        (true, _) => format!(" · ‹{}› highlighted (h)", ed.row_name()),
    };
    let block =
        theme::block(&theme::panel(false)).title(format!(" Preview ({variant_name}){note} "));
    f.render_widget(block, area);
    let buf = f.buffer_mut();
    for y in 0..p.height {
        for x in 0..p.width {
            if let Some(c) = buf.cell_mut(Position::new(p.x + x, p.y + y)) {
                *c = shown[(x, y)].clone();
            }
        }
    }
}

/// Draw the app (and sample modals) with `t` into an off-screen buffer, leaving the app's
/// click areas, scroll positions and the active theme as they were.
fn preview_buffer(
    app: &mut App,
    t: &Theme,
    variant: Variant,
    w: u16,
    h: u16,
) -> ratatui::buffer::Buffer {
    let edited = Arc::new(t.clone());
    let keep = (
        app.sidebar_area,
        app.account_area,
        app.theme_areas.clone(),
        app.theme_switch_area,
        app.scroll,
        app.input_scroll,
    );
    let (prev_theme, prev_var) = (theme::active(), theme::variant());
    let Ok(mut term) = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h));
    let _ = term.draw(|pf| {
        theme::set_theme(edited.clone());
        theme::set_variant(variant);
        crate::ui::draw_main(pf, app);
        draw_modal_samples(pf);
    });
    let out = term.backend().buffer().clone();
    theme::set_theme(prev_theme);
    theme::set_variant(prev_var);
    (
        app.sidebar_area,
        app.account_area,
        app.theme_areas,
        app.theme_switch_area,
        app.scroll,
        app.input_scroll,
    ) = keep;
    out
}

/// Flip reverse video on every cell of `shown` whose colors or attributes differ in
/// `marked` (the render with the selected item recolored). Returns how many were marked.
fn highlight_changes(
    shown: &mut ratatui::buffer::Buffer,
    marked: &ratatui::buffer::Buffer,
) -> usize {
    use ratatui::style::Modifier;
    let mut n = 0;
    for (c, m) in shown.content.iter_mut().zip(marked.content.iter()) {
        if (c.fg, c.bg, c.modifier) != (m.fg, m.bg, m.modifier) {
            c.modifier.toggle(Modifier::REVERSED);
            n += 1;
        }
    }
    n
}

/// Sample help and shell modals in the bottom-right corner of the preview.
fn draw_modal_samples(f: &mut Frame) {
    let a = f.area();
    let (w, h) = (
        34.min(a.width.saturating_sub(2)),
        6.min(a.height.saturating_sub(2)),
    );
    if w < 20 || h < 4 {
        return;
    }
    let r = Rect::new(a.right() - w - 1, a.bottom() - h - 1, w, h);
    let modal = theme::modal_text();
    f.render_widget(Clear, r);
    let lines = vec![
        Line::from("Help text in a modal."),
        Line::from(vec![
            Span::raw("Related: "),
            theme::link(true).span("keys"),
            Span::raw(", "),
            theme::link(false).span("theme"),
        ]),
        Line::from(vec![
            theme::shell_title(false, false).span("✓ ok "),
            theme::shell_title(true, false).span("… running "),
            theme::shell_title(false, true).span("✗ failed"),
        ]),
        Line::from(vec![theme::selected().span("› picked row")]),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .style(modal)
            .block(theme::block(&theme::help_frame()).title(theme::help_title().span(" Help "))),
        r,
    );
    draw_more_samples(f, r);
}

/// Parts the live app may not be showing right now (chat entries in an empty chat, the
/// shell modal's frames): small samples left of and above the help sample.
fn draw_more_samples(f: &mut Frame, help: Rect) {
    use crate::app::EntryKind;
    let a = f.area();
    // Chat entries, left of the help sample.
    let w = 30u16;
    if help.x >= a.x + w + 2 {
        let r = Rect::new(help.x - w - 1, help.y, w, help.height);
        let row = |k: EntryKind, label: &'static str, text: &'static str| {
            Line::from(vec![
                theme::entry_label(k).span(label),
                theme::entry(k).span(text),
            ])
        };
        f.render_widget(Clear, r);
        f.render_widget(
            Paragraph::new(vec![
                row(EntryKind::User, "you: ", "sample message"),
                row(EntryKind::Bot, "bot: ", "sample reply"),
                Line::from(
                    [
                        row(EntryKind::System, "· ", "notice  ").spans,
                        row(EntryKind::Error, "! ", "error").spans,
                    ]
                    .concat(),
                ),
            ])
            .block(theme::block(&theme::chat_panel()).title(" Sample chat ")),
            r,
        );
    }
    // Shell modal frames, above the help sample.
    if help.y >= a.y + 4 && help.width >= 30 {
        let w = help.width / 3;
        for (i, (running, failed, label)) in [
            (false, false, " ✓ ok "),
            (true, false, " … run "),
            (false, true, " ✗ fail "),
        ]
        .into_iter()
        .enumerate()
        {
            let r = Rect::new(help.x + i as u16 * w, help.y - 3, w, 3);
            f.render_widget(Clear, r);
            f.render_widget(
                Paragraph::new("shell").style(theme::modal_text()).block(
                    theme::block(&theme::shell_frame(running, failed))
                        .title(theme::shell_title(running, failed).span(label)),
                ),
                r,
            );
        }
    }
}

fn draw_palette(
    f: &mut Frame,
    ed: &mut ThemeEditor,
    area: Rect,
    target: Target,
    tab: usize,
    cursor: usize,
) {
    let grid = ThemeEditor::grid(target, tab);
    let selected = grid.get(cursor).copied();
    let block = theme::block(&theme::panel(true)).title(format!(
        " Color: {} · Tab: grid · arrows · Enter sets · Esc ",
        selected.map(themes::format_color_ref).unwrap_or_default()
    ));
    let p = block.inner(area);
    f.render_widget(block, area);
    // Tabs.
    ed.palette_tab_areas.clear();
    let mut x = p.x;
    let mut spans = vec![];
    for (k, t) in PICKER_TABS.iter().enumerate() {
        let label = format!(" {t} ");
        let w = label.chars().count() as u16;
        ed.palette_tab_areas
            .push((Rect::new(x, p.y, w.min(p.right().saturating_sub(x)), 1), k));
        x += w + 1;
        let st = if k == tab {
            theme::selected()
        } else {
            theme::modal_text()
        };
        spans.push(st.span(label));
        spans.push(Span::raw(" "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), Rect { height: 1, ..p });
    if p.height < 3 {
        return;
    }
    let g = Rect::new(p.x, p.y + 2, p.width, p.height - 2);
    let cols = ThemeEditor::grid_cols(tab) as u16;
    let v = ed.var().clone();
    if cols == 1 {
        // A list: swatch and name.
        ed.palette_cols = 1;
        ed.palette_cell_w = g.width.max(1);
        let rows = (grid.len() as u16).min(g.height);
        ed.palette_area = Rect::new(g.x, g.y, g.width, rows);
        let lines: Vec<Line> = grid
            .iter()
            .enumerate()
            .take(rows as usize)
            .map(|(k, c)| {
                let st = if k == cursor {
                    theme::selected()
                } else {
                    theme::modal_text()
                };
                Line::from(vec![
                    swatch(v.color(*c)),
                    st.span(format!(" {}", themes::format_color_ref(*c))),
                ])
            })
            .collect();
        let mut lines = lines;
        if matches!(target, Target::Color(_)) && rows < g.height {
            lines.push(Line::raw(""));
            lines.push(theme::key_hint().line("Palette colors take a literal color, not @name."));
        }
        f.render_widget(Paragraph::new(lines), g);
        return;
    }
    let cell_w = (g.width / cols).clamp(1, 3);
    ed.palette_cols = cols;
    ed.palette_cell_w = cell_w;
    let rows = (grid.len() as u16).div_ceil(cols).min(g.height);
    ed.palette_area = Rect::new(g.x, g.y, cols * cell_w, rows);
    let buf = f.buffer_mut();
    for (i, c) in grid.iter().enumerate().take((rows * cols) as usize) {
        let (cx, cy) = (i as u16 % cols, i as u16 / cols);
        let col = v.color(*c);
        for dx in 0..cell_w {
            if let Some(cell) = buf.cell_mut(Position::new(g.x + cx * cell_w + dx, g.y + cy)) {
                let mark = i == cursor && dx == cell_w / 2;
                cell.set_symbol(if mark { "◆" } else { " " }).set_bg(col);
                if mark {
                    let (r, gg, b) = themes::rgb_of(col).unwrap_or((0, 0, 0));
                    let light = (r as u32 * 299 + gg as u32 * 587 + b as u32 * 114) / 1000 > 128;
                    cell.set_fg(if light { Color::Black } else { Color::White });
                }
            }
        }
    }
}

fn draw_detail(f: &mut Frame, ed: &mut ThemeEditor, area: Rect) {
    let block = theme::block(&theme::panel(false)).title(" Selected ");
    let inner = block.inner(area);
    f.render_widget(block, area);
    ed.field_areas.clear();
    let v = ed.var().clone();
    let mut lines: Vec<Line> = vec![];
    match ed.row() {
        Row::Color(i) => {
            lines.push(Line::from(vec![
                theme::help_title().span(COLOR_SLOTS[i]),
                Span::raw(format!(" — {}", COLOR_HELP[i])),
            ]));
            lines.push(Line::from(vec![
                Span::raw("value: "),
                swatch(v.colors[i]),
                Span::raw(format!(" {}", themes::format_color(v.colors[i]))),
            ]));
            let bg = v.colors[color::BACKGROUND];
            lines.push(Line::from(vec![
                Span::raw("sample: "),
                Span::styled(
                    " The quick brown fox ",
                    ratatui::style::Style::default().fg(v.colors[i]).bg(bg),
                ),
            ]));
        }
        Row::Style(s) => {
            let info = STYLE_SLOTS[s];
            lines.push(Line::from(vec![
                theme::help_title().span(info.key),
                Span::raw(format!(" — {}", info.help)),
            ]));
            let spec = v.styles[s];
            let focus = match ed.edit {
                Edit::Form { field } => Some(field),
                _ => None,
            };
            let mut spans = vec![];
            let mut x = inner.x;
            for (fi, name) in FIELDS.iter().enumerate() {
                if fi == 2 && !info.framed {
                    continue;
                }
                let text = if fi < 3 {
                    let val = [spec.fg, spec.bg, spec.border][fi]
                        .map(themes::format_color_ref)
                        .unwrap_or_else(|| "—".into());
                    format!("{name} [{val}]")
                } else {
                    let on = spec.attr(fi - 3);
                    format!("[{}] {name}", if on { "x" } else { " " })
                };
                let w = text.width() as u16;
                if x < inner.right() {
                    ed.field_areas
                        .push((Rect::new(x, inner.y + 1, w.min(inner.right() - x), 1), fi));
                }
                let st = if focus == Some(fi) {
                    theme::selected()
                } else {
                    theme::modal_text()
                };
                spans.push(st.span(text));
                spans.push(Span::raw(" "));
                x += w + 1;
            }
            lines.push(Line::from(spans));
            let sample = if info.framed {
                let b = v.resolve(s).border.unwrap_or(Color::Reset);
                Span::styled("┌ box border ┐", ratatui::style::Style::default().fg(b))
            } else {
                v.style(s).span(" The quick brown fox ")
            };
            lines.push(Line::from(vec![Span::raw("sample: "), sample]));
        }
    }
    let mut info_line = match ed.contrast() {
        Some((r, what)) if r < themes::LOW_CONTRAST => {
            vec![theme::entry(crate::app::EntryKind::Error).span(format!(
                "⚠ low contrast {r:.1}:1 against {what} (below 3:1)"
            ))]
        }
        Some((r, what)) => {
            vec![theme::key_hint().span(format!("contrast {r:.1}:1 against {what}"))]
        }
        None => vec![theme::key_hint().span("contrast: n/a (terminal default colors)")],
    };
    if let Some(n) = ed.truecolor_note() {
        info_line.push(theme::key_hint().span(format!(" · {n}")));
    }
    lines.push(Line::from(info_line));
    match &ed.edit {
        Edit::Text { buf, err, .. } | Edit::Name { buf, err, .. } => {
            let label = if matches!(
                ed.edit,
                Edit::Name {
                    duplicate: true,
                    ..
                }
            ) {
                "duplicate as"
            } else if matches!(ed.edit, Edit::Name { .. }) {
                "new theme"
            } else {
                "color"
            };
            let mut spans = vec![theme::selected().span(format!(" {label}: {buf}▏ "))];
            if let Some(e) = err {
                spans.push(theme::entry(crate::app::EntryKind::Error).span(format!("  {e}")));
            }
            lines.push(Line::from(spans));
        }
        _ => {
            if let Some((msg, is_err)) = &ed.status {
                let st = if *is_err {
                    theme::entry(crate::app::EntryKind::Error)
                } else {
                    theme::key_hint()
                };
                lines.push(st.line(msg.clone()));
            }
        }
    }
    f.render_widget(Paragraph::new(lines).block(Block::new()), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{test_app, type_line};
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::{Terminal, backend::TestBackend};

    fn key(app: &mut App, code: KeyCode) {
        app.editor_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn typed(app: &mut App, s: &str) {
        for c in s.chars() {
            key(app, KeyCode::Char(c));
        }
    }

    fn notices(app: &App) -> String {
        app.notices()
            .iter()
            .map(|e| e.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn pick(app: &mut App, value: &str) {
        let p = app.picker.as_mut().expect("a picker is open");
        p.cursor = p
            .items
            .iter()
            .position(|i| i.value == value)
            .unwrap_or_else(|| panic!("no item {value}: {:?}", p.items));
        app.picker_confirm();
    }

    #[test]
    fn startup_loads_the_selected_theme_from_the_user_dir() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        assert!(
            app.themes_dir().starts_with(&app.store.dir),
            "temp data dir only"
        );
        assert_eq!(themes::list(&app.themes_dir()), vec!["default"]);
        assert_eq!(app.theme.name, "Default");
        assert_eq!(app.theme_problem, None);
        assert!(!app.theme_downloading, "themes present: no download");
        // Only the user dir counts: a theme anywhere else isn't seen.
        let elsewhere = app.store.dir.join("share/grokbot-tui/themes");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("solar.toml"), "[dark]").unwrap();
        type_line(&mut app, "/theme use solar");
        assert_eq!(app.config.theme, "default");
    }

    #[test]
    fn missing_and_broken_themes_fall_back_to_plain_colors_with_a_notice() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        let dir = app.themes_dir();
        // Missing.
        std::fs::remove_file(themes::path(&dir, "default")).unwrap();
        app.load_theme();
        assert_eq!(app.theme_problem, Some("missing"));
        assert_eq!(*app.theme, Theme::unstyled("default"));
        let n = notices(&app);
        assert!(n.contains("default.toml is missing"), "{n}");
        assert!(n.contains("/theme init"), "{n}");
        // Broken: plain colors, and the file is left alone.
        std::fs::write(themes::path(&dir, "default"), "[dark").unwrap();
        app.load_theme();
        assert_eq!(app.theme_problem, Some("broken"));
        assert!(notices(&app).contains("can't be used"));
        assert_eq!(
            std::fs::read_to_string(themes::path(&dir, "default")).unwrap(),
            "[dark"
        );
        // The header says so.
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let top: String = (0..120)
            .map(|x| term.backend().buffer()[(x, 1)].symbol().to_string())
            .collect();
        assert!(top.contains("‹default: broken› ▾"), "{top}");
        // /theme init refuses to overwrite it; fixing the file + reload recovers.
        type_line(&mut app, "/theme init");
        assert!(notices(&app).contains("already exists"));
        std::fs::copy(
            crate::app::tests::repo_themes().join("default.toml"),
            themes::path(&dir, "default"),
        )
        .unwrap();
        type_line(&mut app, "/theme reload");
        assert_eq!(app.theme_problem, None);
        assert!(notices(&app).contains("(fixed)"));
    }

    #[test]
    fn no_themes_at_all_then_theme_init_writes_a_minimal_starter() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        let dir = app.themes_dir();
        std::fs::remove_file(themes::path(&dir, "default")).unwrap();
        app.reload_themes();
        assert_eq!(app.theme_problem, Some("missing"));
        type_line(&mut app, "/theme init");
        assert_eq!(app.theme_problem, None, "{}", notices(&app));
        let (t, w) = themes::load(&dir, "default").unwrap();
        assert!(w.is_empty());
        assert!(t.dark.colors.iter().all(|c| *c == Color::Reset));
        assert_eq!(t.dark.styles, Theme::unstyled("default").dark.styles);
    }

    #[test]
    fn use_list_and_picker_switch_themes_and_save_the_pref() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        let dir = app.themes_dir();
        let mut solar = app.theme.as_ref().clone();
        solar.id = "solar".into();
        solar.name = "Solar".into();
        solar.dark.colors[color::ACCENT] = Color::Indexed(136);
        themes::save(&dir, &solar).unwrap();
        type_line(&mut app, "/theme use solar");
        assert_eq!(app.config.theme, "solar");
        assert_eq!(app.theme.dark.colors[color::ACCENT], Color::Indexed(136));
        app.reload_config().unwrap();
        assert_eq!(app.config.theme, "solar", "saved as a global pref");
        type_line(&mut app, "/theme use nope");
        assert_eq!(app.config.theme, "solar", "a missing theme isn't selected");
        type_line(&mut app, "/theme list");
        let n = notices(&app);
        assert!(n.contains("● solar"), "{n}");
        assert!(n.contains("/theme export"), "{n}");
        assert!(n.contains("default"), "{n}");
        // F5 / header picker: themes, then "Edit themes…".
        app.open_theme_picker();
        let p = app.picker.as_ref().unwrap();
        assert_eq!(p.selected().unwrap().value, "solar");
        assert_eq!(p.items.last().unwrap().value, EDIT_ITEM);
        pick(&mut app, "default");
        assert_eq!(app.config.theme, "default");
        app.open_theme_picker();
        pick(&mut app, EDIT_ITEM);
        assert_eq!(app.editor.as_ref().unwrap().theme.id, "default");
        // /theme dark|light|system still switch the mode.
        app.editor = None;
        type_line(&mut app, "/theme light");
        assert_eq!(app.config.theme_mode, theme::Mode::Light);
    }

    fn solar(app: &App) -> Theme {
        let mut t = app.theme.as_ref().clone();
        t.id = "solar".into();
        t.name = "Solar".into();
        t.dark.colors[color::ACCENT] = Color::Indexed(136);
        t.light.colors[color::ACCENT] = Color::Indexed(136);
        themes::save(&app.themes_dir(), &t).unwrap();
        t
    }

    /// Move the open picker's highlight to `value` the way a key press does.
    fn highlight(app: &mut App, value: &str) {
        let p = app.picker.as_mut().expect("a picker is open");
        let before = p.cursor;
        p.cursor = p.items.iter().position(|i| i.value == value).unwrap();
        app.picker_moved(before);
    }

    #[test]
    fn the_theme_picker_previews_the_highlighted_theme_and_esc_restores() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        solar(&app);
        let orig = app.theme.clone();
        app.open_theme_picker();
        highlight(&mut app, "solar");
        assert_eq!(app.theme.id, "solar", "previewed while highlighted");
        assert_eq!(app.config.theme, "default", "nothing saved by a preview");
        highlight(&mut app, EDIT_ITEM);
        assert_eq!(
            app.theme.id, "default",
            "a non-theme item shows the original"
        );
        highlight(&mut app, "solar");
        app.picker_cancel();
        assert!(
            Arc::ptr_eq(&app.theme, &orig),
            "Esc puts the old theme back"
        );
        assert_eq!(app.config.theme, "default");
        assert!(app.theme_preview.is_none());
        // Enter keeps (and saves) the highlighted one.
        app.open_theme_picker();
        highlight(&mut app, "solar");
        app.picker_confirm();
        assert_eq!(app.theme.id, "solar");
        app.reload_config().unwrap();
        assert_eq!(app.config.theme, "solar");
        assert!(app.theme_preview.is_none());
    }

    #[test]
    fn a_theme_override_is_not_saved_and_survives_config_reloads() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        solar(&app);
        // What main does for --theme solar.
        app.config.theme = "solar".into();
        app.theme_override = Some("solar".into());
        app.load_theme();
        assert_eq!(app.theme.id, "solar");
        app.reload_config().unwrap();
        assert_eq!(app.config.theme, "solar", "kept across a reload");
        let saved = app.store.load_config().unwrap().theme;
        assert_eq!(saved, "default", "the pref is untouched");
        assert!(themes::list_text(&app.themes_dir(), &app.config.theme).contains("● solar"));
        // Picking a theme in the app saves it and ends the override.
        app.set_theme("default");
        assert!(app.theme_override.is_none());
        app.reload_config().unwrap();
        assert_eq!(app.config.theme, "default");
    }

    #[test]
    fn theme_export_copies_the_file_and_never_overwrites() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, d) = test_app(&rt);
        let src = std::fs::read_to_string(themes::path(&app.themes_dir(), "default")).unwrap();
        // Default destination: <data dir>/exports/<name>-<stamp>.toml.
        type_line(&mut app, "/theme export default");
        let exports = app.store.dir.join("exports");
        let files: Vec<_> = std::fs::read_dir(&exports).unwrap().flatten().collect();
        assert_eq!(files.len(), 1);
        let name = files[0].file_name().to_string_lossy().to_string();
        assert!(
            name.starts_with("default-") && name.ends_with("Z.toml"),
            "{name}"
        );
        assert_eq!(
            std::fs::read_to_string(files[0].path()).unwrap(),
            src,
            "exact text"
        );
        // A path with a space; a folder gets <name>.toml.
        let out = d.path().join("my themes");
        std::fs::create_dir_all(&out).unwrap();
        type_line(
            &mut app,
            &format!("/theme export default {}", out.display()),
        );
        assert_eq!(
            std::fs::read_to_string(out.join("default.toml")).unwrap(),
            src
        );
        // Again: refused, the file is kept.
        std::fs::write(out.join("default.toml"), "mine").unwrap();
        type_line(
            &mut app,
            &format!("/theme export default {}", out.display()),
        );
        assert_eq!(
            std::fs::read_to_string(out.join("default.toml")).unwrap(),
            "mine"
        );
        let n = notices(&app);
        assert!(n.contains("not overwritten"), "{n}");
        // Missing and broken themes aren't exported.
        type_line(&mut app, "/theme export nope");
        std::fs::write(themes::path(&app.themes_dir(), "bad"), "not = [toml").unwrap();
        type_line(&mut app, &format!("/theme export bad {}", out.display()));
        assert!(!out.join("bad.toml").exists());
        let n = notices(&app);
        assert!(n.contains("Couldn't export theme 'nope'"), "{n}");
        assert!(n.contains("Couldn't export theme 'bad'"), "{n}");
    }

    #[test]
    fn editor_steps_through_themes_and_closes_with_shift_f5() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        solar(&app);
        app.open_theme_editor(Some("default"));
        key(&mut app, KeyCode::Tab);
        let variant = app.editor.as_ref().unwrap().variant;
        key(&mut app, KeyCode::Char(']'));
        assert_eq!(app.editor.as_ref().unwrap().theme.id, "solar");
        assert_eq!(
            app.editor.as_ref().unwrap().variant,
            variant,
            "same variant"
        );
        key(&mut app, KeyCode::Char(']'));
        assert_eq!(app.editor.as_ref().unwrap().theme.id, "default", "wraps");
        key(&mut app, KeyCode::Char('['));
        assert_eq!(app.editor.as_ref().unwrap().theme.id, "solar");
        // Unsaved changes: stays put and says why.
        app.editor.as_mut().unwrap().theme.dark.colors[color::ACCENT] = Color::Indexed(1);
        app.editor.as_mut().unwrap().theme.light.colors[color::ACCENT] = Color::Indexed(1);
        key(&mut app, KeyCode::Char('['));
        let ed = app.editor.as_ref().unwrap();
        assert_eq!(ed.theme.id, "solar");
        assert!(ed.status.as_ref().unwrap().0.contains("Save"));
        app.open_theme_editor(Some("default"));
        app.editor_key(KeyEvent::new(KeyCode::F(5), KeyModifiers::SHIFT));
        assert!(app.editor.is_none(), "Shift-F5 closes");
        app.open_theme_editor(Some("default"));
        key(&mut app, KeyCode::F(17));
        assert!(app.editor.is_none(), "F17 closes");
    }

    #[test]
    fn low_contrast_items_are_marked_in_the_list() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        app.open_theme_editor(Some("default"));
        let ed = app.editor.as_mut().unwrap();
        let v = ed.variant;
        let var = if v == Variant::Dark {
            &mut ed.theme.dark
        } else {
            &mut ed.theme.light
        };
        var.colors[color::TEXT] = Color::Rgb(10, 10, 10);
        var.colors[color::BACKGROUND] = Color::Rgb(12, 12, 12);
        assert!(ed.low_contrast(color::TEXT));
        var_ok(ed);
        let mut term = Terminal::new(TestBackend::new(140, 45)).unwrap();
        term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let ed = app.editor.as_ref().unwrap();
        let line = (ed.list_area.y..ed.list_area.bottom())
            .map(|y| {
                (ed.list_area.x..ed.list_area.right())
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .find(|l| l.contains("text ") && !l.contains("modal_text"))
            .expect("text row shown");
        assert!(line.contains('⚠'), "{line}");
    }

    fn var_ok(ed: &ThemeEditor) {
        // Text on a clearly different background isn't flagged.
        let mut t = ed.theme.clone();
        t.dark.colors[color::TEXT] = Color::Rgb(250, 250, 250);
        t.dark.colors[color::BACKGROUND] = Color::Rgb(0, 0, 0);
        t.light.colors[color::TEXT] = Color::Rgb(0, 0, 0);
        t.light.colors[color::BACKGROUND] = Color::Rgb(250, 250, 250);
        let mut e2 = ThemeEditor::new(t, None, None, None, ed.variant);
        e2.cursor = color::TEXT;
        assert!(!e2.low_contrast(color::TEXT));
    }

    #[test]
    fn the_color_picker_offers_palette_refs_for_parts() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        app.open_theme_editor(Some("default"));
        let ed = app.editor.as_mut().unwrap();
        let g = ThemeEditor::grid(Target::Fg(theme::slot::TITLE), 3);
        assert_eq!(g.len(), N_COLORS + 1);
        assert_eq!(g[color::OK], ColorRef::Slot(color::OK));
        assert_eq!(*g.last().unwrap(), ColorRef::Lit(Color::Reset));
        assert_eq!(
            [0, 1, 2].map(|t| ThemeEditor::grid(Target::Color(0), t).len()),
            [16, 216, 24]
        );
        // A part that points at @accent opens on that ref; picking @ok sets the ref.
        ed.var_mut().styles[theme::slot::TITLE].fg = Some(ColorRef::Slot(color::ACCENT));
        ed.open_palette(Target::Fg(theme::slot::TITLE));
        assert_eq!(
            ed.edit,
            Edit::Palette {
                target: Target::Fg(theme::slot::TITLE),
                tab: 3,
                cursor: color::ACCENT
            }
        );
        ed.palette_pick(color::OK);
        assert_eq!(
            ed.var().styles[theme::slot::TITLE].fg,
            Some(ColorRef::Slot(color::OK))
        );
        // Indexed 0-15 open on the matching named color in the first tab.
        ed.var_mut().colors[color::WARN] = Color::Indexed(3);
        ed.open_palette(Target::Color(color::WARN));
        assert!(matches!(
            ed.edit,
            Edit::Palette {
                tab: 0,
                cursor: 3,
                ..
            }
        ));
        // The picker draws its tabs.
        let mut term = Terminal::new(TestBackend::new(140, 45)).unwrap();
        term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        for t in PICKER_TABS {
            assert!(text.contains(t), "{t}");
        }
    }

    #[test]
    fn header_switcher_opens_the_theme_picker() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        for (w, h) in [(120, 30), (120, 20)] {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
            let r = app.theme_switch_area;
            assert!(r.width > 0, "{w}x{h}");
            app.click(r.x + 1, r.y);
            assert!(app.picker.as_ref().unwrap().title.starts_with("Theme"));
            app.picker_cancel();
        }
    }

    #[test]
    fn delete_asks_first_and_switches_away_from_the_deleted_theme() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        let dir = app.themes_dir();
        let mut other = app.theme.as_ref().clone();
        other.id = "other".into();
        themes::save(&dir, &other).unwrap();
        type_line(&mut app, "/theme use other");
        type_line(&mut app, "/theme delete other");
        assert_eq!(
            app.picker.as_ref().unwrap().selected().unwrap().value,
            "cancel"
        );
        app.picker_confirm();
        assert!(themes::path(&dir, "other").exists(), "Cancel keeps it");
        type_line(&mut app, "/theme delete other");
        pick(&mut app, "delete:other");
        assert!(!themes::path(&dir, "other").exists());
        assert_eq!(app.config.theme, "default");
        // Any theme can be deleted, including default (then plain colors + notice).
        type_line(&mut app, "/theme delete default");
        pick(&mut app, "delete:default");
        assert_eq!(app.theme_problem, Some("missing"));
    }

    #[test]
    fn editor_edits_colors_and_styles_then_saves_and_applies() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        app.detected_variant = Some(Variant::Dark);
        app.open_theme_editor(None);
        let ed = app.editor.as_ref().unwrap();
        assert!(!ed.dirty());
        assert_eq!(ed.variant, Variant::Dark);
        // accent: type an invalid color, then a hex one.
        app.editor.as_mut().unwrap().cursor = color::ACCENT;
        key(&mut app, KeyCode::Enter);
        for _ in 0..8 {
            key(&mut app, KeyCode::Backspace);
        }
        typed(&mut app, "#zz");
        key(&mut app, KeyCode::Enter);
        assert!(matches!(
            &app.editor.as_ref().unwrap().edit,
            Edit::Text { err: Some(_), .. }
        ));
        for _ in 0..3 {
            key(&mut app, KeyCode::Backspace);
        }
        typed(&mut app, "#ff8800");
        key(&mut app, KeyCode::Enter);
        let ed = app.editor.as_ref().unwrap();
        assert_eq!(ed.edit, Edit::None);
        assert_eq!(ed.theme.dark.colors[color::ACCENT], Color::Rgb(255, 136, 0));
        assert!(ed.dirty() && ed.changed(color::ACCENT));
        // Not applied before saving.
        assert_eq!(app.theme.dark.colors[color::ACCENT], Color::Cyan);
        // Palette: header color from the grid.
        app.editor.as_mut().unwrap().cursor = color::HEADER;
        key(&mut app, KeyCode::Char('p'));
        let cur = |app: &App| match app.editor.as_ref().unwrap().edit {
            Edit::Palette { tab, cursor, .. } => (tab, cursor),
            _ => panic!("palette open"),
        };
        assert_eq!(
            cur(&app),
            (2, 237 - 232),
            "starts on the current color's tab and cell"
        );
        key(&mut app, KeyCode::Right);
        assert_eq!(cur(&app), (2, 6));
        key(&mut app, KeyCode::Up);
        assert_eq!(cur(&app), (2, 0), "clamped at the first cell");
        for _ in 0..3 {
            key(&mut app, KeyCode::Down);
        }
        assert_eq!(cur(&app), (2, 23), "clamped at the last gray");
        key(&mut app, KeyCode::Home);
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            app.editor.as_ref().unwrap().theme.dark.colors[color::HEADER],
            Color::Indexed(233)
        );
        // Tabs: 16 colors · 216 cube · 24 grays · @ palette (palette colors: only reset).
        key(&mut app, KeyCode::Char('p'));
        key(&mut app, KeyCode::Tab);
        assert_eq!(cur(&app), (3, 0));
        assert_eq!(
            ThemeEditor::grid(Target::Color(color::HEADER), 3),
            vec![ColorRef::Lit(Color::Reset)]
        );
        key(&mut app, KeyCode::Tab);
        assert_eq!(cur(&app), (0, 0), "wraps to the 16 colors");
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            app.editor.as_ref().unwrap().theme.dark.colors[color::HEADER],
            Color::Red
        );
        key(&mut app, KeyCode::Char('p'));
        key(&mut app, KeyCode::BackTab);
        key(&mut app, KeyCode::BackTab);
        key(&mut app, KeyCode::End);
        assert_eq!(cur(&app), (2, 23));
        key(&mut app, KeyCode::BackTab);
        key(&mut app, KeyCode::End);
        assert_eq!(cur(&app), (1, 215), "the 216 cube");
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            app.editor.as_ref().unwrap().theme.dark.colors[color::HEADER],
            Color::Indexed(231)
        );
        // Style form: make `title` italic, point its fg at @ok, clear its bg.
        app.editor.as_mut().unwrap().cursor = N_COLORS + theme::slot::TITLE;
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.editor.as_ref().unwrap().edit, Edit::Form { field: 0 });
        key(&mut app, KeyCode::Enter);
        for _ in 0..10 {
            key(&mut app, KeyCode::Backspace);
        }
        typed(&mut app, "@ok");
        key(&mut app, KeyCode::Enter);
        // Right from fg → bg → (border skipped: not a box) → bold → italic.
        for _ in 0..3 {
            key(&mut app, KeyCode::Right);
        }
        assert_eq!(app.editor.as_ref().unwrap().edit, Edit::Form { field: 4 });
        key(&mut app, KeyCode::Char(' '));
        key(&mut app, KeyCode::Esc);
        let title = app.editor.as_ref().unwrap().theme.dark.styles[theme::slot::TITLE];
        assert_eq!(title.fg, Some(ColorRef::Slot(color::OK)));
        assert!(title.italic);
        // Revert just the header.
        app.editor.as_mut().unwrap().cursor = color::HEADER;
        key(&mut app, KeyCode::Char('r'));
        assert_eq!(
            app.editor.as_ref().unwrap().theme.dark.colors[color::HEADER],
            Color::Indexed(237)
        );
        // Light variant is separate.
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.editor.as_ref().unwrap().variant, Variant::Light);
        assert_eq!(
            app.editor.as_ref().unwrap().theme.light.colors[color::ACCENT],
            Color::Indexed(25)
        );
        // Save: written to disk and applied (it's the selected theme).
        key(&mut app, KeyCode::Char('s'));
        assert!(!app.editor.as_ref().unwrap().dirty());
        assert_eq!(
            app.theme.dark.colors[color::ACCENT],
            Color::Rgb(255, 136, 0)
        );
        let (on_disk, w) = themes::load(&app.themes_dir(), "default").unwrap();
        assert!(w.is_empty());
        assert_eq!(on_disk.dark.styles[theme::slot::TITLE], title);
        key(&mut app, KeyCode::Esc);
        assert!(app.editor.is_none(), "no prompt when saved");
    }

    #[test]
    fn closing_with_unsaved_changes_asks_save_discard_cancel() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        app.open_theme_editor(None);
        app.editor.as_mut().unwrap().theme.dark.colors[0] = Color::Indexed(1);
        key(&mut app, KeyCode::Esc);
        let values: Vec<String> = app
            .picker
            .as_ref()
            .unwrap()
            .items
            .iter()
            .map(|i| i.value.clone())
            .collect();
        assert_eq!(values, ["cancel", "save", "discard"]);
        app.picker_confirm();
        assert!(app.editor.is_some(), "Cancel keeps editing");
        key(&mut app, KeyCode::Esc);
        pick(&mut app, "discard");
        assert!(app.editor.is_none());
        let (t, _) = themes::load(&app.themes_dir(), "default").unwrap();
        assert_eq!(t.dark.colors[0], Color::Indexed(252), "file untouched");
        // Save and close.
        app.open_theme_editor(None);
        app.editor.as_mut().unwrap().theme.dark.colors[0] = Color::Indexed(2);
        key(&mut app, KeyCode::Char('q'));
        pick(&mut app, "save");
        assert!(app.editor.is_none());
        assert_eq!(
            themes::load(&app.themes_dir(), "default")
                .unwrap()
                .0
                .dark
                .colors[0],
            Color::Indexed(2)
        );
    }

    #[test]
    fn duplicate_new_and_delete_in_the_editor() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        let dir = app.themes_dir();
        app.open_theme_editor(None);
        key(&mut app, KeyCode::Char('d'));
        typed(&mut app, "Bad!");
        key(&mut app, KeyCode::Enter);
        assert!(matches!(
            &app.editor.as_ref().unwrap().edit,
            Edit::Name { err: Some(_), .. }
        ));
        for _ in 0..4 {
            key(&mut app, KeyCode::Backspace);
        }
        typed(&mut app, "default");
        key(&mut app, KeyCode::Enter);
        assert!(
            matches!(&app.editor.as_ref().unwrap().edit, Edit::Name { err: Some(e), .. } if e.contains("exists"))
        );
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('d'));
        typed(&mut app, "mine");
        key(&mut app, KeyCode::Enter);
        let ed = app.editor.as_ref().unwrap();
        assert_eq!(ed.theme.id, "mine");
        assert_eq!(ed.theme.name, "Default (copy)");
        assert_eq!(ed.theme.dark, themes::load(&dir, "default").unwrap().0.dark);
        assert!(!ed.dirty(), "created on disk");
        // New: a starter.
        key(&mut app, KeyCode::Char('n'));
        typed(&mut app, "blank");
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            app.editor.as_ref().unwrap().theme.dark,
            Theme::unstyled("x").dark
        );
        assert_eq!(themes::list(&dir), vec!["blank", "default", "mine"]);
        // Delete asks first; Cancel is the default.
        key(&mut app, KeyCode::Char('x'));
        app.picker_confirm();
        assert!(themes::path(&dir, "blank").exists());
        key(&mut app, KeyCode::Char('x'));
        pick(&mut app, "delete:blank");
        assert!(!themes::path(&dir, "blank").exists());
        assert_eq!(
            app.editor.as_ref().unwrap().theme.id,
            "default",
            "editor moves on"
        );
    }

    #[test]
    fn saving_over_a_file_changed_elsewhere_or_broken_asks_first() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        let dir = app.themes_dir();
        app.open_theme_editor(None);
        app.editor.as_mut().unwrap().theme.dark.colors[0] = Color::Indexed(3);
        let mut text = std::fs::read_to_string(themes::path(&dir, "default")).unwrap();
        text.push_str("# edited elsewhere\n");
        std::fs::write(themes::path(&dir, "default"), &text).unwrap();
        key(&mut app, KeyCode::Char('s'));
        assert!(
            app.picker
                .as_ref()
                .unwrap()
                .title
                .contains("changed on disk")
        );
        pick(&mut app, "reload");
        assert!(!app.editor.as_ref().unwrap().dirty(), "reloaded from disk");
        // Broken file: the editor opens a starter and saving asks before replacing it.
        std::fs::write(themes::path(&dir, "default"), "[dark").unwrap();
        app.open_theme_editor(None);
        assert!(app.editor.as_ref().unwrap().broken.is_some());
        key(&mut app, KeyCode::Char('s'));
        assert!(app.picker.as_ref().unwrap().title.contains("broken"));
        app.picker_confirm();
        assert_eq!(
            std::fs::read_to_string(themes::path(&dir, "default")).unwrap(),
            "[dark"
        );
        key(&mut app, KeyCode::Char('s'));
        pick(&mut app, "overwrite");
        assert!(themes::load(&dir, "default").is_ok());
    }

    #[test]
    fn contrast_warning_for_low_contrast_parts() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        app.open_theme_editor(None);
        let ed = app.editor.as_mut().unwrap();
        ed.cursor = color::TEXT;
        let (good, what) = ed.contrast().unwrap();
        assert!(good > themes::LOW_CONTRAST, "{good}");
        assert_eq!(what, "background");
        ed.theme.dark.colors[color::TEXT] = Color::Indexed(235);
        assert!(ed.contrast().unwrap().0 < themes::LOW_CONTRAST);
        let mut term = Terminal::new(TestBackend::new(140, 40)).unwrap();
        term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let screen: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(screen.contains("low contrast"), "warning shown");
    }

    #[test]
    fn preview_uses_the_edited_theme_and_mouse_works() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        crate::app::tests::mock_login(&mut app, "t");
        app.open_theme_editor(None);
        app.editor.as_mut().unwrap().theme.dark.colors[color::HEADER] = Color::Indexed(17);
        let mut term = Terminal::new(TestBackend::new(160, 45)).unwrap();
        term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let screen: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            screen.contains("Theme editor: Default (default.toml) ● modified"),
            "{screen}"
        );
        assert!(screen.contains("Preview (dark)"));
        // The preview's header row uses the edited header color; the real app doesn't.
        let preview_has = buf.content().iter().any(|c| c.bg == Color::Indexed(17));
        assert!(preview_has, "edited header color in the preview");
        assert_eq!(app.theme.dark.colors[color::HEADER], Color::Indexed(237));
        // Mouse: click the Light tab, then a list row (selects), click again (edits).
        let (light, _) = app.editor.as_ref().unwrap().tab_areas[1];
        app.editor_click(light.x + 1, light.y);
        assert_eq!(app.editor.as_ref().unwrap().variant, Variant::Light);
        term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let ed = app.editor.as_ref().unwrap();
        let line = ed
            .list_rows
            .iter()
            .position(|r| *r == Some(color::ACCENT))
            .unwrap() as u16;
        let (x, y) = (ed.list_area.x + 2, ed.list_area.y + line);
        app.editor_click(x, y);
        assert_eq!(app.editor.as_ref().unwrap().cursor, color::ACCENT);
        app.editor_click(x, y);
        assert!(matches!(
            app.editor.as_ref().unwrap().edit,
            Edit::Text { .. }
        ));
        // Palette click.
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('p'));
        term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        // Click the 216-cube tab, then a cell.
        let ed = app.editor.as_ref().unwrap();
        let (tr, _) = ed.palette_tab_areas[1];
        app.editor_click(tr.x + 1, tr.y);
        assert!(matches!(
            app.editor.as_ref().unwrap().edit,
            Edit::Palette { tab: 1, .. }
        ));
        term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let ed = app.editor.as_ref().unwrap();
        let (pa, cw, cols) = (ed.palette_area, ed.palette_cell_w, ed.palette_cols);
        assert_eq!(cols, 18);
        app.editor_click(pa.x + 2 * cw, pa.y + 1);
        assert_eq!(
            app.editor.as_ref().unwrap().theme.light.colors[color::ACCENT],
            Color::Indexed(16 + 18 + 2)
        );
        // Wheel moves the selection.
        app.editor_scroll(1);
        assert_eq!(app.editor.as_ref().unwrap().cursor, color::ACCENT + 1);
    }

    /// An app whose data dir has no themes and no shipped dir, downloading from `srv`.
    fn bare_app(rt: &tokio::runtime::Runtime, url: Option<String>) -> (App, tempfile::TempDir) {
        let (mut app, d) = test_app(rt);
        std::fs::remove_file(themes::path(&app.themes_dir(), "default")).unwrap();
        app.theme_download_base = url;
        (app, d)
    }

    fn serve(rt: &tokio::runtime::Runtime, status: u16, body: &str) -> wiremock::MockServer {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
        rt.block_on(async {
            let srv = MockServer::start().await;
            for name in themes::SHIPPED {
                let b = if *name == "default" {
                    body.to_string()
                } else {
                    std::fs::read_to_string(
                        crate::app::tests::repo_themes().join(format!("{name}.toml")),
                    )
                    .unwrap_or_else(|_| body.to_string())
                };
                Mock::given(path(format!("/{name}.toml")))
                    .respond_with(ResponseTemplate::new(status).set_body_string(b))
                    .mount(&srv)
                    .await;
            }
            srv
        })
    }

    fn shipped() -> String {
        std::fs::read_to_string(crate::app::tests::repo_themes().join("default.toml")).unwrap()
    }

    #[test]
    fn downloads_are_off_in_tests_mock_mode_and_by_opt_out() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        assert_eq!(app.theme_download_base, None, "mock mode never downloads");
        app.theme_download_base = Some("http://127.0.0.1:9/".into());
        assert!(app.download_url("default").is_some());
        type_line(&mut app, "/prefs set ui.theme_download false");
        assert_eq!(app.download_url("default"), None);
    }

    #[test]
    fn startup_with_no_themes_downloads_the_default_in_the_background() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let srv = serve(&rt, 200, &shipped());
        let url = format!("{}/default.toml", srv.uri());
        let (mut app, _d) = bare_app(&rt, Some(format!("{}/", srv.uri())));
        app.init_themes();
        // Startup doesn't wait: plain colors and a notice first.
        assert_eq!(app.theme_problem, Some("downloading"));
        assert!(notices(&app).contains(&format!("Downloading the default theme from {url}")));
        assert!(notices(&app).contains("GROKBOT_TUI_NO_THEME_DOWNLOAD=1"));
        app.wait_idle(std::time::Duration::from_secs(10));
        assert_eq!(app.theme_problem, None, "{}", notices(&app));
        assert_eq!(app.theme.name, "Default");
        let p = themes::path(&app.themes_dir(), "default");
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            shipped(),
            "written verbatim"
        );
        assert!(notices(&app).contains(&format!("Downloaded the default theme from {url}")));
    }

    #[test]
    fn a_failed_download_keeps_plain_colors_and_explains() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let srv = serve(&rt, 404, "nope");
        let (mut app, _d) = bare_app(&rt, Some(format!("{}/", srv.uri())));
        app.init_themes();
        app.wait_idle(std::time::Duration::from_secs(10));
        assert_eq!(app.theme_problem, Some("missing"));
        let n = notices(&app);
        assert!(n.contains("Couldn't download the default theme"), "{n}");
        assert!(n.contains("HTTP 404"), "{n}");
        assert!(n.contains("default.toml is missing"), "{n}");
        assert!(themes::list(&app.themes_dir()).is_empty());
        // Opted out: no download, straight to the notice.
        let (mut app, _d) = bare_app(&rt, None);
        app.init_themes();
        assert_eq!(app.theme_problem, Some("missing"));
        assert!(!app.theme_downloading);
    }

    #[test]
    fn theme_init_downloads_shipped_themes_by_name_or_all() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let srv = serve(&rt, 200, &shipped());
        let base = format!("{}/", srv.uri());
        let (mut app, _d) = bare_app(&rt, None);
        app.init_themes();
        assert_eq!(app.theme_problem, Some("missing"));
        app.theme_download_base = Some(base.clone());
        app.open_theme_picker();
        assert!(
            app.picker.as_ref().unwrap().items[0]
                .label
                .contains("Download the default")
        );
        app.picker_cancel();
        type_line(&mut app, "/theme init");
        app.wait_idle(std::time::Duration::from_secs(10));
        assert_eq!(app.theme_problem, None, "{}", notices(&app));
        let dir = app.themes_dir();
        assert_eq!(
            std::fs::read_to_string(themes::path(&dir, "default")).unwrap(),
            shipped()
        );
        // `all` = every shipped theme; existing files are kept, the rest download.
        type_line(&mut app, "/theme init all");
        assert!(notices(&app).contains("default.toml already exists; it was kept"));
        app.wait_idle(std::time::Duration::from_secs(30));
        let have = themes::list(&dir);
        assert_eq!(have.len(), themes::SHIPPED.len(), "{have:?}");
        for n in themes::SHIPPED {
            assert!(have.iter().any(|h| h == n), "missing {n} in {have:?}");
        }
        // A second `all` keeps every file.
        type_line(&mut app, "/theme init all");
        assert!(!app.theme_downloading);
        assert!(notices(&app).contains("already exists; it was kept"));
        // A name is that theme's file in the release, not the default renamed.
        type_line(&mut app, "/theme init mine");
        app.wait_idle(std::time::Duration::from_secs(10));
        assert!(!themes::path(&dir, "mine").exists());
        let n = notices(&app);
        assert!(
            n.contains(&format!(
                "Couldn't download the mine theme from {base}mine.toml"
            )),
            "{n}"
        );
        assert!(n.contains("Shipped themes:"), "{n}");
        // Downloads off: the starter is written instead.
        app.theme_download_base = None;
        type_line(&mut app, "/theme init other");
        assert!(notices(&app).contains("writing the minimal starter instead"));
        assert_eq!(
            themes::load(&dir, "other").unwrap().0.dark,
            Theme::unstyled("x").dark
        );
        // /theme starter never downloads.
        app.theme_download_base = Some(base);
        type_line(&mut app, "/theme starter plain");
        assert!(!app.theme_downloading);
        assert_eq!(
            themes::load(&dir, "plain").unwrap().0.dark,
            Theme::unstyled("x").dark
        );
    }

    fn screen(app: &mut App) -> ratatui::buffer::Buffer {
        let mut term = Terminal::new(TestBackend::new(160, 45)).unwrap();
        term.draw(|f| crate::ui::draw(f, app)).unwrap();
        term.backend().buffer().clone()
    }

    fn text(b: &ratatui::buffer::Buffer) -> String {
        b.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn preview_highlights_the_selected_color_or_part() {
        use ratatui::style::Modifier;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        crate::app::tests::mock_login(&mut app, "t");
        app.detected_variant = Some(Variant::Dark);
        app.open_theme_editor(None);
        // A color nothing else on screen uses, so its cells are only in the preview.
        let probe = Color::Rgb(9, 9, 9);
        let ed = app.editor.as_mut().unwrap();
        ed.theme.dark.colors[color::ACCENT] = probe;
        ed.cursor = color::ACCENT;
        ed.highlight = false;
        let plain = screen(&mut app);
        assert!(text(&plain).contains("Preview (dark) · h: highlight"));
        let pa = app.editor.as_ref().unwrap().preview_area;
        let inside = |i: usize| {
            let (x, y) = ((i % 160) as u16, (i / 160) as u16);
            x > pa.x && x + 1 < pa.right() && y > pa.y && y + 1 < pa.bottom()
        };
        let painted: Vec<usize> = (0..plain.content.len())
            .filter(|&i| inside(i))
            .filter(|&i| plain.content[i].fg == probe || plain.content[i].bg == probe)
            .collect();
        assert!(painted.len() > 20, "accent is used in the preview");
        key(&mut app, KeyCode::Char('h'));
        let lit = screen(&mut app);
        assert!(
            text(&lit).contains("‹accent› highlighted (h)"),
            "{}",
            text(&lit)
        );
        for &i in &painted {
            let (a, b) = (&plain.content[i], &lit.content[i]);
            assert_ne!(
                a.modifier.contains(Modifier::REVERSED),
                b.modifier.contains(Modifier::REVERSED),
                "accent cell {i} is marked"
            );
        }
        // Cells outside the preview (editor chrome, list) aren't touched.
        assert!(
            (0..plain.content.len())
                .filter(|&i| !inside(i) && i / 160 != pa.y as usize)
                .all(|i| plain.content[i] == lit.content[i])
        );
        // A part: its box is marked; the highlight follows the selection.
        app.editor.as_mut().unwrap().cursor = N_COLORS + theme::slot::PANEL_FOCUSED;
        let part = screen(&mut app);
        assert!(text(&part).contains("‹panel_focused› highlighted (h)"));
        assert_ne!(
            (0..part.content.len())
                .filter(|&i| part.content[i].modifier.contains(Modifier::REVERSED))
                .count(),
            (0..lit.content.len())
                .filter(|&i| lit.content[i].modifier.contains(Modifier::REVERSED))
                .count()
        );
        // Highlighting is display only: nothing changed in the theme being edited.
        assert!(
            app.editor.as_ref().unwrap().theme.dark.styles[theme::slot::PANEL_FOCUSED] != {
                app.editor.as_ref().unwrap().marked_theme().dark.styles[theme::slot::PANEL_FOCUSED]
            }
        );
        assert_eq!(
            app.editor.as_ref().unwrap().theme.dark.colors[color::ACCENT],
            probe
        );
        // Parts the preview doesn't draw say so.
        let shown = |app: &mut App, s: usize| {
            app.editor.as_mut().unwrap().cursor = N_COLORS + s;
            !text(&screen(app)).contains("isn't in the preview")
        };
        assert!(shown(&mut app, theme::slot::ENTRY_USER), "sample chat");
        assert!(
            shown(&mut app, theme::slot::SHELL_FRAME_FAILED),
            "sample shell frames"
        );
        let hidden: Vec<&str> = (0..N_STYLES)
            .filter(|&s| !shown(&mut app, s))
            .map(|s| STYLE_SLOTS[s].key)
            .collect();
        // Only parts that depend on the app's state: the compact header, mouse capture off,
        // the sidebar focused.
        assert_eq!(
            hidden,
            [
                "brand",
                "mode_normal",
                "mode_insert",
                "mode_command",
                "mode_visual",
                "mouse_off",
                "cursor_row",
                "selection",
                "unread_badge",
                "reaction_pill",
                "md_heading",
                "md_code",
                "md_code_block",
                "md_quote",
                "md_link",
                "md_list_marker",
                "md_rule",
                "md_table_border",
                "menu_active",
                "tooltip",
                "toast",
                "toast_warn",
                "toast_error"
            ]
        );
    }

    #[test]
    fn highlight_changes_marks_only_differing_cells() {
        use ratatui::buffer::Buffer;
        use ratatui::style::{Modifier, Style};
        let r = Rect::new(0, 0, 4, 1);
        let mut a = Buffer::empty(r);
        let mut b = Buffer::empty(r);
        a.set_string(0, 0, "abcd", Style::new().fg(Color::Red));
        b.set_string(0, 0, "abcd", Style::new().fg(Color::Red));
        b[(1, 0)].set_fg(Color::Rgb(1, 2, 3));
        b[(3, 0)].set_bg(Color::Rgb(1, 2, 3));
        a[(3, 0)].modifier.insert(Modifier::REVERSED);
        b[(3, 0)].modifier.insert(Modifier::REVERSED);
        assert_eq!(highlight_changes(&mut a, &b), 2);
        let rev: Vec<bool> = (0..4)
            .map(|x| a[(x, 0)].modifier.contains(Modifier::REVERSED))
            .collect();
        assert_eq!(
            rev,
            [false, true, false, false],
            "an already reversed cell flips back"
        );
    }
}
