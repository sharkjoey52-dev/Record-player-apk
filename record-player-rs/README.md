# Record Player (Rust)

Spotify controller with a spinning vinyl: Player, Search and Settings tabs.
Built with Rust + egui. Targets old Fire tablets (Fire OS 5+, 32-bit and 64-bit).

- Controls need Spotify Premium, and the Spotify app must be open on the tablet (or another device).
- Spotify developer dashboard: your app must list `http://127.0.0.1:8888/` as a Redirect URI.
- The APK is built online by GitHub Actions (`.github/workflows/build-apk.yml`):
  Actions tab -> "Build APK" -> latest run -> Artifacts -> `record-player-apk`.

## Settings tab
- **Open Record Player when the tablet starts** - writes/removes a marker file that `BootReceiver.java` checks at boot.
- **Keep the screen on** - stops the tablet sleeping while the app is open.
- **Interface size** - zoom the whole UI.
- **Built-in keyboard** - on-screen keyboard for search if the tablet's own keyboard doesn't type.
- **Sign out**.

## Try it on a PC
    cargo run --release
