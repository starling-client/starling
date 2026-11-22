//! CPAL microphone capture on platforms with independently enumerable input endpoints.

use super::*;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

#[cfg(target_os = "macos")]
#[path = "macos_permission.rs"]
mod macos_permission;

pub(super) struct CpalBackend;

#[derive(Clone)]
pub(super) struct CpalDevice {
    device: cpal::Device,
    label: Option<String>,
    #[cfg(target_os = "macos")]
    coreaudio_id: Option<u32>,
    #[cfg(not(target_os = "macos"))]
    selector: Option<String>,
}

fn pcm_format_from_cpal(format: cpal::SampleFormat) -> Option<PcmFormat> {
    use cpal::SampleFormat as F;
    Some(match format {
        F::I8 => PcmFormat::I8,
        F::I16 => PcmFormat::I16,
        F::I24 => PcmFormat::I24,
        F::I32 => PcmFormat::I32,
        F::I64 => PcmFormat::I64,
        F::U8 => PcmFormat::U8,
        F::U16 => PcmFormat::U16,
        F::U32 => PcmFormat::U32,
        F::U64 => PcmFormat::U64,
        F::F32 => PcmFormat::F32,
        F::F64 => PcmFormat::F64,
        _ => return None,
    })
}

fn map_devices_error(err: cpal::DevicesError) -> mod_api::MicRefusal {
    match err {
        cpal::DevicesError::BackendSpecific { err } => classify_error_message(&err.description),
    }
}

fn map_default_config_error(err: cpal::DefaultStreamConfigError) -> mod_api::MicRefusal {
    match err {
        cpal::DefaultStreamConfigError::DeviceNotAvailable => mod_api::MicRefusal::NoDevice,
        cpal::DefaultStreamConfigError::BackendSpecific { err } => {
            classify_error_message(&err.description)
        }
        cpal::DefaultStreamConfigError::StreamTypeNotSupported => mod_api::MicRefusal::DeviceFailed,
    }
}

fn map_build_stream_error(err: cpal::BuildStreamError) -> mod_api::MicRefusal {
    match err {
        cpal::BuildStreamError::DeviceNotAvailable => mod_api::MicRefusal::NoDevice,
        cpal::BuildStreamError::BackendSpecific { err } => classify_error_message(&err.description),
        cpal::BuildStreamError::StreamConfigNotSupported
        | cpal::BuildStreamError::InvalidArgument
        | cpal::BuildStreamError::StreamIdOverflow => mod_api::MicRefusal::DeviceFailed,
    }
}

fn map_play_stream_error(err: cpal::PlayStreamError) -> mod_api::MicRefusal {
    match err {
        cpal::PlayStreamError::DeviceNotAvailable => mod_api::MicRefusal::NoDevice,
        cpal::PlayStreamError::BackendSpecific { err } => classify_error_message(&err.description),
    }
}

impl CpalDevice {
    fn from_device(device: cpal::Device) -> Self {
        let label = device.name().ok().filter(|name| !name.is_empty());
        #[cfg(target_os = "macos")]
        let coreaudio_id = {
            let cpal::platform::DeviceInner::CoreAudio(dev) = device.as_inner();
            parse_coreaudio_device_id(&format!("{dev:?}"))
        };
        #[cfg(windows)]
        let selector = None;
        #[cfg(not(any(target_os = "macos", windows)))]
        let selector = label.clone();
        Self {
            device,
            label,
            #[cfg(target_os = "macos")]
            coreaudio_id,
            #[cfg(not(target_os = "macos"))]
            selector,
        }
    }

    fn default_input_config(&self) -> Result<InputConfig, mod_api::MicRefusal> {
        let cfg = self
            .device
            .default_input_config()
            .map_err(map_default_config_error)?;
        let format =
            pcm_format_from_cpal(cfg.sample_format()).ok_or(mod_api::MicRefusal::DeviceFailed)?;
        Ok(InputConfig {
            channels: cfg.channels(),
            rate: cfg.sample_rate().0,
            format,
        })
    }
}

impl BackendDevice for CpalDevice {
    type InputConfigs = Vec<InputConfigRange>;

    fn label(&self) -> Option<String> {
        self.label.clone()
    }

    fn stable_selector(&self) -> Option<String> {
        #[cfg(target_os = "macos")]
        {
            self.coreaudio_id.map(|id| format!("coreaudio:{id}"))
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.selector.clone()
        }
    }

    fn same_endpoint(&self, other: &Self) -> bool {
        #[cfg(target_os = "macos")]
        {
            if let (Some(a), Some(b)) = (self.coreaudio_id, other.coreaudio_id) {
                return a == b;
            }
            let (
                cpal::platform::DeviceInner::CoreAudio(a),
                cpal::platform::DeviceInner::CoreAudio(b),
            ) = (self.device.as_inner(), other.device.as_inner());
            a == b
        }
        #[cfg(windows)]
        {
            let (cpal::platform::DeviceInner::Wasapi(a), cpal::platform::DeviceInner::Wasapi(b)) =
                (self.device.as_inner(), other.device.as_inner());
            a == b
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            self.selector.is_some() && self.selector == other.selector
        }
    }

    #[cfg(target_os = "macos")]
    fn has_input_metadata(&self) -> Result<bool, mod_api::MicRefusal> {
        use mod_api::MicRefusal;
        use sunrise_audio_input::macos::{InputMetadataError, has_input_streams};

        let id = self.coreaudio_id.ok_or(MicRefusal::DeviceFailed)?;
        has_input_streams(id).map_err(|err| match err {
            InputMetadataError::Denied => MicRefusal::OsDenied,
            InputMetadataError::QueryFailed(status) => {
                log::warn!("Sunrise: CoreAudio input discovery failed for device {id}: {status}");
                MicRefusal::DeviceFailed
            }
        })
    }

    fn input_configs(&self) -> Result<Option<Self::InputConfigs>, mod_api::MicRefusal> {
        match self.device.supported_input_configs() {
            Ok(configs) => {
                let ranges: Vec<InputConfigRange> = configs
                    .filter_map(|r| {
                        Some(InputConfigRange {
                            channels: r.channels(),
                            min_rate: r.min_sample_rate().0,
                            max_rate: r.max_sample_rate().0,
                            format: pcm_format_from_cpal(r.sample_format())?,
                        })
                    })
                    .collect();
                Ok((!ranges.is_empty()).then_some(ranges))
            }
            Err(cpal::SupportedStreamConfigsError::DeviceNotAvailable)
            | Err(cpal::SupportedStreamConfigsError::InvalidArgument) => {
                Err(mod_api::MicRefusal::NoDevice)
            }
            Err(cpal::SupportedStreamConfigsError::BackendSpecific { err }) => {
                Err(classify_error_message(&err.description))
            }
        }
    }
}

impl CaptureBackend for CpalBackend {
    type Device = CpalDevice;
    type Stream = cpal::Stream;

    fn devices(
        &self,
        _target: Option<&mod_api::MicTarget>,
        _listed_label_ids: &[String],
    ) -> Result<DeviceInventory<Self::Device>, mod_api::MicRefusal> {
        Ok(DeviceInventory {
            devices: cpal::default_host()
                .devices()
                .map_err(map_devices_error)?
                .map(CpalDevice::from_device)
                .collect(),
            discovery_error: None,
        })
    }

    fn default_input_device(&self) -> Result<Option<Self::Device>, mod_api::MicRefusal> {
        let Some(device) = cpal::default_host().default_input_device() else {
            return Ok(None);
        };
        let device = CpalDevice::from_device(device);
        #[cfg(target_os = "macos")]
        {
            // Only enumerated CPAL devices register a removal listener. Open the same
            // CoreAudio endpoint via that route even when the OS selected it by default.
            Ok(enumerated_default(
                &device,
                self.devices(None, &[])?.devices,
            ))
        }
        #[cfg(not(target_os = "macos"))]
        Ok(Some(device))
    }

    #[cfg(target_os = "macos")]
    fn with_input_authorization(
        &self,
        open: impl FnOnce() -> Result<Self::Stream, mod_api::MicRefusal>,
    ) -> Result<Self::Stream, mod_api::MicRefusal> {
        macos_permission::open(open)
    }

    fn open_stream(
        &self,
        device: &Self::Device,
        probed: Option<Vec<InputConfigRange>>,
        spec: &mod_api::MicSpec,
        ring: Arc<CaptureRing>,
        events: Arc<StreamEvents>,
    ) -> Result<Self::Stream, mod_api::MicRefusal> {
        let config = if cfg!(target_os = "macos") {
            // Changing the nominal rate on CoreAudio mutates the hardware clock for
            // all streams. Keep its current rate and resample through InputConverter.
            device.default_input_config()?
        } else {
            let configs = match probed {
                Some(configs) => configs,
                None => device
                    .input_configs()?
                    .ok_or(mod_api::MicRefusal::NoDevice)?,
            };
            choose_best_config(&configs, spec).ok_or(mod_api::MicRefusal::DeviceFailed)?
        };
        let stream_config = cpal::StreamConfig {
            channels: config.channels,
            sample_rate: cpal::SampleRate(config.rate),
            buffer_size: cpal::BufferSize::Default,
        };
        let device = &device.device;
        let stream = match config.format {
            PcmFormat::I16 => build_i16_stream(device, &stream_config, spec, ring, events),
            PcmFormat::I8 => build_typed_stream::<i8>(device, &stream_config, spec, ring, events),
            PcmFormat::I24 => {
                build_typed_stream::<cpal::I24>(device, &stream_config, spec, ring, events)
            }
            PcmFormat::I32 => build_typed_stream::<i32>(device, &stream_config, spec, ring, events),
            PcmFormat::I64 => build_typed_stream::<i64>(device, &stream_config, spec, ring, events),
            PcmFormat::U8 => build_typed_stream::<u8>(device, &stream_config, spec, ring, events),
            PcmFormat::U16 => build_typed_stream::<u16>(device, &stream_config, spec, ring, events),
            PcmFormat::U32 => build_typed_stream::<u32>(device, &stream_config, spec, ring, events),
            PcmFormat::U64 => build_typed_stream::<u64>(device, &stream_config, spec, ring, events),
            PcmFormat::F32 => build_typed_stream::<f32>(device, &stream_config, spec, ring, events),
            PcmFormat::F64 => build_typed_stream::<f64>(device, &stream_config, spec, ring, events),
        }
        .map_err(map_build_stream_error)?;
        stream.play().map_err(map_play_stream_error)?;
        Ok(stream)
    }
}

fn build_i16_stream(
    device: &cpal::Device,
    stream_config: &cpal::StreamConfig,
    spec: &mod_api::MicSpec,
    ring: Arc<CaptureRing>,
    events: Arc<StreamEvents>,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let mut converter = InputConverter::new(
        stream_config.channels,
        stream_config.sample_rate.0,
        spec.channels,
        spec.rate,
    );
    let input_events = Arc::clone(&events);
    device.build_input_stream(
        stream_config,
        move |data: &[i16], _info: &cpal::InputCallbackInfo| {
            if !input_events.device_open.load(Ordering::Relaxed) {
                return;
            }
            converter.push_i16_samples(data, &ring);
        },
        move |e| report_stream_error(&events, e),
        None,
    )
}

fn build_typed_stream<T>(
    device: &cpal::Device,
    stream_config: &cpal::StreamConfig,
    spec: &mod_api::MicSpec,
    ring: Arc<CaptureRing>,
    events: Arc<StreamEvents>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let mut converter = InputConverter::new(
        stream_config.channels,
        stream_config.sample_rate.0,
        spec.channels,
        spec.rate,
    );
    let input_events = Arc::clone(&events);
    device.build_input_stream(
        stream_config,
        move |data: &[T], _info: &cpal::InputCallbackInfo| {
            if !input_events.device_open.load(Ordering::Relaxed) {
                return;
            }
            converter.push_cpal_samples(data, &ring);
        },
        move |e| report_stream_error(&events, e),
        None,
    )
}

fn report_stream_error(events: &StreamEvents, err: cpal::StreamError) {
    let refusal = match &err {
        cpal::StreamError::DeviceNotAvailable => mod_api::MicRefusal::NoDevice,
        cpal::StreamError::BackendSpecific { err } => classify_error_message(&err.description),
    };
    events.report_error(refusal);
    log::warn!("Sunrise: microphone stream error: {err}");
}
