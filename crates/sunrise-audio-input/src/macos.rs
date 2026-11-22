//! CoreAudio input discovery and AVFoundation's application-wide microphone authorization.

use block2::RcBlock;
use objc2::{rc::autoreleasepool, runtime::Bool};
use objc2_av_foundation::{AVCaptureDevice, AVMediaTypeAudio};
use objc2_core_audio::{
    AudioObjectGetPropertyDataSize, AudioObjectPropertyAddress,
    kAudioDevicePermissionsError as AUDIO_DEVICE_PERMISSIONS_ERROR, kAudioDevicePropertyStreams,
    kAudioHardwareBadDeviceError as AUDIO_HARDWARE_BAD_DEVICE_ERROR,
    kAudioHardwareBadObjectError as AUDIO_HARDWARE_BAD_OBJECT_ERROR,
    kAudioHardwareNoError as AUDIO_HARDWARE_NO_ERROR,
    kAudioHardwareUnknownPropertyError as AUDIO_HARDWARE_UNKNOWN_PROPERTY_ERROR,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeInput,
};
use std::{
    ptr::{NonNull, null},
    sync::mpsc,
};

pub use objc2_av_foundation::AVAuthorizationStatus as MicrophoneAuthorization;

/// Failure to read a CoreAudio device's input metadata.
#[derive(Debug)]
pub enum InputMetadataError {
    Denied,
    QueryFailed(i32),
}

/// Reports whether a CoreAudio device has any input streams, without creating an AudioUnit or
/// requesting authorization. Output-only and unplugged devices have no input streams.
pub fn has_input_streams(device_id: u32) -> Result<bool, InputMetadataError> {
    let address = AudioObjectPropertyAddress {
        mSelector: kAudioDevicePropertyStreams,
        mScope: kAudioObjectPropertyScopeInput,
        mElement: kAudioObjectPropertyElementMain,
    };
    let mut size = 0;
    // SAFETY: Both pointers borrow live stack variables. This property requires no qualifier
    // and writes only the size of the input AudioStreamID array to `size`.
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            device_id,
            NonNull::from(&address),
            0,
            null(),
            NonNull::from(&mut size),
        )
    };
    match status {
        AUDIO_HARDWARE_NO_ERROR => Ok(size != 0),
        AUDIO_HARDWARE_UNKNOWN_PROPERTY_ERROR
        | AUDIO_HARDWARE_BAD_OBJECT_ERROR
        | AUDIO_HARDWARE_BAD_DEVICE_ERROR => Ok(false),
        AUDIO_DEVICE_PERMISSIONS_ERROR => Err(InputMetadataError::Denied),
        status => Err(InputMetadataError::QueryFailed(status)),
    }
}

/// Reads the OS microphone authorization without asking for permission.
pub fn microphone_authorization() -> Option<MicrophoneAuthorization> {
    // SAFETY: This immutable framework export is a documented input to the authorization API.
    let audio = unsafe { AVMediaTypeAudio }?;
    // SAFETY: AVMediaTypeAudio is one of the two media types accepted by the method.
    Some(autoreleasepool(|_| unsafe {
        AVCaptureDevice::authorizationStatusForMediaType(audio)
    }))
}

/// Waits on the capture worker for the OS's microphone permission decision.
pub fn request_microphone_access() -> Option<bool> {
    // SAFETY: This immutable framework export is a documented input to the authorization API.
    let audio = unsafe { AVMediaTypeAudio }?;
    let (tx, rx) = mpsc::sync_channel(1);
    autoreleasepool(|_| {
        let completed = RcBlock::new(move |authorized: Bool| {
            let _ = tx.send(authorized.as_bool());
        });
        // SAFETY: The media type is AVMediaTypeAudio. AVFoundation copies the block for
        // asynchronous use; it owns the thread-safe sender and borrows no Rust state.
        unsafe {
            AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &completed);
        }
    });
    // AVFoundation calls the block on a dispatch queue, not on this waiting thread.
    rx.recv().ok()
}
