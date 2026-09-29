use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use ratatui::layout::{Position, Rect};
use ratatui::widgets::TableState;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::art::Art;
use crate::audio::Route;
use crate::config::{Config, Paths};
use crate::eq;
use crate::event::AppEvent;
use crate::player::{Player, PlayerEvent};
use crate::tidal::Client;
use crate::tidal::auth::{self, Credentials, DeviceCode, Session};
use crate::tidal::models::{StreamInfo, StreamSource, Track};
use crate::visualizer::Visualizer;
use crate::{search, ui};
use ratatui_image::picker::Picker;

const STATUS_TTL: Duration = Duration::from_secs(4);
const SEEK_STEP: f64 = 5.0;
const VOLUME_STEP: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Search,
    Queue,
    NowPlaying,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Input,
    List,
}

pub enum Auth {
    Starting,
    Pending(DeviceCode),
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Error,
}

pub struct Status {
    pub text: String,
    pub level: Level,
    at: Instant,
}

#[derive(Default)]
pub struct Search {
    pub input: String,
    /// Cursor position in chars.
    pub cursor: usize,
    pub results: Vec<Track>,
    pub table: TableState,
    pub loading: bool,
    pub last_query: String,
    /// What the current results are, e.g. `Top tracks · Radiohead`.
    pub heading: String,
}

#[derive(Default)]
pub struct Queue {
    pub tracks: Vec<Track>,
    pub current: Option<usize>,
    pub table: TableState,
}

impl Queue {
    pub fn current_track(&self) -> Option<&Track> {
        self.current.and_then(|i| self.tracks.get(i))
    }

    pub fn total_secs(&self) -> u64 {
        self.tracks.iter().map(|t| t.duration).sum()
    }
}

pub struct NowPlaying {
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub volume: f64,
    pub loading: bool,
    pub stream: Option<StreamInfo>,
}

/// Screen areas that respond to taps/clicks, recorded by the UI on every draw.
#[derive(Default)]
pub struct Hits {
    pub tabs: Vec<(Rect, View)>,
    pub search_input: Option<Rect>,
    /// The data rows of the visible track table (below its header).
    pub list_rows: Option<Rect>,
    pub prev: Option<Rect>,
    pub play: Option<Rect>,
    pub next: Option<Rect>,
    pub progress: Option<Rect>,
    pub eq: Option<Rect>,
}

pub struct App {
    pub running: bool,
    pub view: View,
    pub focus: Focus,
    pub auth: Auth,
    pub search: Search,
    pub queue: Queue,
    pub now: NowPlaying,
    pub status: Option<Status>,
    pub show_help: bool,
    pub hits: Hits,
    /// Animation frame counter, advanced on every tick.
    pub frame: usize,
    pub quality: String,
    pub art: Art,
    pub visualizer: Visualizer,
    /// Index into `eq::PRESETS`.
    pub eq: usize,
    pub route: Route,
    client: Option<Arc<Client>>,
    player: Player,
    tx: UnboundedSender<AppEvent>,
    http: reqwest::Client,
    config: Config,
    paths: Paths,
    play_generation: u64,
    dirty: bool,
}

impl App {
    pub fn new(
        config: Config,
        paths: Paths,
        player: Player,
        route: Route,
        picker: Picker,
        tx: UnboundedSender<AppEvent>,
    ) -> Self {
        let eq = eq::find(&config.eq);
        player.set_audio_filter(eq::PRESETS[eq].filter().as_deref());
        Self {
            running: true,
            view: View::Search,
            focus: Focus::Input,
            auth: Auth::Starting,
            search: Search::default(),
            queue: Queue::default(),
            now: NowPlaying {
                position: 0.0,
                duration: 0.0,
                paused: false,
                volume: 100.0,
                loading: false,
                stream: None,
            },
            status: None,
            show_help: false,
            hits: Hits::default(),
            frame: 0,
            quality: config.quality.to_string(),
            art: Art::new(picker),
            visualizer: Visualizer::start(route.pulse_server()),
            route,
            eq,
            client: None,
            player,
            tx,
            http: reqwest::Client::builder()
                .user_agent(concat!("tidally/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("TLS backend should initialize"),
            config,
            paths,
            play_generation: 0,
            dirty: true,
        }
    }

    pub async fn run(
        mut self,
        terminal: &mut DefaultTerminal,
        mut rx: UnboundedReceiver<AppEvent>,
    ) -> Result<()> {
        match Session::load(&self.paths.session_file()) {
            Some(session) if session.client_id == self.config.client_id => {
                self.on_logged_in(session, false)
            }
            Some(_) => {
                // Signed in through a different client (e.g. the older AAC-only default):
                // its tokens won't work with this one.
                self.start_login();
                self.set_status(
                    "Please sign in again (the app now uses a login that allows lossless)",
                    Level::Info,
                );
            }
            None => self.start_login(),
        }
        match &self.route {
            #[cfg(feature = "remote")]
            Route::Pulse(server) => self.set_status(format!("Audio → {server}"), Level::Success),
            #[cfg(feature = "remote")]
            Route::Unreachable(_) => self.set_status(
                "Forwarded audio isn't answering: is pulseaudio running on your device? Reconnect with music.sh",
                Level::Error,
            ),
            #[cfg(feature = "remote")]
            Route::LocalOverSsh => self.set_status(
                "Over SSH without forwarded audio: sound plays on this machine (see README)",
                Level::Info,
            ),
            Route::Local => {}
        }

        let mut terminal_events = EventStream::new();
        let mut tick = tokio::time::interval(Duration::from_millis(33));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        while self.running {
            if self.dirty {
                terminal.draw(|f| ui::draw(f, &mut self))?;
                self.dirty = false;
                // The UI records how big it wants the cover; encode that size off-thread.
                self.art.sync(&self.tx);
            }
            tokio::select! {
                ev = terminal_events.next() => match ev {
                    Some(Ok(ev)) => self.on_terminal_event(ev),
                    Some(Err(e)) => return Err(e.into()),
                    None => break,
                },
                Some(ev) = rx.recv() => self.on_app_event(ev),
                _ = tick.tick() => self.on_tick(),
            }
        }
        Ok(())
    }

    fn credentials(&self) -> Credentials {
        Credentials {
            client_id: self.config.client_id.clone(),
            client_secret: self.config.client_secret.clone(),
        }
    }

    // ── async actions ────────────────────────────────────────────────────────

    fn start_login(&mut self) {
        self.auth = Auth::Starting;
        let (http, creds, tx) = (self.http.clone(), self.credentials(), self.tx.clone());
        let session_path = self.paths.session_file();
        tokio::spawn(async move {
            let result = async {
                let code = auth::request_device_code(&http, &creds).await?;
                let _ = tx.send(AppEvent::LoginCode(code.clone()));
                let session = auth::poll_for_session(&http, &creds, &code).await?;
                session.save(&session_path)?;
                anyhow::Ok(session)
            }
            .await;
            let _ = tx.send(match result {
                Ok(session) => AppEvent::LoggedIn(session),
                Err(e) => AppEvent::LoginFailed(format!("{e:#}")),
            });
        });
    }

    fn on_logged_in(&mut self, session: Session, fresh: bool) {
        self.client = Some(Arc::new(Client::new(
            self.http.clone(),
            self.credentials(),
            session,
            self.paths.session_file(),
            self.config.quality,
        )));
        self.auth = Auth::Ready;
        if fresh {
            self.set_status("Signed in to Tidal", Level::Success);
        }
    }

    fn submit_search(&mut self) {
        let query = self.search.input.trim().to_string();
        if query.is_empty() {
            return;
        }
        let Some(client) = self.client.clone() else {
            self.set_status("Not signed in yet", Level::Error);
            return;
        };
        self.search.loading = true;
        self.search.last_query = query.clone();
        let (tx, limit) = (self.tx.clone(), self.config.search_limit);
        tokio::spawn(async move {
            let parsed = search::Query::parse(&query);
            let result = search::run(&client, &parsed, limit)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AppEvent::SearchResults { query, result });
        });
    }

    fn play_index(&mut self, index: usize) {
        let Some(track) = self.queue.tracks.get(index) else {
            return;
        };
        let Some(client) = self.client.clone() else {
            return;
        };
        self.queue.current = Some(index);
        self.queue.table.select(Some(index));
        self.play_generation += 1;
        self.now.loading = true;
        self.now.position = 0.0;
        self.now.duration = track.duration as f64;
        self.now.stream = None;
        self.player.stop();
        let cover = track.album.as_ref().and_then(|a| a.cover.clone());
        self.art.show(cover, &self.http, &self.tx);
        if self.art.cover.is_none() {
            ui::reset_palette();
        }

        let (tx, generation, id) = (self.tx.clone(), self.play_generation, track.id);
        tokio::spawn(async move {
            let result = client.stream(id).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(AppEvent::StreamReady { generation, result });
        });
    }

    fn start_stream(&mut self, info: StreamInfo) {
        let target = match &info.source {
            StreamSource::Url(url) => url.clone(),
            StreamSource::Dash(mpd) => {
                let id = self.queue.current_track().map_or(0, |t| t.id);
                let path = self.paths.cache_dir.join(format!("track-{id}.mpd"));
                if let Err(e) = std::fs::write(&path, mpd) {
                    self.set_status(format!("Couldn't write DASH manifest: {e}"), Level::Error);
                    return;
                }
                path.display().to_string()
            }
        };
        self.player.load(&target);
        self.now.stream = Some(info);
    }

    fn next(&mut self) {
        match self.queue.current {
            Some(i) if i + 1 < self.queue.tracks.len() => self.play_index(i + 1),
            None if !self.queue.tracks.is_empty() => self.play_index(0),
            _ => {}
        }
    }

    fn previous(&mut self) {
        if self.now.position > 3.0 {
            self.player.seek_absolute(0.0);
            return;
        }
        match self.queue.current {
            Some(i) if i > 0 => self.play_index(i - 1),
            Some(_) => self.player.seek_absolute(0.0),
            None => {}
        }
    }

    // ── events ───────────────────────────────────────────────────────────────

    fn on_tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        if self
            .status
            .as_ref()
            .is_some_and(|s| s.at.elapsed() > STATUS_TTL)
        {
            self.status = None;
            self.dirty = true;
        }
        // Keep the spectrum moving while music plays.
        if self.queue.current.is_some() && !self.now.paused && !self.now.loading {
            self.dirty = true;
        }
        let animating =
            self.search.loading || self.now.loading || matches!(self.auth, Auth::Starting);
        if animating && self.frame.is_multiple_of(3) {
            self.dirty = true;
        }
    }

    fn on_app_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::LoginCode(code) => self.auth = Auth::Pending(code),
            AppEvent::LoggedIn(session) => self.on_logged_in(session, true),
            AppEvent::LoginFailed(err) => self.auth = Auth::Failed(err),
            AppEvent::SearchResults { query, result } => {
                // Ignore results for a query the user has since replaced.
                if query != self.search.last_query {
                    return;
                }
                self.search.loading = false;
                match result {
                    Ok(outcome) => {
                        let tracks = outcome.tracks;
                        self.search.table.select((!tracks.is_empty()).then_some(0));
                        if tracks.is_empty() {
                            self.set_status(format!("No tracks found for “{query}”"), Level::Info);
                        }
                        self.search.results = tracks;
                        self.search.heading = outcome.heading;
                    }
                    Err(e) => self.set_status(format!("Search failed: {e}"), Level::Error),
                }
            }
            AppEvent::StreamReady { generation, result } => {
                if generation != self.play_generation {
                    return;
                }
                self.now.loading = false;
                match result {
                    Ok(info) => self.start_stream(info),
                    Err(e) => self.set_status(format!("Can't play track: {e}"), Level::Error),
                }
            }
            AppEvent::CoverLoaded {
                cover,
                image,
                palette,
            } => {
                if self.art.on_loaded(cover, image) {
                    ui::set_palette(&palette);
                }
            }
            AppEvent::CoverEncoded {
                cover,
                size,
                protocol,
            } => {
                self.art.on_encoded(cover, size, protocol);
            }
            AppEvent::Player(ev) => match ev {
                PlayerEvent::Position(p) => self.now.position = p,
                PlayerEvent::Duration(d) => self.now.duration = d,
                PlayerEvent::Paused(p) => self.now.paused = p,
                PlayerEvent::Volume(v) => self.now.volume = v,
                PlayerEvent::Finished => self.next(),
                PlayerEvent::Failed(e) => {
                    self.set_status(format!("Playback error: {e}"), Level::Error);
                    self.next();
                }
                PlayerEvent::Exited => {
                    self.set_status("mpv exited unexpectedly — restart tidally", Level::Error);
                }
            },
        }
        self.dirty = true;
    }

    fn on_terminal_event(&mut self, ev: Event) {
        match ev {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.on_key(key),
            Event::Mouse(mouse) => {
                if !self.on_mouse(mouse) {
                    return;
                }
            }
            Event::Resize(..) => {}
            _ => return,
        }
        self.dirty = true;
    }

    /// Taps (Termux sends touches as left clicks) and swipes (scroll wheel).
    /// Returns false for events that don't change anything, to skip a redraw.
    fn on_mouse(&mut self, mouse: MouseEvent) -> bool {
        let pos = Position::new(mouse.column, mouse.row);
        let hit = |r: Option<Rect>| r.is_some_and(|r| r.contains(pos));
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {}
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp if hit(self.hits.list_rows) => {
                let code = if mouse.kind == MouseEventKind::ScrollDown {
                    KeyCode::Down
                } else {
                    KeyCode::Up
                };
                let (table, len) = match self.view {
                    View::Search => (&mut self.search.table, self.search.results.len()),
                    View::Queue => (&mut self.queue.table, self.queue.tracks.len()),
                    View::NowPlaying => return false,
                };
                for _ in 0..3 {
                    navigate(table, len, code);
                }
                return true;
            }
            _ => return false,
        }

        if self.show_help {
            self.show_help = false;
            return true;
        }
        match &self.auth {
            Auth::Ready => {}
            Auth::Pending(_) | Auth::Failed(_) => {
                // Same as pressing Enter on the sign-in popup.
                self.on_key(KeyEvent::from(KeyCode::Enter));
                return true;
            }
            Auth::Starting => return false,
        }

        if let Some((_, view)) = self.hits.tabs.iter().find(|(r, _)| r.contains(pos)) {
            self.view = *view;
        } else if hit(self.hits.search_input) {
            self.view = View::Search;
            self.focus = Focus::Input;
            let s = &mut self.search;
            s.cursor = s.input.chars().count();
        } else if hit(self.hits.prev) {
            self.previous();
        } else if hit(self.hits.play) {
            self.player.toggle_pause();
        } else if hit(self.hits.next) {
            self.next();
        } else if hit(self.hits.eq) {
            self.cycle_eq(1);
        } else if let Some(bar) = self.hits.progress.filter(|r| r.contains(pos)) {
            if self.now.duration > 0.0 && bar.width > 1 {
                let ratio = (pos.x - bar.x) as f64 / (bar.width - 1) as f64;
                self.player.seek_absolute(ratio * self.now.duration);
            }
        } else if let Some(rows) = self.hits.list_rows.filter(|r| r.contains(pos)) {
            let (table, len) = match self.view {
                View::Search => (&mut self.search.table, self.search.results.len()),
                View::Queue => (&mut self.queue.table, self.queue.tracks.len()),
                View::NowPlaying => return false,
            };
            let index = table.offset() + (pos.y - rows.y) as usize;
            if index >= len {
                return false;
            }
            self.focus = Focus::List;
            // First tap selects, a second tap on the same row plays it.
            if table.selected() == Some(index) {
                self.list_key(KeyEvent::from(KeyCode::Enter));
            } else {
                table.select(Some(index));
            }
        } else {
            return false;
        }
        true
    }

    fn list_key(&mut self, key: KeyEvent) {
        match self.view {
            View::Search => self.on_results_key(key),
            View::Queue => self.on_queue_key(key),
            View::NowPlaying => {}
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.running = false;
            return;
        }

        if self.show_help {
            self.show_help = false;
            return;
        }

        match &self.auth {
            Auth::Pending(code) => {
                match key.code {
                    KeyCode::Char('o') | KeyCode::Enter => {
                        if let Err(e) = open::that_detached(code.link()) {
                            self.set_status(format!("Couldn't open browser: {e}"), Level::Error);
                        }
                    }
                    KeyCode::Char('q') | KeyCode::Esc => self.running = false,
                    _ => {}
                }
                return;
            }
            Auth::Failed(_) => {
                match key.code {
                    KeyCode::Char('r') | KeyCode::Enter => self.start_login(),
                    KeyCode::Char('q') | KeyCode::Esc => self.running = false,
                    _ => {}
                }
                return;
            }
            Auth::Starting => {
                if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                    self.running = false;
                }
                return;
            }
            Auth::Ready => {}
        }

        if self.view == View::Search && self.focus == Focus::Input {
            self.on_input_key(key);
            return;
        }

        // Global keys.
        match key.code {
            KeyCode::Char('q') => self.running = false,
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('/') => {
                self.view = View::Search;
                self.focus = Focus::Input;
            }
            KeyCode::Tab => {
                self.view = match self.view {
                    View::Search => View::Queue,
                    View::Queue => View::NowPlaying,
                    View::NowPlaying => View::Search,
                };
            }
            KeyCode::BackTab => {
                self.view = match self.view {
                    View::Search => View::NowPlaying,
                    View::Queue => View::Search,
                    View::NowPlaying => View::Queue,
                };
            }
            KeyCode::Char('1') => self.view = View::Search,
            KeyCode::Char('2') => self.view = View::Queue,
            KeyCode::Char('3') => self.view = View::NowPlaying,
            KeyCode::Char('e') => self.cycle_eq(1),
            KeyCode::Char('E') => self.cycle_eq(eq::PRESETS.len() - 1),
            KeyCode::Char(' ') => self.player.toggle_pause(),
            KeyCode::Char('n') => self.next(),
            KeyCode::Char('p') => self.previous(),
            KeyCode::Left | KeyCode::Char('h') => self.player.seek_relative(-SEEK_STEP),
            KeyCode::Right | KeyCode::Char('l') => self.player.seek_relative(SEEK_STEP),
            KeyCode::Char('+') | KeyCode::Char('=') => self.player.add_volume(VOLUME_STEP),
            KeyCode::Char('-') => self.player.add_volume(-VOLUME_STEP),
            _ => match self.view {
                View::Search => self.on_results_key(key),
                View::Queue => self.on_queue_key(key),
                View::NowPlaying => {}
            },
        }
    }

    fn on_input_key(&mut self, key: KeyEvent) {
        let s = &mut self.search;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter => {
                self.focus = Focus::List;
                self.submit_search();
            }
            KeyCode::Esc | KeyCode::Down | KeyCode::Tab => {
                if self.client.is_some() {
                    self.focus = Focus::List;
                }
            }
            KeyCode::Char('u') if ctrl => {
                s.input.clear();
                s.cursor = 0;
            }
            KeyCode::Char('w') if ctrl => {
                let before: String = s.input.chars().take(s.cursor).collect();
                let trimmed = before.trim_end();
                let keep = trimmed.rfind(char::is_whitespace).map_or(0, |i| i + 1);
                let removed = before.chars().count() - trimmed[..keep].chars().count();
                let start = byte_index(&s.input, s.cursor - removed);
                let end = byte_index(&s.input, s.cursor);
                s.input.replace_range(start..end, "");
                s.cursor -= removed;
            }
            KeyCode::Char('a') if ctrl => s.cursor = 0,
            KeyCode::Char('e') if ctrl => s.cursor = s.input.chars().count(),
            KeyCode::Char(c) if !ctrl => {
                s.input.insert(byte_index(&s.input, s.cursor), c);
                s.cursor += 1;
            }
            KeyCode::Backspace if s.cursor > 0 => {
                s.cursor -= 1;
                s.input.remove(byte_index(&s.input, s.cursor));
            }
            KeyCode::Delete if s.cursor < s.input.chars().count() => {
                s.input.remove(byte_index(&s.input, s.cursor));
            }
            KeyCode::Left => s.cursor = s.cursor.saturating_sub(1),
            KeyCode::Right => s.cursor = (s.cursor + 1).min(s.input.chars().count()),
            KeyCode::Home => s.cursor = 0,
            KeyCode::End => s.cursor = s.input.chars().count(),
            _ => {}
        }
    }

    fn on_results_key(&mut self, key: KeyEvent) {
        // Up from the first result goes back to the search box.
        if key.code == KeyCode::Up && self.search.table.selected().unwrap_or(0) == 0 {
            self.focus = Focus::Input;
            return;
        }
        if navigate(&mut self.search.table, self.search.results.len(), key.code) {
            return;
        }
        let Some(selected) = self.search.table.selected() else {
            return;
        };
        let Some(track) = self.search.results.get(selected).cloned() else {
            return;
        };
        match key.code {
            KeyCode::Enter => {
                // Replace the queue with the result list so playback continues through it.
                self.queue.tracks = self.search.results.clone();
                self.play_index(selected);
            }
            KeyCode::Char('a') => {
                self.queue.tracks.push(track.clone());
                self.set_status(
                    format!("Queued “{}”", track.display_title()),
                    Level::Success,
                );
            }
            KeyCode::Char('N') => {
                let at = self
                    .queue
                    .current
                    .map_or(self.queue.tracks.len(), |i| i + 1);
                self.queue.tracks.insert(at, track.clone());
                self.set_status(
                    format!("Playing “{}” next", track.display_title()),
                    Level::Success,
                );
            }
            _ => {}
        }
    }

    fn on_queue_key(&mut self, key: KeyEvent) {
        let len = self.queue.tracks.len();
        if navigate(&mut self.queue.table, len, key.code) {
            return;
        }
        let Some(selected) = self.queue.table.selected().filter(|&i| i < len) else {
            return;
        };
        match key.code {
            KeyCode::Enter => self.play_index(selected),
            KeyCode::Char('d') | KeyCode::Char('x') | KeyCode::Delete => {
                self.queue.tracks.remove(selected);
                self.queue.current = match self.queue.current {
                    Some(c) if c == selected => {
                        self.play_generation += 1;
                        self.now.loading = false;
                        self.player.stop();
                        None
                    }
                    Some(c) if c > selected => Some(c - 1),
                    other => other,
                };
                let len = self.queue.tracks.len();
                self.queue
                    .table
                    .select((len > 0).then(|| selected.min(len - 1)));
            }
            KeyCode::Char('K') if selected > 0 => self.move_queue_item(selected, selected - 1),
            KeyCode::Char('J') if selected + 1 < len => {
                self.move_queue_item(selected, selected + 1)
            }
            KeyCode::Char('c') => {
                self.queue = Queue::default();
                self.play_generation += 1;
                self.now.loading = false;
                self.player.stop();
                self.set_status("Queue cleared", Level::Info);
            }
            _ => {}
        }
    }

    fn cycle_eq(&mut self, step: usize) {
        self.eq = (self.eq + step) % eq::PRESETS.len();
        let preset = &eq::PRESETS[self.eq];
        self.player.set_audio_filter(preset.filter().as_deref());
        self.set_status(format!("EQ · {}", preset.name), Level::Info);
    }

    fn move_queue_item(&mut self, from: usize, to: usize) {
        self.queue.tracks.swap(from, to);
        self.queue.current = self.queue.current.map(|c| match c {
            c if c == from => to,
            c if c == to => from,
            c => c,
        });
        self.queue.table.select(Some(to));
    }

    pub fn set_status(&mut self, text: impl Into<String>, level: Level) {
        self.status = Some(Status {
            text: text.into(),
            level,
            at: Instant::now(),
        });
        self.dirty = true;
    }
}

/// Handles list movement keys. Returns true if the key was consumed.
fn navigate(state: &mut TableState, len: usize, code: KeyCode) -> bool {
    if len == 0 {
        return matches!(
            code,
            KeyCode::Up | KeyCode::Down | KeyCode::Char('j' | 'k' | 'g' | 'G')
        );
    }
    let current = state.selected().unwrap_or(0).min(len - 1);
    let next = match code {
        KeyCode::Down | KeyCode::Char('j') => (current + 1).min(len - 1),
        KeyCode::Up | KeyCode::Char('k') => current.saturating_sub(1),
        KeyCode::PageDown => (current + 10).min(len - 1),
        KeyCode::PageUp => current.saturating_sub(10),
        KeyCode::Home | KeyCode::Char('g') => 0,
        KeyCode::End | KeyCode::Char('G') => len - 1,
        _ => return false,
    };
    state.select(Some(next));
    true
}

fn byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map_or(s.len(), |(i, _)| i)
}
