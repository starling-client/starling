//! macOS's app-wide microphone authorization. CPAL 0.16 erases the native
//! `kAudioUnitErr_Unauthorized` status when building an input stream.

use mod_api::MicRefusal;
use sunrise_audio_input::macos::{
    MicrophoneAuthorization as Status, microphone_authorization, request_microphone_access,
};

pub(super) fn open<T>(start: impl FnOnce() -> Result<T, MicRefusal>) -> Result<T, MicRefusal> {
    with_authorization(
        microphone_authorization,
        || request_microphone_access().ok_or(MicRefusal::DeviceFailed),
        start,
    )
}

fn with_authorization<T>(
    status: impl Fn() -> Option<Status>,
    request: impl FnOnce() -> Result<bool, MicRefusal>,
    start: impl FnOnce() -> Result<T, MicRefusal>,
) -> Result<T, MicRefusal> {
    match status().ok_or(MicRefusal::DeviceFailed)? {
        Status::Authorized => {}
        Status::NotDetermined => {
            if !request()? {
                return Err(MicRefusal::OsDenied);
            }
        }
        Status::Denied | Status::Restricted => return Err(MicRefusal::OsDenied),
        _ => return Err(MicRefusal::DeviceFailed),
    }
    let result = start();
    // Permission may change during startup. In particular a CPAL DeviceNotAvailable
    // is ambiguous by itself: return OsDenied only if the OS reports it.
    match status() {
        Some(Status::Denied | Status::Restricted) => Err(MicRefusal::OsDenied),
        _ => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denied_or_restricted_authorization_does_not_start_capture() {
        for status in [Status::Denied, Status::Restricted] {
            assert_eq!(
                with_authorization(
                    || Some(status),
                    || panic!("must not request again"),
                    || -> Result<(), _> { panic!("must not start capture") },
                ),
                Err(MicRefusal::OsDenied)
            );
        }
    }

    #[test]
    fn a_denied_first_os_request_is_not_a_device_error() {
        assert_eq!(
            with_authorization(
                || Some(Status::NotDetermined),
                || Ok(false),
                || -> Result<(), _> { panic!("must not start capture") },
            ),
            Err(MicRefusal::OsDenied)
        );
    }

    #[test]
    fn denial_during_startup_is_not_reported_as_device_loss() {
        let status = std::cell::Cell::new(Status::Authorized);
        assert_eq!(
            with_authorization(
                || Some(status.get()),
                || panic!("must not request again"),
                || {
                    status.set(Status::Denied);
                    Err::<(), _>(MicRefusal::NoDevice)
                },
            ),
            Err(MicRefusal::OsDenied)
        );
    }

    #[test]
    fn a_missing_device_with_os_permission_remains_no_device() {
        assert_eq!(
            with_authorization(
                || Some(Status::Authorized),
                || panic!("must not request again"),
                || Err::<(), _>(MicRefusal::NoDevice),
            ),
            Err(MicRefusal::NoDevice)
        );
    }
}
