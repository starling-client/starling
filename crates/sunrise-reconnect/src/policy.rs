//! Whether a lost multiplayer connection may be retried, and when. Only lost
//! connections and the server's own shutdown or restart are retried, never
//! kicks, bans or refusals; attempts back off and stop after a few.
use std::time::{Duration, Instant};

/// Seconds before each attempt; there are no more after the last.
pub(crate) const DELAYS: [u64; 5] = [5, 10, 20, 40, 60];
/// Staying connected this long counts the next drop as a fresh one.
pub(crate) const STABLE: Duration = Duration::from_secs(60);

/// One classification rule: a translation key's prefix, an English keyword
/// in a plain reason, or both, and whether that reason may be retried.
struct Rule {
    key: Option<&'static str>,
    keyword: Option<&'static str>,
    retry: bool,
}

/// The one table behind both decisions. The never rules stay before the
/// retry ones: a reason that matches both is never retried.
const RULES: &[Rule] = &[
    // Never: kicks, bans, whitelists, version refusals, the account's session
    // refusals (`disconnect.loginFailedInfo.*` but the unreachable session
    // servers, the server's failed username check, joining an online-mode
    // server without an account) and Sunrise's own refusals.
    Rule {
        key: Some("multiplayer.disconnect.banned"),
        keyword: Some("ban"),
        retry: false,
    },
    Rule {
        key: Some("multiplayer.disconnect.ip_banned"),
        keyword: None,
        retry: false,
    },
    Rule {
        key: Some("multiplayer.disconnect.kicked"),
        keyword: Some("kick"),
        retry: false,
    },
    Rule {
        key: Some("multiplayer.disconnect.not_whitelisted"),
        keyword: Some("white-list"),
        retry: false,
    },
    Rule {
        key: None,
        keyword: Some("whitelist"),
        retry: false,
    },
    Rule {
        key: Some("multiplayer.disconnect.incompatible"),
        keyword: Some("incompatible"),
        retry: false,
    },
    Rule {
        key: Some("multiplayer.disconnect.outdated_client"),
        keyword: Some("outdated"),
        retry: false,
    },
    Rule {
        key: Some("multiplayer.disconnect.outdated_server"),
        keyword: None,
        retry: false,
    },
    Rule {
        key: Some("multiplayer.disconnect.duplicate_login"),
        keyword: Some("another location"),
        retry: false,
    },
    Rule {
        key: Some("sunrise.multiplayer.not_signed_in"),
        keyword: Some("online mode"),
        retry: false,
    },
    Rule {
        key: None,
        keyword: Some("microsoft account"),
        retry: false,
    },
    Rule {
        key: Some("sunrise.multiplayer.invalid_address"),
        keyword: Some("invalid server address"),
        retry: false,
    },
    Rule {
        key: Some("disconnect.loginFailedInfo.invalidSession"),
        keyword: Some("invalid session"),
        retry: false,
    },
    Rule {
        key: Some("disconnect.loginFailedInfo.insufficientPrivileges"),
        keyword: Some("multiplayer is disabled"),
        retry: false,
    },
    Rule {
        key: Some("disconnect.loginFailedInfo.userBanned"),
        keyword: None,
        retry: false,
    },
    Rule {
        key: Some("multiplayer.disconnect.unverified_username"),
        keyword: Some("failed to verify username"),
        retry: false,
    },
    // Retry: the server going away for a while — vanilla's shutdown and
    // time-out reasons, a full server, a slow login, the server's own session
    // servers being down, and restart messages.
    Rule {
        key: Some("multiplayer.disconnect.server_shutdown"),
        keyword: Some("server closed"),
        retry: true,
    },
    Rule {
        key: None,
        keyword: Some("restart"),
        retry: true,
    },
    Rule {
        key: Some("multiplayer.disconnect.server_full"),
        keyword: Some("the server is full"),
        retry: true,
    },
    Rule {
        key: Some("multiplayer.disconnect.slow_login"),
        keyword: Some("took too long to log in"),
        retry: true,
    },
    Rule {
        key: Some("multiplayer.disconnect.authservers_down"),
        keyword: Some("authentication servers"),
        retry: true,
    },
    Rule {
        key: Some("disconnect.loginFailedInfo.serversUnavailable"),
        keyword: None,
        retry: true,
    },
    Rule {
        key: Some("disconnect.timeout"),
        keyword: Some("timed out"),
        retry: true,
    },
];

/// Whether a disconnect may be retried. `from_server`: the server (or a
/// permanent refusal) gave `reason`, rather than the connection failing.
///
/// `key`: the reason's translation key, when it was a translatable
/// component or a known Sunrise error. A key the classification knows
/// decides; the English keyword scan of the text is only the fallback for
/// plain reasons (OS errors, servers' literal kicks), whose language is
/// the server's own.
pub(crate) fn allowed(reason: &str, key: Option<&str>, from_server: bool) -> bool {
    if let Some(decision) = key.and_then(classified_key) {
        return decision;
    }
    let r = reason.to_lowercase();
    for rule in RULES {
        let Some(keyword) = rule.keyword else {
            continue;
        };
        if !r.contains(keyword) {
            continue;
        }
        if !rule.retry {
            return false;
        }
        if from_server {
            return true;
        }
    }
    !from_server
}

/// The table's key-based side: `Some` when the key alone decides. Prefix
/// match, so a key's arguments' variants
/// (`multiplayer.disconnect.banned.expiration`) share the base's verdict.
fn classified_key(key: &str) -> Option<bool> {
    RULES.iter().find_map(|rule| {
        let prefix = rule.key?;
        key.starts_with(prefix).then_some(rule.retry)
    })
}

/// Attempts at one address.
#[derive(Debug, Default)]
pub(crate) struct Tracker {
    address: Option<String>,
    /// The player name the connection used.
    name: String,
    lan: bool,
    /// Attempts already made since the last stable connection.
    attempts: usize,
    joined: Option<Instant>,
}
/// A planned attempt.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Retry {
    pub(crate) address: String,
    pub(crate) name: String,
    pub(crate) lan: bool,
    at: Instant,
    /// 1-based, of `DELAYS.len()`.
    attempt: usize,
}
impl Retry {
    /// Whole seconds left, rounded up.
    pub(crate) fn seconds_left(&self, now: Instant) -> u64 {
        let left = self.at.saturating_duration_since(now);
        left.as_secs() + u64::from(left.subsec_nanos() > 0)
    }
    pub(crate) fn due(&self, now: Instant) -> bool {
        now >= self.at
    }
    pub(crate) fn text(&self, now: Instant) -> String {
        format!(
            "Reconnecting in {} s (attempt {} of {})",
            self.seconds_left(now),
            self.attempt,
            DELAYS.len()
        )
    }
}
impl Tracker {
    /// A connection to `address` as `name` starts: a new address starts
    /// counting anew.
    pub(crate) fn connecting(&mut self, address: &str, name: &str, lan: bool) {
        if self.address.as_deref() != Some(address) {
            self.address = Some(address.to_owned());
            self.attempts = 0;
        }
        self.name = name.to_owned();
        self.lan = lan;
        self.joined = None;
    }
    /// The server let the player in.
    pub(crate) fn joined(&mut self, now: Instant) {
        self.joined = Some(now);
    }
    /// The player left on purpose, or cancelled: nothing to retry.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
    /// The connection ended: the next attempt, or None (not allowed, or
    /// out of attempts).
    pub(crate) fn ended(
        &mut self,
        reason: &str,
        key: Option<&str>,
        from_server: bool,
        now: Instant,
    ) -> Option<Retry> {
        if self
            .joined
            .take()
            .is_some_and(|t| now.saturating_duration_since(t) >= STABLE)
        {
            self.attempts = 0;
        }
        let address = self.address.clone()?;
        if !allowed(reason, key, from_server) || self.attempts >= DELAYS.len() {
            self.reset();
            return None;
        }
        let delay = Duration::from_secs(DELAYS[self.attempts]);
        self.attempts += 1;
        Some(Retry {
            address,
            name: self.name.clone(),
            lan: self.lan,
            at: now + delay,
            attempt: self.attempts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_keeps_the_selected_server_kind() {
        let mut tracker = Tracker::default();
        let now = Instant::now();
        tracker.connecting("127.0.0.1:25565", "Tester", true);
        assert!(tracker.ended("Server closed", None, true, now).unwrap().lan);
        tracker.connecting("127.0.0.1:25565", "Tester", false);
        assert!(!tracker.ended("Server closed", None, true, now).unwrap().lan);
    }

    #[test]
    fn only_lost_connections_and_shutdowns_are_retried() {
        assert!(allowed(
            "Connection reset by peer (os error 104)",
            None,
            false
        ));
        assert!(allowed("Connection refused (os error 111)", None, false));
        assert!(allowed("Server closed", None, true));
        assert!(allowed("Timed out", None, true));
        assert!(allowed("The server is full!", None, true));
        assert!(allowed("Server is restarting, back soon", None, true));
        assert!(!allowed("Kicked by an operator", None, true));
        assert!(!allowed(
            "You are banned from this server.\nReason: griefing",
            None,
            true
        ));
        assert!(!allowed(
            "You are not white-listed on this server!",
            None,
            true
        ));
        assert!(!allowed("You logged in from another location", None, true));
        assert!(!allowed("Incompatible client! Please use 26.3", None, true));
        assert!(!allowed("You have been idle for too long!", None, true));
        assert!(
            !allowed("Go away", None, true),
            "unknown server reasons are not retried"
        );
        assert!(!allowed(
            "This server is in online mode. Sunrise only joins offline-mode servers.",
            Some("sunrise.multiplayer.not_signed_in"),
            true
        ));
        assert!(!allowed("Kicked for spamming", None, false));
    }

    #[test]
    fn known_keys_decide_before_text_and_refusals_beat_retry_keywords() {
        let reason = "Banned during a server restart";
        assert!(allowed(
            reason,
            Some("multiplayer.disconnect.server_shutdown"),
            true,
        ));
        assert!(!allowed(reason, None, true));
        assert!(allowed("Server closed", Some("unknown.disconnect"), true));
    }

    #[test]
    fn online_mode_login_failures() {
        // The client's join with the session service (net.rs: Err, with
        // vanilla's disconnect.loginFailedInfo texts and the inner key).
        let failed = |why: &str| format!("Failed to log in: {why}");
        assert!(!allowed(
            &failed("Invalid session (Try restarting your game and the launcher)"),
            Some("disconnect.loginFailedInfo.invalidSession"),
            false
        ));
        assert!(!allowed(
            &failed("You are banned from playing online"),
            Some("disconnect.loginFailedInfo.userBanned"),
            false
        ));
        assert!(!allowed(
            &failed("Multiplayer is disabled. Please check your Microsoft account settings."),
            Some("disconnect.loginFailedInfo.insufficientPrivileges"),
            false
        ));
        assert!(!allowed(
            "This server is in online mode. Sign in with your Microsoft account in the Sunrise launcher to join it.",
            Some("sunrise.multiplayer.not_signed_in"),
            false
        ));
        // Unreachable session servers are worth another try.
        assert!(allowed(
            &failed("The authentication servers are currently not reachable. Please try again."),
            Some("disconnect.loginFailedInfo.serversUnavailable"),
            false
        ));
        assert!(allowed(&failed("Connection timed out"), None, false));
        // The server's side: its username check failed, or its session
        // servers are down.
        assert!(!allowed("Failed to verify username!", None, true));
        assert!(allowed(
            "Authentication servers are down. Please try again later. Sorry!",
            Some("multiplayer.disconnect.authservers_down"),
            true
        ));
    }

    #[test]
    fn keys_decide_whatever_language_resolved_the_text() {
        // A zh_cn client resolves the server's and the session service's
        // texts: the keyword scan of the text would see none of the
        // English words, the keys do. The texts are zh_cn's.
        fn zh(key: &str) -> &str {
            match key {
                "multiplayer.disconnect.server_shutdown" => "服务器已关闭",
                "multiplayer.disconnect.server_full" => "服务器已满！",
                "multiplayer.disconnect.slow_login" => "登录超时",
                "multiplayer.disconnect.kicked" => "被管理员踢出游戏",
                "multiplayer.disconnect.banned" => "你已被此服务器封禁",
                "multiplayer.disconnect.not_whitelisted" => "你不在此服务器的白名单中！",
                "disconnect.loginFailedInfo.invalidSession" => "无效会话（请尝试重启游戏及启动器）",
                "sunrise.multiplayer.not_signed_in" => {
                    "该服务器为在线模式服务器。请先在 Sunrise 启动器中登录 Microsoft 账户再加入。"
                }
                _ => key,
            }
        }
        // Shutdowns, a full server and a slow login are retried, kicks and
        // bans are not — whatever the resolved text says.
        assert!(allowed(
            zh("multiplayer.disconnect.server_shutdown"),
            Some("multiplayer.disconnect.server_shutdown"),
            true
        ));
        assert!(allowed(
            zh("multiplayer.disconnect.server_full"),
            Some("multiplayer.disconnect.server_full"),
            true
        ));
        assert!(allowed(
            zh("multiplayer.disconnect.slow_login"),
            Some("multiplayer.disconnect.slow_login"),
            true
        ));
        assert!(!allowed(
            zh("multiplayer.disconnect.kicked"),
            Some("multiplayer.disconnect.kicked"),
            true
        ));
        assert!(!allowed(
            zh("multiplayer.disconnect.banned"),
            Some("multiplayer.disconnect.banned"),
            true
        ));
        assert!(!allowed(
            zh("multiplayer.disconnect.not_whitelisted"),
            Some("multiplayer.disconnect.not_whitelisted"),
            true
        ));
        assert!(!allowed(
            zh("disconnect.loginFailedInfo.invalidSession"),
            Some("disconnect.loginFailedInfo.invalidSession"),
            false
        ));
        assert!(!allowed(
            zh("sunrise.multiplayer.not_signed_in"),
            Some("sunrise.multiplayer.not_signed_in"),
            true
        ));
        // Unknown and missing keys fall back to the text scan.
        assert!(allowed(
            "Connection reset by peer (os error 104)",
            Some("made.up.key"),
            false
        ));
        assert!(allowed("Connection refused (os error 111)", None, false));
        assert!(!allowed("Go away", Some("made.up.key"), true));
    }

    #[test]
    fn attempts_back_off_and_stop() {
        let mut t = Tracker::default();
        let now = Instant::now();
        t.connecting("localhost:25565", "Tester", false);
        let mut delays = Vec::new();
        while let Some(r) = t.ended("Connection refused", None, false, now) {
            assert_eq!(
                (r.address.as_str(), r.name.as_str()),
                ("localhost:25565", "Tester")
            );
            assert_eq!(r.attempt, delays.len() + 1);
            delays.push(r.at.duration_since(now).as_secs());
            t.connecting(&r.address, &r.name, false);
        }
        assert_eq!(delays, DELAYS);
        // Out of attempts, the tracker forgets the address.
        assert_eq!(t.ended("Connection refused", None, false, now), None);
    }

    #[test]
    fn a_short_stay_does_not_reset_the_count() {
        let mut t = Tracker::default();
        let now = Instant::now();
        t.connecting("a", "P", false);
        for _ in 0..DELAYS.len() {
            t.connecting("a", "P", false);
            t.joined(now);
            assert!(
                t.ended("Server closed", None, true, now + Duration::from_secs(5))
                    .is_some()
            );
        }
        t.connecting("a", "P", false);
        t.joined(now);
        assert_eq!(
            t.ended("Server closed", None, true, now + Duration::from_secs(5)),
            None
        );
        // A minute connected starts over.
        t.connecting("a", "P", false);
        for _ in 0..3 {
            t.ended("Timed out", None, true, now);
            t.connecting("a", "P", false);
        }
        t.joined(now);
        let r = t.ended("Timed out", None, true, now + STABLE).unwrap();
        assert_eq!(r.attempt, 1);
    }

    #[test]
    fn kicks_and_leaving_reset_and_new_addresses_start_over() {
        let mut t = Tracker::default();
        let now = Instant::now();
        t.connecting("a", "P", false);
        assert!(t.ended("Server closed", None, true, now).is_some());
        assert_eq!(t.ended("Kicked by an operator", None, true, now), None);
        assert_eq!(
            t.ended("Server closed", None, true, now),
            None,
            "forgotten after a kick"
        );
        t.connecting("a", "P", false);
        t.ended("Server closed", None, true, now);
        t.connecting("b", "P", false);
        assert_eq!(
            t.ended("Server closed", None, true, now).unwrap().attempt,
            1
        );
        let mut t = Tracker::default();
        assert_eq!(
            t.ended("Server closed", None, true, now),
            None,
            "never connected"
        );
    }

    #[test]
    fn the_countdown_rounds_up() {
        let now = Instant::now();
        let r = Retry {
            address: "a".into(),
            name: "P".into(),
            lan: false,
            at: now + Duration::from_millis(4200),
            attempt: 2,
        };
        assert_eq!(r.seconds_left(now), 5);
        assert_eq!(r.text(now), "Reconnecting in 5 s (attempt 2 of 5)");
        assert!(!r.due(now));
        assert!(r.due(now + Duration::from_secs(5)));
        assert_eq!(r.seconds_left(now + Duration::from_secs(9)), 0);
    }
}
