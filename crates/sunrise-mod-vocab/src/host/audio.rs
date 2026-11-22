//! Host method declarations; access defaults and game forwarders are generated.
use crate::*;
/// Audio streams and microphone capture for [`crate::ClientHost`]. Streams use a pool separate from vanilla's
/// sound instances, fire no sound hooks and require a mod call on a live client from setup
/// onward. The mixer resamples to the output's rate and applies the stream's volume, category
/// and master gain, `muffle_sounds`, and the position and linear range fade for spatial mono.
/// Muted streams still advance. Keep the returned handle for as long as playback
/// should continue; dropping it stops the stream.
#[sunrise_mod_api_macros::host_area]
pub trait AudioClientHost {
    /// Side: client.
    /// Gives the mixer a native source of interleaved signed 16-bit PCM, like NeoForge's
    /// `SoundInstance.getStream`. The mixer takes ownership and calls [`AudioStream::read`]
    /// serially on the output's shared worker as it needs frames, with no `Host` or world borrow.
    /// The device callback requests PCM but never calls or waits for the source. Returning `0`
    /// signals EOF so already-queued frames drain before playback stops; return `n > 0` without
    /// writing to the zeroed buffer for temporary silence. An async or network producer bridges
    /// through its own nonblocking buffer inside [`AudioStream::read`].
    ///
    /// Refuses with [`AudioRefusal`] when playback is unavailable, the spec is invalid, a stream
    /// limit is reached, or the pull worker cannot take the source.
    ///
    /// ```
    /// use mod_api::{AudioStream, SoundSource, StreamSpec};
    /// # use mod_api::{AudioRefusal, AudioStreamRef, ClientHost};
    ///
    /// struct Clip(std::vec::IntoIter<i16>);
    /// impl AudioStream for Clip {
    ///     fn read(&mut self, pcm: &mut [i16]) -> usize {
    ///         let mut frames = 0;
    ///         for (out, sample) in pcm.iter_mut().zip(self.0.by_ref()) {
    ///             *out = sample;
    ///             frames += 1; // One sample per frame in this mono stream.
    ///         }
    ///         frames // 0 signals EOF.
    ///     }
    /// }
    ///
    /// # fn open(host: &dyn ClientHost, samples: Vec<i16>) -> Result<AudioStreamRef, AudioRefusal> {
    /// let spec = StreamSpec::new(SoundSource::Music, 1, 48_000);
    /// host.open_audio_stream(&spec, Box::new(Clip(samples.into_iter())))
    /// # }
    /// ```
    fn open_audio_stream(
        &self,
        _spec: &StreamSpec,
        _source: Box<dyn AudioStream>,
    ) -> Result<AudioStreamRef, AudioRefusal>;

    /// Side: client.
    /// Lists the client's available audio input devices, each with a distinct opaque selector
    /// ([`MicDevice::id`]) and a human-readable display label ([`MicDevice::label`]). Returns an
    /// empty list when no input devices exist or client audio capture is disabled. An individual
    /// inaccessible endpoint does not hide other available inputs. On macOS, listing does not
    /// request microphone permission.
    ///
    /// Refuses with [`MicRefusal::OnServer`] on a server, [`MicRefusal::OsDenied`] if OS permissions
    /// block enumeration or leave no listable inputs, [`MicRefusal::Unavailable`] outside client
    /// setup or later, [`MicRefusal::Busy`] if probing reports a busy device with no available
    /// inputs, or [`MicRefusal::DeviceFailed`] if device enumeration fails.
    ///
    /// ```
    /// use mod_api::{ClientHost, MicDevice, MicRefusal};
    ///
    /// fn list(host: &dyn ClientHost) -> Result<Vec<MicDevice>, MicRefusal> {
    ///     host.microphone_devices()
    /// }
    /// ```
    fn microphone_devices(&self) -> Result<Vec<MicDevice>, MicRefusal>;

    /// Side: client.
    /// Opens live PCM capture from the system default or selected input device directly on the
    /// client. Returns a client-local [`MicRef`] readable from any thread off the OS audio
    /// callback: [`MicRef::read`] writes only validated whole interleaved signed 16-bit PCM frames
    /// from a bounded [`MIC_RING_MAX_MS`] ms ring that drops the oldest frames on overflow,
    /// leaving any unreturned suffix untouched and returning `0` immediately on concurrent reader
    /// contention. Calling [`MicRef::close`] or dropping the handle stops capture and waits for
    /// the stream's teardown thread to release the OS input device before returning; a backend
    /// device error also releases the OS stream promptly without waiting for `close`, drop, or a
    /// tick. The engine never records or transmits captured audio. On macOS, opening a matching
    /// input can wait for the operating system's first microphone permission decision, before
    /// probing the input configuration. A label matching only an output does not request
    /// microphone permission.
    ///
    /// Refuses with [`MicRefusal`] when called on a server, outside client setup or later, with
    /// an invalid [`MicSpec`], when the OS denies permission or prevents checking the selected
    /// input, when an input vanished, when discovery cannot establish a unique label match,
    /// or when no matching device can be opened.
    ///
    /// ```
    /// use mod_api::{ClientHost, MicRef, MicRefusal, MicSpec};
    ///
    /// fn open(host: &dyn ClientHost) -> Result<MicRef, MicRefusal> {
    ///     host.open_microphone(&MicSpec::new(1, 48_000))
    /// }
    ///
    /// // On any thread: read whole interleaved frames without blocking on a tick or callback.
    /// fn drain(mic: &MicRef, pcm: &mut [i16]) -> usize {
    ///     mic.read(pcm) as usize
    /// }
    /// ```
    fn open_microphone(&self, _spec: &MicSpec) -> Result<MicRef, MicRefusal>;
}
