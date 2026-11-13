// Copyright (c) 2026 Ludovic Marie and the Horizon contributors
// SPDX-License-Identifier: MIT
//! Bounded local diagnostics, independent of opt-in frame profiling.
//! No signal handlers: Android owns native tombstones and low-memory exits.
#[cfg(target_os = "android")]
mod exit_reports;
#[path = "native_tombstone.rs"]
mod native_tombstone;
#[cfg(target_os = "android")]
pub fn collect_exits(home: &Path) {
    exit_reports::collect(home);
}
#[cfg(target_os = "android")]
pub(crate) fn recovery_exits(
    home: &Path,
    env: &mut jni::JNIEnv<'_>,
    activity: &jni::objects::JObject<'_>,
) -> Result<(), String> {
    exit_reports::collect_now(home, env, activity)
}
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const LOG_LIMIT: u64 = 512 * 1024;
const FILE_LIMIT: u64 = 256 * 1024;
const BUNDLE_LIMIT: usize = 4 * 1024 * 1024;
static STATE: Mutex<Option<State>> = Mutex::new(None);
struct State {
    sender: mpsc::SyncSender<Event>,
    session: Value,
    heartbeat: Instant,
}
enum Event {
    Session(Value),
    Flush(mpsc::SyncSender<()>),
}
pub fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn reason_name(reason: i32) -> &'static str {
    match reason {
        1 => "exit_self",
        2 => "signaled",
        3 => "low_memory",
        4 => "java_crash",
        5 => "native_crash",
        6 => "anr",
        7 => "initialization_failure",
        8 => "permission_change",
        9 => "excessive_resource_usage",
        10 => "user_requested",
        11 => "user_stopped",
        12 => "dependency_died",
        13 => "other",
        14 => "freezer",
        15 => "package_state_change",
        16 => "package_updated",
        _ => "unknown",
    }
}
pub fn tail(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    file.seek(SeekFrom::Start(size.saturating_sub(limit)))?;
    let mut out = Vec::new();
    file.take(limit).read_to_end(&mut out)?;
    Ok(out)
}
fn atomic_json(path: &Path, value: &Value) -> io::Result<()> {
    let temp = path.with_extension(format!("partial-{}", std::process::id()));
    fs::write(&temp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(temp, path)
}
pub fn initialize(home: &Path, device: Value) -> io::Result<()> {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.is_some() {
        return Ok(());
    }
    let dir = home.join("performance");
    fs::create_dir_all(&dir)?;
    if let Ok(data) = tail(&dir.join("current-session.json"), LOG_LIMIT) {
        fs::write(dir.join("previous-session.json"), data)?;
    }
    let session = json!({"schema":1,"pid":std::process::id(),"start_ms":timestamp(),
        "version":crate::crash::version(),"source_commit":option_env!("HORIZON_ANDROID_BUILD_COMMIT"),
        "source_dirty":option_env!("HORIZON_ANDROID_BUILD_DIRTY"),"abi":std::env::consts::ARCH,
        "phase":"native_start","clean_exit":false,"device":device});
    atomic_json(&dir.join("current-session.json"), &session)?;
    let (sender, receiver) = mpsc::sync_channel(128);
    std::thread::Builder::new()
        .name("Local diagnostics".into())
        .spawn(move || {
            for event in receiver {
                match event {
                    Event::Session(mut value) => {
                        value["memory"] = memory();
                        let _ = atomic_json(&dir.join("current-session.json"), &value);
                    }
                    Event::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
        })?;
    *state = Some(State {
        sender,
        session,
        heartbeat: Instant::now(),
    });
    Ok(())
}
fn memory() -> Value {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let mut values = serde_json::Map::new();
    for line in status.lines() {
        if let Some((key, value)) = line.split_once(':')
            && matches!(key, "VmRSS" | "VmHWM" | "VmSize" | "VmPeak" | "Threads")
        {
            values.insert(key.into(), value.trim().into());
        }
    }
    json!(values)
}
/// Frame-safe heartbeat: a busy diagnostic state skips this update rather than
/// making rendering wait. Memory collection and disk writes happen off-thread.
pub fn heartbeat(phase: &str, context: impl FnOnce() -> Value) {
    let mut guard = match STATE.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::WouldBlock) => return,
        Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
    };
    if let Some(state) = guard.as_mut() {
        if state.session["phase"] == phase && state.heartbeat.elapsed() < Duration::from_secs(5) {
            return;
        }
        state.heartbeat = Instant::now();
        state.session["phase"] = phase.into();
        state.session["updated_ms"] = timestamp().into();
        state.session["context"] = context();
        let _ = state.sender.try_send(Event::Session(state.session.clone()));
    }
}
/// Persist a critical startup boundary before entering a driver or resource load.
/// Used outside the per-frame path; flush has a bounded wait.
pub fn checkpoint(phase: &str, context: impl FnOnce() -> Value) {
    heartbeat(phase, context);
    flush();
}
pub fn record_error(error: &str) {
    if let Some(state) = STATE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        state.session["last_handled_error"] =
            redact(&error.chars().take(4096).collect::<String>()).into();
        state.session["error_ms"] = timestamp().into();
        let _ = state.sender.try_send(Event::Session(state.session.clone()));
    }
}
pub fn fatal_error(error: &str) {
    record_error(error);
    if let Some(state) = STATE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        state.session["fatal_error"] = true.into();
    }
}
pub fn finish() {
    if let Some(state) = STATE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        state.session["clean_exit"] = state.session["fatal_error"].is_null().into();
        state.session["updated_ms"] = timestamp().into();
        let _ = state.sender.try_send(Event::Session(state.session.clone()));
    }
    flush();
}
/// Logger flush must not make startup/rendering wait for the diagnostic disk worker.
pub fn request_flush() {
    let (tx, _rx) = mpsc::sync_channel(1);
    let guard = match STATE.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::WouldBlock) => return,
        Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
    };
    if let Some(state) = guard.as_ref() {
        let _ = state.sender.try_send(Event::Flush(tx));
    }
}
pub fn flush() {
    let (tx, rx) = mpsc::sync_channel(1);
    if let Some(state) = STATE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = state.sender.try_send(Event::Flush(tx));
    }
    let _ = rx.recv_timeout(Duration::from_millis(500));
}
pub fn previous_unclean(home: &Path) -> bool {
    fs::read(home.join("performance/previous-session.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .is_some_and(|v| v["clean_exit"] != true)
}
pub fn summary(home: &Path) -> String {
    let previous = fs::read(home.join("performance/previous-session.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    match previous {
        Some(v) if v["clean_exit"] != true => format!("No completion record for the previous session; last phase: {}. Android may stop background apps without a completion record. This alone does not indicate a crash.", v["phase"].as_str().unwrap_or("unknown")),
        _ => "Export one diagnostic ZIP to send with your report. It includes local logs, Android exit history and retained performance recordings. Review before sharing.".into(),
    }
}
/// Best-effort redaction, not a promise that arbitrary third-party text is public.
pub fn redact(text: &str) -> String {
    static PRIVATE: OnceLock<regex::Regex> = OnceLock::new();
    static ADDRESS: OnceLock<regex::Regex> = OnceLock::new();
    static DOMAIN: OnceLock<regex::Regex> = OnceLock::new();
    let private = PRIVATE.get_or_init(|| regex::Regex::new(r"(?i)access.?token|refresh.?token|authorization|bearer |password|client.?secret|xbl3\.0|chat message|connecting to|joining server|player name|username").unwrap());
    let address = ADDRESS.get_or_init(|| regex::Regex::new(r#"(?i)https?://[^\s\"<>]+|\b(?:[0-9]{1,3}\.){3}[0-9]{1,3}(?::[0-9]+)?|/data/(?:user|user_de)/[^\s\"<>]+|/storage/emulated/[^\s\"<>]+|[a-z]:\\Users\\[^\\\s]+"#).unwrap());
    let domain = DOMAIN.get_or_init(|| {
        regex::Regex::new(r"(?i)\b(?:[a-z0-9_-]+\.)+[a-z]{2,}\b|\[[a-f0-9:]+\](?::[0-9]+)?")
            .unwrap()
    });
    text.lines()
        .map(|line| {
            if private.is_match(line) {
                "[private log line removed]".into()
            } else {
                let line = address.replace_all(line, "[redacted]");
                domain
                    .replace_all(&line, |caps: &regex::Captures<'_>| {
                        let name = &caps[0];
                        // Bracketed log timestamps are not IPv6 addresses.
                        if let Some(rest) = name.strip_prefix('[')
                            && let Some((candidate, _)) = rest.split_once(']')
                            && candidate.parse::<std::net::Ipv6Addr>().is_err()
                        {
                            return name.to_owned();
                        }
                        let extension = name.rsplit('.').next().unwrap_or_default();
                        if matches!(
                            extension,
                            "rs" | "so"
                                | "dll"
                                | "wgsl"
                                | "json"
                                | "jsonl"
                                | "txt"
                                | "java"
                                | "kt"
                                | "toml"
                                | "trace"
                                | "log"
                                | "zip"
                                | "apk"
                        ) {
                            name.to_owned()
                        } else {
                            "[address removed]".into()
                        }
                    })
                    .into_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
/// Explicit user action only. No accounts/configuration/worlds, no uploads.
pub fn export(home: &Path) -> Result<PathBuf, String> {
    export_from(home, &crate::logging::player_dir())
}
pub(crate) fn export_from(home: &Path, player: &Path) -> Result<PathBuf, String> {
    flush();
    let dir = home.join("performance");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("starling-diagnostics-{}.zip", timestamp()));
    let stage = path.with_extension("partial");
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut files = Vec::new();
        for folder in [&dir, &player.join("crash-reports"), &player.join("logs")] {
            if let Ok(entries) = fs::read_dir(folder) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if entry.file_type()?.is_file()
                        && !entry.file_type()?.is_symlink()
                        && (name.ends_with(".log")
                            || name.ends_with(".trace")
                            || (name.starts_with("horizon-performance-")
                                && name.ends_with(".jsonl"))
                            || name.starts_with("crash-")
                            || matches!(
                                name.as_str(),
                                "android-exits.json"
                                    | "perf-sessions.json"
                                    | "current-session.json"
                                    | "previous-session.json"
                                    | "last-rust-panic.txt"
                                    | "previous-error.txt"
                            ))
                    {
                        files.push(entry.path());
                    }
                }
            }
        }
        files.sort_by_key(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            let priority = match name.as_ref() {
                "current-session.json"
                | "previous-session.json"
                | "android-exits.json"
                | "perf-sessions.json" => 0,
                "latest.log" | "previous.log" | "previous-run.log" | "previous-error.txt" => 1,
                n if n.starts_with("horizon-performance-") => 2,
                _ => 3,
            };
            (
                priority,
                std::cmp::Reverse(p.file_name().map(|s| s.to_owned())),
            )
        });
        let mut archive = zip::ZipWriter::new(File::create(&stage)?);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let mut included = Vec::new();
        let mut omitted = Vec::new();
        let mut details = Vec::new();
        let mut total = 0;
        files.dedup_by(|a, b| a.file_name() == b.file_name());
        if files.len() > 32 {
            omitted.push(json!({"count":files.len()-32,"reason":"file count limit"}));
        }
        for file in files.into_iter().take(32) {
            let mut name = file.file_name().unwrap().to_string_lossy().into_owned();
            let metadata = file.metadata()?;
            let source_size = metadata.len();
            let modified_ms = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis());
            let mut data = if name.ends_with(".trace") {
                let mut data = Vec::new();
                File::open(&file)?
                    .take(1024 * 1024)
                    .read_to_end(&mut data)?;
                data
            } else if name.ends_with(".jsonl") && source_size > FILE_LIMIT {
                // Keep a complete header and complete trailing rows, so the
                // bounded export remains valid JSONL after a long recording.
                let mut header = Vec::new();
                File::open(&file)?
                    .take(16 * 1024)
                    .read_to_end(&mut header)?;
                header.truncate(header.iter().position(|b| *b == b'\n').map_or(0, |i| i + 1));
                let data = tail(&file, FILE_LIMIT - header.len() as u64)?;
                if let Some(at) = data.iter().position(|b| *b == b'\n') {
                    header.extend_from_slice(&data[at + 1..]);
                }
                header
            } else {
                tail(&file, FILE_LIMIT)?
            };
            // A live writer may be partway through its last row. Never export
            // that incomplete JSON record as if it were a completed sample.
            if name.ends_with(".jsonl") && data.last() != Some(&b'\n') {
                data.truncate(data.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1));
            }
            let binary_trace = name.ends_with(".trace")
                && (std::str::from_utf8(&data).is_err()
                    || data.iter().any(|b| *b < 32 && !matches!(b, 9 | 10 | 13)));
            let text = if binary_trace {
                omitted.push(json!({"file":name,"reason":"binary system trace: original retained privately on device"}));
                let Some(summary) = native_tombstone::summarize(&data) else {
                    continue;
                };
                name.push_str("-summary.txt");
                redact(&summary)
            } else {
                redact(&String::from_utf8_lossy(&data))
            };
            if total + text.len() > BUNDLE_LIMIT {
                omitted.push(json!({"file":name,"reason":"bundle size limit"}));
                continue;
            }
            if included.contains(&name) {
                continue;
            }
            archive.start_file(&name, options)?;
            archive.write_all(text.as_bytes())?;
            total += text.len();
            details.push(json!({"file":name,"source_bytes":source_size,"input_truncated_by_export":source_size>if name.ends_with("summary.txt") { 1024*1024 } else { FILE_LIMIT },"source_modified_ms":modified_ms}));
            included.push(name);
        }
        archive.start_file("manifest.json", options)?;
        archive.write_all(&serde_json::to_vec_pretty(&json!({"schema":1,"version":crate::crash::version(),"source_commit":option_env!("HORIZON_ANDROID_BUILD_COMMIT"),"included":included,"files":details,"omitted":omitted,"note":"Logs are bounded tails with best-effort redaction. Review before sharing. Binary Android tombstones contribute a bounded crashed-thread summary; original memory/register dumps are excluded and remain private on device. No account files, worlds or configuration are included."}))?)?;
        archive.finish()?;
        fs::rename(&stage, &path)?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(stage);
        return Err(error.to_string());
    }
    let mut old: Vec<_> = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("starling-diagnostics-"))
                && p.extension().is_some_and(|s| s == "zip")
        })
        .collect();
    old.sort();
    let excess = old.len().saturating_sub(3);
    for file in old.into_iter().take(excess) {
        let _ = fs::remove_file(file);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn home() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "starling-diagnostics-unit-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(path.join("performance")).unwrap();
        path
    }
    #[test]
    fn redaction_keeps_native_frames_but_removes_credentials_and_addresses() {
        let text = redact(
            "access_token=sealed-token-value\nconnection 192.0.2.7:25565 leo.spcfy.eu https://example.org/private\n/data/user/0/dev.horizon.android/files/saves/PrivateWorld\nC:\\Users\\Alice\\source\\game.rs\n#00 pc 00004abc libvulkan.so (BuildId: abcdef123456)",
        );
        for secret in [
            "sealed-token-value",
            "192.0.2.7",
            "leo.spcfy.eu",
            "example.org",
            "PrivateWorld",
            "Alice",
        ] {
            assert!(!text.contains(secret), "{secret}: {text}");
        }
        assert!(text.contains("libvulkan.so"));
        assert!(text.contains("abcdef123456"));
    }
    #[test]
    fn redaction_preserves_log_timestamps_while_hiding_ipv6() {
        let result = redact(
            "[22:30:07] [main/ERROR]: failed at game.rs:82\npeer [2001:db8::1]:25565\npeer [::1]",
        );
        assert!(result.contains("[22:30:07] [main/ERROR]: failed at game.rs:82"));
        assert!(!result.contains("2001:db8"));
        assert!(!result.contains("[::1]"));
    }
    #[test]
    fn export_allowlist_bounds_and_binary_trace_manifest() {
        let home = home();
        let dir = home.join("performance");
        fs::write(home.join("account.json"), b"secret-account").unwrap();
        fs::write(dir.join("account.json"), b"secret-account").unwrap();
        fs::write(
            dir.join("diagnostic.log"),
            "Vulkan initialized\nAuthorization: secret\n",
        )
        .unwrap();
        fs::write(dir.join("exit-1.trace"), [0xff, 0, 0xff]).unwrap();
        fs::write(
            dir.join("previous-session.json"),
            r#"{"phase":"game_loading","clean_exit":false}"#,
        )
        .unwrap();
        assert!(previous_unclean(&home));
        assert!(summary(&home).contains("game_loading"));
        let file = export_from(&home, &home).unwrap();
        let mut zip = zip::ZipArchive::new(File::open(file).unwrap()).unwrap();
        assert!(zip.by_name("account.json").is_err());
        assert!(zip.by_name("exit-1.trace").is_err());
        let mut manifest = String::new();
        zip.by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut manifest)
            .unwrap();
        assert!(manifest.contains("binary system trace"));
        let mut log = String::new();
        zip.by_name("diagnostic.log")
            .unwrap()
            .read_to_string(&mut log)
            .unwrap();
        assert!(log.contains("Vulkan initialized"));
        assert!(!log.contains("secret"));
        drop(zip);
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn long_recordings_export_complete_rows_and_keep_recent_bounded_archives() {
        let home = home();
        let dir = home.join("performance");
        let mut report = File::create(dir.join("horizon-performance-123.jsonl")).unwrap();
        writeln!(report, "{{\"type\":\"header\",\"schema\":1}}").unwrap();
        for i in 0..10000 {
            writeln!(
                report,
                "{{\"type\":\"sample\",\"i\":{i},\"payload\":\"{}\"}}",
                "x".repeat(80)
            )
            .unwrap();
        }
        drop(report);
        for i in 0..35 {
            fs::write(
                dir.join(format!("crash-{i:03}.txt")),
                "x".repeat(FILE_LIMIT as usize + 10),
            )
            .unwrap();
        }
        fs::write(dir.join("android-exits.json"), "{}").unwrap();
        for _ in 0..4 {
            let path = export_from(&home, &home).unwrap();
            let mut archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
            assert!(archive.len() <= 33);
            let total: u64 = (0..archive.len())
                .map(|i| archive.by_index(i).unwrap().size())
                .sum();
            assert!(total <= BUNDLE_LIMIT as u64 + 64 * 1024);
            assert!(archive.by_name("android-exits.json").is_ok());
            let mut text = String::new();
            archive
                .by_name("horizon-performance-123.jsonl")
                .unwrap()
                .read_to_string(&mut text)
                .unwrap();
            assert!(text.len() <= FILE_LIMIT as usize);
            let rows: Vec<Value> = text
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(rows[0]["type"], "header");
            assert_eq!(rows.last().unwrap()["i"], 9999);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .filter(|f| f.path().extension().is_some_and(|e| e == "zip"))
                .count(),
            3
        );
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn writer_survives_errors_and_heartbeat_is_lazy() {
        let home = home();
        fs::write(
            home.join("performance/current-session.json"),
            r#"{"phase":"prior_world","clean_exit":false}"#,
        )
        .unwrap();
        initialize(&home, json!({"model":"probe"})).unwrap();
        heartbeat("in_world", || json!({"backend":"Vulkan"}));
        heartbeat("in_world", || {
            panic!("context must not be built each frame")
        });
        {
            // A frame must never wait for a diagnostic lock held elsewhere.
            let _guard = STATE.lock().unwrap();
            heartbeat("blocked", || panic!("busy heartbeat must skip context"));
            request_flush();
        }
        record_error("VK_ERROR_DEVICE_LOST");
        finish();
        let current: Value = serde_json::from_slice(
            &fs::read(home.join("performance/current-session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(current["clean_exit"], true);
        assert_eq!(current["last_handled_error"], "VK_ERROR_DEVICE_LOST");
        assert_eq!(current["context"]["backend"], "Vulkan");
        assert!(summary(&home).contains("prior_world"));
        fatal_error("renderer initialization failed");
        finish();
        let failed: Value = serde_json::from_slice(
            &fs::read(home.join("performance/current-session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(failed["clean_exit"], false);
        STATE.lock().unwrap().take();
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn exit_reasons_distinguish_native_java_and_low_memory() {
        assert_eq!(reason_name(3), "low_memory");
        assert_eq!(reason_name(4), "java_crash");
        assert_eq!(reason_name(5), "native_crash");
        assert_eq!(reason_name(6), "anr");
        assert_eq!(reason_name(999), "unknown");
    }
}
