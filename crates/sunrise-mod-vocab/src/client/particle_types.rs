//! Typed particle registration and the shared structured/stream dispatch.
use crate::callback_owner::CallbackOwner;
use crate::wire::{Reader, Wire, Writer};
use crate::{Compound, FluidKey, Identifier, ItemKey, ParticleKey, ParticleOptions};
use std::{any::Any, fmt, marker::PhantomData, sync::Arc};
use sunrise_mod_runtime::registry_keys::Names;

/// The load-stable identity of a registered particle type and its Rust options type.
// no wire: only the type's identifier and declared stream value travel.
// exhaustive: only common registration creates the typed handle.
pub struct ParticleType<T> {
    key: ParticleKey,
    value: PhantomData<fn() -> T>,
}
impl<T> Copy for ParticleType<T> {}
impl<T> Clone for ParticleType<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> ParticleType<T> {
    pub(crate) fn new(key: ParticleKey) -> Self {
        Self {
            key,
            value: PhantomData,
        }
    }
    /// The same registry slot returned by `Registrar::particle_key` for this ID.
    pub fn key(self) -> ParticleKey {
        self.key
    }
}
impl<T: PartialEq + Send + Sync + 'static> ParticleType<T> {
    /// Pair an owned value with this registered type for a local spawn or a level send.
    pub fn options(self, value: T) -> ParticleOptions {
        ParticleOptions::from_typed(TypedParticleOptions::new(self.key, value))
    }
}

/// Read-only registry lookups available to a particle codec. Use names in encoded
/// data; key slots are local to the installed game and can differ at the other end.
// no wire: a borrowed view of the frozen registries used by the codec.
// exhaustive: only the game constructs the borrowed registry view.
pub struct ParticleRegistryView<'a> {
    particles: Option<&'a Names>,
    items: Option<&'a Names>,
    fluids: Option<&'a Names>,
}
impl ParticleRegistryView<'static> {
    pub(crate) fn installed() -> Self {
        let installed = crate::parts::installed();
        Self {
            particles: installed.and_then(|i| i.particles.get()).map(|p| &p.names),
            items: installed.and_then(|i| i.resolved_items.get()),
            fluids: installed.and_then(|i| i.fluids.get()),
        }
    }
}
impl ParticleRegistryView<'_> {
    /// Resolve a particle identifier without writing to the registry.
    pub fn particle(&self, id: &Identifier) -> Option<ParticleKey> {
        self.particles?
            .identifiers
            .get(id)
            .copied()
            .map(ParticleKey)
    }
    /// Resolve an item identifier without writing to the registry.
    pub fn item(&self, id: &Identifier) -> Option<ItemKey> {
        self.items?.identifiers.get(id).copied().map(ItemKey)
    }
    /// Resolve a fluid identifier without writing to the registry.
    pub fn fluid(&self, id: &Identifier) -> Option<FluidKey> {
        self.fluids?.identifiers.get(id).copied().map(FluidKey)
    }
    /// Borrow the registered name of a particle key.
    pub fn particle_id(&self, key: ParticleKey) -> Option<&str> {
        self.particles?.id(key.0)
    }
    /// Borrow the registered name of an item key.
    pub fn item_id(&self, key: ItemKey) -> Option<&str> {
        self.items?.id(key.0)
    }
    /// Borrow the registered name of a fluid key.
    pub fn fluid_id(&self, key: FluidKey) -> Option<&str> {
        self.fluids?.id(key.0)
    }
}

type MapDecode<T> = fn(&Compound, &ParticleRegistryView<'_>) -> Result<T, String>;
type MapEncode<T> = fn(&T, &ParticleRegistryView<'_>) -> Result<Compound, String>;

/// A registered type's options in commands and structured data (`ParticleType.codec`).
/// The engine dispatches on `type`; the codec reads and writes only that type's fields.
// no wire: process-local codec functions.
// exhaustive: mods construct it with new, not fields.
pub struct ParticleMapCodec<T> {
    decode: MapDecode<T>,
    encode: MapEncode<T>,
}
impl<T> ParticleMapCodec<T> {
    /// Pair the type's map decoder and encoder. Return an error for invalid data.
    pub fn new(decode: MapDecode<T>, encode: MapEncode<T>) -> Self {
        Self { decode, encode }
    }
}
impl ParticleMapCodec<()> {
    /// `SimpleParticleType` carries no fields.
    pub fn unit() -> Self {
        Self::new(|_, _| Ok(()), |_, _| Ok(Compound::new()))
    }
}

type StreamDecode<T> = dyn Fn(&[u8], &ParticleRegistryView<'_>) -> Result<T, String> + Send + Sync;
type StreamEncode<T> =
    dyn Fn(&T, &ParticleRegistryView<'_>, usize) -> Result<Vec<u8>, String> + Send + Sync;
/// A registered type's play-stream codec (`ParticleType.streamCodec`). The transmitted
/// value has an explicit [`Wire`] form and carries identifiers, not local key slots.
// no wire: only the declared value crosses, never the codec itself.
// exhaustive: mods construct it with wire or mapped, not fields.
pub struct ParticleStreamCodec<T> {
    decode: Box<StreamDecode<T>>,
    encode: Box<StreamEncode<T>>,
}
impl<T: Wire + 'static> ParticleStreamCodec<T> {
    /// Use the options' own derived Wire representation.
    pub fn wire() -> Self {
        Self {
            decode: Box::new(|bytes, _| Reader::decode(bytes).map_err(|error| error.to_string())),
            encode: Box::new(|value, _, limit| encode_value(value, limit)),
        }
    }
}
impl<T: 'static> ParticleStreamCodec<T> {
    /// Convert between options and a derived Wire value with registry context.
    pub fn mapped<W: Wire + 'static>(
        from_wire: fn(W, &ParticleRegistryView<'_>) -> Result<T, String>,
        to_wire: fn(&T, &ParticleRegistryView<'_>) -> Result<W, String>,
    ) -> Self {
        Self {
            decode: Box::new(move |bytes, registries| {
                from_wire(
                    Reader::decode(bytes).map_err(|error| error.to_string())?,
                    registries,
                )
            }),
            encode: Box::new(move |value, registries, limit| {
                encode_value(&to_wire(value, registries)?, limit)
            }),
        }
    }
}

fn encode_value(value: &impl Wire, limit: usize) -> Result<Vec<u8>, String> {
    let mut writer = Writer::new();
    writer.limit(limit);
    value.put(&mut writer);
    if let Some(error) = writer.error() {
        return Err(error.to_string());
    }
    Ok(writer.into_bytes())
}

trait Value: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn equals(&self, other: &dyn Value) -> bool;
}
impl<T: PartialEq + Send + Sync + 'static> Value for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn equals(&self, other: &dyn Value) -> bool {
        other.as_any().downcast_ref().is_some_and(|rhs| self == rhs)
    }
}

/// An owned particle value shared by a whole spawn request. Only its registered
/// codec and the type's provider interpret it.
// hidden: the engine shares a decoded value across a spawn request.
#[doc(hidden)]
#[derive(Clone)]
// exhaustive: only registration and the codecs create the owned value.
pub struct TypedParticleOptions {
    key: ParticleKey,
    value: Arc<dyn Value>,
}
impl TypedParticleOptions {
    fn new<T: PartialEq + Send + Sync + 'static>(key: ParticleKey, value: T) -> Self {
        Self {
            key,
            value: Arc::new(value),
        }
    }
    pub fn key(&self) -> ParticleKey {
        self.key
    }
    pub(crate) fn value<T: 'static>(&self) -> Option<&T> {
        self.value.as_ref().as_any().downcast_ref()
    }
}
impl PartialEq for TypedParticleOptions {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.value.as_ref().equals(other.value.as_ref())
    }
}
impl fmt::Debug for TypedParticleOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TypedParticleOptions")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

// hidden: the registration table stores each type's erased codecs.
#[doc(hidden)]
pub trait ParticleCodec: Send + Sync {
    fn decode_map(
        &self,
        key: ParticleKey,
        map: &Compound,
        view: &ParticleRegistryView<'_>,
    ) -> Result<TypedParticleOptions, String>;
    fn encode_map(
        &self,
        value: &TypedParticleOptions,
        view: &ParticleRegistryView<'_>,
    ) -> Result<Compound, String>;
    fn decode_stream(
        &self,
        key: ParticleKey,
        data: &[u8],
        view: &ParticleRegistryView<'_>,
    ) -> Result<TypedParticleOptions, String>;
    fn encode_stream(
        &self,
        value: &TypedParticleOptions,
        view: &ParticleRegistryView<'_>,
        limit: usize,
    ) -> Result<Vec<u8>, String>;
}

/// Attribution and the existing retained-callback failure policy for a codec or factory.
pub(super) struct ParticleCallback(CallbackOwner);

impl ParticleCallback {
    pub(super) fn new(owner: CallbackOwner) -> Self {
        Self(owner)
    }

    pub(super) fn call<T>(
        &self,
        key: ParticleKey,
        operation: &'static str,
        invoke: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.0
            .call("particle", operation, invoke)
            // A codec's own error names its mod; a panic or disablement already does.
            .and_then(|value| value.map_err(|error| format!("{}: {error}", self.0.id())))
            .map_err(|error| {
                let name =
                    super::particle_keys::particle_name(key).unwrap_or("unresolved particle type");
                format!("{name}: {error}")
            })
    }
}

struct OwnedParticleCodec {
    codec: Arc<dyn ParticleCodec>,
    callback: ParticleCallback,
}

/// Bind the declaration's type-erased codec to its registering mod.
pub fn own_particle_codec(
    owner: &'static str,
    index: usize,
    codec: Arc<dyn ParticleCodec>,
) -> Arc<dyn ParticleCodec> {
    Arc::new(OwnedParticleCodec {
        codec,
        callback: ParticleCallback::new(CallbackOwner::for_index(owner, index)),
    })
}

impl ParticleCodec for OwnedParticleCodec {
    fn decode_map(
        &self,
        key: ParticleKey,
        map: &Compound,
        view: &ParticleRegistryView<'_>,
    ) -> Result<TypedParticleOptions, String> {
        self.callback.call(key, "decode particle map", || {
            self.codec.decode_map(key, map, view)
        })
    }

    fn encode_map(
        &self,
        value: &TypedParticleOptions,
        view: &ParticleRegistryView<'_>,
    ) -> Result<Compound, String> {
        self.callback.call(value.key(), "encode particle map", || {
            self.codec.encode_map(value, view)
        })
    }

    fn decode_stream(
        &self,
        key: ParticleKey,
        data: &[u8],
        view: &ParticleRegistryView<'_>,
    ) -> Result<TypedParticleOptions, String> {
        self.callback.call(key, "decode particle stream", || {
            self.codec.decode_stream(key, data, view)
        })
    }

    fn encode_stream(
        &self,
        value: &TypedParticleOptions,
        view: &ParticleRegistryView<'_>,
        limit: usize,
    ) -> Result<Vec<u8>, String> {
        self.callback
            .call(value.key(), "encode particle stream", || {
                self.codec.encode_stream(value, view, limit)
            })
    }
}

pub(crate) struct TypedCodec<T>(pub ParticleMapCodec<T>, pub ParticleStreamCodec<T>);
impl<T: PartialEq + Send + Sync + 'static> ParticleCodec for TypedCodec<T> {
    fn decode_map(
        &self,
        key: ParticleKey,
        map: &Compound,
        view: &ParticleRegistryView<'_>,
    ) -> Result<TypedParticleOptions, String> {
        (self.0.decode)(map, view).map(|value| TypedParticleOptions::new(key, value))
    }
    fn encode_map(
        &self,
        value: &TypedParticleOptions,
        view: &ParticleRegistryView<'_>,
    ) -> Result<Compound, String> {
        (self.0.encode)(value.value().ok_or("wrong particle options type")?, view)
    }
    fn decode_stream(
        &self,
        key: ParticleKey,
        data: &[u8],
        view: &ParticleRegistryView<'_>,
    ) -> Result<TypedParticleOptions, String> {
        (self.1.decode)(data, view).map(|value| TypedParticleOptions::new(key, value))
    }
    fn encode_stream(
        &self,
        value: &TypedParticleOptions,
        view: &ParticleRegistryView<'_>,
        limit: usize,
    ) -> Result<Vec<u8>, String> {
        (self.1.encode)(
            value.value().ok_or("wrong particle options type")?,
            view,
            limit,
        )
    }
}
