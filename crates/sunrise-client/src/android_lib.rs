//! Sunrise on Android (Starling): the cdylib's entry points.
//!
//! The crate root of sunrise-android (crates/sunrise-android/Cargo.toml). Android and the JVM look
//! these up by name in the library, so they live here, in the library that is linked last; each
//! one calls `sunrise_client::android`, where the game's Android hooks and its start-up are.
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
    unsafe { sunrise_client::android::on_create(activity, saved, saved_size) }
}

/// android-activity's entry point calls it on the game's thread.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    sunrise_client::android::main(app, install_platform, start);
}

/// What the desktop's mod host installs before the engine starts (sunrise-mod-host's `main_with`),
/// in its order: the Mods button, the mods' instrumentation, the engine platform socket and the
/// render points. The phone starts the engine from here instead of through `main_with`; without
/// these the game stops at its first mod guard ("engine platform socket is not installed").
fn install_platform() -> Result<(), String> {
    register_parts();
    sunrise_mod_host::install_platform()?;
    mod_render::render::install();
    Ok(())
}

/// Sunrise's in-game parts Starling ships, registered before the game reads them, as the official
/// desktop game does (sunrise-official's `register`): the performance overlay (Sunrise Settings,
/// Extras, Performance Overlay) and auto-reconnect.
fn register_parts() {
    use sunrise_hooks::{hud, lang, pages, settings};
    let results = [
        sunrise_hooks::register(&sunrise_reconnect::HOOKS),
        sunrise_hooks::register(&sunrise_perf_overlay::HOOKS),
        settings::register(&sunrise_reconnect::SETTING),
        settings::register(&sunrise_perf_overlay::SETTING),
        lang::register(sunrise_reconnect::LANG),
        pages::register(&sunrise_reconnect::SECTION),
        hud::register(&sunrise_perf_overlay::LAYER),
    ];
    for result in results {
        if let Err(e) = result {
            log::warn!("Starling: an in-game part did not register: {e}");
        }
    }
}

/// Where the native mod libraries for this game build are, in the app's own storage
/// (`SUNRISE_PRIVATE`): `native-mods.json`, each mod's library, and each mod's package in
/// `sunrise-native-mods/<id>/`, as the desktop keeps them beside its fixed executable.
const NATIVE_LIBRARIES: &str = "native-libs";

/// The engine's start with the native mod libraries of [`NATIVE_LIBRARIES`], loaded as the
/// desktop's fixed executable loads the ones beside it (sunrise-mod-host's `open`, then
/// `load_code`). A build of this library with native mods compiled in
/// (packaging/android/native_build.sh) has none in that folder.
fn start(startup: sunrise_client::entry::Startup) {
    let (entries, native) = native_libraries();
    sunrise_client::entry::start(
        native,
        move |native, launch| sunrise_mod_host::load_code(entries, native, launch),
        startup,
    );
}

/// The native mod libraries of [`NATIVE_LIBRARIES`], checked and opened, with their packages. Each
/// was built for this exact game build (its `sunrise_native_build`). A folder that does not check
/// out is left out with a line in logcat, and the game starts without those mods.
fn native_libraries() -> (
    Vec<mod_platform::ModEntry>,
    sunrise_mod_package::runtime::Runtime,
) {
    let Some(private) = std::env::var_os("SUNRISE_PRIVATE") else {
        return Default::default();
    };
    let folder = std::path::PathBuf::from(private).join(NATIVE_LIBRARIES);
    if !folder.join("native-mods.json").is_file() {
        return Default::default();
    }
    let opened = match sunrise_mod_host::open(&folder) {
        Ok(opened) => opened,
        Err(error) => {
            sunrise_client::android::warn(&format!("native mods left out: {error}"));
            return Default::default();
        }
    };
    match sunrise_mod_host::packages_beside(&folder, opened.built) {
        Ok(native) => {
            sunrise_client::android::set_native_libs_opened(
                opened.built.iter().map(|built| built.id).collect(),
            );
            (opened.entries, native)
        }
        Err(error) => {
            // Their patch copies are installed already; none of their hooks will be called.
            sunrise_client::android::warn(&format!("native mods left out: {error}"));
            Default::default()
        }
    }
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
        sunrise_client::android::set_shared_folder(String::from(folder).into());
    }
}

/// `StarlingActivity.nativeDocumentResult`: the launcher's file picker answered (request 4101).
#[unsafe(no_mangle)]
extern "system" fn Java_io_github_juanluci_starling_StarlingActivity_nativeDocumentResult(
    env: jni::JNIEnv,
    activity: jni::objects::JObject,
    request: i32,
    result: i32,
    uri: jni::objects::JString,
) {
    sunrise_client::ha_launcher::document_result(env, activity, request, result, uri);
}

/// `StarlingActivity.onImeHeight` (packaging/android/java), called on the UI thread.
#[unsafe(no_mangle)]
extern "system" fn Java_io_github_juanluci_starling_StarlingActivity_onImeHeight(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
    pixels: i32,
) {
    sunrise_client::android::set_ime_height(pixels);
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
    sunrise_client::android::set_safe_insets([left, top, right, bottom]);
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
    sunrise_client::android::mirror_edit(String::from(text), cursor, anchor, generation);
}

/// `TextMirror.onMirrorAction`, called on the UI thread.
#[unsafe(no_mangle)]
extern "system" fn Java_io_github_juanluci_starling_TextMirror_onMirrorAction(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
) {
    sunrise_client::android::mirror_action();
}

/// `TextMirror.onMirrorKey`, called on the UI thread.
#[unsafe(no_mangle)]
extern "system" fn Java_io_github_juanluci_starling_TextMirror_onMirrorKey(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
    code: i32,
) {
    sunrise_client::android::mirror_key(code);
}

/// `StarlingActivity.onPointerCaptureLost`, called on the UI thread.
#[unsafe(no_mangle)]
extern "system" fn Java_io_github_juanluci_starling_StarlingActivity_onPointerCaptureLost(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
) {
    sunrise_client::android::note_pointer_capture_lost();
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
            sunrise_client::android::recovery_report(
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
