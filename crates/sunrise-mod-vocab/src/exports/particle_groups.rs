mod particle_groups;
use particle_groups::ParticleGroupRegistration;
pub use particle_groups::{ParticleBlend, ParticleGroup};
mod __private {
    mod game {
        pub use crate::particle_groups::ParticleGroupRegistration;
    }
}
