//! Fallible ALSA capture discovery. The pinned alsa crate does not expose its control
//! handle or the errors from its PCM device iterator.

use alsa_sys as sys;
use std::ffi::CStr;
use std::ptr;

/// Owns one native hardware card control.
pub struct CaptureControl(*mut sys::snd_ctl_t);

impl CaptureControl {
    /// Opens a hardware card's nonblocking control, not its PCM stream.
    pub fn open(card: alsa::Card) -> alsa::Result<Self> {
        let name = format!("hw:{}\0", card.get_index());
        let mut handle = ptr::null_mut();
        const SND_CTL_NONBLOCK: i32 = 1;
        // SAFETY: The name contains just ASCII and a trailing NUL. On success ALSA
        // initializes the out pointer and transfers ownership of the control to us.
        let code =
            unsafe { sys::snd_ctl_open(&mut handle, name.as_ptr().cast(), SND_CTL_NONBLOCK) };
        status("snd_ctl_open", code)?;
        Ok(Self(handle))
    }

    /// Returns the ALSA card ID and display name from the same control used for PCM discovery.
    pub fn card_info(&self) -> alsa::Result<(String, String)> {
        let mut info = ptr::null_mut();
        // SAFETY: ALSA initializes this out pointer to an owned allocation on success.
        status("snd_ctl_card_info_malloc", unsafe {
            sys::snd_ctl_card_info_malloc(&mut info)
        })?;
        // SAFETY: Both the control and allocation are alive and valid for this call.
        let result = status("snd_ctl_card_info", unsafe {
            sys::snd_ctl_card_info(self.0, info)
        })
        .and_then(|()| {
            // SAFETY: The successful query populated these NUL-terminated strings.
            // Copy them before the allocation is freed below.
            let id = unsafe { CStr::from_ptr(sys::snd_ctl_card_info_get_id(info)) };
            let name = unsafe { CStr::from_ptr(sys::snd_ctl_card_info_get_name(info)) };
            let to_string = |s: &CStr| {
                s.to_str()
                    .map(str::to_owned)
                    .map_err(|_| alsa::Error::new("snd_ctl_card_info", libc::EILSEQ))
            };
            Ok((to_string(id)?, to_string(name)?))
        });
        // SAFETY: This is the allocation owned by this call.
        unsafe { sys::snd_ctl_card_info_free(info) };
        result
    }

    /// Lists native PCM device numbers. A failed step yields an error, not exhaustion.
    pub fn pcm_devices(&self) -> impl Iterator<Item = alsa::Result<u32>> + '_ {
        const NEXT_DEVICE: &str = "snd_ctl_pcm_next_device";
        let mut cursor = Some(-1);
        std::iter::from_fn(move || {
            let device = cursor.as_mut()?;
            // SAFETY: The iterator borrows the open control and owns the mutable cursor.
            let result = status(NEXT_DEVICE, unsafe {
                sys::snd_ctl_pcm_next_device(self.0, device)
            })
            .and_then(|()| match *device {
                -1 => Ok(None),
                index => u32::try_from(index)
                    .map(Some)
                    .map_err(|_| alsa::Error::new(NEXT_DEVICE, libc::EINVAL)),
            });
            if !matches!(result, Ok(Some(_))) {
                cursor = None;
            }
            result.transpose()
        })
    }

    /// Queries the first capture subdevice. ALSA reports an output-only PCM as absent.
    pub fn query_capture(&self, device: u32) -> alsa::Result<()> {
        let mut info = ptr::null_mut();
        // SAFETY: ALSA initializes this out pointer to an owned allocation on success.
        status("snd_pcm_info_malloc", unsafe {
            sys::snd_pcm_info_malloc(&mut info)
        })?;
        // SAFETY: Both the control and allocation are alive. The allocation is freed here
        // even if ALSA reports a failed capture query.
        let code = unsafe {
            sys::snd_pcm_info_set_device(info, device);
            sys::snd_pcm_info_set_subdevice(info, 0);
            sys::snd_pcm_info_set_stream(info, sys::SND_PCM_STREAM_CAPTURE);
            let code = sys::snd_ctl_pcm_info(self.0, info);
            sys::snd_pcm_info_free(info);
            code
        };
        status("snd_ctl_pcm_info", code)
    }
}

impl Drop for CaptureControl {
    fn drop(&mut self) {
        // SAFETY: This closes the control opened by Self::open, exactly once.
        unsafe { sys::snd_ctl_close(self.0) };
    }
}

fn status(function: &'static str, code: i32) -> alsa::Result<()> {
    if code < 0 {
        Err(alsa::Error::new(function, -code))
    } else {
        Ok(())
    }
}
