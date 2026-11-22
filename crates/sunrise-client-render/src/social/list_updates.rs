use super::service::{Answer, Failure, Service, fetch_friends, session};
use super::{FriendData, ResultCode, lock, parse_friends};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

/// `RemoteFriendListUpdateHandler.State`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Loading,
    UpgradeNeeded,
    ConnectionIssue,
    UserMayLackActiveProfile,
    Unauthorized,
    TemporaryUnavailable,
    GenericError,
    Success,
}
impl State {
    /// `RemoteFriendListUpdateHandler.mapResultCodeToState`.
    fn of(code: ResultCode) -> Self {
        match code {
            ResultCode::Success => Self::Success,
            ResultCode::UpgradeNeeded => Self::UpgradeNeeded,
            ResultCode::ConnectionIssue => Self::ConnectionIssue,
            ResultCode::UnknownProfile => Self::UserMayLackActiveProfile,
            ResultCode::Unauthorized => Self::Unauthorized,
            ResultCode::TemporaryUnavailable
            | ResultCode::Forbidden
            | ResultCode::ServiceNotAvailable
            | ResultCode::TooManyRequests => Self::TemporaryUnavailable,
            ResultCode::Error | ResultCode::GenericError => Self::GenericError,
        }
    }
}

/// `RemoteFriendListUpdateHandler`.
pub(super) struct Handler {
    pub(super) enabled: bool,
    pub(super) state: State,
    pub(super) latest: FriendData,
    /// `lastUpdateNanos`: a read at that moment is due; `None` reads at once.
    pub(super) last_update: Option<Instant>,
    /// A forced read (`forceUpdate`) is due, whatever the interval.
    pub(super) forced: bool,
    /// `getUpdateIntervalNanos`'s foreground: the friends overlay is open.
    foreground: bool,
}

impl Default for Handler {
    fn default() -> Self {
        Self {
            enabled: false,
            state: State::Loading,
            latest: FriendData::default(),
            last_update: None,
            forced: false,
            foreground: false,
        }
    }
}

impl Handler {
    /// `getUpdateIntervalNanos`: the service's interval (a minute when it named none),
    /// five times as long in the background.
    fn interval(&self, service: &Service) -> Duration {
        let foreground = service.friends_poll.unwrap_or(Duration::from_secs(60));
        if self.foreground {
            foreground
        } else {
            foreground * 5
        }
    }
}

/// `RemoteFriendListUpdateHandler`'s foreground interval: the friends overlay is open.
pub fn set_foreground(open: bool) {
    lock().handler.foreground = open;
}

/// `RemoteFriendListUpdateHandler.start`'s worker: one read per interval while the list
/// is enabled, the first one as soon as it starts.
pub(super) fn spawn_updater() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = thread::Builder::new()
        .name("Friends List".into())
        .spawn(|| {
            loop {
                thread::sleep(Duration::from_secs(1));
                let due = {
                    let inner = lock();
                    if !inner.handler.enabled {
                        continue;
                    }
                    inner.handler.forced
                        || inner
                            .handler
                            .last_update
                            .is_none_or(|at| at.elapsed() >= inner.handler.interval(&inner.service))
                };
                if due {
                    update_friend_data();
                }
            }
        });
}

/// `RemoteFriendListUpdateHandler.runUpdateFriendDataInternal`.
pub(super) fn update_friend_data() {
    let etag = {
        let mut inner = lock();
        if inner.service.pending {
            // Another read is in flight; its answer is the cache's.
            return;
        }
        if !inner.service.can_make_request() {
            inner.handler.forced = false;
            inner.handler.state = State::Success;
            inner.handler.latest = inner.service.friends_cache.clone();
            inner.handler.last_update = Some(Instant::now());
            return;
        }
        inner.service.pending = true;
        inner.service.cooldown = Some(Instant::now() + Duration::from_secs(10));
        inner.service.friends_etag.clone()
    };
    log::debug!("Performing Friends List update");
    let result = match session() {
        Some(session) => fetch_friends(&session, etag.as_deref()),
        None => Err(Failure {
            code: ResultCode::Unauthorized,
        }),
    };
    let mut inner = lock();
    inner.service.pending = false;
    inner.handler.forced = false;
    inner.handler.last_update = Some(Instant::now());
    let inner = &mut *inner;
    apply_friend_data_answer(&mut inner.service, &mut inner.handler, result);
}

/// `getFriendData`'s answer: the cached lists and their ETag change only together,
/// when a fresh body parses. A 304, an empty body or a parse failure keeps both.
fn apply_friend_data_answer(
    service: &mut Service,
    handler: &mut Handler,
    result: Result<Answer, Failure>,
) {
    match result {
        Ok(answer) => {
            if let Some(retry) = answer.retry_after {
                service.friends_poll = Some(retry);
            }
            if answer.status != 304 && !answer.body.is_empty() {
                match parse_friends(&answer.body) {
                    Some(data) => {
                        service.friends_cache = data;
                        service.friends_etag = answer.etag;
                    }
                    None => {
                        // authlib's parse failure is an ERROR result: the handler keeps
                        // the cached lists and shows the generic error.
                        handler.latest = service.friends_cache.clone();
                        handler.state = State::GenericError;
                        return;
                    }
                }
            }
            handler.latest = service.friends_cache.clone();
            handler.state = State::Success;
        }
        Err(failure) => {
            handler.latest = service.friends_cache.clone();
            handler.state = State::of(failure.code);
            log::warn!(
                "Friends List update failed with result code: {:?}",
                failure.code
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Friend;
    use super::*;

    fn answer(status: u16, etag: Option<&str>, body: &str) -> Answer {
        Answer {
            status,
            etag: etag.map(str::to_owned),
            retry_after: None,
            body: body.to_owned(),
        }
    }

    fn data(name: &str) -> FriendData {
        FriendData {
            friends: vec![Friend {
                id: 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef,
                name: name.into(),
            }],
            ..Default::default()
        }
    }

    fn seeded() -> (Service, Handler) {
        let mut service = Service::default();
        service.friends_etag = Some("A".into());
        service.friends_cache = data("Alice");
        (service, Handler::default())
    }

    const BOB: &str = r#"{"friends":[{"profileId":"0123456789abcdef0123456789abcdef","name":"Bob"}],"incomingRequests":[],"outgoingRequests":[]}"#;

    #[test]
    fn etag_survives_304_with_a_new_header() {
        let (mut service, mut handler) = seeded();
        apply_friend_data_answer(&mut service, &mut handler, Ok(answer(304, Some("B"), "")));
        assert_eq!(service.friends_etag.as_deref(), Some("A"));
        assert_eq!(service.friends_cache, data("Alice"));
        assert_eq!(handler.state, State::Success);
        assert_eq!(handler.latest, data("Alice"));
    }

    #[test]
    fn etag_survives_empty_200_with_a_new_header() {
        let (mut service, mut handler) = seeded();
        apply_friend_data_answer(&mut service, &mut handler, Ok(answer(200, Some("B"), "")));
        assert_eq!(service.friends_etag.as_deref(), Some("A"));
        assert_eq!(service.friends_cache, data("Alice"));
        assert_eq!(handler.state, State::Success);
    }

    #[test]
    fn etag_survives_malformed_200_body() {
        let (mut service, mut handler) = seeded();
        apply_friend_data_answer(&mut service, &mut handler, Ok(answer(200, Some("B"), "{")));
        assert_eq!(service.friends_etag.as_deref(), Some("A"));
        assert_eq!(service.friends_cache, data("Alice"));
        assert_eq!(handler.state, State::GenericError);
        assert_eq!(handler.latest, data("Alice"));
    }

    #[test]
    fn valid_200_without_etag_clears_old_etag() {
        let (mut service, mut handler) = seeded();
        apply_friend_data_answer(&mut service, &mut handler, Ok(answer(200, None, BOB)));
        assert_eq!(service.friends_etag, None);
        assert_eq!(service.friends_cache, data("Bob"));
        assert_eq!(handler.state, State::Success);
        assert_eq!(handler.latest, data("Bob"));
    }

    #[test]
    fn valid_200_with_etag_replaces_both() {
        let (mut service, mut handler) = seeded();
        apply_friend_data_answer(&mut service, &mut handler, Ok(answer(200, Some("B"), BOB)));
        assert_eq!(service.friends_etag.as_deref(), Some("B"));
        assert_eq!(service.friends_cache, data("Bob"));
    }

    #[test]
    fn failed_request_keeps_etag() {
        let (mut service, mut handler) = seeded();
        apply_friend_data_answer(
            &mut service,
            &mut handler,
            Err(Failure {
                code: ResultCode::Unauthorized,
            }),
        );
        assert_eq!(service.friends_etag.as_deref(), Some("A"));
        assert_eq!(service.friends_cache, data("Alice"));
        assert_eq!(handler.state, State::Unauthorized);
        assert_eq!(handler.latest, data("Alice"));
    }

    #[test]
    fn result_codes_map_to_the_handlers_states() {
        assert_eq!(State::of(ResultCode::Success), State::Success);
        assert_eq!(
            State::of(ResultCode::TooManyRequests),
            State::TemporaryUnavailable
        );
        assert_eq!(
            State::of(ResultCode::UnknownProfile),
            State::UserMayLackActiveProfile
        );
        assert_eq!(State::of(ResultCode::UpgradeNeeded), State::UpgradeNeeded);
        assert_eq!(
            State::of(ResultCode::ConnectionIssue),
            State::ConnectionIssue
        );
    }
}
