//! Prepare client listeners against the new pack selection before publishing
//! any assets, and apply them on the thread that owns the game. The initial
//! load runs the same three steps: the loader thread registers and stages,
//! and the game applies once it exists (`start_game`).
use mod_api::__private::game::*;

mod texture_atlases;
pub(in crate::game) use texture_atlases::{collect_texture_atlases, stage_texture_atlases};

#[cfg(test)]
#[path = "client_reload_tests.rs"]
mod tests;
use super::Game;
use super::packs::reload_files;
use mod_api::{
    __private::reload::{PreparedStep, ReloadStageError, prepare_listeners},
    ClientReloadContext, ClientReloadHost, ClientReloadListener, ClientReloadListenerContext,
    HookId,
};
use sunrise_client::{datapack::DataPack, settings::Settings};

/// One client reload step: a mod's apply action or model publication.
pub(super) type PreparedClientReload = PreparedStep<()>;

/// The registration context of the first load: no game or published assets
/// exist yet, so [`published`](ClientReloadListenerContext::published) is
/// `None` while process, mod-roster, settings, math-profile, and local text
/// queries return real values. Later reloads pass `&Game` directly.
pub(super) struct StartupReloadContext<'a> {
    settings: Option<&'a Settings>,
}

impl ClientReloadListenerContext for StartupReloadContext<'_> {
    fn dist(&self) -> mod_api::Dist {
        crate::entry::runtime_environment::try_get().map_or(mod_api::Dist::Client, |env| env.dist())
    }
    fn game_version(&self) -> &'static str {
        crate::entry::runtime_environment::try_get()
            .map_or(sunrise_core::VANILLA_VERSION, |env| env.game_version())
    }
    fn server_data_version(&self) -> i32 {
        sunrise_core::version::DATA_VERSION
    }
    fn sunrise_version(&self) -> &'static str {
        crate::entry::runtime_environment::try_get().map_or("", |env| env.sunrise_version())
    }
    fn game_tests_enabled(&self) -> bool {
        crate::entry::runtime_environment::try_get().is_some_and(|env| env.game_tests_enabled())
    }
    fn on(&self, mod_id: &str) -> bool {
        self.settings
            .is_none_or(|settings| !settings.mods.disabled.contains(mod_id))
    }
    fn is_loaded(&self, id: &str) -> bool {
        mod_api::__private::loaded_mods().get(id).is_some()
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        Game::startup_setting(mod_id, key)
    }
    fn math_profile(&self) -> mod_api::MathProfile {
        Game::startup_math_profile()
    }
    fn text_string(&self, text: &mod_api::Text) -> String {
        sunrise_client::lang::scoped(sunrise_client::lang::Language::default(), || {
            sunrise_client_lib::api_text::to_engine_inprocess(text).plain()
        })
        .0
    }
    fn published(&self) -> Option<&dyn ClientReloadHost> {
        None
    }
}

impl Game<'_> {
    /// Registers a later reload's client listeners through the game
    /// (`native_add_client_reload_listeners`), gated on the hook being
    /// listened to.
    pub(super) fn register_client_reload_listeners(&self) -> mod_api::ClientListenerRegistry {
        let mut listeners = <mod_api::ClientListenerRegistry as mod_api::__private::game::ClientReloadListenersOrder>::new();
        if mod_api::__private::listened(HookId::AddClientReloadListeners) {
            self.native_add_client_reload_listeners(&mut listeners);
        }
        listeners
    }
}

/// Registers the first load's client listeners: no game exists yet, so
/// [`StartupReloadContext`] answers the process/mod/settings facts known on
/// the loader thread and leaves `published()` as `None`.
pub(super) fn register_startup_client_reload_listeners(
    settings: Option<&Settings>,
) -> mod_api::ClientListenerRegistry {
    let mut listeners = <mod_api::ClientListenerRegistry as mod_api::__private::game::ClientReloadListenersOrder>::new();
    if mod_api::__private::listened(HookId::AddClientReloadListeners) {
        let context = StartupReloadContext { settings };
        mod_api::dispatch_add_client_reload_listeners(&context, |listen| {
            listeners.register_listener(listen)
        });
    }
    listeners
}

/// Prepare listeners against the merged asset view, running the model build
/// at the model manager's position in their order. The models travel separately
/// so renderer preflight can use them without publishing them.
pub(super) fn stage_client_reload<V, E: From<ReloadStageError>>(
    root: &sunrise_assets::resource_source::ResourceSource,
    listeners: mod_api::ClientListenerRegistry,
    build_models: impl FnOnce() -> Result<V, E>,
) -> Result<(V, Vec<PreparedClientReload>), E> {
    let pack = DataPack::from_source("vanilla".into(), root.clone());
    let ordered = listeners.finish().map_err(ReloadStageError::Order)?;
    let mut build_models = Some(build_models);
    let mut models = None;
    let steps = prepare_listeners(
        ordered,
        HookId::AddClientReloadListeners,
        "client_reload_prepare",
        || -> Result<(), E> {
            models = Some(build_models.take().expect("one model manager")()?);
            Ok(())
        },
        |id, converter| ClientReloadContext::new(reload_files("assets", &[&pack], id, converter)),
        ClientReloadListener::prepare,
    )?;
    Ok((models.expect("model manager is always registered"), steps))
}

#[cfg(test)]
thread_local! {
    static APPLY_GENERATIONS: std::cell::RefCell<Option<Vec<u64>>> = const { std::cell::RefCell::new(None) };
}

impl Game<'_> {
    /// Publish the models in order with the mod listener actions on the game-owning
    /// thread, before the resources-reloaded notice.
    pub(super) fn apply_client_reload(
        &mut self,
        prepared: Vec<PreparedClientReload>,
        publish_models: impl FnOnce(&mut Self),
    ) {
        let mut publish_models = Some(publish_models);
        for step in prepared {
            #[cfg(test)]
            if matches!(step, PreparedStep::Mod { .. }) {
                APPLY_GENERATIONS.with_borrow_mut(|trace| {
                    if let Some(trace) = trace {
                        trace.push(self.pack.render.generation);
                    }
                });
            }
            let _ = step
                .apply(
                    HookId::AddClientReloadListeners,
                    "client_reload_apply",
                    |()| {
                        publish_models.take().expect("one model manager")(self);
                        Ok::<_, std::convert::Infallible>(())
                    },
                )
                .unwrap();
        }
    }
}
