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
    horizon_client::android::main(app, install_platform, start);
}

/// What the desktop's mod host installs before the engine starts (horizon-mod-host's `main_with`,
/// crates/horizon-mod-host/src/loading.rs), in its order: the engine's seal, the mods'
/// instrumentation, the engine platform socket (packages, registration data, parts, profiler) and
/// the render points. The phone starts the engine from here instead of through `main_with`, which
/// loads native libraries next to an executable the phone does not have; without these the game
/// stops at its first mod guard ("engine platform socket is not installed").
fn install_platform() -> Result<(), String> {
    horizon_engine_slots::install_seal(mod_points::__private::seal)
        .map_err(|error| format!("engine seal: {error}"))?;
    mod_platform::__private::initialize()
        .map_err(|error| format!("mod instrumentation is unavailable: {error}"))?;
    horizon_engine_slots::install_platform(horizon_engine_slots::Platform {
        install_packages: mod_platform::__private::install_packages,
        install_registration_data: mod_platform::__private::install_registration_data,
        install_parts: mod_platform::__private::install_parts,
        profiler_is_enabled: mod_platform::__private::profiler::is_enabled,
        profiler_start: mod_platform::__private::profiler::start,
        profiler_stop: mod_platform::__private::profiler::stop,
        profiler_summary: mod_platform::__private::profiler::summary,
        profiler_firefox_profile: mod_platform::__private::profiler::firefox_profile,
    })
    .map_err(|error| error.to_string())?;
    mod_render::render::install();
    Ok(())
}

/// Where the native mod libraries for this game build are, in the app's own storage
/// (`HORIZON_PRIVATE`): `native-mods.json`, each mod's library, and each mod's package in
/// `horizon-native-mods/<id>/`, as the desktop keeps them beside its fixed executable.
const NATIVE_LIBRARIES: &str = "native-libs";

/// The engine's start with the native mod libraries of [`NATIVE_LIBRARIES`], loaded as the
/// desktop's fixed executable loads the ones beside it (horizon-mod-host's `open`, then
/// `load_code`). A build of this library with native mods compiled in
/// (packaging/android/native_build.sh) has none in that folder.
fn start(startup: horizon_client::entry::Startup) {
    let (entries, slots, native) = native_libraries();
    horizon_client::entry::start(
        native,
        move |native, launch| horizon_mod_host::load_code(entries, &slots, native, launch),
        horizon_mod_host::code_runtime(),
        startup,
    );
}

/// The native mod libraries of [`NATIVE_LIBRARIES`], checked and opened, with their packages. Each
/// was built for this exact game build (its `horizon_native_build`). A folder that does not check
/// out is left out with a line in logcat, and the game starts without those mods.
fn native_libraries() -> (
    Vec<mod_platform::ModEntry>,
    Vec<horizon_mod_host::SlotListener>,
    horizon_mod_package::runtime::Runtime,
) {
    let Some(private) = std::env::var_os("HORIZON_PRIVATE") else {
        return Default::default();
    };
    let folder = std::path::PathBuf::from(private).join(NATIVE_LIBRARIES);
    if !folder.join("native-mods.json").is_file() {
        return Default::default();
    }
    let opened = match horizon_mod_host::open(&folder) {
        Ok(opened) => opened,
        Err(error) => {
            horizon_client::android::warn(&format!("native mods left out: {error}"));
            return Default::default();
        }
    };
    match horizon_mod_host::packages_beside(&folder, opened.built) {
        Ok(native) => {
            horizon_client::android::set_native_libs_opened(
                opened.built.iter().map(|built| built.id).collect(),
            );
            (opened.entries, opened.slots, native)
        }
        Err(error) => {
            // Their patch copies are installed already; none of their hooks will be called.
            horizon_client::android::warn(&format!("native mods left out: {error}"));
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
        horizon_client::android::set_shared_folder(String::from(folder).into());
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
    horizon_client::ha_launcher::document_result(env, activity, request, result, uri);
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
