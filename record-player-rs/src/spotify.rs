//! Spotify login (PKCE, no client secret) and Web API calls.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::platform;

// Your Spotify Client ID (the Redirect URI below must be added to that app in
// the Spotify developer dashboard).
pub const CLIENT_ID: &str = "62fb3ba99dc441cd90b5e18a35710fb8";
const REDIRECT_URI: &str = "http://127.0.0.1:8888/";
const LISTEN_ADDR: &str = "127.0.0.1:8888";
const SCOPES: &str =
    "user-read-playback-state user-modify-playback-state user-read-currently-playing";
const API: &str = "https://api.spotify.com/v1";

#[derive(Debug, Clone)]
pub enum ApiError {
    /// Login is gone or expired - the user must sign in again.
    Auth,
    Msg(String),
}

impl ApiError {
    pub fn text(&self) -> String {
        match self {
            ApiError::Auth => "Login expired. Please connect Spotify again.".into(),
            ApiError::Msg(m) => friendly(m),
        }
    }
}

/// Turn Spotify's error text into something helpful.
pub fn friendly(msg: &str) -> String {
    let up = msg.to_uppercase();
    if up.contains("PREMIUM") {
        "Controls need Spotify Premium.".into()
    } else if up.contains("NO_ACTIVE_DEVICE") || up.contains("NO ACTIVE DEVICE") {
        "Open Spotify on this tablet and press play once.".into()
    } else {
        msg.to_string()
    }
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Image {
    pub url: String,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Named {
    pub name: String,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Album {
    pub name: String,
    pub images: Vec<Image>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Show {
    pub name: String,
    pub images: Vec<Image>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Item {
    pub name: String,
    pub duration_ms: u64,
    pub album: Option<Album>,
    pub artists: Vec<Named>,
    pub show: Option<Show>,
    pub images: Vec<Image>,
}

impl Item {
    pub fn artist_line(&self) -> String {
        let a: Vec<&str> = self.artists.iter().map(|a| a.name.as_str()).collect();
        if !a.is_empty() {
            a.join(", ")
        } else {
            self.show.as_ref().map(|s| s.name.clone()).unwrap_or_default()
        }
    }

    pub fn album_name(&self) -> String {
        self.album.as_ref().map(|a| a.name.clone()).unwrap_or_default()
    }

    /// Medium-size cover (Spotify lists images biggest first).
    pub fn cover_url(&self) -> Option<String> {
        let imgs: &Vec<Image> = match &self.album {
            Some(a) if !a.images.is_empty() => &a.images,
            _ if !self.images.is_empty() => &self.images,
            _ => match &self.show {
                Some(s) => &s.images,
                None => return None,
            },
        };
        let img = if imgs.len() > 1 { &imgs[1] } else { imgs.first()? };
        Some(img.url.clone())
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Device {
    pub volume_percent: Option<u32>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PlayerState {
    pub is_playing: bool,
    pub progress_ms: Option<u64>,
    pub shuffle_state: bool,
    pub repeat_state: String,
    pub device: Option<Device>,
    pub item: Option<Item>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Track,
    Album,
    Artist,
    Playlist,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Track, Kind::Album, Kind::Artist, Kind::Playlist];

    pub fn api_type(self) -> &'static str {
        match self {
            Kind::Track => "track",
            Kind::Album => "album",
            Kind::Artist => "artist",
            Kind::Playlist => "playlist",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Track => "Songs",
            Kind::Album => "Albums",
            Kind::Artist => "Artists",
            Kind::Playlist => "Playlists",
        }
    }

    fn result_key(self) -> &'static str {
        match self {
            Kind::Track => "tracks",
            Kind::Album => "albums",
            Kind::Artist => "artists",
            Kind::Playlist => "playlists",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SearchItem {
    pub kind: Kind,
    pub title: String,
    pub subtitle: String,
    pub uri: String,
    pub thumb: Option<String>,
}

fn names(v: &Value) -> String {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x["name"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// Smallest image in a Spotify "images" array.
fn small_image(v: &Value) -> Option<String> {
    v.as_array()?.last()?["url"].as_str().map(String::from)
}

pub fn parse_search(kind: Kind, v: &Value) -> Vec<SearchItem> {
    let items = match v[kind.result_key()]["items"].as_array() {
        Some(i) => i,
        None => return vec![],
    };
    items
        .iter()
        // Spotify sometimes returns null entries (especially for playlists).
        .filter(|it| it.is_object())
        .filter_map(|it| {
            let title = it["name"].as_str()?.to_string();
            let uri = it["uri"].as_str()?.to_string();
            let (subtitle, thumb) = match kind {
                Kind::Track => {
                    let artists = names(&it["artists"]);
                    let album = it["album"]["name"].as_str().unwrap_or("");
                    let sub = if album.is_empty() {
                        artists
                    } else {
                        format!("{artists} \u{b7} {album}")
                    };
                    (sub, small_image(&it["album"]["images"]))
                }
                Kind::Album => (names(&it["artists"]), small_image(&it["images"])),
                Kind::Artist => ("Artist".to_string(), small_image(&it["images"])),
                Kind::Playlist => (
                    it["owner"]["display_name"]
                        .as_str()
                        .map(|o| format!("Playlist \u{b7} {o}"))
                        .unwrap_or_else(|| "Playlist".into()),
                    small_image(&it["images"]),
                ),
            };
            Some(SearchItem { kind, title, subtitle, uri, thumb })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------
#[derive(Serialize, Deserialize, Default, Clone)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    expires_at: f64,
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn token_path() -> std::path::PathBuf {
    platform::data_dir().join("spotify_tokens.json")
}

#[derive(Clone)]
pub struct Spotify {
    agent: ureq::Agent,
    tokens: Arc<Mutex<Tokens>>,
    refresh_lock: Arc<Mutex<()>>,
}

impl Spotify {
    pub fn new() -> Spotify {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(20)))
            .build()
            .into();
        let tokens = std::fs::read_to_string(token_path())
            .ok()
            .and_then(|s| serde_json::from_str::<Tokens>(&s).ok())
            .unwrap_or_default();
        Spotify {
            agent,
            tokens: Arc::new(Mutex::new(tokens)),
            refresh_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn has_login(&self) -> bool {
        !self.tokens.lock().unwrap().refresh_token.is_empty()
    }

    pub fn logout(&self) {
        *self.tokens.lock().unwrap() = Tokens::default();
        let _ = std::fs::remove_file(token_path());
    }

    fn save_tokens(&self) {
        let t = self.tokens.lock().unwrap().clone();
        if let Ok(s) = serde_json::to_string(&t) {
            let _ = std::fs::write(token_path(), s);
        }
    }

    /// POST to the token endpoint. Ok(json) on success.
    fn token_request(&self, form: &[(&str, &str)]) -> Result<Value, ApiError> {
        let mut resp = self
            .agent
            .post("https://accounts.spotify.com/api/token")
            .send_form(form.iter().copied())
            .map_err(|e| ApiError::Msg(format!("Network problem: {e}")))?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| ApiError::Msg(format!("Network problem: {e}")))?;
        let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        if status == 200 {
            Ok(v)
        } else if status == 400 || status == 401 {
            if v["error"].as_str() == Some("invalid_grant") || status == 401 {
                Err(ApiError::Auth)
            } else {
                Err(ApiError::Msg(format!(
                    "Spotify login error: {}",
                    v["error_description"].as_str().unwrap_or("unknown")
                )))
            }
        } else {
            Err(ApiError::Msg(format!("Spotify login error {status}")))
        }
    }

    fn store(&self, v: &Value) -> Result<(), ApiError> {
        let access = v["access_token"]
            .as_str()
            .ok_or_else(|| ApiError::Msg("Spotify sent no access token".into()))?;
        {
            let mut t = self.tokens.lock().unwrap();
            t.access_token = access.to_string();
            if let Some(r) = v["refresh_token"].as_str() {
                t.refresh_token = r.to_string();
            }
            t.expires_at = now() + v["expires_in"].as_f64().unwrap_or(3600.0);
        }
        self.save_tokens();
        Ok(())
    }

    fn refresh(&self) -> Result<(), ApiError> {
        let rt = self.tokens.lock().unwrap().refresh_token.clone();
        if rt.is_empty() {
            return Err(ApiError::Auth);
        }
        let v = self.token_request(&[
            ("client_id", CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", &rt),
        ])?;
        self.store(&v)
    }

    fn access_token(&self) -> Result<String, ApiError> {
        let _g = self.refresh_lock.lock().unwrap();
        let (empty, expired) = {
            let t = self.tokens.lock().unwrap();
            (t.refresh_token.is_empty(), now() > t.expires_at - 60.0)
        };
        if empty {
            return Err(ApiError::Auth);
        }
        if expired {
            self.refresh()?;
        }
        Ok(self.tokens.lock().unwrap().access_token.clone())
    }

    // ---- Login in the browser --------------------------------------------
    /// Opens Spotify's login page in the browser and waits for the redirect
    /// back to this device. Blocks (run it on a thread).
    pub fn login(&self, cancel: &AtomicBool) -> Result<(), String> {
        let mut raw = [0u8; 64];
        getrandom::getrandom(&mut raw).map_err(|e| e.to_string())?;
        let verifier = URL_SAFE_NO_PAD.encode(raw);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut st = [0u8; 16];
        getrandom::getrandom(&mut st).map_err(|e| e.to_string())?;
        let state = URL_SAFE_NO_PAD.encode(st);

        let listener = TcpListener::bind(LISTEN_ADDR)
            .map_err(|e| format!("Could not listen on {LISTEN_ADDR} ({e}). Is the app already open?"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;

        let enc = |s: &str| urlencoding::encode(s).into_owned();
        let url = format!(
            "https://accounts.spotify.com/authorize?client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge_method=S256&code_challenge={}&state={}",
            enc(CLIENT_ID), enc(REDIRECT_URI), enc(SCOPES), enc(&challenge), enc(&state)
        );
        platform::open_url(&url)?;

        let deadline = Instant::now() + Duration::from_secs(300);
        let (code, error) = loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("Login cancelled.".into());
            }
            if Instant::now() > deadline {
                return Err("Login timed out. Please try again.".into());
            }
            match listener.accept() {
                Ok((mut s, _)) => {
                    let _ = s.set_nonblocking(false);
                    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut buf = [0u8; 4096];
                    let n = s.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let (code, error, rstate) = parse_callback(&req);
                    let done = code.is_some() || error.is_some();
                    let (status, html) = if done {
                        ("200 OK", "<html><body style='font-family:sans-serif;background:#2a1a10;color:#f1e6d4;text-align:center;padding-top:80px'><h2>Connected!</h2><p>You can close this tab and go back to Record Player.</p></body></html>")
                    } else {
                        ("404 Not Found", "")
                    };
                    let resp = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                        html.len()
                    );
                    let _ = s.write_all(resp.as_bytes());
                    let _ = s.flush();
                    if done {
                        if rstate.as_deref() != Some(state.as_str()) {
                            return Err("Login failed (state mismatch). Please try again.".into());
                        }
                        break (code, error);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(120));
                }
                Err(e) => return Err(e.to_string()),
            }
        };
        drop(listener);

        if let Some(e) = error {
            return Err(format!("Spotify login failed: {e}"));
        }
        let code = code.ok_or("Spotify sent no login code")?;
        let v = self
            .token_request(&[
                ("client_id", CLIENT_ID),
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", REDIRECT_URI),
                ("code_verifier", &verifier),
            ])
            .map_err(|e| e.text())?;
        self.store(&v).map_err(|e| e.text())
    }

    // ---- Web API ----------------------------------------------------------
    fn call_once(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, String)],
        body: &Option<Value>,
    ) -> Result<(u16, String), ApiError> {
        let token = self.access_token()?;
        let url = format!("{API}{path}");
        let auth = format!("Bearer {token}");
        let net = |e: ureq::Error| ApiError::Msg(format!("Network problem: {e}"));

        let result = match method {
            "GET" => {
                let mut r = self.agent.get(&url).header("Authorization", &auth);
                for (k, v) in query {
                    r = r.query(k, v);
                }
                r.call()
            }
            "PUT" | "POST" => {
                let mut r = if method == "PUT" {
                    self.agent.put(&url)
                } else {
                    self.agent.post(&url)
                };
                r = r.header("Authorization", &auth);
                for (k, v) in query {
                    r = r.query(k, v);
                }
                match body {
                    Some(b) => r.send_json(b),
                    None => r.send_empty(),
                }
            }
            _ => return Err(ApiError::Msg("Unsupported method".into())),
        };
        let mut resp = result.map_err(net)?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        Ok((status, text))
    }

    /// Ok(None) for empty answers (204 etc).
    pub fn api(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Option<Value>, ApiError> {
        let mut r = self.call_once(method, path, query, &body)?;
        if r.0 == 401 {
            // Token may have been revoked/expired early: refresh once and retry.
            {
                let _g = self.refresh_lock.lock().unwrap();
                self.refresh()?;
            }
            r = self.call_once(method, path, query, &body)?;
            if r.0 == 401 {
                return Err(ApiError::Auth);
            }
        }
        let (status, text) = r;
        match status {
            200 => Ok(serde_json::from_str(&text).ok()),
            201 | 202 | 204 => Ok(None),
            429 => Err(ApiError::Msg("Spotify says slow down - try again in a moment.".into())),
            _ => {
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let msg = v["error"]["message"].as_str().unwrap_or("");
                let reason = v["error"]["reason"].as_str().unwrap_or("");
                if msg.is_empty() && reason.is_empty() {
                    Err(ApiError::Msg(format!("Spotify error {status}")))
                } else {
                    Err(ApiError::Msg(format!("{msg} [{reason}]")))
                }
            }
        }
    }

    pub fn player_state(&self) -> Result<Option<PlayerState>, ApiError> {
        let v = self.api(
            "GET",
            "/me/player",
            &[("additional_types", "track,episode".to_string())],
            None,
        )?;
        match v {
            None => Ok(None),
            Some(v) => serde_json::from_value(v)
                .map(Some)
                .map_err(|e| ApiError::Msg(format!("Unexpected Spotify answer: {e}"))),
        }
    }

    pub fn search(&self, q: &str, kind: Kind) -> Result<Vec<SearchItem>, ApiError> {
        let v = self.api(
            "GET",
            "/search",
            &[
                ("q", q.to_string()),
                ("type", kind.api_type().to_string()),
                ("limit", "10".to_string()),
            ],
            None,
        )?;
        Ok(v.map(|v| parse_search(kind, &v)).unwrap_or_default())
    }

    /// Play a song (or a whole album / artist / playlist). If Spotify has no
    /// active device but one is open (e.g. the Spotify app on this tablet is
    /// idle), wake that device up and try again.
    pub fn play(&self, kind: Kind, uri: &str) -> Result<(), ApiError> {
        let body = if kind == Kind::Track {
            json!({ "uris": [uri] })
        } else {
            json!({ "context_uri": uri })
        };
        match self.api("PUT", "/me/player/play", &[], Some(body.clone())) {
            Ok(_) => Ok(()),
            Err(ApiError::Msg(m)) if m.to_uppercase().contains("NO_ACTIVE_DEVICE") => {
                let devs = self.api("GET", "/me/player/devices", &[], None)?;
                let id = devs
                    .as_ref()
                    .and_then(|d| d["devices"].as_array())
                    .and_then(|a| {
                        a.iter()
                            .find(|d| d["is_restricted"].as_bool() != Some(true))
                            .and_then(|d| d["id"].as_str())
                    })
                    .map(String::from);
                match id {
                    Some(id) => self
                        .api("PUT", "/me/player/play", &[("device_id", id)], Some(body))
                        .map(|_| ()),
                    None => Err(ApiError::Msg("NO_ACTIVE_DEVICE".into())),
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Download and decode an image (album cover or thumbnail).
    pub fn fetch_image(&self, url: &str) -> Option<eframe::egui::ColorImage> {
        let mut resp = self.agent.get(url).call().ok()?;
        if resp.status().as_u16() != 200 {
            return None;
        }
        let bytes = resp.body_mut().read_to_vec().ok()?;
        let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        Some(eframe::egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
    }
}

/// Pull `code`, `error` and `state` out of "GET /?code=...&state=... HTTP/1.1".
fn parse_callback(req: &str) -> (Option<String>, Option<String>, Option<String>) {
    let line = req.lines().next().unwrap_or("");
    let path = line.split_whitespace().nth(1).unwrap_or("");
    let query = path.split_once('?').map(|x| x.1).unwrap_or("");
    let (mut code, mut error, mut state) = (None, None, None);
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let v = urlencoding::decode(v).map(|c| c.into_owned()).unwrap_or_default();
            match k {
                "code" => code = Some(v),
                "error" => error = Some(v),
                "state" => state = Some(v),
                _ => {}
            }
        }
    }
    (code, error, state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_parsing() {
        let r = "GET /?code=abc%2B123&state=xyz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let (c, e, s) = parse_callback(r);
        assert_eq!(c.as_deref(), Some("abc+123"));
        assert_eq!(e, None);
        assert_eq!(s.as_deref(), Some("xyz"));
        let (c, e, _) = parse_callback("GET /?error=access_denied&state=q HTTP/1.1\r\n\r\n");
        assert_eq!(c, None);
        assert_eq!(e.as_deref(), Some("access_denied"));
        let (c, e, _) = parse_callback("GET /favicon.ico HTTP/1.1\r\n\r\n");
        assert!(c.is_none() && e.is_none());
    }

    #[test]
    fn search_parsing_skips_nulls() {
        let v: Value = serde_json::from_str(
            r#"{"playlists":{"items":[null,{"name":"Chill","uri":"spotify:playlist:1","owner":{"display_name":"Bob"},"images":[{"url":"u1"},{"url":"u2"}]}]},
                "tracks":{"items":[{"name":"Song","uri":"spotify:track:1","artists":[{"name":"A"},{"name":"B"}],"album":{"name":"Alb","images":[{"url":"big"},{"url":"small"}]}}]}}"#,
        )
        .unwrap();
        let p = parse_search(Kind::Playlist, &v);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].subtitle, "Playlist \u{b7} Bob");
        assert_eq!(p[0].thumb.as_deref(), Some("u2"));
        let t = parse_search(Kind::Track, &v);
        assert_eq!(t[0].subtitle, "A, B \u{b7} Alb");
        assert_eq!(t[0].thumb.as_deref(), Some("small"));
    }

    #[test]
    fn state_parsing() {
        let v: Value = serde_json::from_str(
            r#"{"is_playing":true,"progress_ms":1000,"shuffle_state":false,"repeat_state":"off",
                "device":{"volume_percent":40},
                "item":{"name":"T","duration_ms":200000,"artists":[{"name":"X"}],
                        "album":{"name":"Al","images":[{"url":"a"},{"url":"b"}]}}}"#,
        )
        .unwrap();
        let s: PlayerState = serde_json::from_value(v).unwrap();
        let it = s.item.unwrap();
        assert_eq!(it.artist_line(), "X");
        assert_eq!(it.cover_url().as_deref(), Some("b"));
        assert_eq!(s.device.unwrap().volume_percent, Some(40));
    }

    #[test]
    fn friendly_messages() {
        assert_eq!(friendly("Player command failed: Premium required [PREMIUM_REQUIRED]"), "Controls need Spotify Premium.");
        assert!(friendly("x [NO_ACTIVE_DEVICE]").contains("press play"));
    }
}
