//! ALSA input via capture-only probes. CPAL 0.16 opens both directions while enumerating,
//! discards endpoints whose opens both fail, and does not expose their PCM addresses.

use super::{
    BackendDevice, CaptureBackend, CaptureRing, DeviceInventory, InputConverter, StreamEvents,
    record_input_error, stop,
};
use alsa::{
    Direction::Capture,
    ValueOr::Nearest,
    pcm,
    poll::{Descriptors, Flags},
};
use mod_api::{MicRefusal, MicSpec, MicTarget};
use std::sync::Arc;
use sunrise_audio_input::alsa::CaptureControl;

const DEFAULT_PCM: &str = "default";

pub(super) struct AlsaBackend;

#[derive(Clone)]
pub(super) struct AlsaDevice {
    pcm: String,
    card_index: Option<i32>,
    label: String,
    // This enumeration's capture result, not a cache. The endpoint survives a failed probe.
    capture: Result<(), MicRefusal>,
}

impl AlsaDevice {
    fn probe<T>(
        pcm: String,
        card_index: Option<i32>,
        label: String,
        open: impl FnOnce(&str) -> alsa::Result<T>,
    ) -> Self {
        let capture = open(&pcm).map(drop).map_err(map_error);
        Self {
            pcm,
            card_index,
            label,
            capture,
        }
    }

    fn open<T>(&self, open: impl FnOnce(&str) -> alsa::Result<T>) -> Result<T, MicRefusal> {
        self.capture?;
        open(&self.pcm).map_err(map_error)
    }
}

impl BackendDevice for AlsaDevice {
    type InputConfigs = ();

    fn label(&self) -> Option<String> {
        Some(self.label.clone())
    }

    fn stable_selector(&self) -> Option<String> {
        Some(match self.card_index {
            Some(index) => format!("alsa:{index}:{}", self.pcm),
            None => format!("alsa:{}", self.pcm),
        })
    }

    fn same_endpoint(&self, other: &Self) -> bool {
        self.card_index == other.card_index && self.pcm == other.pcm
    }

    fn input_configs(&self) -> Result<Option<()>, MicRefusal> {
        self.capture.map(Some)
    }
}

fn open_pcm(name: &str) -> alsa::Result<pcm::PCM> {
    // Hardware addresses already include plug; apply the same PCM conversion to the
    // named routes, whose configured devices need not natively support signed 16-bit PCM.
    if name.starts_with("plughw:") {
        pcm::PCM::new(name, Capture, true)
    } else {
        pcm::PCM::new(&format!("plug:{name}"), Capture, true)
    }
}

fn map_error(err: alsa::Error) -> MicRefusal {
    match err.errno() {
        libc::EACCES | libc::EPERM => MicRefusal::OsDenied,
        libc::EBUSY => MicRefusal::Busy,
        libc::ENOENT | libc::ENODEV | libc::ENXIO => MicRefusal::NoDevice,
        _ => MicRefusal::DeviceFailed,
    }
}

/// The control operations used by discovery, separated from PCM opening and testable without ALSA hardware.
trait CardControl {
    fn card_info(&self) -> alsa::Result<(String, String)>;
    fn pcm_devices(&self) -> impl Iterator<Item = alsa::Result<u32>> + '_;
    fn query_capture(&self, device: u32) -> alsa::Result<()>;
}

impl CardControl for CaptureControl {
    fn card_info(&self) -> alsa::Result<(String, String)> {
        self.card_info()
    }

    fn pcm_devices(&self) -> impl Iterator<Item = alsa::Result<u32>> + '_ {
        self.pcm_devices()
    }

    fn query_capture(&self, device: u32) -> alsa::Result<()> {
        self.query_capture(device)
    }
}

#[derive(Clone, Copy)]
struct HardwareSelector<'a> {
    card_index: i32,
    card_id: &'a str,
    device: u32,
}

impl<'a> HardwareSelector<'a> {
    fn parse(id: &'a str) -> Option<Self> {
        let (index, pcm) = id.strip_prefix("alsa:")?.split_once(':')?;
        let (card_id, device) = pcm.strip_prefix("plughw:CARD=")?.rsplit_once(",DEV=")?;
        Some(Self {
            card_index: index.parse().ok()?,
            card_id,
            device: device.parse().ok()?,
        })
    }
}

fn discover_devices<C: CardControl, T>(
    cards: impl Iterator<Item = alsa::Result<alsa::Card>>,
    target: Option<&MicTarget>,
    listed_label_ids: &[String],
    mut open_control: impl FnMut(alsa::Card) -> alsa::Result<C>,
    mut open_pcm: impl FnMut(&str) -> alsa::Result<T>,
) -> DeviceInventory<AlsaDevice> {
    let mut inventory = DeviceInventory {
        devices: Vec::new(),
        discovery_error: None,
    };
    // Probe named ALSA routes independently of hardware cards (e.g. PipeWire).
    for name in [DEFAULT_PCM, "pipewire", "pulse", "jack", "oss"] {
        let matches = match target {
            Some(MicTarget::DeviceId(id)) => id.strip_prefix("alsa:") == Some(name),
            Some(MicTarget::Label(label)) => label == name,
            None => true,
        };
        if matches {
            inventory.devices.push(AlsaDevice::probe(
                name.to_owned(),
                None,
                name.to_owned(),
                &mut open_pcm,
            ));
        }
    }
    let (hardware, target_label) = match target {
        Some(MicTarget::DeviceId(id)) => match HardwareSelector::parse(id) {
            Some(hardware) => (Some(hardware), None),
            None => return inventory,
        },
        Some(MicTarget::Label(label)) => (None, Some(label.as_str())),
        None => (None, None),
    };
    let listed_cards: Vec<_> = listed_label_ids
        .iter()
        .filter_map(|id| HardwareSelector::parse(id).map(|selector| selector.card_index))
        .collect();
    for card in cards {
        let card = match card {
            Ok(card) => card,
            Err(err) => {
                // snd_card_next itself failed: there is no next card to advance from.
                record_input_error(&mut inventory.discovery_error, map_error(err));
                break;
            }
        };
        let index = card.get_index();
        if let Some(wanted) = hardware {
            if index > wanted.card_index {
                break;
            }
            if index != wanted.card_index {
                continue;
            }
        }
        let (ctl, card_id, label) = match open_control(card)
            .and_then(|ctl| ctl.card_info().map(|(id, name)| (ctl, id, name)))
        {
            Ok(info) => info,
            Err(err) => {
                // A listed ID or label can identify a relevant card even when its control
                // can't be read. Do not attribute a fault on an unrelated card to this label.
                let refusal = map_error(err);
                if refusal != MicRefusal::NoDevice
                    && (target_label.is_none() || listed_cards.contains(&index))
                {
                    record_input_error(&mut inventory.discovery_error, refusal);
                }
                if hardware.is_some() {
                    break;
                }
                continue;
            }
        };
        if hardware.is_some_and(|wanted| wanted.card_id != card_id) {
            // The slot is occupied by a different card, not the requested endpoint.
            break;
        }
        if target_label.is_some_and(|wanted| wanted != label) {
            continue;
        }
        for device in ctl.pcm_devices() {
            let device = match device {
                Ok(device) => device,
                Err(err) => {
                    log::warn!("Sunrise: ALSA PCM discovery failed on card {index}: {err}");
                    record_input_error(&mut inventory.discovery_error, map_error(err));
                    break;
                }
            };
            if let Some(wanted) = hardware {
                if device > wanted.device {
                    break;
                }
                if device != wanted.device {
                    continue;
                }
            }
            // Output-only devices have no capture PCM. A failed control query can leave
            // the number of capture matches unknown, unlike a probe of a known capture PCM.
            match ctl.query_capture(device).map_err(map_error) {
                Ok(()) => inventory.devices.push(AlsaDevice::probe(
                    format!("plughw:CARD={card_id},DEV={device}"),
                    Some(index),
                    label.clone(),
                    &mut open_pcm,
                )),
                Err(MicRefusal::NoDevice) => {}
                Err(refusal) => record_input_error(&mut inventory.discovery_error, refusal),
            }
            if hardware.is_some() {
                break;
            }
        }
        if hardware.is_some() {
            break;
        }
    }
    inventory
}

pub(super) struct AlsaStream {
    pcm: pcm::PCM,
    fds: Vec<alsa::poll::pollfd>,
    samples: Vec<i16>,
    channels: usize,
    converter: InputConverter,
    ring: Arc<CaptureRing>,
    events: Arc<StreamEvents>,
}

impl CaptureBackend for AlsaBackend {
    type Device = AlsaDevice;
    type Stream = AlsaStream;

    fn devices(
        &self,
        target: Option<&MicTarget>,
        listed_label_ids: &[String],
    ) -> Result<DeviceInventory<Self::Device>, MicRefusal> {
        Ok(discover_devices(
            alsa::card::Iter::new(),
            target,
            listed_label_ids,
            CaptureControl::open,
            open_pcm,
        ))
    }

    fn default_input_device(&self) -> Result<Option<Self::Device>, MicRefusal> {
        Ok(Some(AlsaDevice {
            pcm: DEFAULT_PCM.to_owned(),
            card_index: None,
            label: DEFAULT_PCM.to_owned(),
            capture: Ok(()),
        }))
    }

    fn open_stream(
        &self,
        device: &Self::Device,
        _probed: Option<()>,
        spec: &MicSpec,
        ring: Arc<CaptureRing>,
        events: Arc<StreamEvents>,
    ) -> Result<Self::Stream, MicRefusal> {
        let pcm = device.open(open_pcm)?;
        let (channels, rate, period) = (|| {
            let params = pcm::HwParams::any(&pcm)?;
            params.set_access(pcm::Access::RWInterleaved)?;
            params.set_format(pcm::Format::s16())?;
            let channels = params.set_channels_near(u32::from(spec.channels))?;
            let rate = params.set_rate_near(spec.rate, Nearest)?;
            // Match the default period and buffer times of the CPAL ALSA path.
            params.set_period_time_near(25_000, Nearest)?;
            params.set_buffer_time_near(100_000, Nearest)?;
            pcm.hw_params(&params)?;
            Ok((channels, rate, params.get_period_size()?))
        })()
        .map_err(map_error)?;
        let channels = u16::try_from(channels).map_err(|_| MicRefusal::DeviceFailed)?;
        let sample_count = usize::try_from(period)
            .ok()
            .and_then(|period| period.checked_mul(usize::from(channels)))
            .filter(|&n| n > 0 && rate > 0)
            .ok_or(MicRefusal::DeviceFailed)?;
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(sample_count)
            .map_err(|_| MicRefusal::DeviceFailed)?;
        samples.resize(sample_count, 0);
        let mut fds = pcm.get().map_err(map_error)?;
        if fds.is_empty() {
            return Err(MicRefusal::DeviceFailed);
        }
        fds.try_reserve(1).map_err(|_| MicRefusal::DeviceFailed)?;
        pcm.prepare().map_err(map_error)?;
        pcm.start().map_err(map_error)?;
        Ok(AlsaStream {
            pcm,
            fds,
            samples,
            channels: usize::from(channels),
            converter: InputConverter::new(channels, rate, spec.channels, spec.rate),
            ring,
            events,
        })
    }

    fn run_stream(&self, mut stream: Self::Stream, stop: stop::Receiver) {
        let stop_index = stream.fds.len();
        stream.fds.push(stop.pollfd());
        let result = (|| {
            let io = stream.pcm.io_i16()?;
            loop {
                match alsa::poll::poll(&mut stream.fds, -1) {
                    Ok(_) => {}
                    Err(err) if err.errno() == libc::EINTR => continue,
                    Err(err) => return Err(err),
                }
                // The socket is closed for writing on stop. Prioritize it over ready audio.
                if stream.fds[stop_index].revents != 0 {
                    return Ok(());
                }
                if stream.fds[..stop_index]
                    .iter()
                    .any(|fd| fd.revents & (libc::POLLHUP | libc::POLLNVAL) != 0)
                {
                    return Err(alsa::Error::new("capture poll", libc::ENODEV));
                }
                let readiness = stream.pcm.revents(&stream.fds[..stop_index])?;
                if readiness.intersects(Flags::HUP | Flags::NVAL) {
                    return Err(alsa::Error::new("capture poll", libc::ENODEV));
                }
                if !readiness.intersects(Flags::IN | Flags::ERR) {
                    continue;
                }
                match io.readi(&mut stream.samples) {
                    Ok(frames) => stream.converter.push_i16_samples(
                        &stream.samples[..frames * stream.channels],
                        &stream.ring,
                    ),
                    Err(err) if err.errno() == libc::EAGAIN => {}
                    // An ALSA capture overrun requires preparing and restarting the same PCM.
                    // Like CPAL, keep the stream open across an xrun, not a real device error.
                    Err(err) if err.errno() == libc::EPIPE => {
                        stream.pcm.prepare()?;
                        stream.pcm.start()?;
                    }
                    Err(err) => return Err(err),
                }
            }
        })();
        if let Err(err) = result {
            stream.events.report_error(map_error(err));
            log::warn!("Sunrise: ALSA microphone stream error: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::MicPoolInner;
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    struct Discovered<F> {
        discover: F,
        opened: Mutex<Vec<String>>,
    }

    impl<F> Discovered<F>
    where
        F: Fn(Option<&MicTarget>, &[String]) -> DeviceInventory<AlsaDevice> + Send + Sync + 'static,
    {
        fn new(discover: F) -> Self {
            Self {
                discover,
                opened: Mutex::new(Vec::new()),
            }
        }
    }

    fn probe_endpoints<T>(
        endpoints: Vec<(String, String)>,
        mut open: impl FnMut(&str) -> alsa::Result<T>,
    ) -> Vec<AlsaDevice> {
        endpoints
            .into_iter()
            .enumerate()
            .map(|(index, (pcm, label))| {
                AlsaDevice::probe(pcm, Some(index as i32), label, &mut open)
            })
            .collect()
    }

    struct FakeControl<const N: usize> {
        identity: alsa::Result<(&'static str, &'static str)>,
        devices: [alsa::Result<u32>; N],
        capture_error: Option<(u32, alsa::Error)>,
    }

    impl<const N: usize> FakeControl<N> {
        fn new(
            identity: alsa::Result<(&'static str, &'static str)>,
            devices: [alsa::Result<u32>; N],
        ) -> Self {
            Self {
                identity,
                devices,
                capture_error: None,
            }
        }
    }

    impl<const N: usize> CardControl for FakeControl<N> {
        fn card_info(&self) -> alsa::Result<(String, String)> {
            self.identity
                .map(|(id, name)| (id.to_owned(), name.to_owned()))
        }

        fn pcm_devices(&self) -> impl Iterator<Item = alsa::Result<u32>> + '_ {
            self.devices.iter().copied()
        }

        fn query_capture(&self, device: u32) -> alsa::Result<()> {
            match self.capture_error {
                Some((index, error)) if index == device => Err(error),
                _ => Ok(()),
            }
        }
    }

    impl<F> CaptureBackend for Discovered<F>
    where
        F: Fn(Option<&MicTarget>, &[String]) -> DeviceInventory<AlsaDevice> + Send + Sync + 'static,
    {
        type Device = AlsaDevice;
        type Stream = ();

        fn devices(
            &self,
            target: Option<&MicTarget>,
            listed_label_ids: &[String],
        ) -> Result<DeviceInventory<Self::Device>, MicRefusal> {
            Ok((self.discover)(target, listed_label_ids))
        }

        fn default_input_device(&self) -> Result<Option<Self::Device>, MicRefusal> {
            Ok(None)
        }

        fn open_stream(
            &self,
            device: &Self::Device,
            _probed: Option<()>,
            _spec: &MicSpec,
            _ring: Arc<CaptureRing>,
            _events: Arc<StreamEvents>,
        ) -> Result<(), MicRefusal> {
            device.open(|pcm| {
                self.opened.lock().unwrap().push(pcm.to_owned());
                Ok(())
            })
        }
    }

    #[test]
    fn a_denied_pcm_does_not_hide_a_working_input_but_still_refuses_open() {
        let denied = "plughw:CARD=Denied,DEV=0";
        let available = "plughw:CARD=Available,DEV=0";
        let devices = probe_endpoints(
            vec![
                (denied.into(), "Denied Mic".into()),
                (available.into(), "Desk Mic".into()),
            ],
            |name| {
                if name == denied {
                    Err(alsa::Error::new("snd_pcm_open", libc::EACCES))
                } else {
                    Ok(())
                }
            },
        );
        let devices = Arc::new(Mutex::new(devices));
        let backend = Arc::new(Discovered::new({
            let devices = Arc::clone(&devices);
            move |_target: Option<&MicTarget>, _listed_label_ids: &[String]| DeviceInventory {
                devices: devices.lock().unwrap().clone(),
                discovery_error: None,
            }
        }));
        let pool = MicPoolInner::new(true, Arc::clone(&backend));
        assert_eq!(
            pool.devices().unwrap(),
            [mod_api::MicDevice::new(
                "alsa:1:plughw:CARD=Available,DEV=0".into(),
                "Desk Mic".into()
            )]
        );
        let spec = MicSpec::new(1, 48_000);
        for target in [
            spec.clone().with_label("Denied Mic"),
            spec.clone().with_device("alsa:0:plughw:CARD=Denied,DEV=0"),
        ] {
            assert_eq!(pool.open(&target).err(), Some(MicRefusal::OsDenied));
        }
        let mic = pool.open(&spec.with_label("Desk Mic")).unwrap();
        assert_eq!(*backend.opened.lock().unwrap(), [available]);
        mic.close();

        devices.lock().unwrap().retain(|dev| dev.pcm == denied);
        assert_eq!(pool.devices(), Err(MicRefusal::OsDenied));
    }

    #[test]
    fn denied_card_control_does_not_block_independent_routes_or_later_cards() {
        let working_inputs = Arc::new(AtomicBool::new(true));
        let backend = Arc::new(Discovered::new({
            let working_inputs = Arc::clone(&working_inputs);
            move |target, listed_label_ids| {
                let working = working_inputs.load(Ordering::Relaxed);
                discover_devices(
                    (0..if working { 2 } else { 1 }).map(|index| Ok(alsa::Card::new(index))),
                    target,
                    listed_label_ids,
                    |card| {
                        if card.get_index() == 0 {
                            return Err(alsa::Error::new("snd_ctl_open", libc::EACCES));
                        }
                        Ok(FakeControl::new(Ok(("Desk", "Desk Mic")), [Ok(0)]))
                    },
                    |name| {
                        if working && matches!(name, "pipewire" | "plughw:CARD=Desk,DEV=0") {
                            Ok(())
                        } else {
                            Err(alsa::Error::new("snd_pcm_open", libc::ENOENT))
                        }
                    },
                )
            }
        }));
        let pool = MicPoolInner::new(true, Arc::clone(&backend));
        assert_eq!(
            pool.devices().unwrap(),
            [
                mod_api::MicDevice::new("alsa:pipewire".into(), "pipewire".into()),
                mod_api::MicDevice::new("alsa:1:plughw:CARD=Desk,DEV=0".into(), "Desk Mic".into())
            ]
        );
        working_inputs.store(false, Ordering::Relaxed);
        assert_eq!(pool.devices(), Err(MicRefusal::OsDenied));
    }

    #[test]
    fn reopening_listed_input_reports_its_own_card_control_denial_not_another_cards() {
        #[derive(Clone, Copy)]
        enum CardState {
            Available,
            OpenDenied,
            InfoDenied,
            Removed,
            RemovedDuringLookup,
            Replaced,
        }
        let card_state = Arc::new(Mutex::new(CardState::Available));
        let backend = Arc::new(Discovered::new({
            let card_state = Arc::clone(&card_state);
            move |target, listed_label_ids| {
                let state = *card_state.lock().unwrap();
                let first_index = if matches!(state, CardState::Removed) {
                    1
                } else {
                    0
                };
                discover_devices(
                    (first_index..3).map(|index| Ok(alsa::Card::new(index))),
                    target,
                    listed_label_ids,
                    |card| match card.get_index() {
                        0 => match state {
                            CardState::OpenDenied => {
                                Err(alsa::Error::new("snd_ctl_open", libc::EACCES))
                            }
                            CardState::InfoDenied => Ok(FakeControl::new(
                                Err(alsa::Error::new("snd_ctl_card_info", libc::EACCES)),
                                [Ok(0)],
                            )),
                            CardState::RemovedDuringLookup => {
                                Err(alsa::Error::new("snd_ctl_open", libc::ENODEV))
                            }
                            CardState::Replaced => {
                                Ok(FakeControl::new(Ok(("Replacement", "Desk Mic")), [Ok(0)]))
                            }
                            _ => Ok(FakeControl::new(Ok(("Desk", "Desk Mic")), [Ok(0)])),
                        },
                        1 => Ok(FakeControl::new(Ok(("Headset", "Headset Mic")), [Ok(0)])),
                        _ => Err(alsa::Error::new("snd_ctl_open", libc::EACCES)),
                    },
                    |pcm| {
                        if pcm.starts_with("plughw:") {
                            Ok(())
                        } else {
                            Err(alsa::Error::new("snd_pcm_open", libc::ENOENT))
                        }
                    },
                )
            }
        }));
        let pool = MicPoolInner::new(true, Arc::clone(&backend));
        let listed = pool.devices().unwrap();
        assert_eq!(listed[0].label, "Desk Mic");
        let spec = MicSpec::new(1, 48_000);
        let desk_id = spec.clone().with_device(&listed[0].id);
        let desk_label = spec.clone().with_label(&listed[0].label);
        let headset_label = spec.clone().with_label(&listed[1].label);
        let headset_id = spec.with_device(&listed[1].id);
        for (state, expected) in [
            (CardState::OpenDenied, MicRefusal::OsDenied),
            (CardState::InfoDenied, MicRefusal::OsDenied),
            (CardState::Removed, MicRefusal::NoDevice),
            (CardState::RemovedDuringLookup, MicRefusal::NoDevice),
        ] {
            *card_state.lock().unwrap() = state;
            assert_eq!(pool.open(&desk_id).err(), Some(expected));
            assert_eq!(pool.open(&desk_label).err(), Some(expected));
        }
        *card_state.lock().unwrap() = CardState::Replaced;
        assert_eq!(pool.open(&desk_id).err(), Some(MicRefusal::NoDevice));
        assert!(backend.opened.lock().unwrap().is_empty());
        // The formerly denied label can select a present match; the ID may not retarget.
        pool.open(&desk_label).unwrap().close();
        *card_state.lock().unwrap() = CardState::OpenDenied;
        pool.open(&headset_label).unwrap().close();
        pool.open(&headset_id).unwrap().close();
        assert_eq!(
            *backend.opened.lock().unwrap(),
            [
                "plughw:CARD=Replacement,DEV=0",
                "plughw:CARD=Headset,DEV=0",
                "plughw:CARD=Headset,DEV=0",
            ]
        );
    }

    #[test]
    fn pcm_device_walk_error_is_not_exhaustion_and_keeps_an_independent_card() {
        let other_card = Arc::new(AtomicBool::new(false));
        let backend = Arc::new(Discovered::new({
            let other_card = Arc::clone(&other_card);
            move |target, listed_label_ids| {
                let end = if other_card.load(Ordering::Relaxed) {
                    2
                } else {
                    1
                };
                discover_devices(
                    (0..end).map(|index| Ok(alsa::Card::new(index))),
                    target,
                    listed_label_ids,
                    |card| {
                        if card.get_index() == 0 {
                            Ok(FakeControl::new(
                                Ok(("Desk", "Desk Mic")),
                                [Err(alsa::Error::new("snd_ctl_pcm_next_device", libc::EIO))],
                            ))
                        } else {
                            Ok(FakeControl::new(Ok(("Headset", "Headset Mic")), [Ok(2)]))
                        }
                    },
                    |pcm| {
                        if pcm == "plughw:CARD=Headset,DEV=2" {
                            Ok(())
                        } else {
                            Err(alsa::Error::new("snd_pcm_open", libc::ENOENT))
                        }
                    },
                )
            }
        }));
        let pool = MicPoolInner::new(true, Arc::clone(&backend));
        assert_eq!(pool.devices(), Err(MicRefusal::DeviceFailed));
        other_card.store(true, Ordering::Relaxed);
        let listed = pool.devices().unwrap();
        assert_eq!(
            listed,
            [mod_api::MicDevice::new(
                "alsa:1:plughw:CARD=Headset,DEV=2".into(),
                "Headset Mic".into()
            )]
        );
        let spec = MicSpec::new(1, 48_000);
        assert_eq!(
            pool.open(&spec.clone().with_label("Desk Mic")).err(),
            Some(MicRefusal::DeviceFailed)
        );
        pool.open(&spec.with_device(&listed[0].id)).unwrap().close();
        assert_eq!(
            *backend.opened.lock().unwrap(),
            ["plughw:CARD=Headset,DEV=2"]
        );
    }

    #[test]
    fn incomplete_pcm_discovery_does_not_open_a_partial_label_match() {
        #[derive(Clone, Copy)]
        enum Fault {
            Walk(i32),
            CaptureQuery,
        }
        for (fault, second_card, expected) in [
            (Fault::Walk(libc::EIO), false, MicRefusal::DeviceFailed),
            (Fault::Walk(libc::EACCES), false, MicRefusal::OsDenied),
            (Fault::Walk(libc::EIO), true, MicRefusal::DeviceFailed),
            (Fault::CaptureQuery, false, MicRefusal::DeviceFailed),
        ] {
            let backend = Arc::new(Discovered::new(move |target, listed_label_ids| {
                discover_devices(
                    (0..if second_card { 2 } else { 1 }).map(|index| Ok(alsa::Card::new(index))),
                    target,
                    listed_label_ids,
                    |card| {
                        if card.get_index() != 0 {
                            return Ok(FakeControl::new(
                                Ok(("OtherDesk", "Desk Mic")),
                                [
                                    Err(alsa::Error::new("snd_ctl_pcm_next_device", libc::EIO)),
                                    Ok(0),
                                ],
                            ));
                        }
                        let mut ctl = FakeControl::new(
                            Ok(("Desk", "Desk Mic")),
                            [
                                Ok(0),
                                match fault {
                                    Fault::Walk(errno) if !second_card => {
                                        Err(alsa::Error::new("snd_ctl_pcm_next_device", errno))
                                    }
                                    _ => Ok(1),
                                },
                            ],
                        );
                        if matches!(fault, Fault::CaptureQuery) {
                            ctl.capture_error =
                                Some((1, alsa::Error::new("snd_ctl_pcm_info", libc::EIO)));
                        }
                        Ok(ctl)
                    },
                    |pcm| {
                        if pcm == "plughw:CARD=Desk,DEV=0" {
                            Ok(())
                        } else {
                            Err(alsa::Error::new("snd_pcm_open", libc::ENODEV))
                        }
                    },
                )
            }));
            let pool = MicPoolInner::new(true, Arc::clone(&backend));
            assert_eq!(
                pool.devices().unwrap(),
                [mod_api::MicDevice::new(
                    "alsa:0:plughw:CARD=Desk,DEV=0".into(),
                    "Desk Mic".into(),
                )]
            );
            let spec = MicSpec::new(1, 48_000);
            assert_eq!(
                pool.open(&spec.clone().with_label("Desk Mic")).err(),
                Some(expected)
            );
            assert!(backend.opened.lock().unwrap().is_empty());
            pool.open(&spec.with_device("alsa:0:plughw:CARD=Desk,DEV=0"))
                .unwrap()
                .close();
            assert_eq!(*backend.opened.lock().unwrap(), ["plughw:CARD=Desk,DEV=0"]);
        }
    }

    #[test]
    fn a_busy_alsa_card_keeps_its_selector_instead_of_retargeting_a_same_label_input() {
        let first = "plughw:CARD=Headset,DEV=0";
        let second = "plughw:CARD=Headset_1,DEV=0";
        let endpoints = vec![
            (first.into(), "Headset".into()),
            (second.into(), "Headset".into()),
        ];
        let devices = Arc::new(Mutex::new(probe_endpoints(endpoints.clone(), |_| Ok(()))));
        let backend = Arc::new(Discovered::new({
            let devices = Arc::clone(&devices);
            move |_target: Option<&MicTarget>, _listed_label_ids: &[String]| DeviceInventory {
                devices: devices.lock().unwrap().clone(),
                discovery_error: None,
            }
        }));
        let pool = MicPoolInner::new(true, Arc::clone(&backend));
        let initial = pool.devices().unwrap();
        assert_eq!(initial[0].id, "alsa:0:plughw:CARD=Headset,DEV=0");
        assert_eq!(initial[1].id, "alsa:1:plughw:CARD=Headset_1,DEV=0");

        // Same control inventory, but the first capture probe now fails. Only the second can be
        // listed; it must not inherit the first endpoint's old selector.
        *devices.lock().unwrap() = probe_endpoints(endpoints, |name| {
            if name == first {
                Err(alsa::Error::new("snd_pcm_open", libc::EBUSY))
            } else {
                Ok(())
            }
        });
        assert_eq!(pool.devices().unwrap().as_slice(), &initial[1..]);
        let spec = MicSpec::new(1, 48_000).with_device(&initial[0].id);
        assert_eq!(pool.open(&spec).err(), Some(MicRefusal::Busy));
        assert!(backend.opened.lock().unwrap().is_empty());

        // Removal must not reinterpret that same selector as the other device either.
        devices.lock().unwrap().retain(|dev| dev.pcm != first);
        assert_eq!(pool.open(&spec).err(), Some(MicRefusal::NoDevice));
        let mic = pool
            .open(&MicSpec::new(1, 48_000).with_device(&initial[1].id))
            .unwrap();
        assert_eq!(*backend.opened.lock().unwrap(), [second]);
        mic.close();
    }
}
