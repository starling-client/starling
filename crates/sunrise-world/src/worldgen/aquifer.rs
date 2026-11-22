//! Global fluid selection and the 26.3 aquifer terrain paths.
mod config;
mod noise_based;
use crate::registry::BlockStateId;
pub use config::{AquiferConfig, AquiferField, AquiferSamplers};
pub use noise_based::{
    AquiferBlockStates, AquiferError, NoiseBasedAquifer, RegistryAquiferBlockStates,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FluidStatus {
    pub level: i32,
    pub state: BlockStateId,
}
impl FluidStatus {
    pub const fn at(self, y: i32, air: BlockStateId) -> BlockStateId {
        if y < self.level { self.state } else { air }
    }
}
pub trait FluidPicker: std::fmt::Debug + Send + Sync {
    fn compute(&self, position: [i32; 3]) -> FluidStatus;
    /// The highest `level` that `compute` returns for any position whose x and z lie in
    /// `min..=max` (`[x, z]`, inclusive), at any y. A higher value is allowed and only costs
    /// per-block calls; `None` when the picker can't bound it, and every block then asks
    /// `compute`. Generation skips `compute` above this level, so a picker whose level
    /// varies with x/z must answer for the whole box it is given.
    fn level_bound(&self, min: [i32; 2], max: [i32; 2]) -> Option<i32>;
}
impl<P: FluidPicker + ?Sized> FluidPicker for &P {
    fn compute(&self, position: [i32; 3]) -> FluidStatus {
        P::compute(self, position)
    }
    fn level_bound(&self, min: [i32; 2], max: [i32; 2]) -> Option<i32> {
        P::level_bound(self, min, max)
    }
}
/// State IDs must come from one immutable block registry. Default fluid can be
/// any registered block state; vanilla does not validate its material here.
#[derive(Clone, Copy, Debug)]
pub struct GlobalFluidPicker {
    sea_level: i32,
    default_fluid: BlockStateId,
    lava: BlockStateId,
    air: BlockStateId,
    disabled: bool,
}
impl GlobalFluidPicker {
    pub const fn new(
        sea_level: i32,
        default_fluid: BlockStateId,
        lava: BlockStateId,
        air: BlockStateId,
    ) -> Self {
        Self {
            sea_level,
            default_fluid,
            lava,
            air,
            disabled: false,
        }
    }
    /// `NoiseBasedChunkGenerator.createFluidPicker`'s picker. The two methods are inherent so
    /// mods can replace them: below the game crate, a `#[replaceable]` method of a trait impl
    /// is listed under its type's path but indexed under `<Type as Trait>`, so it is refused.
    ///
    /// A replacement that raises the level in any column must replace [`Self::level_bound`]
    /// too: the terrain fill skips this function above the bound, so with the vanilla bound
    /// the raised fluid is silently dropped there.
    ///
    /// Called once per block by the aquifer: it must not allocate.
    #[sunrise_mod_api_macros::replaceable]
    pub fn compute(&self, position: [i32; 3]) -> FluidStatus {
        let y = position[1];
        if self.disabled {
            FluidStatus {
                level: -4064,
                state: self.air,
            }
        } else if y < (-54).min(self.sea_level) {
            FluidStatus {
                level: -54,
                state: self.lava,
            }
        } else {
            FluidStatus {
                level: self.sea_level,
                state: self.default_fluid,
            }
        }
    }
    /// The highest of `compute`'s levels: the sea, or the lava at -54 when the sea is lower.
    ///
    /// A replacement of [`Self::compute`] that raises the level in some columns must replace this
    /// too, returning at least the highest level `compute` gives for the columns `min..=max`
    /// (`[x, z]`) or `None`; otherwise the fill drops the raised fluid above the vanilla bound.
    /// Called once per chunk and per far-terrain tile, never per block.
    #[sunrise_mod_api_macros::replaceable]
    pub fn level_bound(&self, _min: [i32; 2], _max: [i32; 2]) -> Option<i32> {
        Some(if self.disabled {
            -4064
        } else {
            self.sea_level.max(-54)
        })
    }
}
impl FluidPicker for GlobalFluidPicker {
    fn compute(&self, position: [i32; 3]) -> FluidStatus {
        GlobalFluidPicker::compute(self, position)
    }
    fn level_bound(&self, min: [i32; 2], max: [i32; 2]) -> Option<i32> {
        GlobalFluidPicker::level_bound(self, min, max)
    }
}
#[derive(Debug)]
pub struct DisabledAquifer<P> {
    picker: P,
    air: BlockStateId,
}
impl<P: FluidPicker> DisabledAquifer<P> {
    pub const fn new(picker: P, air: BlockStateId) -> Self {
        Self { picker, air }
    }
    /// None selects the generator's default solid block. Java DCMPL treats NaN
    /// as -1, so NaN and density <= zero call the picker; density > zero does not.
    pub fn compute_substance(&self, position: [i32; 3], density: f64) -> Option<BlockStateId> {
        if density > 0. {
            None
        } else {
            Some(self.picker.compute(position).at(position[1], self.air))
        }
    }
    pub const fn should_schedule_fluid_update(&self) -> bool {
        false
    }
}
