/// Load-owned particle key requests, resolved before any world can emit particles.
pub trait ParticlesRegistrar {
    /// Request a vanilla particle or a custom sprite provider by canonical registry ID.
    /// Unknown IDs are reported with the requesting mod when the content registry loads.
    fn particle_key(&mut self, id: &str) -> crate::ParticleKey;

    // hidden: the typed registration wrapper inserts its erased codec through this method.
    #[doc(hidden)]
    fn add_particle_type(
        &mut self,
        id: &str,
        override_limiter: bool,
        codec: std::sync::Arc<dyn crate::__private::ParticleCodec>,
    ) -> crate::ParticleKey;
}

impl dyn super::Registrar + '_ {
    /// Register a common `ParticleType` with its structured and play-stream codecs.
    /// This does not need client assets. Register its sprite-set provider separately
    /// on the client. Duplicate names and foreign namespaces refuse startup.
    pub fn register_particle_type<T: PartialEq + Send + Sync + 'static>(
        &mut self,
        id: &str,
        override_limiter: bool,
        codec: crate::ParticleMapCodec<T>,
        stream_codec: crate::ParticleStreamCodec<T>,
    ) -> crate::ParticleType<T> {
        use crate::client::particle_types::{ParticleType, TypedCodec};
        ParticleType::new(self.add_particle_type(
            id,
            override_limiter,
            std::sync::Arc::new(TypedCodec(codec, stream_codec)),
        ))
    }
}
