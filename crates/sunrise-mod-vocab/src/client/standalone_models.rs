//! Scoped standalone model registration and resource-time assembly.
use crate::StandaloneModelsEvent;
use std::fmt;

impl StandaloneModelsEvent<'_> {
    /// Register an unturned, namespaced model file such as `mymod:block/shaft`.
    /// If a parent is missing or rejected, the missing model takes that parent's place.
    /// A successfully baked file takes precedence over a baker for the same ID.
    pub fn register_model(&mut self, id: impl Into<String>) {
        self.registrations
            .push(StandaloneModelRegistration::File(id.into()));
    }

    /// Register a baker for a namespaced result ID. Declare every model file that
    /// the baker may append. The result ID need not use the registering mod's namespace.
    ///
    /// At each resource load, the game runs bakers in registration order after loading
    /// their dependencies. A baker runs only if the ID is still unfilled and every declared
    /// file loads. Like blockstate models, a missing or rejected parent uses the missing model
    /// at that link; it does not invalidate the loaded file. Capture owned data; the baker can
    /// run on a resource worker.
    /// Returning an error or failing any append discards the whole result, even if the
    /// callback ignores the append's error.
    pub fn register(
        &mut self,
        id: impl Into<String>,
        dependencies: impl IntoIterator<Item = impl Into<String>>,
        baker: impl Fn(&mut StandaloneModelBakeContext<'_>) -> Result<(), StandaloneModelBakeError>
        + Send
        + Sync
        + 'static,
    ) {
        self.registrations
            .push(StandaloneModelRegistration::Baker(StandaloneModelBaker {
                id: id.into(),
                dependencies: dependencies.into_iter().map(Into::into).collect(),
                bake: Box::new(baker),
            }));
    }
}

// no wire: the context borrows the assembly for this resource generation.
/// Append declared model dependencies into one standalone model during its bake.
// exhaustive: the borrowed append operation is private and supplied only by the engine.
pub struct StandaloneModelBakeContext<'a> {
    append: &'a mut AppendDependency<'a>,
}

type AppendDependency<'a> =
    dyn FnMut(&str, [u8; 3], bool) -> Result<(), StandaloneModelBakeError> + 'a;

impl StandaloneModelBakeContext<'_> {
    /// Append the quads of a declared model `id`, in call order. `quarter_turns` are
    /// negative X, Y and Z quarter-turn counts (modulo four) around the block centre.
    /// With `uvlock`, textures keep their orientation in the resulting face's world basis.
    /// On error, the assembly is unchanged; return the error (for example with `?`) to
    /// discard the model, or handle it and return `Ok(())` to keep the valid appends.
    pub fn append_dependency(
        &mut self,
        id: &str,
        quarter_turns: [u8; 3],
        uvlock: bool,
    ) -> Result<(), StandaloneModelBakeError> {
        (self.append)(id, quarter_turns, uvlock)
    }
}

/// Supply the engine's assembly operation to a bake context.
pub fn standalone_model_bake_context<'a>(
    append: &'a mut (impl FnMut(&str, [u8; 3], bool) -> Result<(), StandaloneModelBakeError> + 'a),
) -> StandaloneModelBakeContext<'a> {
    StandaloneModelBakeContext { append }
}

// no wire: a bake failure is local to a retained callback and its resource load.
/// Why a standalone model bake did not produce a result.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StandaloneModelBakeError {
    /// The baker tried to append an ID it did not declare at registration.
    UndeclaredDependency(String),
    /// A declared dependency could not be baked.
    DependencyBake {
        /// The requested model file's ID.
        id: String,
        /// Diagnostic from the resource loader or baker.
        reason: String,
    },
    /// The baker declined to produce a model. Include a diagnostic for the log.
    Rejected(String),
}

impl fmt::Display for StandaloneModelBakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UndeclaredDependency(id) => write!(f, "undeclared dependency {id}"),
            Self::DependencyBake { id, reason } => write!(f, "dependency {id}: {reason}"),
            Self::Rejected(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for StandaloneModelBakeError {}

// no wire: entries may carry executable native code.
/// The engine-only staging entries for the standalone model hook.
// exhaustive: engine-only staging; the loader handles both file and baker registrations.
pub enum StandaloneModelRegistration {
    /// Register a file directly.
    File(String),
    /// Assemble a model at resource load time.
    Baker(StandaloneModelBaker),
}

// no wire: the baker is retained executable native code.
/// A staged baker with its output ID and the model files it may append.
#[non_exhaustive]
pub struct StandaloneModelBaker {
    /// The model ID exposed to listeners after a successful bake.
    pub id: String,
    /// The source model IDs the baker may append.
    pub dependencies: Vec<String>,
    /// The callback executed for each resource generation.
    pub bake: Box<Bake>,
}

type Bake = dyn Fn(&mut StandaloneModelBakeContext<'_>) -> Result<(), StandaloneModelBakeError>
    + Send
    + Sync;
