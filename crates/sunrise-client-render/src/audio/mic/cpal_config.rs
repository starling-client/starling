//! CPAL input configurations and error classification.

/// Classifies an OS audio backend error message into a [`mod_api::MicRefusal`].
pub(super) fn classify_error_message(msg: &str) -> mod_api::MicRefusal {
    const DENIED_PATTERNS: &[&str] = &[
        "permission denied",
        "access denied",
        "access is denied",
        "not authorized",
        "eperm",
        "eacces",
        "denied",
        "unauthorized",
        "e_accessdenied",
        "0x80070005",
    ];
    const BUSY_PATTERNS: &[&str] = &[
        "device or resource busy",
        "ebusy",
        "in use",
        "exclusive",
        "busy",
        "audclnt_e_device_in_use",
        "0x8889000a",
    ];
    let lower = msg.to_ascii_lowercase();
    if DENIED_PATTERNS.iter().any(|p| lower.contains(p)) {
        return mod_api::MicRefusal::OsDenied;
    }
    if BUSY_PATTERNS.iter().any(|p| lower.contains(p)) {
        return mod_api::MicRefusal::Busy;
    }
    mod_api::MicRefusal::DeviceFailed
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PcmFormat {
    I8,
    I16,
    I24,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
}

impl PcmFormat {
    pub(super) fn rank(self) -> u8 {
        match self {
            Self::I16 => 3,
            Self::F32 => 2,
            Self::I24 | Self::I32 | Self::F64 => 1,
            Self::I8 | Self::I64 | Self::U8 | Self::U16 | Self::U32 | Self::U64 => 0,
        }
    }
}

/// `pub`, not `pub(super)`: it is `CpalDevice`'s `BackendDevice::InputConfigs`, and SDK staging
/// widens that trait while leaving this platform-only module's items as written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputConfigRange {
    pub(super) channels: u16,
    pub(super) min_rate: u32,
    pub(super) max_rate: u32,
    pub(super) format: PcmFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct InputConfig {
    pub(super) channels: u16,
    pub(super) rate: u32,
    pub(super) format: PcmFormat,
}

pub(super) fn choose_best_config(
    configs: &[InputConfigRange],
    spec: &mod_api::MicSpec,
) -> Option<InputConfig> {
    let target_channels = u16::from(spec.channels);
    let mut best: Option<((bool, bool, bool, u8, u32, u16), InputConfig)> = None;

    for range in configs {
        if range.channels == 0 || range.min_rate == 0 || range.max_rate < range.min_rate {
            continue;
        }
        let rate = spec.rate.clamp(range.min_rate, range.max_rate);
        let exact_rate = rate == spec.rate;
        let exact_channels = range.channels == target_channels;
        let enough_channels = range.channels >= target_channels;
        let format_rank = range.format.rank();
        let rate_closeness = u32::MAX - rate.abs_diff(spec.rate);
        let channel_closeness = u16::MAX - range.channels.abs_diff(target_channels);
        let key = (
            exact_rate,
            exact_channels,
            enough_channels,
            format_rank,
            rate_closeness,
            channel_closeness,
        );
        let cfg = InputConfig {
            channels: range.channels,
            rate,
            format: range.format,
        };
        if best.as_ref().is_none_or(|(best_key, _)| key > *best_key) {
            best = Some((key, cfg));
        }
    }
    best.map(|(_, cfg)| cfg)
}
