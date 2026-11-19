//! Things that differ between Android and a desktop PC:
//! where files live, opening the browser, start-on-boot, keeping the screen on.

use std::path::PathBuf;
use std::sync::OnceLock;

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Called once at startup. On Android this is the app's private files folder.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn init_data_dir(p: PathBuf) {
    let _ = DATA_DIR.set(p);
}

pub fn data_dir() -> PathBuf {
    DATA_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Android
// ---------------------------------------------------------------------------
#[cfg(target_os = "android")]
mod imp {
    use super::data_dir;
    use android_activity::{AndroidApp, WindowManagerFlags};
    use jni::objects::{JObject, JValue};
    use jni::JavaVM;
    use std::cell::RefCell;

    thread_local! {
        // AndroidApp may only be used from the main (android_main) thread,
        // which is also the thread egui draws on.
        static APP: RefCell<Option<AndroidApp>> = const { RefCell::new(None) };
    }

    pub fn android_init(app: &AndroidApp) {
        if let Some(p) = app.internal_data_path() {
            super::init_data_dir(p);
        }
        APP.with(|a| *a.borrow_mut() = Some(app.clone()));
    }

    pub fn open_url(url: &str) -> Result<(), String> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| e.to_string())?;
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        let activity = unsafe { JObject::from_raw(ctx.context().cast()) };

        let result: Result<(), jni::errors::Error> = (|| {
            let url_j = env.new_string(url)?;
            let uri = env
                .call_static_method(
                    "android/net/Uri",
                    "parse",
                    "(Ljava/lang/String;)Landroid/net/Uri;",
                    &[JValue::Object(&url_j)],
                )?
                .l()?;
            let action = env.new_string("android.intent.action.VIEW")?;
            let intent = env.new_object(
                "android/content/Intent",
                "(Ljava/lang/String;Landroid/net/Uri;)V",
                &[JValue::Object(&action), JValue::Object(&uri)],
            )?;
            // FLAG_ACTIVITY_NEW_TASK
            env.call_method(
                &intent,
                "addFlags",
                "(I)Landroid/content/Intent;",
                &[JValue::Int(0x1000_0000)],
            )?;
            env.call_method(
                &activity,
                "startActivity",
                "(Landroid/content/Intent;)V",
                &[JValue::Object(&intent)],
            )?;
            Ok(())
        })();

        if result.is_err() {
            let _ = env.exception_clear();
        }
        result.map_err(|e| format!("Could not open the browser: {e}"))
    }

    pub fn autostart_supported() -> bool {
        true
    }

    /// The boot receiver (BootReceiver.java) starts the app on boot only if this
    /// marker file exists.
    pub fn set_autostart(on: bool) -> Result<(), String> {
        let marker = data_dir().join("autostart");
        if on {
            std::fs::write(&marker, b"1").map_err(|e| e.to_string())
        } else {
            match std::fs::remove_file(&marker) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        }
    }

    pub fn keep_screen_on(on: bool) {
        APP.with(|a| {
            if let Some(app) = a.borrow().as_ref() {
                if on {
                    app.set_window_flags(
                        WindowManagerFlags::KEEP_SCREEN_ON,
                        WindowManagerFlags::empty(),
                    );
                } else {
                    app.set_window_flags(
                        WindowManagerFlags::empty(),
                        WindowManagerFlags::KEEP_SCREEN_ON,
                    );
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Desktop (Windows / macOS / Linux) - lets you test on a PC
// ---------------------------------------------------------------------------
#[cfg(not(target_os = "android"))]
mod imp {
    use std::process::Command;

    pub fn open_url(url: &str) -> Result<(), String> {
        #[cfg(target_os = "windows")]
        let r = Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn();
        #[cfg(target_os = "macos")]
        let r = Command::new("open").arg(url).spawn();
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let r = Command::new("xdg-open").arg(url).spawn();
        r.map(|_| ())
            .map_err(|e| format!("Could not open the browser: {e}"))
    }

    pub fn autostart_supported() -> bool {
        false
    }

    pub fn set_autostart(_on: bool) -> Result<(), String> {
        Err("Start-on-boot is only available in the Android app.".into())
    }

    pub fn keep_screen_on(_on: bool) {}
}

pub use imp::*;
