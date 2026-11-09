//! The voice chat client: the plugin-channel handshake, the UDP connection
//! (authenticate, connection check, keep-alives, pings), the microphone
//! and the speakers, behind a few calls the game makes.
//!
//! The audio devices live apart from the connection: they're open while on
//! a voice chat server or while testing the microphone, and reopen when the
//! player picks another device. One long-lived thread takes the
//! microphone's frames: it measures them, plays them back for the test, and
//! sends them while connected and talking.
use crate::device;
use crate::mixer::{Listener, Mixer};
use crate::opus::Encoder;
use crate::proto::{self, Cipher, Group, GroupKind, PlayerState, Secret, Voice, channel};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A plugin message to send: (channel, data).
pub type Payload = (String, Vec<u8>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Not on a server, or the server has no voice chat (yet).
    Off,
    /// Asked the server for a secret.
    Waiting,
    /// Have the secret, reaching the voice server over UDP.
    Connecting,
    Connected,
    /// Was connected; the voice server stopped answering.
    Lost,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Talk when loud enough instead of while the key is held.
    pub voice_activation: bool,
    /// Everyone's volume, 0..=2.
    pub volume: f32,
    /// Microphone amplification, 0..=4.
    pub mic_gain: f32,
    /// Voice activation level in dBFS (-60..=0).
    pub threshold_db: f32,
    /// Post-AEC RNNoise neural noise suppression and smart speaker-echo ducking.
    pub rnnoise: bool,
    /// The microphone and speakers by name; None = Windows' default.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            voice_activation: cfg!(target_os = "android"),
            volume: 2.0,
            mic_gain: 1.0,
            threshold_db: -45.0,
            rnnoise: true,
            input_device: None,
            output_device: None,
        }
    }
}

/// A player the server says has voice chat.
#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    pub uuid: u128,
    pub name: String,
    /// Turned their voice chat off.
    pub disabled: bool,
    /// Has no working voice connection.
    pub disconnected: bool,
    pub talking: bool,
    /// 0..=2, 1 = as received.
    pub volume: f32,
    /// The group they're in.
    pub group: Option<u128>,
}

/// A group the server lists, with who's in it.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupInfo {
    pub id: u128,
    pub name: String,
    pub has_password: bool,
    pub persistent: bool,
    pub kind: GroupKind,
    /// Members' names, this player included (as "you" is up to the caller).
    pub members: Vec<(u128, String)>,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub state: State,
    pub transmitting: bool,
    pub muted: bool,
    pub disabled: bool,
    /// Names of the players heard right now.
    pub talking: Vec<String>,
    /// Something to tell the player (a device or connection problem).
    pub problem: Option<String>,
    /// The microphone's last level in dBFS.
    pub level_db: f32,
    pub voice_activation: bool,
    pub group: Option<String>,
    /// The group this player is in, by id.
    pub own_group: Option<u128>,
    /// Every group the server shows (hidden ones only when we're in them).
    pub groups: Vec<GroupInfo>,
    /// The server allows groups.
    pub groups_enabled: bool,
    /// The last group join failed (wrong password).
    pub group_problem: Option<String>,
    /// This player's id on the server (0 before it says).
    pub own: u128,
    /// Testing the microphone (hearing yourself).
    pub testing: bool,
    /// Everyone else the server lists, by name.
    pub players: Vec<Player>,
    /// The devices in use, as opened (None while closed).
    pub microphone: Option<String>,
    pub speakers: Option<String>,
}

struct Shared {
    socket: UdpSocket,
    server: SocketAddr,
    cipher: Cipher,
    player: u128,
    secret: [u8; 16],
    keep_alive: Duration,
    application: i32,
    running: AtomicBool,
    authenticated: AtomicBool,
    connected: AtomicBool,
    lost: AtomicBool,
    /// Milliseconds since `started` of the last keep-alive.
    last_keep_alive: AtomicU64,
    started: Instant,
    mixer: Arc<Mutex<Mixer>>,
    auth_nonce: Mutex<Option<[u8; 16]>>,
}
impl Shared {
    fn send(&self, packet: &Voice) {
        let datagram = proto::wrap(&self.cipher, self.player, packet);
        if let Err(e) = self.socket.send_to(&datagram, self.server) {
            log::debug!("Voice chat: send failed: {e}");
        }
    }
    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
}

/// Microphone controls the mic thread reads.
struct Mic {
    ptt: AtomicBool,
    muted: AtomicBool,
    disabled: AtomicBool,
    voice_activation: AtomicBool,
    rnnoise: AtomicBool,
    testing: AtomicBool,
    threshold_db: AtomicU32,
    gain: AtomicU32,
    transmitting: AtomicBool,
    level_db: AtomicU32,
}
fn f(v: &AtomicU32) -> f32 {
    f32::from_bits(v.load(Relaxed))
}

/// The open devices.
struct Audio {
    /// Dropping it closes them.
    stop: Option<Sender<()>>,
    /// Which ones were asked for (None = default).
    opened_with: (Option<String>, Option<String>),
    names: (Option<String>, Option<String>),
    problem: Option<String>,
}

struct Session {
    shared: Arc<Shared>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.shared.running.store(false, Relaxed);
    }
}

type Resolver = Box<dyn FnOnce() -> Option<IpAddr> + Send>;

struct Inner {
    state: State,
    problem: Option<String>,
    session: Option<Session>,
    /// Bumped on every join, leave and secret: a connect that finishes for an
    /// older one is dropped.
    generation: u64,
    resolver: Option<Arc<Mutex<Option<Resolver>>>>,
    game_ip: Option<IpAddr>,
    players: HashMap<u128, PlayerState>,
    groups: HashMap<u128, Group>,
    groups_enabled: bool,
    group_problem: Option<String>,
    own: u128,
    settings: Settings,
    version_idx: usize,
    waiting_since: Option<Instant>,
}

struct Global {
    inner: Mutex<Inner>,
    mixer: Arc<Mutex<Mixer>>,
    mic: Arc<Mic>,
    audio: Mutex<Audio>,
    /// The connection the mic thread sends on.
    sending: Mutex<Option<Arc<Shared>>>,
    frames: SyncSender<Vec<f32>>,
    frames_rx: Mutex<Option<Receiver<Vec<f32>>>>,
}

fn global() -> &'static Global {
    static G: OnceLock<Global> = OnceLock::new();
    G.get_or_init(|| {
        let settings = Settings::default();
        let (frames, frames_rx) = std::sync::mpsc::sync_channel(16);
        Global {
            inner: Mutex::new(Inner {
                state: State::Off,
                problem: None,
                session: None,
                generation: 0,
                resolver: None,
                game_ip: None,
                players: HashMap::new(),
                groups: HashMap::new(),
                groups_enabled: false,
                group_problem: None,
                own: 0,
                settings: settings.clone(),
                version_idx: 0,
                waiting_since: None,
            }),
            mixer: Arc::new(Mutex::new({
                let mut m = Mixer::default();
                m.volume = settings.volume;
                m
            })),
            mic: Arc::new(Mic {
                ptt: AtomicBool::new(false),
                muted: AtomicBool::new(false),
                disabled: AtomicBool::new(false),
                voice_activation: AtomicBool::new(settings.voice_activation),
                rnnoise: AtomicBool::new(settings.rnnoise),
                testing: AtomicBool::new(false),
                threshold_db: AtomicU32::new(settings.threshold_db.to_bits()),
                gain: AtomicU32::new(settings.mic_gain.to_bits()),
                transmitting: AtomicBool::new(false),
                level_db: AtomicU32::new((-100f32).to_bits()),
            }),
            audio: Mutex::new(Audio {
                stop: None,
                opened_with: (None, None),
                names: (None, None),
                problem: None,
            }),
            sending: Mutex::new(None),
            frames,
            frames_rx: Mutex::new(Some(frames_rx)),
        }
    })
}

/// Joined a server's world: announce the channels and ask for a secret.
/// `resolve` finds the game server's IP (it may block: it runs on the
/// connect thread).
pub fn join(resolve: impl FnOnce() -> Option<IpAddr> + Send + 'static) -> Vec<Payload> {
    let g = global();
    {
        let mut inner = g.inner.lock().unwrap();
        inner.session = None;
        inner.generation += 1;
        inner.state = State::Waiting;
        inner.problem = None;
        inner.players.clear();
        inner.groups.clear();
        inner.group_problem = None;
        inner.game_ip = None;
        inner.resolver = Some(Arc::new(Mutex::new(Some(Box::new(resolve)))));
        inner.version_idx = 0;
        inner.waiting_since = Some(Instant::now());
    }
    *g.sending.lock().unwrap() = None;
    if let Ok(mut m) = g.mixer.lock() {
        m.clear();
    }
    let ver = proto::COMPATIBILITY_VERSIONS[0];
    log::info!("Voice chat: requesting secret (compatibility version {ver})");
    vec![
        ("minecraft:register".into(), proto::register_payload()),
        (
            channel::REQUEST_SECRET.into(),
            proto::request_secret_version(ver),
        ),
    ]
}

/// Retries `voicechat:request_secret` with fallback compatibility versions `[20, 21, 19, 18, 22]`
/// every 4 seconds if the server has not answered yet.
pub fn poll_handshake_retry() -> Vec<Payload> {
    let g = global();
    let mut inner = g.inner.lock().unwrap();
    if inner.state != State::Waiting || inner.session.is_some() {
        return Vec::new();
    }
    let Some(since) = inner.waiting_since else {
        return Vec::new();
    };
    if since.elapsed() < Duration::from_secs(4) {
        return Vec::new();
    }
    let next_idx = inner.version_idx + 1;
    if next_idx >= proto::COMPATIBILITY_VERSIONS.len() {
        inner.waiting_since = None;
        return Vec::new();
    }
    inner.version_idx = next_idx;
    inner.waiting_since = Some(Instant::now());
    let ver = proto::COMPATIBILITY_VERSIONS[next_idx];
    log::info!("Voice chat: no secret reply yet; retrying with compatibility version {ver}");
    vec![(
        channel::REQUEST_SECRET.into(),
        proto::request_secret_version(ver),
    )]
}

/// Left the server.
pub fn leave() {
    let g = global();
    {
        let mut inner = g.inner.lock().unwrap();
        inner.session = None;
        inner.generation += 1;
        inner.state = State::Off;
        inner.problem = None;
        inner.players.clear();
        inner.groups.clear();
        inner.group_problem = None;
        inner.resolver = None;
        inner.waiting_since = None;
    }
    *g.sending.lock().unwrap() = None;
    g.mic.transmitting.store(false, Relaxed);
    if let Ok(mut m) = g.mixer.lock() {
        m.clear();
    }
    maintain();
}

/// A plugin message from the server on a `voicechat:` channel. Returns the
/// messages to send back.
pub fn payload(channel: &str, data: &[u8]) -> Vec<Payload> {
    let g = global();
    let mut out = Vec::new();
    match channel {
        channel::SECRET => match Secret::decode(data) {
            Ok(secret) => {
                let disabled = g.mic.disabled.load(Relaxed);
                {
                    let mut inner = g.inner.lock().unwrap();
                    let ver = proto::COMPATIBILITY_VERSIONS
                        .get(inner.version_idx)
                        .copied()
                        .unwrap_or(proto::COMPATIBILITY_VERSION);
                    log::info!(
                        "Voice chat: received secret (compatibility version {ver}, port {}, codec {:?}, distance {})",
                        secret.server_port,
                        secret.codec,
                        secret.distance
                    );
                    inner.groups_enabled = secret.groups_enabled;
                    inner.waiting_since = None;
                }
                out.push((channel::UPDATE_STATE.into(), proto::update_state(disabled)));
                connect(secret);
            }
            Err(e) => log::warn!("Voice chat: bad secret packet: {e}"),
        },
        channel::STATE => {
            if let Ok(s) = PlayerState::read(&mut proto::Reader::new(data)) {
                g.inner.lock().unwrap().players.insert(s.uuid, s);
            }
        }
        channel::STATES => {
            if let Ok(states) = proto::decode_states(data) {
                let mut inner = g.inner.lock().unwrap();
                inner.players = states.into_iter().map(|s| (s.uuid, s)).collect();
            }
        }
        channel::REMOVE_STATE => {
            if let Ok(uuid) = proto::Reader::new(data).uuid() {
                g.inner.lock().unwrap().players.remove(&uuid);
            }
        }
        channel::ADD_GROUP => match Group::decode(data) {
            Ok(group) => {
                g.inner.lock().unwrap().groups.insert(group.id, group);
            }
            Err(e) => log::warn!("Voice chat: bad group packet: {e}"),
        },
        channel::JOINED_GROUP => {
            if let Ok((group, wrong_password)) = proto::decode_joined_group(data) {
                let mut inner = g.inner.lock().unwrap();
                let own = inner.own;
                inner.group_problem = wrong_password.then(|| "Wrong password".to_owned());
                // The state update follows; show the change at once.
                if let Some(me) = inner.players.get_mut(&own) {
                    me.group = group;
                }
            }
        }
        channel::REMOVE_GROUP => {
            if let Ok(id) = proto::Reader::new(data).uuid() {
                g.inner.lock().unwrap().groups.remove(&id);
            }
        }
        _ => {}
    }
    out
}

fn connect(secret: Secret) {
    let g = global();
    let (generation, resolver, known_ip) = {
        let mut inner = g.inner.lock().unwrap();
        inner.session = None;
        inner.generation += 1;
        inner.state = State::Connecting;
        inner.problem = None;
        inner.own = secret.player;
        (inner.generation, inner.resolver.clone(), inner.game_ip)
    };
    *g.sending.lock().unwrap() = None;
    std::thread::Builder::new()
        .name("voicechat-connect".into())
        .spawn(move || {
            let game_ip = known_ip.or_else(|| {
                resolver
                    .and_then(|r| r.lock().ok()?.take())
                    .and_then(|f| f())
            });
            let result = open(&secret, game_ip);
            let g = global();
            {
                let mut inner = g.inner.lock().unwrap();
                if inner.generation != generation {
                    return;
                }
                inner.game_ip = game_ip;
                match result {
                    Ok(session) => {
                        log::info!("Voice chat: connecting to {}", session.shared.server);
                        *g.sending.lock().unwrap() = Some(session.shared.clone());
                        inner.session = Some(session);
                    }
                    Err(e) => {
                        log::warn!("Voice chat: {e}");
                        inner.state = State::Off;
                        inner.problem = Some(e);
                    }
                }
            }
            maintain();
        })
        .ok();
}

fn open(secret: &Secret, game_ip: Option<IpAddr>) -> Result<Session, String> {
    let game_host = game_ip.map(|ip| ip.to_string()).unwrap_or_default();
    let (host, port) = secret.voice_address(&game_host);
    if host.is_empty() {
        return Err("couldn't find the server's address".into());
    }
    let server = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| format!("voice server {host}: {e}"))?
        .next()
        .ok_or_else(|| format!("voice server {host}: no address"))?;
    let bind: SocketAddr = if server.is_ipv4() {
        "0.0.0.0:0".parse().unwrap()
    } else {
        "[::]:0".parse().unwrap()
    };
    let socket = UdpSocket::bind(bind).map_err(|e| format!("UDP socket: {e}"))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(250)))
        .ok();
    let shared = Arc::new(Shared {
        socket,
        server,
        cipher: Cipher::new(&secret.secret),
        player: secret.player,
        secret: secret.secret,
        keep_alive: Duration::from_millis(secret.keep_alive_ms.clamp(100, 60_000) as u64),
        application: secret.codec.opus_application(),
        running: AtomicBool::new(true),
        authenticated: AtomicBool::new(false),
        connected: AtomicBool::new(false),
        lost: AtomicBool::new(false),
        last_keep_alive: AtomicU64::new(0),
        started: Instant::now(),
        mixer: global().mixer.clone(),
        auth_nonce: Mutex::new(None),
    });
    let receiver = shared.clone();
    std::thread::Builder::new()
        .name("voicechat-receive".into())
        .spawn(move || receive(receiver))
        .map_err(|e| e.to_string())?;
    let maintainer = shared.clone();
    std::thread::Builder::new()
        .name("voicechat-auth".into())
        .spawn(move || keep_up(maintainer))
        .map_err(|e| e.to_string())?;
    Ok(Session { shared })
}

fn receive(s: Arc<Shared>) {
    let mut buf = vec![0u8; 4096];
    while s.running.load(Relaxed) {
        let (n, from) = match s.socket.recv_from(&mut buf) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if from.ip() != s.server.ip() {
            continue;
        }
        let packet = match proto::unwrap(&s.cipher, &buf[..n]) {
            Ok(p) => p,
            Err(e) => {
                log::debug!("Voice chat: dropped a packet: {e}");
                continue;
            }
        };
        match packet {
            Voice::AuthenticateAck => {
                if let Ok(mut n) = s.auth_nonce.lock() {
                    *n = None;
                }
                if !s.authenticated.swap(true, Relaxed) {
                    log::info!("Voice chat: authenticated (v18-v20 ack)");
                }
            }
            Voice::AuthenticateAckNonce(nonce) => {
                if let Ok(mut n) = s.auth_nonce.lock() {
                    *n = Some(nonce);
                }
                if !s.authenticated.swap(true, Relaxed) {
                    log::info!("Voice chat: authenticated (v21+ nonce ack)");
                }
            }
            Voice::ConnectionCheckAck => {
                if s.authenticated.load(Relaxed) && !s.connected.swap(true, Relaxed) {
                    log::info!("Voice chat: connected");
                    s.last_keep_alive.store(s.now_ms(), Relaxed);
                }
            }
            Voice::KeepAlive => {
                s.last_keep_alive.store(s.now_ms(), Relaxed);
                s.send(&Voice::KeepAlive);
            }
            p @ Voice::Ping { .. } => s.send(&p),
            p @ (Voice::PlayerSound { .. }
            | Voice::GroupSound { .. }
            | Voice::LocationSound { .. }) => {
                if let Ok(mut m) = s.mixer.lock() {
                    m.push(p);
                }
            }
            _ => {}
        }
    }
}

/// Authenticate until acknowledged, then check the connection until
/// acknowledged (once a second each, like the mod), then watch keep-alives.
fn keep_up(s: Arc<Shared>) {
    while s.running.load(Relaxed) {
        if !s.authenticated.load(Relaxed) {
            s.send(&Voice::Authenticate {
                player: s.player,
                secret: s.secret,
            });
        } else if !s.connected.load(Relaxed) {
            let check = match s.auth_nonce.lock().ok().and_then(|n| *n) {
                Some(nonce) => Voice::ConnectionCheckNonce(nonce),
                None => Voice::ConnectionCheck,
            };
            s.send(&check);
        } else {
            let silent = s.now_ms().saturating_sub(s.last_keep_alive.load(Relaxed));
            if silent > s.keep_alive.as_millis() as u64 * 10 {
                log::warn!("Voice chat: connection timed out");
                s.lost.store(true, Relaxed);
                s.running.store(false, Relaxed);
                return;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// The microphone thread: started once, runs for the game's life.
fn microphone(frames: Receiver<Vec<f32>>) {
    let g = global();
    let mic = g.mic.clone();
    let mut filter = crate::mic_filter::MicFilter::new();
    let mut encoder: Option<(i32, Encoder)> = None;
    let mut sequence: i64 = 0;
    let mut last_on: Option<*const Shared> = None;
    loop {
        let Ok(frame) = frames.recv() else { return };
        let (speaker_ref, remote_rms) = crate::mic_filter::pull_speaker_reference(frame.len());
        let filtered = filter.process_frame(
            &frame,
            &speaker_ref,
            remote_rms,
            f(&mic.gain),
            f(&mic.threshold_db),
            mic.rnnoise.load(Relaxed),
        );
        mic.level_db.store(filtered.level_db.to_bits(), Relaxed);
        let voice = mic.voice_activation.load(Relaxed);
        let speaking = if voice {
            filtered.speaking
        } else {
            mic.ptt.load(Relaxed)
        };
        let samples = filtered.samples;
        if mic.testing.load(Relaxed)
            && let Ok(mut m) = g.mixer.lock()
        {
            // The test plays what would be sent: with voice activation, only
            // above the level, so the level can be tuned by ear.
            if !voice || speaking {
                m.push_local(&samples);
            }
        }
        let shared = g.sending.lock().unwrap().clone();
        let Some(s) = shared.filter(|s| s.connected.load(Relaxed) && s.running.load(Relaxed))
        else {
            mic.transmitting.store(false, Relaxed);
            continue;
        };
        // A new connection starts its sequence and encoder afresh.
        if last_on != Some(Arc::as_ptr(&s)) {
            last_on = Some(Arc::as_ptr(&s));
            sequence = 0;
            encoder = None;
            filter.reset();
        }
        if encoder
            .as_ref()
            .is_none_or(|(app, _)| *app != s.application)
        {
            encoder = Encoder::new(s.application).map(|e| (s.application, e));
        }
        let Some((_, enc)) = encoder.as_mut() else {
            continue;
        };
        let wanted = speaking && !mic.muted.load(Relaxed) && !mic.disabled.load(Relaxed);
        if wanted {
            let pcm: Vec<i16> = samples.iter().map(|&x| (x * 32767.0) as i16).collect();
            if let Some(data) = enc.encode(&pcm) {
                s.send(&Voice::Mic {
                    data,
                    sequence,
                    whispering: false,
                });
                sequence += 1;
                mic.transmitting.store(true, Relaxed);
            }
        } else if mic.transmitting.swap(false, Relaxed) {
            s.send(&Voice::Mic {
                data: Vec::new(),
                sequence,
                whispering: false,
            });
            sequence += 1;
            enc.reset();
        }
    }
}

/// Open, reopen or close the devices as needed: open while on a voice chat
/// server or testing, with the devices the settings name.
pub fn maintain() {
    let g = global();
    if let Some(rx) = g.frames_rx.lock().unwrap().take() {
        std::thread::Builder::new()
            .name("voicechat-mic".into())
            .spawn(move || microphone(rx))
            .ok();
    }
    let (wanted, devices) = {
        let inner = g.inner.lock().unwrap();
        let on_server = inner.session.is_some();
        (
            on_server || g.mic.testing.load(Relaxed),
            (
                inner.settings.input_device.clone(),
                inner.settings.output_device.clone(),
            ),
        )
    };
    let mut audio = g.audio.lock().unwrap();
    let open = audio.stop.is_some();
    if open && (!wanted || audio.opened_with != devices) {
        audio.stop = None;
        audio.names = (None, None);
        audio.problem = None;
    }
    if wanted && audio.stop.is_none() {
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let errors = device::run(
            g.mixer.clone(),
            g.frames.clone(),
            stop_rx,
            devices.0.clone(),
            devices.1.clone(),
        );
        let mut problems = Vec::new();
        if let Some(e) = &errors.speakers {
            problems.push(format!("speakers: {e}"));
        }
        if let Some(e) = &errors.microphone {
            problems.push(format!("microphone: {e}"));
        }
        audio.problem = (!problems.is_empty()).then(|| problems.join("; "));
        if let Some(p) = &audio.problem {
            log::warn!("Voice chat: {p}");
        }
        audio.names = (
            errors.microphone.is_none().then(|| {
                devices
                    .0
                    .clone()
                    .unwrap_or_else(|| "Default microphone".into())
            }),
            errors.speakers.is_none().then(|| {
                devices
                    .1
                    .clone()
                    .unwrap_or_else(|| "Default speakers".into())
            }),
        );
        audio.opened_with = devices;
        audio.stop = Some(stop_tx);
    }
}

/// The microphones Windows offers, by name.
pub fn microphones() -> Vec<String> {
    device::input_names()
}

/// The speakers and headphones Windows offers, by name.
pub fn speakers() -> Vec<String> {
    device::output_names()
}

/// The push-to-talk key went down or up.
pub fn push_to_talk(held: bool) {
    global().mic.ptt.store(held, Relaxed);
}

/// Mute or unmute the microphone; returns whether it's muted now.
pub fn toggle_mute() -> bool {
    !global().mic.muted.fetch_xor(true, Relaxed)
}

/// Hear yourself (or stop); returns whether testing now.
pub fn set_testing(on: bool) {
    let g = global();
    g.mic.testing.store(on, Relaxed);
    if !on && let Ok(mut m) = g.mixer.lock() {
        m.push_local(&[]);
    }
    maintain();
}

/// Turn voice chat off or on (hear nobody, nobody hears you); returns
/// whether it's off now and the state update for the server.
pub fn toggle_disabled() -> (bool, Vec<Payload>) {
    let g = global();
    let disabled = !g.mic.disabled.fetch_xor(true, Relaxed);
    if let Ok(mut m) = g.mixer.lock() {
        m.deafened = disabled;
        if disabled {
            m.clear();
        }
    }
    let connected = g.inner.lock().unwrap().state != State::Off;
    let out = if connected {
        vec![(channel::UPDATE_STATE.into(), proto::update_state(disabled))]
    } else {
        Vec::new()
    };
    (disabled, out)
}

pub fn set_settings(settings: Settings) {
    let g = global();
    g.mic
        .voice_activation
        .store(settings.voice_activation, Relaxed);
    g.mic.rnnoise.store(settings.rnnoise, Relaxed);
    g.mic
        .threshold_db
        .store(settings.threshold_db.to_bits(), Relaxed);
    g.mic.gain.store(settings.mic_gain.to_bits(), Relaxed);
    if let Ok(mut m) = g.mixer.lock() {
        m.volume = settings.volume;
    }
    g.inner.lock().unwrap().settings = settings;
    maintain();
}

fn connected_payload(channel: &str, data: Vec<u8>) -> Vec<Payload> {
    let inner = global().inner.lock().unwrap();
    if inner.state == State::Off || !inner.groups_enabled {
        return Vec::new();
    }
    vec![(channel.to_owned(), data)]
}

/// Make a group (and join it): its name, an optional password, its kind.
pub fn create_group(name: &str, password: Option<&str>, kind: GroupKind) -> Vec<Payload> {
    let name = name.trim();
    if name.is_empty() {
        return Vec::new();
    }
    let password = password.map(str::trim).filter(|p| !p.is_empty());
    global().inner.lock().unwrap().group_problem = None;
    connected_payload(
        channel::CREATE_GROUP,
        proto::create_group(name, password, kind),
    )
}

/// Join group `id`, with its password if it has one.
pub fn join_group(id: u128, password: Option<&str>) -> Vec<Payload> {
    global().inner.lock().unwrap().group_problem = None;
    connected_payload(channel::SET_GROUP, proto::join_group(id, password))
}

/// Leave the group this player is in.
pub fn leave_group() -> Vec<Payload> {
    connected_payload(channel::LEAVE_GROUP, Vec::new())
}

/// One player's volume, 0..=2.
pub fn set_player_volume(uuid: u128, volume: f32) {
    if let Ok(mut m) = global().mixer.lock() {
        if (volume - 1.0).abs() < 1e-3 {
            m.player_volume.remove(&uuid);
        } else {
            m.player_volume.insert(uuid, volume.clamp(0.0, 2.0));
        }
    }
}

/// Each tick: where the listener is and the positions of the entities
/// voices come from.
pub fn update_listener(
    position: [f64; 3],
    yaw: f32,
    mut locate: impl FnMut(u128) -> Option<[f64; 3]>,
) {
    let g = global();
    let wanted = match g.mixer.lock() {
        Ok(m) => m.wanted_positions(),
        Err(_) => return,
    };
    let found: HashMap<u128, [f64; 3]> = wanted
        .into_iter()
        .filter_map(|e| locate(e).map(|p| (e, p)))
        .collect();
    if let Ok(mut m) = g.mixer.lock() {
        m.listener = Listener { position, yaw };
        m.positions = found;
    }
}

/// Also updates the state: a lost connection shows as Lost.
pub fn status() -> Status {
    let g = global();
    let (talking_ids, volumes) = g
        .mixer
        .lock()
        .map(|m| (m.talking(), m.player_volume.clone()))
        .unwrap_or_default();
    let (audio_problem, names) = {
        let a = g.audio.lock().unwrap();
        (a.problem.clone(), a.names.clone())
    };
    let mut inner = g.inner.lock().unwrap();
    let mut lost = false;
    if let Some(s) = &inner.session {
        if s.shared.lost.load(Relaxed) {
            lost = true;
        } else if s.shared.connected.load(Relaxed) {
            inner.state = State::Connected;
        } else if inner.state == State::Connecting
            && s.shared.started.elapsed() > Duration::from_secs(10)
            && inner.problem.is_none()
        {
            inner.problem = Some(format!(
                "no answer from the voice server at {} (its UDP port may be blocked)",
                s.shared.server
            ));
        }
    }
    if lost {
        inner.session = None;
        inner.state = State::Lost;
        inner.problem = Some("the voice server stopped answering".into());
        *g.sending.lock().unwrap() = None;
    }
    let talking: Vec<String> = talking_ids
        .iter()
        .filter(|id| **id != inner.own)
        .map(|id| {
            inner
                .players
                .get(id)
                .map_or_else(|| "someone".to_owned(), |p| p.name.clone())
        })
        .collect();
    let own_group = inner.players.get(&inner.own).and_then(|p| p.group);
    let group = own_group.map(|id| {
        inner
            .groups
            .get(&id)
            .map_or_else(|| "a group".into(), |g| g.name.clone())
    });
    let mut groups: Vec<GroupInfo> = inner
        .groups
        .values()
        .filter(|g| !g.hidden || Some(g.id) == own_group)
        .map(|g| {
            let mut members: Vec<(u128, String)> = inner
                .players
                .values()
                .filter(|p| p.group == Some(g.id))
                .map(|p| (p.uuid, p.name.clone()))
                .collect();
            members.sort_by_key(|m| m.1.to_lowercase());
            GroupInfo {
                id: g.id,
                name: g.name.clone(),
                has_password: g.has_password,
                persistent: g.persistent,
                kind: g.kind,
                members,
            }
        })
        .collect();
    groups.sort_by_key(|g| (Some(g.id) != own_group, g.name.to_lowercase()));
    let mut players: Vec<Player> = inner
        .players
        .values()
        .filter(|p| p.uuid != inner.own)
        .map(|p| Player {
            uuid: p.uuid,
            name: p.name.clone(),
            disabled: p.disabled,
            disconnected: p.disconnected,
            talking: talking_ids.contains(&p.uuid),
            volume: volumes.get(&p.uuid).copied().unwrap_or(1.0),
            group: p.group,
        })
        .collect();
    players.sort_by_key(|p| p.name.to_lowercase());
    let problem = inner.problem.clone().or(audio_problem);
    let status = Status {
        state: inner.state,
        transmitting: g.mic.transmitting.load(Relaxed),
        muted: g.mic.muted.load(Relaxed),
        disabled: g.mic.disabled.load(Relaxed),
        talking,
        problem,
        level_db: f(&g.mic.level_db),
        voice_activation: inner.settings.voice_activation,
        group,
        own_group,
        groups,
        groups_enabled: inner.groups_enabled,
        group_problem: inner.group_problem.clone(),
        own: inner.own,
        testing: g.mic.testing.load(Relaxed),
        players,
        microphone: names.0,
        speakers: names.1,
    };
    drop(inner);
    if lost {
        maintain();
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny voice server on localhost: answers the handshake, keep-alive
    /// and ping like a real one.
    #[test]
    fn handshake_against_a_local_server() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let port = server.local_addr().unwrap().port();
        let key = [5u8; 16];
        let me: u128 = 0xABCD;
        let mut w = proto::Writer::new();
        w.data.extend_from_slice(&key);
        w.i32(port as i32);
        w.uuid(me);
        w.u8(0);
        w.i32(1024);
        w.f64(48.0);
        w.i32(1000);
        w.bool(true);
        w.string("");
        w.bool(false);

        let first = join(|| Some("127.0.0.1".parse().unwrap()));
        assert_eq!(first[0].0, "minecraft:register");
        assert_eq!(
            first[1],
            (channel::REQUEST_SECRET.to_owned(), vec![0, 0, 0, 20])
        );
        let reply = payload(channel::SECRET, &w.data);
        assert_eq!(reply, vec![(channel::UPDATE_STATE.to_owned(), vec![0])]);

        let cipher = Cipher::new(&key);
        let mut buf = [0u8; 4096];
        let read = |buf: &mut [u8]| -> (Voice, SocketAddr) {
            let (n, from) = server.recv_from(buf).unwrap();
            assert_eq!(buf[0], 0xFF);
            assert_eq!(u128::from_be_bytes(buf[1..17].try_into().unwrap()), me);
            let mut body = vec![0xFF];
            body.extend_from_slice(&buf[17..n]);
            (proto::unwrap(&cipher, &body).unwrap(), from)
        };
        let send = |p: &Voice, to: SocketAddr| {
            let mut w = proto::Writer::new();
            w.u8(0xFF);
            w.byte_array(&cipher.encrypt(&p.encode()));
            server.send_to(&w.data, to).unwrap();
        };
        let (p, client) = read(&mut buf);
        assert_eq!(
            p,
            Voice::Authenticate {
                player: me,
                secret: key
            }
        );
        send(&Voice::AuthenticateAck, client);
        loop {
            let (p, _) = read(&mut buf);
            if p == Voice::ConnectionCheck {
                break;
            }
        }
        send(&Voice::ConnectionCheckAck, client);
        send(&Voice::KeepAlive, client);
        loop {
            let (p, _) = read(&mut buf);
            if p == Voice::KeepAlive {
                break;
            }
        }
        send(
            &Voice::Ping {
                id: 9,
                timestamp: 1,
            },
            client,
        );
        loop {
            let (p, _) = read(&mut buf);
            if p == (Voice::Ping {
                id: 9,
                timestamp: 1,
            }) {
                break;
            }
        }
        assert_eq!(status().state, State::Connected);

        // The server lists two players; we see the other one.
        let mut w = proto::Writer::new();
        w.i32(2);
        PlayerState {
            disabled: false,
            disconnected: false,
            uuid: me,
            name: "me".into(),
            group: None,
        }
        .write(&mut w);
        PlayerState {
            disabled: true,
            disconnected: false,
            uuid: 77,
            name: "Steve".into(),
            group: None,
        }
        .write(&mut w);
        payload(channel::STATES, &w.data);
        set_player_volume(77, 1.5);
        let players = status().players;
        assert_eq!(players.len(), 1);
        assert_eq!(
            (
                players[0].name.as_str(),
                players[0].disabled,
                players[0].volume
            ),
            ("Steve", true, 1.5)
        );

        // Groups: the server lists one, we ask to join, it says we're in.
        let group = Group {
            id: 5,
            name: "Base".into(),
            has_password: true,
            persistent: false,
            hidden: false,
            kind: GroupKind::Open,
        };
        payload(channel::ADD_GROUP, &group.encode());
        let asks = join_group(5, Some("pw"));
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].0, channel::SET_GROUP);
        let mut w = proto::Writer::new();
        w.bool(true);
        w.uuid(5);
        w.bool(false);
        payload(channel::JOINED_GROUP, &w.data);
        let s = status();
        assert_eq!(s.own_group, Some(5));
        assert_eq!(s.group.as_deref(), Some("Base"));
        assert_eq!(s.groups.len(), 1);
        assert_eq!(s.groups[0].members, vec![(me, "me".to_owned())]);
        assert_eq!(
            leave_group(),
            vec![(channel::LEAVE_GROUP.to_owned(), Vec::new())]
        );
        payload(channel::JOINED_GROUP, &[0, 1]);
        assert_eq!(status().group_problem.as_deref(), Some("Wrong password"));
        assert_eq!(create_group("  ", None, GroupKind::Normal), Vec::new());

        let (off, update) = toggle_disabled();
        assert!(off);
        assert_eq!(update, vec![(channel::UPDATE_STATE.to_owned(), vec![1])]);
        toggle_disabled();
        leave();
        assert_eq!(status().state, State::Off);
    }
}
