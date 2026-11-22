//! The `console_line` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Observe an operator-visible log line on a dedicated or integrated server. `level` is
    /// `INFO`, `WARN` or `ERROR`, `thread` names the logging thread, and `message` is the text
    /// without its timestamp, thread or level prefix. Lines from every logging thread are
    /// delivered in order on the server thread at the start of the next server tick. The
    /// tap starts on the first tick with an enabled listener; without one, no log is captured and no
    /// queue is allocated or locked in the idle path. A listener's own log calls are not tapped.
    /// The queue keeps the oldest 4096 lines and discards later ones until it drains. `dropped`
    /// reports the number discarded on the first line of that batch and is zero on all other
    /// lines. `message` keeps its first 8192 bytes and `thread` its first 128, at a character
    /// boundary. A listener cannot change or suppress the log.
    #[notes = r#"## Console line

- **`console_line(host, event)`** observes operator log lines on dedicated and integrated servers. The server thread delivers them in order at the next tick's start. The queue keeps the oldest 4096 lines and drops newer ones until the next drain; the first event of that batch reports the count in `dropped`, with zero on every other event. It keeps 8192 bytes of message and 128 of thread name per line at character boundaries. Nothing is captured or queued while idle, and a listener's own log calls are not captured. Neither Paper nor NeoForge has a matching event; a Java mod or plugin can attach a Log4j appender."#]
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        event_per_listener = true,
        version = 2,
        neoforge = "none (NeoForge has no log event; mods add a Log4j appender)",
        paper = "none (plugins add a Log4j appender)"
    )]
    fn console_line(level: &str, thread: &str, message: &str, dropped: u64);
}
