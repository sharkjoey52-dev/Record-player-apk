// Desktop entry point (handy for testing on a PC). The Android app starts from
// `android_main` in lib.rs instead.

fn main() -> eframe::Result<()> {
    #[allow(unused_mut)]
    let mut size = [1000.0_f32, 600.0];
    #[cfg(debug_assertions)]
    if let Ok(s) = std::env::var("RP_SIZE") {
        if let Some((w, h)) = s.split_once('x') {
            size = [w.parse().unwrap_or(size[0]), h.parse().unwrap_or(size[1])];
        }
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Record Player")
            .with_inner_size(size),
        ..Default::default()
    };
    record_player::run(options)
}
