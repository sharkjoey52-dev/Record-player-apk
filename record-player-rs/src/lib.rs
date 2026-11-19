//! Record Player - a Spotify controller with a spinning vinyl.
//! Player tab, Search tab and Settings tab. Runs on Android (Fire tablets) and desktop.

mod platform;
mod settings;
mod spotify;
mod widgets;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, vec2, Align2, Color32, ColorImage, FontId, Key, RichText, Sense, TextStyle, TextureHandle,
    TextureOptions, Ui,
};

use settings::Settings;
use spotify::{ApiError, Item, Kind, PlayerState, SearchItem, Spotify};
use widgets::{bar_slider, draw_turntable, icon_button, Icon, BG, DARK, DIM, FG, GOLD, PANEL};

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------
pub fn run(options: eframe::NativeOptions) -> eframe::Result<()> {
    eframe::run_native(
        "Record Player",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: android_activity::AndroidApp) {
    platform::android_init(&app);
    let options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };
    let _ = run(options);
}

// ---------------------------------------------------------------------------
// Messages from background threads to the UI
// ---------------------------------------------------------------------------
enum Event {
    State(Result<Option<PlayerState>, ApiError>),
    Toast(String),
    Cover(String, ColorImage),
    Thumb(String, ColorImage),
    Search(u64, Result<Vec<SearchItem>, String>),
    LoginDone(Result<(), String>),
}

#[derive(Clone)]
struct Notifier {
    tx: Sender<Event>,
    ctx: egui::Context,
}

impl Notifier {
    fn send(&self, e: Event) {
        let _ = self.tx.send(e);
        self.ctx.request_repaint();
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Player,
    Search,
    Settings,
}

#[derive(PartialEq)]
enum Auth {
    LoggedOut,
    LoggingIn,
    LoggedIn,
}

// ---------------------------------------------------------------------------
// The app
// ---------------------------------------------------------------------------
struct App {
    client: Spotify,
    rx: Receiver<Event>,
    notify: Notifier,
    wake: Sender<()>,

    settings: Settings,
    scale_edit: f32,
    tab: Tab,
    auth: Auth,
    login_cancel: Arc<AtomicBool>,
    login_error: String,

    state: Option<PlayerState>,
    last_sync: Instant,
    note: String,
    toast: String,
    toast_until: Instant,

    cover_url: Option<String>,
    cover_tex: Option<TextureHandle>,
    angle: f32,
    arm: f32,
    seek_drag: Option<f32>,
    vol_drag: Option<f32>,
    volume: f32,

    search_text: String,
    search_kind: Kind,
    search_gen: u64,
    searching: bool,
    searched_once: bool,
    results: Vec<SearchItem>,
    thumbs: HashMap<String, TextureHandle>,
    thumbs_requested: HashSet<String>,
}

fn dur_text(ms: f32) -> String {
    let s = (ms / 1000.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> App {
        let ctx = cc.egui_ctx.clone();
        let settings = Settings::load();
        setup_style(&ctx);
        ctx.set_zoom_factor(settings.ui_scale);
        platform::keep_screen_on(settings.keep_screen_on);

        let client = Spotify::new();
        let (tx, rx) = channel();
        let notify = Notifier { tx, ctx };
        let (wake, wake_rx) = channel::<()>();

        // Background poller: asks Spotify what is playing every 3 seconds.
        {
            let client = client.clone();
            let notify = notify.clone();
            std::thread::spawn(move || loop {
                if client.has_login() {
                    notify.send(Event::State(client.player_state()));
                }
                let _ = wake_rx.recv_timeout(Duration::from_secs(3));
                while wake_rx.try_recv().is_ok() {}
            });
        }

        let auth = if client.has_login() { Auth::LoggedIn } else { Auth::LoggedOut };
        let mut app = App {
            client,
            rx,
            notify,
            wake,
            scale_edit: settings.ui_scale,
            settings,
            tab: Tab::Player,
            auth,
            login_cancel: Arc::new(AtomicBool::new(false)),
            login_error: String::new(),
            state: None,
            last_sync: Instant::now(),
            note: String::new(),
            toast: String::new(),
            toast_until: Instant::now(),
            cover_url: None,
            cover_tex: None,
            angle: 0.0,
            arm: 4.0,
            seek_drag: None,
            vol_drag: None,
            volume: 50.0,
            search_text: String::new(),
            search_kind: Kind::Track,
            search_gen: 0,
            searching: false,
            searched_once: false,
            results: vec![],
            thumbs: HashMap::new(),
            thumbs_requested: HashSet::new(),
        };
        #[cfg(debug_assertions)]
        if std::env::var("RP_DEMO").is_ok() {
            app.demo(&cc.egui_ctx);
        }
        app.scale_edit = app.settings.ui_scale;
        app
    }

    /// Debug builds only: fake data so the screens can be checked without Spotify.
    #[cfg(debug_assertions)]
    fn demo(&mut self, ctx: &egui::Context) {
        self.auth = Auth::LoggedIn;
        let v: serde_json::Value = serde_json::json!({
            "is_playing": true, "progress_ms": 83000, "shuffle_state": true, "repeat_state": "track",
            "device": {"volume_percent": 65},
            "item": {"name": "Midnight City", "duration_ms": 243000,
                     "artists": [{"name": "M83"}],
                     "album": {"name": "Hurry Up, We're Dreaming", "images": [{"url": "a"}, {"url": "b"}]}}
        });
        self.state = serde_json::from_value(v).ok();
        self.cover_url = Some("demo".into());
        let n = 300usize;
        let mut px = Vec::with_capacity(n * n);
        for y in 0..n {
            for x in 0..n {
                let (fx, fy) = (x as f32 / n as f32, y as f32 / n as f32);
                let stripe = (((fx * 8.0 + fy * 3.0).sin() * 0.5 + 0.5) * 255.0) as u8;
                px.push(Color32::from_rgb((fx * 255.0) as u8, stripe, (fy * 255.0) as u8));
            }
        }
        let img = ColorImage { size: [n, n], source_size: vec2(n as f32, n as f32), pixels: px };
        self.cover_tex = Some(ctx.load_texture("cover", img, TextureOptions::LINEAR));
        self.angle = 30.0;
        self.arm = 25.0;
        let sv: serde_json::Value = serde_json::from_str(
            r#"{"tracks":{"items":[
              {"name":"Midnight City","uri":"u:1","artists":[{"name":"M83"}],"album":{"name":"Hurry Up, We're Dreaming","images":[]}},
              {"name":"Wait","uri":"u:2","artists":[{"name":"M83"}],"album":{"name":"Hurry Up, We're Dreaming","images":[]}},
              {"name":"Reunion","uri":"u:3","artists":[{"name":"M83"}],"album":{"name":"Hurry Up","images":[]}}]}}"#,
        )
        .unwrap();
        self.results = spotify::parse_search(Kind::Track, &sv);
        self.search_text = "m83".into();
        self.searched_once = true;
        self.tab = match std::env::var("RP_TAB").as_deref() {
            Ok("search") => Tab::Search,
            Ok("settings") => Tab::Settings,
            _ => Tab::Player,
        };
        if std::env::var("RP_KB").is_ok() {
            self.settings.builtin_keyboard = true;
        }
    }

    // ---- helpers ----------------------------------------------------------
    fn is_playing(&self) -> bool {
        self.state.as_ref().map(|s| s.is_playing).unwrap_or(false)
    }

    fn item(&self) -> Option<&Item> {
        self.state.as_ref().and_then(|s| s.item.as_ref())
    }

    fn progress_ms(&self) -> f32 {
        let Some(s) = &self.state else { return 0.0 };
        let Some(it) = &s.item else { return 0.0 };
        let mut p = s.progress_ms.unwrap_or(0) as f32;
        if s.is_playing {
            p += self.last_sync.elapsed().as_millis() as f32;
        }
        p.min(it.duration_ms as f32)
    }

    fn show_toast(&mut self, msg: impl Into<String>) {
        self.toast = msg.into();
        self.toast_until = Instant::now() + Duration::from_secs(6);
    }

    /// Fire-and-forget Spotify command on a background thread.
    fn cmd(&self, method: &'static str, path: &'static str, query: Vec<(&'static str, String)>) {
        let client = self.client.clone();
        let notify = self.notify.clone();
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            if let Err(e) = client.api(method, path, &query, None) {
                match e {
                    ApiError::Auth => notify.send(Event::State(Err(ApiError::Auth))),
                    other => notify.send(Event::Toast(other.text())),
                }
            }
            std::thread::sleep(Duration::from_millis(400));
            let _ = wake.send(());
        });
    }

    fn toggle_play(&mut self) {
        let playing = self.is_playing();
        let prog = self.progress_ms();
        if let Some(s) = self.state.as_mut() {
            if s.item.is_some() {
                s.progress_ms = Some(prog as u64);
                s.is_playing = !playing;
                self.last_sync = Instant::now();
            }
        }
        self.cmd("PUT", if playing { "/me/player/pause" } else { "/me/player/play" }, vec![]);
    }

    fn toggle_shuffle(&mut self) {
        let on = self.state.as_ref().map(|s| s.shuffle_state).unwrap_or(false);
        self.cmd("PUT", "/me/player/shuffle", vec![("state", (!on).to_string())]);
    }

    fn toggle_repeat(&mut self) {
        let cur = self.state.as_ref().map(|s| s.repeat_state.clone()).unwrap_or_default();
        let next = match cur.as_str() {
            "off" | "" => "context",
            "context" => "track",
            _ => "off",
        };
        self.cmd("PUT", "/me/player/repeat", vec![("state", next.to_string())]);
    }

    fn start_login(&mut self) {
        self.auth = Auth::LoggingIn;
        self.login_error.clear();
        self.login_cancel = Arc::new(AtomicBool::new(false));
        let client = self.client.clone();
        let notify = self.notify.clone();
        let cancel = self.login_cancel.clone();
        std::thread::spawn(move || {
            let r = client.login(&cancel);
            notify.send(Event::LoginDone(r));
        });
    }

    fn start_search(&mut self) {
        let q = self.search_text.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.search_gen += 1;
        self.searching = true;
        self.searched_once = true;
        let gen = self.search_gen;
        let kind = self.search_kind;
        let client = self.client.clone();
        let notify = self.notify.clone();
        std::thread::spawn(move || {
            let r = client.search(&q, kind).map_err(|e| e.text());
            notify.send(Event::Search(gen, r));
        });
    }

    fn play_item(&mut self, it: SearchItem) {
        self.show_toast(format!("Starting {}...", it.title));
        self.tab = Tab::Player;
        let client = self.client.clone();
        let notify = self.notify.clone();
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            match client.play(it.kind, &it.uri) {
                Ok(()) => {}
                Err(ApiError::Auth) => notify.send(Event::State(Err(ApiError::Auth))),
                Err(e) => notify.send(Event::Toast(e.text())),
            }
            std::thread::sleep(Duration::from_millis(700));
            let _ = wake.send(());
        });
    }

    fn fetch_cover(&self, url: String) {
        let client = self.client.clone();
        let notify = self.notify.clone();
        std::thread::spawn(move || {
            if let Some(img) = client.fetch_image(&url) {
                notify.send(Event::Cover(url, img));
            }
        });
    }

    fn fetch_thumb(&mut self, url: String) {
        if !self.thumbs_requested.insert(url.clone()) {
            return;
        }
        let client = self.client.clone();
        let notify = self.notify.clone();
        std::thread::spawn(move || {
            if let Some(img) = client.fetch_image(&url) {
                notify.send(Event::Thumb(url, img));
            }
        });
    }

    fn apply_state(&mut self, s: Option<PlayerState>) {
        self.last_sync = Instant::now();
        match s {
            Some(st) if st.item.is_some() => {
                self.note.clear();
                if self.vol_drag.is_none() {
                    if let Some(v) = st.device.as_ref().and_then(|d| d.volume_percent) {
                        self.volume = v as f32;
                    }
                }
                let url = st.item.as_ref().and_then(|i| i.cover_url());
                if url != self.cover_url {
                    self.cover_url = url.clone();
                    self.cover_tex = None;
                    if let Some(u) = url {
                        self.fetch_cover(u);
                    }
                }
                self.state = Some(st);
            }
            Some(st) => {
                self.note = "Nothing playing. Start music in Spotify.".into();
                self.state = Some(st);
                self.cover_url = None;
                self.cover_tex = None;
            }
            None => {
                self.note = "No active device. Open Spotify on this tablet and press play once.".into();
                self.state = None;
                self.cover_url = None;
                self.cover_tex = None;
            }
        }
    }

    fn handle_events(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::State(Ok(s)) => {
                    if self.auth == Auth::LoggedIn {
                        self.apply_state(s);
                    }
                }
                Event::State(Err(ApiError::Auth)) => {
                    if self.auth == Auth::LoggedIn {
                        self.client.logout();
                        self.auth = Auth::LoggedOut;
                        self.state = None;
                        self.login_error = "Your Spotify login expired. Please connect again.".into();
                    }
                }
                Event::State(Err(e)) => self.note = e.text(),
                Event::Toast(t) => self.show_toast(t),
                Event::Cover(url, img) => {
                    if Some(&url) == self.cover_url.as_ref() {
                        self.cover_tex = Some(ctx.load_texture("cover", img, TextureOptions::LINEAR));
                    }
                }
                Event::Thumb(url, img) => {
                    let tex = ctx.load_texture(format!("thumb-{url}"), img, TextureOptions::LINEAR);
                    self.thumbs.insert(url, tex);
                }
                Event::Search(gen, r) => {
                    if gen == self.search_gen {
                        self.searching = false;
                        match r {
                            Ok(items) => {
                                for it in &items {
                                    if let Some(t) = &it.thumb {
                                        self.fetch_thumb(t.clone());
                                    }
                                }
                                self.results = items;
                            }
                            Err(e) => {
                                self.results.clear();
                                self.show_toast(e);
                            }
                        }
                    }
                }
                Event::LoginDone(r) => match r {
                    Ok(()) => {
                        self.auth = Auth::LoggedIn;
                        self.login_error.clear();
                        let _ = self.wake.send(());
                    }
                    Err(e) => {
                        self.auth = Auth::LoggedOut;
                        self.login_error = e;
                    }
                },
            }
        }
    }

    // ---- Screens ----------------------------------------------------------
    fn login_ui(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.2);
            ui.label(RichText::new("RECORD PLAYER").size(30.0).strong().color(GOLD));
            ui.add_space(8.0);
            ui.label(
                RichText::new("Connect your Spotify account to control playback.\nSpotify Premium is needed for the controls.")
                    .color(FG),
            );
            ui.add_space(16.0);
            match self.auth {
                Auth::LoggingIn => {
                    ui.label(RichText::new("Waiting for you to approve in the browser...").color(GOLD));
                    ui.add_space(8.0);
                    ui.spinner();
                    ui.add_space(8.0);
                    if ui.button("Cancel").clicked() {
                        self.login_cancel.store(true, Ordering::Relaxed);
                    }
                    ui.ctx().request_repaint_after(Duration::from_millis(300));
                }
                _ => {
                    let b = egui::Button::new(RichText::new("Connect Spotify").size(22.0).color(DARK))
                        .fill(GOLD)
                        .min_size(vec2(240.0, 56.0));
                    if ui.add(b).clicked() {
                        self.start_login();
                    }
                }
            }
            if !self.login_error.is_empty() {
                ui.add_space(12.0);
                ui.label(RichText::new(&self.login_error).color(Color32::from_rgb(0xe0, 0x7a, 0x5f)));
            }
        });
    }

    fn player_ui(&mut self, ui: &mut Ui) {
        if self.auth != Auth::LoggedIn {
            self.login_ui(ui);
            return;
        }
        let avail = ui.available_size();
        if avail.x > avail.y * 1.2 {
            // Landscape tablet: record on the left, controls on the right.
            let left_w = (avail.y * 1.05).min(avail.x * 0.5);
            let right_w = (avail.x - left_w - 24.0).max(200.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui(vec2(left_w, avail.y), |ui| {
                    self.turntable_ui(ui, vec2(left_w, avail.y));
                });
                ui.add_space(12.0);
                ui.allocate_ui(vec2(right_w, avail.y), |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(((avail.y - 300.0) / 2.0).max(0.0));
                        self.controls_ui(ui, right_w);
                    });
                });
            });
        } else {
            // Portrait: record on top, controls underneath.
            let h = (avail.y * 0.45).min(avail.x);
            self.turntable_ui(ui, vec2(avail.x, h));
            ui.vertical_centered(|ui| self.controls_ui(ui, avail.x));
        }
    }

    fn turntable_ui(&mut self, ui: &mut Ui, size: egui::Vec2) {
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        draw_turntable(
            &ui.painter_at(rect),
            rect,
            self.angle,
            self.arm,
            self.cover_tex.as_ref(),
        );
        if resp.double_clicked() {
            self.toggle_play();
        }
    }

    fn controls_ui(&mut self, ui: &mut Ui, w: f32) {
        let (title, artist, album, dur) = match self.item() {
            Some(it) => (it.name.clone(), it.artist_line(), it.album_name(), it.duration_ms as f32),
            None => ("Nothing playing".to_string(), "Start music in Spotify".to_string(), String::new(), 0.0),
        };
        ui.add(egui::Label::new(RichText::new(title).size(24.0).strong().color(FG)).truncate());
        ui.add(egui::Label::new(RichText::new(artist).size(18.0).color(GOLD)).truncate());
        ui.add(egui::Label::new(RichText::new(album).size(14.0).color(DIM)).truncate());
        ui.add_space(6.0);

        // Seek bar
        let prog = self.progress_ms();
        let shown = self.seek_drag.unwrap_or(if dur > 0.0 { prog / dur } else { 0.0 });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let tw = 50.0;
            ui.add_sized([tw, 38.0], egui::Label::new(RichText::new(dur_text(shown * dur)).color(FG)));
            let (v, touching, released) = bar_slider(ui, (w - 2.0 * tw - 16.0).max(80.0), shown);
            if touching {
                self.seek_drag = Some(v);
            }
            if released && dur > 0.0 {
                let pos = (v * dur) as u64;
                if let Some(s) = self.state.as_mut() {
                    s.progress_ms = Some(pos);
                }
                self.last_sync = Instant::now();
                self.cmd("PUT", "/me/player/seek", vec![("position_ms", pos.to_string())]);
                self.seek_drag = None;
            } else if !touching {
                self.seek_drag = None;
            }
            ui.add_sized([tw, 38.0], egui::Label::new(RichText::new(dur_text(dur)).color(FG)));
        });

        // Buttons
        let small = 56.0;
        let big = 76.0;
        let gap = 12.0;
        let total = small * 4.0 + big + gap * 4.0;
        let (shuffle, repeat) = self
            .state
            .as_ref()
            .map(|s| (s.shuffle_state, s.repeat_state.clone()))
            .unwrap_or((false, "off".into()));
        let playing = self.is_playing();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.add_space(((w - total) / 2.0).max(0.0));
            if icon_button(ui, Icon::Shuffle, small, shuffle, false).clicked() {
                self.toggle_shuffle();
            }
            if icon_button(ui, Icon::Prev, small, false, false).clicked() {
                self.cmd("POST", "/me/player/previous", vec![]);
            }
            let pp = if playing { Icon::Pause } else { Icon::Play };
            if icon_button(ui, pp, big, false, true).clicked() {
                self.toggle_play();
            }
            if icon_button(ui, Icon::Next, small, false, false).clicked() {
                self.cmd("POST", "/me/player/next", vec![]);
            }
            let ricon = if repeat == "track" { Icon::RepeatOne } else { Icon::Repeat };
            if icon_button(ui, ricon, small, repeat != "off" && !repeat.is_empty(), false).clicked() {
                self.toggle_repeat();
            }
        });

        // Volume
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_sized([50.0, 38.0], egui::Label::new(RichText::new("Vol").color(DIM)));
            let shown = self.vol_drag.unwrap_or(self.volume / 100.0);
            let (v, touching, released) = bar_slider(ui, (w - 50.0 - 66.0).max(80.0), shown);
            if touching {
                self.vol_drag = Some(v);
            }
            if released {
                self.volume = v * 100.0;
                self.cmd(
                    "PUT",
                    "/me/player/volume",
                    vec![("volume_percent", ((v * 100.0).round() as i32).to_string())],
                );
                self.vol_drag = None;
            } else if !touching {
                self.vol_drag = None;
            }
            ui.add_sized([50.0, 38.0], egui::Label::new(RichText::new(format!("{}%", (shown * 100.0).round() as i32)).color(DIM)));
        });

        // Status line
        let msg = if Instant::now() < self.toast_until { self.toast.clone() } else { self.note.clone() };
        if !msg.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(msg).size(14.0).color(DIM));
        }
    }

    fn search_ui(&mut self, ui: &mut Ui) {
        if self.auth != Auth::LoggedIn {
            self.login_ui(ui);
            return;
        }
        let mut go = false;
        ui.horizontal(|ui| {
            let kb_w = 70.0;
            let btn_w = 96.0;
            let te = egui::TextEdit::singleline(&mut self.search_text)
                .hint_text("Search songs, albums, artists...")
                .font(FontId::proportional(20.0))
                .interactive(!self.settings.builtin_keyboard)
                .desired_width((ui.available_width() - kb_w - btn_w - 24.0).max(100.0));
            let r = ui.add_sized([ui.available_width() - kb_w - btn_w - 24.0, 44.0], te);
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                go = true;
            }
            if ui.add_sized([btn_w, 44.0], egui::Button::new("Search").fill(GOLD)).clicked() {
                go = true;
            }
            let on = self.settings.builtin_keyboard;
            let kb = egui::Button::new(RichText::new("Keys").color(if on { DARK } else { FG }))
                .fill(if on { GOLD } else { PANEL });
            if ui.add_sized([kb_w, 44.0], kb).clicked() {
                self.settings.builtin_keyboard = !self.settings.builtin_keyboard;
                self.settings.save();
            }
        });
        ui.horizontal(|ui| {
            for k in Kind::ALL {
                let sel = self.search_kind == k;
                let b = egui::Button::new(RichText::new(k.label()).color(if sel { DARK } else { FG }))
                    .fill(if sel { GOLD } else { PANEL });
                if ui.add_sized([100.0, 38.0], b).clicked() && !sel {
                    self.search_kind = k;
                    go = true;
                }
            }
        });
        if go {
            self.start_search();
        }

        if self.settings.builtin_keyboard {
            egui::Panel::bottom("builtin_kb")
                .frame(egui::Frame::new().fill(BG))
                .show(ui, |ui| self.keyboard_ui(ui));
        }

        ui.add_space(4.0);
        if self.searching {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Searching...").color(DIM));
            });
        } else if self.results.is_empty() && self.searched_once {
            ui.label(RichText::new("No results.").color(DIM));
        }

        let mut clicked: Option<SearchItem> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for it in &self.results {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 66.0), Sense::click());
                let p = ui.painter_at(rect);
                if resp.is_pointer_button_down_on() {
                    p.rect_filled(rect, 8.0, PANEL);
                } else {
                    p.rect_filled(rect, 8.0, DARK);
                }
                let th = egui::Rect::from_min_size(rect.min + vec2(7.0, 7.0), vec2(52.0, 52.0));
                match it.thumb.as_ref().and_then(|u| self.thumbs.get(u)) {
                    Some(t) => {
                        p.image(
                            t.id(),
                            th,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    None => {
                        p.rect_filled(th, 6.0, PANEL);
                    }
                }
                let text_clip = egui::Rect::from_min_max(
                    egui::pos2(th.right() + 10.0, rect.top()),
                    egui::pos2(rect.right() - 44.0, rect.bottom()),
                );
                let tp = p.with_clip_rect(text_clip);
                tp.text(
                    egui::pos2(text_clip.left(), rect.top() + 9.0),
                    Align2::LEFT_TOP,
                    &it.title,
                    FontId::proportional(18.0),
                    FG,
                );
                tp.text(
                    egui::pos2(text_clip.left(), rect.top() + 36.0),
                    Align2::LEFT_TOP,
                    &it.subtitle,
                    FontId::proportional(14.0),
                    DIM,
                );
                // little play triangle
                let c = egui::pos2(rect.right() - 24.0, rect.center().y);
                p.add(egui::Shape::convex_polygon(
                    vec![c + vec2(-6.0, -9.0), c + vec2(-6.0, 9.0), c + vec2(9.0, 0.0)],
                    GOLD,
                    egui::Stroke::NONE,
                ));
                if resp.clicked() {
                    clicked = Some(it.clone());
                }
                ui.add_space(4.0);
            }
        });
        if let Some(it) = clicked {
            self.play_item(it);
        }
    }

    /// A simple on-screen keyboard, for devices whose own keyboard doesn't work well here.
    fn keyboard_ui(&mut self, ui: &mut Ui) {
        let rows = ["1234567890", "qwertyuiop", "asdfghjkl", "zxcvbnm"];
        let gap = 6.0;
        ui.spacing_mut().item_spacing = vec2(gap, gap);
        let kw = ((ui.available_width() - gap * 9.0) / 10.0).clamp(30.0, 90.0);
        let kh = 46.0;
        let mut go = false;
        for row in rows {
            ui.horizontal(|ui| {
                let used = row.len() as f32 * kw + (row.len() as f32 - 1.0) * gap;
                ui.add_space(((ui.available_width() - used) / 2.0).max(0.0));
                for ch in row.chars() {
                    if ui.add_sized([kw, kh], egui::Button::new(RichText::new(ch.to_string()).size(20.0))).clicked() {
                        self.search_text.push(ch);
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            let total = kw * 2.0 + kw * 5.0 + kw * 2.0 + gap * 2.0;
            ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
            if ui.add_sized([kw * 2.0, kh], egui::Button::new(RichText::new("Del").size(18.0))).clicked() {
                self.search_text.pop();
            }
            if ui.add_sized([kw * 5.0, kh], egui::Button::new(RichText::new("space").size(18.0))).clicked() {
                self.search_text.push(' ');
            }
            if ui
                .add_sized([kw * 2.0, kh], egui::Button::new(RichText::new("Go").size(18.0).color(DARK)).fill(GOLD))
                .clicked()
            {
                go = true;
            }
        });
        if go {
            self.start_search();
        }
    }

    fn settings_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.label(RichText::new("Settings").size(28.0).strong().color(GOLD));
            ui.add_space(8.0);

            ui.label(RichText::new("STARTUP").size(14.0).color(DIM));
            let mut open = self.settings.open_on_startup;
            let cb = ui.add_enabled(
                platform::autostart_supported(),
                egui::Checkbox::new(&mut open, "Open Record Player when the tablet starts"),
            );
            if cb.changed() {
                match platform::set_autostart(open) {
                    Ok(()) => {
                        self.settings.open_on_startup = open;
                        self.settings.save();
                    }
                    Err(e) => self.show_toast(e),
                }
            }
            if platform::autostart_supported() {
                ui.label(
                    RichText::new("Open the app by hand once after installing. After a restart it can take a minute to appear.")
                        .size(13.0)
                        .color(DIM),
                );
            } else {
                ui.label(RichText::new("Only available in the Android app.").size(13.0).color(DIM));
            }
            ui.add_space(14.0);

            ui.label(RichText::new("DISPLAY").size(14.0).color(DIM));
            if ui
                .checkbox(&mut self.settings.keep_screen_on, "Keep the screen on while the app is open")
                .changed()
            {
                platform::keep_screen_on(self.settings.keep_screen_on);
                self.settings.save();
            }
            ui.add_space(6.0);
            ui.label(RichText::new("Interface size").color(FG));
            ui.spacing_mut().slider_width = (ui.available_width() - 120.0).clamp(160.0, 420.0);
            let r = ui.add(egui::Slider::new(&mut self.scale_edit, 0.7..=1.8).step_by(0.05));
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                self.settings.ui_scale = self.scale_edit;
                ctx.set_zoom_factor(self.scale_edit);
                self.settings.save();
            }
            ui.add_space(14.0);

            ui.label(RichText::new("SEARCH").size(14.0).color(DIM));
            if ui
                .checkbox(&mut self.settings.builtin_keyboard, "Use the built-in keyboard for search")
                .changed()
            {
                self.settings.save();
            }
            ui.add_space(14.0);

            ui.label(RichText::new("SPOTIFY").size(14.0).color(DIM));
            if self.auth == Auth::LoggedIn {
                if ui.button("Sign out").clicked() {
                    self.client.logout();
                    self.auth = Auth::LoggedOut;
                    self.state = None;
                    self.cover_tex = None;
                    self.cover_url = None;
                    self.results.clear();
                }
            } else {
                ui.label(RichText::new("Not connected.").color(FG));
            }
            ui.add_space(14.0);
            ui.label(
                RichText::new(format!(
                    "Record Player {}\nControls need Spotify Premium, and Spotify must be open on this tablet (or another device).",
                    env!("CARGO_PKG_VERSION")
                ))
                .size(13.0)
                .color(DIM),
            );
        });
    }
}

// ---------------------------------------------------------------------------
// eframe glue
// ---------------------------------------------------------------------------
impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_events(ctx);

        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let playing = self.is_playing();
        if playing {
            self.angle = (self.angle + 90.0 * dt) % 360.0; // one turn every 4 seconds
        }
        let target = if playing { 25.0 } else { 4.0 };
        self.arm += (target - self.arm) * (dt * 3.0).min(1.0);

        let moving = playing || (self.arm - target).abs() > 0.05;
        if moving && self.tab == Tab::Player {
            ctx.request_repaint_after(Duration::from_millis(33));
        } else {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        egui::Panel::bottom("tabs")
            .exact_size(64.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(6.0))
            .show(ui, |ui| {
                ui.columns(3, |cols| {
                    let tabs = [(Tab::Player, "Player"), (Tab::Search, "Search"), (Tab::Settings, "Settings")];
                    for (i, (tab, label)) in tabs.into_iter().enumerate() {
                        let sel = self.tab == tab;
                        let b = egui::Button::new(
                            RichText::new(label).size(20.0).color(if sel { DARK } else { FG }),
                        )
                        .fill(if sel { GOLD } else { DARK });
                        let w = cols[i].available_width();
                        if cols[i].add_sized([w, 48.0], b).clicked() {
                            self.tab = tab;
                        }
                    }
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(12.0))
            .show(ui, |ui| match self.tab {
                Tab::Player => self.player_ui(ui),
                Tab::Search => self.search_ui(ui),
                Tab::Settings => self.settings_ui(ui),
            });
    }
}

fn setup_style(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    style.visuals = egui::Visuals::dark();
    let v = &mut style.visuals;
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = DARK;
    v.override_text_color = Some(FG);
    v.selection.bg_fill = Color32::from_rgb(0x8a, 0x66, 0x2a);
    v.widgets.inactive.bg_fill = PANEL;
    v.widgets.inactive.weak_bg_fill = PANEL;
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x4a, 0x30, 0x1d);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0x5a, 0x3a, 0x22);
    style.spacing.item_spacing = vec2(10.0, 10.0);
    style.spacing.button_padding = vec2(14.0, 10.0);
    style.spacing.interact_size.y = 40.0;
    style.spacing.icon_width = 28.0;
    style.spacing.icon_width_inner = 16.0;
    for (ts, size) in [
        (TextStyle::Body, 18.0),
        (TextStyle::Button, 18.0),
        (TextStyle::Heading, 28.0),
        (TextStyle::Small, 13.0),
        (TextStyle::Monospace, 16.0),
    ] {
        if let Some(f) = style.text_styles.get_mut(&ts) {
            f.size = size;
        }
    }
    ctx.set_global_style(style);
}
