//! Horizon on Android (Starling): the cdylib's entry points.
//!
//! The crate root of horizon-android (crates/horizon-android/Cargo.toml). Android and the JVM look
//! these up by name in the library, so they live here, in the library that is linked last; each
//! one calls `horizon_client::android`, where the game's Android hooks and its start-up are.
#![cfg(target_os = "android")]

use winit::platform::android::activity::AndroidApp;

/// NativeActivity's entry point (the manifest's `android.app.func_name`): the game of this
/// library, or of the build with native mods StarlingActivity loaded in its place.
///
/// # Safety
///
/// NativeActivity's call: an `ANativeActivity` and its saved state.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn starling_on_create(
    activity: *mut std::ffi::c_void,
    saved: *mut std::ffi::c_void,
    saved_size: usize,
) {
    // SAFETY: NativeActivity's own arguments, passed on.
    unsafe { horizon_client::android::on_create(activity, saved, saved_size) }
}

/// android-activity's entry point calls it on the game's thread.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    horizon_client::android::main(app);
}

/// `StarlingActivity.onStorage`, called on the UI thread before the game's thread starts.
#[unsafe(no_mangle)]
extern "system" fn Java_io_github_juanluci_starling_StarlingActivity_onStorage(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    folder: jni::objects::JString,
) {
    if folder.is_null() {
        return;
    }
    if let Ok(folder) = env.get_string(&folder) {
        horizon_client::android::set_shared_folder(String::from(folder).into());
    }
}

/// `StarlingActivity.onImeHeight` (packaging/android/java), called on the UI thread.
#[unsafe(no_mangle)]
extern "system" fn Java_io_github_juanluci_starling_StarlingActivity_onImeHeight(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
    pixels: i32,
) {
    horizon_client::android::set_ime_height(pixels);
}

/// `StarlingActivity.onSafeInsets` (packaging/android/java), called on the UI thread.
#[unsafe(no_mangle)]
extern "system" fn Java_io_github_juanluci_starling_StarlingActivity_onSafeInsets(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
) {
    horizon_client::android::set_safe_insets([left, top, right, bottom]);
}

/// `TextMirror.onMirrorEdit`, called on the UI thread.
#[unsafe(no_mangle)]
extern "system" fn Java_io_github_juanluci_starling_TextMirror_onMirrorEdit(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    text: jni::objects::JString,
    cursor: i32,
    anchor: i32,
    generation: i32,
) {
    let Ok(text) = env.get_string(&text) else {
        return;
    };
    horizon_client::android::mirror_edit(String::from(text), cursor, anchor, generation);
}

/// `TextMirror.onMirrorAction`, called on the UI thread.
#[unsafe(no_mangle)]
extern "system" fn Java_io_github_juanluci_starling_TextMirror_onMirrorAction(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
) {
    horizon_client::android::mirror_action();
}

/// `TextMirror.onMirrorKey`, called on the UI thread.
#[unsafe(no_mangle)]
extern "system" fn Java_io_github_juanluci_starling_TextMirror_onMirrorKey(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
    code: i32,
) {
    horizon_client::android::mirror_key(code);
}

/// `StarlingActivity.onPointerCaptureLost`, called on the UI thread.
#[unsafe(no_mangle)]
extern "system" fn Java_io_github_juanluci_starling_StarlingActivity_onPointerCaptureLost(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
) {
    horizon_client::android::note_pointer_capture_lost();
}

/// The system diagnostics window calls this on its worker, without starting the game.
#[unsafe(no_mangle)]
extern "system" fn Java_io_github_juanluci_starling_DiagnosticsActivity_makeReport(
    mut env: jni::JNIEnv,
    activity: jni::objects::JObject,
    private: jni::objects::JString,
    game: jni::objects::JString,
    export: jni::sys::jboolean,
) -> jni::sys::jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<String, String> {
            let private = std::path::PathBuf::from(String::from(
                env.get_string(&private).map_err(|e| e.to_string())?,
            ));
            let game = std::path::PathBuf::from(String::from(
                env.get_string(&game).map_err(|e| e.to_string())?,
            ));
            horizon_client::android::recovery_report(
                &mut env,
                &activity,
                &private,
                &game,
                export != 0,
            )
        },
    ))
    .unwrap_or_else(|_| Err("Diagnostic worker failed".into()));
    match result {
        Ok(text) => env
            .new_string(text)
            .map_or(std::ptr::null_mut(), |text| text.into_raw()),
        Err(error) => {
            let _ = env.throw_new("java/io/IOException", error);
            std::ptr::null_mut()
        }
    }
}
