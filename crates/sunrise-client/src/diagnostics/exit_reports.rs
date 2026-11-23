// Copyright (c) 2026 Ludovic Marie and the Horizon contributors
// SPDX-License-Identifier: MIT
//! Read Android's previous-process exits off the game/UI thread.
use jni::{
    JNIEnv,
    objects::{JObject, JString, JValue},
};
use serde_json::{Value, json};
use std::path::Path;

fn clear<T>(env: &mut JNIEnv<'_>, value: jni::errors::Result<T>) -> Option<T> {
    match value {
        Ok(v) => Some(v),
        Err(_) => {
            let _ = env.exception_clear();
            None
        }
    }
}
fn description(env: &mut JNIEnv<'_>, item: &JObject<'_>) -> jni::errors::Result<String> {
    let text = env
        .call_method(item, "getDescription", "()Ljava/lang/String;", &[])?
        .l()?;
    if text.is_null() {
        return Ok(String::new());
    }
    Ok(env
        .get_string(&JString::from(text))?
        .to_string_lossy()
        .chars()
        .take(4096)
        .collect())
}
fn trace(
    env: &mut JNIEnv<'_>,
    item: &JObject<'_>,
    dir: &Path,
    stamp: i64,
) -> Result<Option<String>, String> {
    let stream = env
        .call_method(item, "getTraceInputStream", "()Ljava/io/InputStream;", &[])
        .and_then(|v| v.l())
        .map_err(|e| e.to_string())?;
    if stream.is_null() {
        return Ok(None);
    }
    let result = (|| -> Result<Option<String>, Box<dyn std::error::Error>> {
        let buf = env.new_byte_array(8192)?;
        let mut data = Vec::new();
        while data.len() < 1024 * 1024 {
            let n = env
                .call_method(&stream, "read", "([B)I", &[JValue::Object(&buf)])?
                .i()?;
            if n <= 0 {
                break;
            }
            if n > 8192 {
                return Err("Invalid Android trace read length".into());
            }
            let bytes = env.convert_byte_array(&buf)?;
            let count = (n as usize).min(1024 * 1024 - data.len());
            data.extend_from_slice(&bytes[..count]);
        }
        if data.is_empty() {
            return Ok(None);
        }
        let name = format!("exit-{stamp}.trace");
        std::fs::write(dir.join(&name), data)?;
        Ok(Some(name))
    })();
    // A failed read must not leave a pending exception hiding the close or the next row.
    let _ = env.exception_clear();
    let _ = env.call_method(&stream, "close", "()V", &[]);
    let _ = env.exception_clear();
    result.map_err(|e| e.to_string())
}
fn query(env: &mut JNIEnv<'_>, activity: &JObject<'_>, dir: &Path) -> jni::errors::Result<Value> {
    let sdk = env
        .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
        .i()?;
    let mut device = json!({"sdk":sdk,"abi":std::env::consts::ARCH});
    for field in ["MANUFACTURER", "MODEL", "HARDWARE"] {
        let result = env
            .get_static_field("android/os/Build", field, "Ljava/lang/String;")
            .and_then(|v| v.l());
        if let Some(obj) = clear(env, result)
            && !obj.is_null()
        {
            let value = env
                .get_string(&JString::from(obj))?
                .to_string_lossy()
                .into_owned();
            device[field.to_lowercase()] = value.into();
        }
    }
    let mut report = json!({"schema":2,"version":crate::crash::version(),
        "source_commit":option_env!("SUNRISE_ANDROID_BUILD_COMMIT"),"collected_at_ms":crate::diagnostics::timestamp(),
        "device":device,"memory_note":"PSS/RSS are last OS samples in KiB; zero may mean no sample, not zero memory use.","collection_status":"unsupported_before_android_11","exits":[],
        "note":"Exit reasons are OS observations, not proof of the root cause. No trace can mean the OS did not retain one. Trace files are bounded raw system data; review before sharing."});
    if sdk < 30 {
        return Ok(report);
    }
    let name = JObject::from(env.new_string("activity")?);
    let manager = env
        .call_method(
            activity,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&name)],
        )?
        .l()?;
    let null = JObject::null();
    let exits = env
        .call_method(
            manager,
            "getHistoricalProcessExitReasons",
            "(Ljava/lang/String;II)Ljava/util/List;",
            &[JValue::Object(&null), JValue::Int(0), JValue::Int(8)],
        )?
        .l()?;
    let count = env
        .call_method(&exits, "size", "()I", &[])?
        .i()?
        .clamp(0, 8);
    let mut rows = Vec::new();
    for i in 0..count {
        let row = env.with_local_frame(64, |env| -> jni::errors::Result<Value> {
            let item = env.call_method(&exits,"get","(I)Ljava/lang/Object;",&[JValue::Int(i)])?.l()?;
            let stamp = env.call_method(&item,"getTimestamp","()J",&[])?.j()?;
            let reason = env.call_method(&item,"getReason","()I",&[])?.i()?;
            let status = env.call_method(&item,"getStatus","()I",&[])?.i()?;
            let mut row = json!({"timestamp_ms":stamp,"reason":reason,"reason_name":crate::diagnostics::reason_name(reason),"status":status});
            let desc = description(env, &item);
            row["description"] = clear(env, desc).unwrap_or_default().into();
            for (method, key) in [("getPss","pss_kib"),("getRss","rss_kib")] {
                let value = env.call_method(&item,method,"()J",&[]).and_then(|v| v.j());
                row[key] = clear(env,value).map_or(Value::Null, Value::from);
            }
            for (method, key) in [("getPid","pid"),("getImportance","importance")] {
                let value = env.call_method(&item,method,"()I",&[]).and_then(|v| v.i());
                row[key] = clear(env,value).map_or(Value::Null, Value::from);
            }
            if matches!(reason,4..=6) {
                match trace(env, &item, dir, stamp) {
                    Ok(file) => { row["trace_available"] = file.is_some().into(); row["trace_file"] = file.into(); }
                    Err(error) => { let _ = env.exception_clear(); row["trace_error"] = error.into(); }
                }
            }
            Ok(row)
        });
        match row {
            Ok(row) => rows.push(row),
            Err(error) => {
                let _ = env.exception_clear();
                rows.push(json!({"entry":i,"collection_error":error.to_string()}));
            }
        }
    }
    report["collection_status"] = "ok".into();
    report["exits"] = rows.into();
    Ok(report)
}
/// Explicit recovery export on a worker attached to the recovery activity's VM.
/// No NativeActivity, renderer, game startup or session rotation is involved.
pub(super) fn collect_now(
    home: &Path,
    env: &mut JNIEnv<'_>,
    activity: &JObject<'_>,
) -> Result<(), String> {
    let dir = home.join("performance");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let report = query(env, activity, &dir).map_err(|e| {
        let _ = env.exception_clear();
        e.to_string()
    })?;
    super::atomic_json(&dir.join("android-exits.json"), &report).map_err(|e| e.to_string())?;
    prune(&dir);
    Ok(())
}
fn prune(dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut files: Vec<_> = entries
            .flatten()
            .filter(|e| {
                e.file_name().to_string_lossy().starts_with("exit-")
                    && e.path().extension().is_some_and(|s| s == "trace")
            })
            .collect();
        files.sort_by_key(|e| e.file_name());
        let excess = files.len().saturating_sub(8);
        for file in files.into_iter().take(excess) {
            let _ = std::fs::remove_file(file.path());
        }
    }
}
pub fn collect(home: &Path) {
    let dir = home.join("performance");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    // Preserve raw details locally; the user-requested ZIP redacts text exports.
    for (source, name) in [
        (
            crate::logging::player_dir().join("logs/latest.log"),
            "previous-run.log",
        ),
        (home.join("android-error.txt"), "previous-error.txt"),
    ] {
        if let Ok(data) = crate::diagnostics::tail(&source, 256 * 1024) {
            let _ = std::fs::write(dir.join(name), data);
        }
    }
    let context = crate::android::with_activity(|env, activity| {
        Ok((env.get_java_vm()?, env.new_global_ref(activity)?))
    });
    let Some((vm, activity)) = context else {
        let _ = std::fs::write(
            dir.join("android-exits.json"),
            b"{\"schema\":2,\"collection_status\":\"jni_context_unavailable\"}",
        );
        return;
    };
    let _ = std::fs::write(dir.join("android-exits.json"), json!({"schema":2,"collection_status":"pending","collected_at_ms":crate::diagnostics::timestamp()}).to_string());
    let output = dir.clone();
    let spawn = std::thread::Builder::new().name("Android exit history".into()).spawn(move || {
        let result = vm.attach_current_thread().and_then(|mut env| {
            let result = env.with_local_frame(32, |env| query(env, activity.as_obj(), &dir));
            // Release the Java reference before the attach guard detaches this worker.
            drop(activity);
            result
        });
        let report = result.unwrap_or_else(|e|json!({"schema":2,"collection_status":"failed","error":e.to_string(),"exits":[]}));
        let _ = super::atomic_json(&dir.join("android-exits.json"), &report);
        prune(&dir);
    });
    if let Err(error) = spawn {
        let _ = std::fs::write(
            output.join("android-exits.json"),
            json!({"schema":2,"collection_status":"worker_failed","error":error.to_string()})
                .to_string(),
        );
    }
}
