//! The sound engine's hook points for native mods. The source and load notices fire from the
//! engine with the client's [`LoopContext`](mod_api::LoopContext). `play_sound` takes a `ClientHost`,
//! which only the game has: the game fires it inside its one play method, after
//! [`SoundEngine::begin_play`] prepares the start, and [`SoundEngine::finish_prepared`] applies the
//! answer and plays at once. A start made without the game is held ([`SoundEngine::next_deferred`])
//! and fired before the game's call returns. Live refreshes ([`SoundEngine::live_requests`],
//! [`SoundEngine::set_live_answer`]) stay on the audio tick.
use super::{Attenuation, ChannelId, SoundEngine, SoundInstance};
use mod_api::__private::built;
use mod_api::{ChannelRef, HookId, SoundAttenuation, SoundPlay, SoundSource};

/// Hook point for native mods (docs/modding/src/how-to/native-mods.md, declared in `hooks/`
/// with the game's): a sound that is not streamed has started on `channel`
/// (`PlaySoundSourceEvent`), after the engine set its pitch, volume and position.
#[inline(never)]
pub fn native_play_sound_source(
    context: &sunrise_world::native_load_context::LoopContext,
    sound: &SoundPlay,
    channel: &ChannelRef,
) {
    if !mod_api::__private::listened(HookId::PlaySoundSource) {
        return;
    }
    mod_api::dispatch_play_sound_source(
        &crate::api_context::LoopReader(context).context(),
        sound,
        channel,
    );
}

/// Hook point for native mods (docs/modding/src/how-to/native-mods.md, declared in `hooks/`
/// with the game's): a streamed sound (music, a record) has started on `channel`
/// (`PlayStreamingSourceEvent`).
#[inline(never)]
pub fn native_play_streaming_source(
    context: &sunrise_world::native_load_context::LoopContext,
    sound: &SoundPlay,
    channel: &ChannelRef,
) {
    if !mod_api::__private::listened(HookId::PlayStreamingSource) {
        return;
    }
    mod_api::dispatch_play_streaming_source(
        &crate::api_context::LoopReader(context).context(),
        sound,
        channel,
    );
}

/// Hook point for native mods (docs/modding/src/how-to/native-mods.md, declared in `hooks/`
/// with the game's): the sound engine opened its output, at window start, after a resource
/// reload and when the output device changes (`SoundEngineLoadEvent`).
#[inline(never)]
pub fn native_sound_engine_load(context: &sunrise_world::native_load_context::LoopContext) {
    if !mod_api::__private::listened(HookId::SoundEngineLoad) {
        return;
    }
    mod_api::dispatch_sound_engine_load(&crate::api_context::LoopReader(context).context());
}

// Validate a mod's Sunrise-specific effects at the playback boundary. Volume and pitch go
// through the engine's own calculations (`calculate_volume`, `calculate_pitch`) as any
// replacement sound does (NeoForge `PlaySoundEvent`); starts and refreshes use one contract.
fn valid_playback_edit(original: &SoundPlay, edited: &SoundPlay) -> bool {
    edited.effects == original.effects || edited.effects.valid()
}

/// A `play_sound` the engine prepared for the game to fire: the sound as asked for.
#[derive(Clone, Debug)]
pub struct PreparedPlay {
    pub instance: super::InstanceId,
    pub starting: bool,
    pub original: SoundPlay,
}

impl SoundInstance {
    fn play_settings(&self) -> SoundPlay {
        let mut sound = SoundPlay::ui("");
        self.write_play_settings(&mut sound);
        sound
    }
    // Reuse string storage for live playback; starts prepare the buffers.
    fn write_play_settings(&self, sound: &mut SoundPlay) {
        sound.identifier.clone_from(&self.identifier);
        sound.source = SoundSource::ALL[self.source];
        sound.volume = self.volume;
        sound.pitch = self.pitch;
        sound.position = self.position;
        sound.looping = self.looping;
        sound.delay = self.delay;
        sound.attenuation = match self.attenuation {
            Attenuation::None => SoundAttenuation::None,
            Attenuation::Linear => SoundAttenuation::Linear,
        };
        sound.relative = self.relative;
        sound.effects = self.effects;
    }

    /// A sound a mod asks for (`ClientHost::play_local_sound`), through the same mapping
    /// as a `play_sound` edit.
    pub fn from_play(sound: &SoundPlay, random: sunrise_core::random::LegacyRandom) -> Self {
        let mut instance = Self::new(
            &sound.identifier,
            0,
            1.0,
            1.0,
            random,
            false,
            0,
            Attenuation::None,
            [0.0; 3],
            true,
        );
        instance.apply_play_settings(sound.clone());
        instance
    }

    /// Take a mod's edits of [`Self::play_settings`]. A new identifier is resolved again
    /// at the play, as every instance is, even one whose sound a caller chose.
    fn apply_play_settings(&mut self, edited: SoundPlay) {
        if edited.identifier != self.identifier {
            self.identifier = edited.identifier;
            self.chosen = false;
        }
        self.source = edited.source as usize;
        self.volume = edited.volume;
        self.pitch = edited.pitch;
        self.position = edited.position;
        self.looping = edited.looping;
        self.delay = edited.delay;
        self.attenuation = match edited.attenuation {
            SoundAttenuation::None => Attenuation::None,
            SoundAttenuation::Linear => Attenuation::Linear,
            _ => self.attenuation,
        };
        self.relative = edited.relative;
        self.effects = edited.effects;
    }
}

impl SoundEngine {
    /// The loop context of this client's current resource generation.
    pub fn loop_context(&self) -> Option<&sunrise_world::native_load_context::LoopContext> {
        self.loop_context.as_ref()
    }

    /// Replace the loop context when a resource generation starts or reloads.
    pub fn set_loop_context(&mut self, context: sunrise_world::native_load_context::LoopContext) {
        self.loop_context = Some(context);
    }

    /// Whether `play_sound` is asked for the sounds this engine plays: a client with its context
    /// and a listener. With none, a start costs this one check and nothing is prepared.
    pub(super) fn asks_play_sound(&self) -> bool {
        self.loop_context.is_some() && mod_api::__private::listened(HookId::PlaySound)
    }

    /// Prepares the start of `id` for the game to fire `play_sound` for (`starting`), when a
    /// listener asks and the engine is loaded. The instance stays referenced until
    /// [`Self::finish_prepared`]. `None`: nothing to fire, the caller plays it with `play`.
    pub fn begin_play(&mut self, id: super::InstanceId) -> Option<PreparedPlay> {
        if !self.loaded || !self.asks_play_sound() {
            return None;
        }
        let original = self.instances.get(&id)?.play_settings();
        self.deferred.retain(|i| *i != id);
        self.inflight.push(id);
        Some(PreparedPlay {
            instance: id,
            starting: true,
            original,
        })
    }

    /// The next start held without the game, to fire `play_sound` for, in the order they were
    /// made.
    pub fn next_deferred(&mut self) -> Option<PreparedPlay> {
        while !self.deferred.is_empty() {
            let id = self.deferred.remove(0);
            if let Some(prepared) = self.begin_play(id) {
                return Some(prepared);
            }
            self.collect(id);
        }
        None
    }

    /// Apply the game's answer to a prepared start and start it: `None` plays nothing, an edit
    /// with invalid `effects` refuses the play, a new identifier is resolved again.
    pub fn finish_prepared(
        &mut self,
        id: super::InstanceId,
        answer: Option<SoundPlay>,
        backend: &mut impl super::Backend,
    ) -> super::PlayResult {
        self.inflight.retain(|i| *i != id);
        let Some(instance) = self.instances.get_mut(&id) else {
            return super::PlayResult::NotStarted;
        };
        let original = instance.play_settings();
        let Some(edited) = answer.filter(|edited| valid_playback_edit(&original, edited)) else {
            self.collect(id);
            return super::PlayResult::NotStarted;
        };
        if edited != original {
            instance.apply_play_settings(edited);
        }
        self.decided.insert(id);
        let result = self.play(id, backend);
        self.decided.remove(&id);
        result
    }

    /// The live refreshes the game fires `play_sound` for (`starting` false): each playing
    /// sound, as asked for. Empty with no listener. The answers replace the earlier ones.
    pub fn live_requests(&mut self) -> Vec<PreparedPlay> {
        self.live_answers.clear();
        if !self.asks_play_sound() {
            return Vec::new();
        }
        self.instance_to_channel
            .iter()
            .filter_map(|(id, _)| {
                Some(PreparedPlay {
                    instance: *id,
                    starting: false,
                    original: self.instances.get(id)?.play_settings(),
                })
            })
            .collect()
    }

    /// The game's answer to a live refresh request; `None` stops the channel.
    pub fn set_live_answer(&mut self, id: super::InstanceId, answer: Option<SoundPlay>) {
        self.live_answers.insert(id, answer);
    }

    /// Live volume, pitch and effects refresh, from the game's answer for this instance (unchanged
    /// when it has none). Other settings cannot be changed on an already allocated channel.
    pub(super) fn playback_update_parameters(
        &self,
        id: super::InstanceId,
    ) -> Option<(f32, f32, mod_api::SoundEffects)> {
        let instance = self.instances.get(&id)?;
        let current = (instance.volume, instance.pitch, instance.effects);
        if !self.asks_play_sound() {
            return Some(current);
        }
        let Some(answer) = self.live_answers.get(&id) else {
            return Some(current);
        };
        let edited = answer.as_ref()?;
        if !valid_playback_edit(&instance.play_settings(), edited) {
            return None;
        }
        Some((edited.volume, edited.pitch, edited.effects))
    }

    /// `play_sound_source` or `play_streaming_source`, for a sound that just started.
    pub(super) fn notify_source_started(
        &self,
        id: super::InstanceId,
        channel: ChannelId,
        streaming: bool,
    ) {
        let Some(source) = &self.loop_context else {
            return;
        };
        let hook = if streaming {
            HookId::PlayStreamingSource
        } else {
            HookId::PlaySoundSource
        };
        if !mod_api::__private::listened(hook) {
            return;
        }
        let Some(instance) = self.instances.get(&id) else {
            return;
        };
        let sound = instance.play_settings();
        let channel = built::ChannelRef { id: channel }.into();
        if streaming {
            native_play_streaming_source(source, &sound, &channel);
        } else {
            native_play_sound_source(source, &sound, &channel);
        }
    }

    /// `sound_engine_load`, once the output is open and the context is current.
    pub fn notify_loaded(&self) {
        let Some(source) = &self.loop_context else {
            return;
        };
        if self.loaded && mod_api::__private::listened(HookId::SoundEngineLoad) {
            native_sound_engine_load(source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound_engine::test_backend::Null;
    use crate::sound_engine::{MASTER, PlayResult, TickContext, Ticking};
    use crate::sound_events::SoundRegistry;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use sunrise_core::random::LegacyRandom;
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};

    /// What the mod heard, in order.
    type Log = Arc<Mutex<Vec<String>>>;

    /// Subscribes to `play_sound` so the engine prepares starts for the game to fire (the game's
    /// own dispatch is tested in the game crate), and hears the source notices, which the engine
    /// still fires itself.
    struct Subscriber(Log);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Subscriber {
        fn play_sound(
            &self,
            _host: &mut dyn mod_api::ClientHost,
            _event: &mut mod_api::PlaySoundEvent<'_>,
        ) {
        }
        fn play_sound_source(
            &self,
            _context: &dyn mod_api::LoopContext,
            event: &mut mod_api::PlaySoundSourceEvent<'_>,
        ) {
            self.0.lock().unwrap().push(format!(
                "source {} {}",
                event.sound.identifier, event.sound.volume
            ));
        }
        fn play_streaming_source(
            &self,
            _context: &dyn mod_api::LoopContext,
            event: &mut mod_api::PlayStreamingSourceEvent<'_>,
        ) {
            self.0
                .lock()
                .unwrap()
                .push(format!("stream {}", event.sound.identifier));
        }
        fn sound_engine_load(
            &self,
            context: &dyn mod_api::LoopContext,
            _event: &mut mod_api::SoundEngineLoadEvent,
        ) {
            self.0
                .lock()
                .unwrap()
                .push(format!("load {:?}", context.side()));
        }
    }

    fn subscribed(log: Log, body: impl FnOnce()) {
        let mods: Vec<mod_api::ModEntry> = vec![mod_api::ModEntry::new(
            "subscriber",
            Box::new(Subscriber(log)),
        )];
        mod_platform::testing::with_mods(mods.into(), body);
    }

    fn engine() -> SoundEngine {
        let mut registry = SoundRegistry::default();
        registry.add_file(
            "minecraft",
            br#"{"block.mute": {"sounds": ["a"]}, "block.quiet": {"sounds": ["a"]},
                 "block.plain": {"sounds": ["a"]}, "music.menu": {"sounds": [{"name": "m", "stream": true}]}}"#,
            &|_, _| true,
        );
        let mut engine = SoundEngine::new(registry);
        let inputs = Arc::new(LoadInputs::new(BTreeMap::new(), std::env::temp_dir()));
        let context = LoadContext::new(DataLoadOrigin::ClientPreload, inputs, []);
        engine.set_loop_context(context.loop_context(LoopSide::Client, []));
        engine
    }

    /// Starts `id` as the game does with a listener: prepared, the hook answered with `edit`'s
    /// change of the sound as asked for, played at once.
    fn start(
        engine: &mut SoundEngine,
        backend: &mut Null,
        id: crate::sound_engine::InstanceId,
        edit: impl Fn(&mut Option<SoundPlay>),
    ) -> PlayResult {
        let prepared = engine
            .begin_play(id)
            .expect("a listener asks for the start");
        assert!(prepared.starting);
        let mut sound = Some(prepared.original.clone());
        edit(&mut sound);
        engine.finish_prepared(prepared.instance, sound, backend)
    }

    /// Answers every live refresh the way the game would.
    fn answer_refreshes(engine: &mut SoundEngine, edit: impl Fn(&mut Option<SoundPlay>)) {
        for prepared in engine.live_requests() {
            assert!(!prepared.starting);
            let mut sound = Some(prepared.original.clone());
            edit(&mut sound);
            engine.set_live_answer(prepared.instance, sound);
        }
    }

    /// Plays `name` as the game does with a listener: prepared, answered, started.
    fn play(
        engine: &mut SoundEngine,
        backend: &mut Null,
        name: &str,
        edit: impl Fn(&mut Option<SoundPlay>),
    ) -> PlayResult {
        let instance = SoundInstance::for_ui(name, 1.0, 1.0, LegacyRandom::new(1));
        let id = engine.add(instance, false);
        start(engine, backend, id, edit)
    }

    fn unchanged(_: &mut Option<SoundPlay>) {}

    #[test]
    fn a_start_is_prepared_without_side_effects_and_the_answer_decides_it() {
        let log = Log::default();
        subscribed(log.clone(), || {
            let (mut engine, mut backend) = (engine(), Null::default());
            engine.notify_loaded();
            let instance =
                SoundInstance::for_ui("minecraft:block.mute", 1.0, 1.0, LegacyRandom::new(1));
            let id = engine.add(instance, false);
            let prepared = engine.begin_play(id).unwrap();
            assert!(backend.volume.is_empty(), "preparing takes no channel");
            assert_eq!(prepared.original.identifier, "minecraft:block.mute");
            // Cancelled: nothing plays.
            let result = engine.finish_prepared(prepared.instance, None, &mut backend);
            assert_eq!(result, PlayResult::NotStarted);
            assert!(backend.volume.is_empty(), "a muted sound takes no channel");
            let retune = |sound: &mut Option<SoundPlay>| {
                let sound = sound.as_mut().unwrap();
                sound.volume = 0.5;
                sound.pitch = 2.0;
            };
            play(&mut engine, &mut backend, "minecraft:block.quiet", retune);
            assert_eq!(backend.volume.values().copied().collect::<Vec<_>>(), [0.5]);
            play(
                &mut engine,
                &mut backend,
                "minecraft:block.plain",
                unchanged,
            );
            play(&mut engine, &mut backend, "minecraft:music.menu", unchanged);
        });
        assert_eq!(
            *log.lock().unwrap(),
            [
                "load Client",
                "source minecraft:block.quiet 0.5",
                "source minecraft:block.plain 1",
                "stream minecraft:music.menu",
            ]
        );
    }

    #[test]
    fn with_no_listener_a_sound_plays_as_asked() {
        let (mut engine, mut backend) = (engine(), Null::default());
        engine.notify_loaded();
        let instance =
            SoundInstance::for_ui("minecraft:block.mute", 1.0, 1.0, LegacyRandom::new(1));
        assert_eq!(
            engine.play_new(instance, false, &mut backend).1,
            PlayResult::Started
        );
        assert!(engine.next_deferred().is_none() && engine.live_requests().is_empty());
    }

    #[test]
    fn live_play_sound_edits_reach_the_channel_without_changing_the_instance() {
        subscribed(Log::default(), || {
            let (mut engine, mut backend) = (engine(), Null::default());
            let instance =
                SoundInstance::for_ui("minecraft:block.plain", 1.0, 1.0, LegacyRandom::new(1));
            let id = engine.add(instance, true);
            start(&mut engine, &mut backend, id, unchanged);
            let channel = engine.channel(id).unwrap();
            answer_refreshes(&mut engine, |sound| {
                let sound = sound.as_mut().unwrap();
                sound.volume = 0.25;
                sound.pitch = 2.0;
            });
            engine.refresh_category_volume(MASTER, &mut backend);
            assert_eq!(backend.volume[&channel], 0.25);
            assert_eq!(backend.pitch[&channel], 2.0);
            assert_eq!(engine.instance(id).unwrap().volume, 1.0);
            assert_eq!(engine.instance(id).unwrap().pitch, 1.0);
            answer_refreshes(&mut engine, |sound| *sound = None);
            engine.refresh_category_volume(MASTER, &mut backend);
            assert!(engine.instance(id).unwrap().stopped);
            engine.instance_mut(id).unwrap().stopped = false;
            assert_eq!(
                start(&mut engine, &mut backend, id, unchanged),
                PlayResult::Started
            );
            answer_refreshes(&mut engine, unchanged);
            engine.refresh_category_volume(MASTER, &mut backend);
            assert!(
                !engine.instance(id).unwrap().stopped,
                "a replay resets a cancelled live edit"
            );
        });
    }

    #[test]
    fn a_mods_effects_reach_the_channel_and_out_of_range_ones_play_nothing() {
        let refused: [fn(&mut mod_api::SoundEffects); 8] = [
            |e| e.gain = f32::NAN,
            |e| e.direct = 1.5,
            |e| e.lowpass = f32::INFINITY,
            |e| e.highpass = -1.0,
            |e| e.room = f32::NEG_INFINITY,
            |e| e.hall = -f32::NAN,
            |e| e.cavern = 1.0001,
            |e| e.reverb_lowpass = 20_001.0,
        ];
        subscribed(Log::default(), || {
            for edit in refused {
                let (mut engine, mut backend) = (engine(), Null::default());
                let result = play(
                    &mut engine,
                    &mut backend,
                    "minecraft:block.plain",
                    |sound| edit(&mut sound.as_mut().unwrap().effects),
                );
                assert_eq!(result, PlayResult::NotStarted);
                assert!(backend.effects.is_empty() && backend.volume.is_empty());
            }
            let (mut engine, mut backend) = (engine(), Null::default());
            let shape = |sound: &mut Option<SoundPlay>| {
                let sound = sound.as_mut().unwrap();
                sound.effects.lowpass = 600.0;
                sound.effects.cavern = 0.5;
            };
            assert_eq!(
                play(&mut engine, &mut backend, "minecraft:block.quiet", shape),
                PlayResult::Started
            );
            let effects = backend.effects.values().copied().collect::<Vec<_>>();
            assert_eq!(effects.len(), 1);
            assert_eq!((effects[0].lowpass, effects[0].cavern), (600.0, 0.5));
            play(
                &mut engine,
                &mut backend,
                "minecraft:block.plain",
                unchanged,
            );
            assert!(
                backend
                    .effects
                    .values()
                    .any(|e| *e == mod_api::SoundEffects::NONE)
            );
        });
    }

    #[test]
    fn a_ticking_sounds_live_refresh_moves_its_effects_and_a_refused_value_stops_it() {
        for refused in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 10.0, 30_000.0] {
            subscribed(Log::default(), || {
                let (mut engine, mut backend) = (engine(), Null::default());
                let mut instance =
                    SoundInstance::for_ui("minecraft:block.plain", 1.0, 1.0, LegacyRandom::new(1));
                instance.ticking = Ticking::Native;
                let id = engine.add(instance, true);
                start(&mut engine, &mut backend, id, unchanged);
                let channel = engine.channel(id).unwrap();
                assert_eq!(backend.effects[&channel], mod_api::SoundEffects::NONE);
                let ctx = TickContext::default();
                answer_refreshes(&mut engine, |sound| {
                    sound.as_mut().unwrap().effects.lowpass = 900.0
                });
                engine.tick(false, &ctx, &mut backend);
                assert_eq!(backend.effects[&channel].lowpass, 900.0);
                answer_refreshes(&mut engine, |sound| {
                    sound.as_mut().unwrap().effects.lowpass = refused
                });
                engine.tick(false, &ctx, &mut backend);
                // Refused, not clamped: the channel keeps what it had and stops.
                assert_eq!(backend.effects[&channel].lowpass, 900.0);
                assert!(engine.instance(id).unwrap().stopped);
            });
        }
    }

    #[test]
    fn finite_negative_play_sound_edits_reach_the_local_sound_mixer() {
        let negative = |sound: &mut Option<SoundPlay>| {
            let sound = sound.as_mut().unwrap();
            sound.volume = -0.5;
            sound.pitch = -1.0;
        };
        for (at_start, category_refresh) in [(true, false), (false, false), (false, true)] {
            subscribed(Log::default(), || {
                let (mut engine, mut backend) = (engine(), Null::default());
                let play_settings = SoundPlay::ui("minecraft:block.plain");
                let mut instance = SoundInstance::from_play(&play_settings, LegacyRandom::new(1));
                instance.ticking = Ticking::Mod { fade: None };
                instance.can_start_silent = true;
                let id = engine.add(instance, true);
                let result = start(&mut engine, &mut backend, id, |sound| {
                    if at_start {
                        negative(sound);
                    }
                });
                let channel = engine.channel(id).unwrap();
                if at_start {
                    assert_eq!(result, PlayResult::StartedSilently);
                } else {
                    assert_eq!(result, PlayResult::Started);
                    assert_eq!(
                        (backend.volume[&channel], backend.pitch[&channel]),
                        (1.0, 1.0)
                    );
                    answer_refreshes(&mut engine, negative);
                    if category_refresh {
                        engine.refresh_category_volume(MASTER, &mut backend);
                    } else {
                        engine.tick(false, &TickContext::default(), &mut backend);
                    }
                    let instance = engine.instance(id).unwrap();
                    assert!(!instance.stopped);
                    assert!(!backend.stopped[&channel]);
                    assert_eq!((instance.volume, instance.pitch), (1.0, 1.0));
                }
                assert_eq!(
                    (backend.volume[&channel], backend.pitch[&channel]),
                    (0.0, 0.5)
                );
            });
        }
    }

    #[test]
    fn invalid_play_sound_edits_refuse_starts_and_stop_live_channels() {
        let poison =
            |sound: &mut Option<SoundPlay>| sound.as_mut().unwrap().effects.gain = f32::NAN;
        for at_start in [true, false] {
            subscribed(Log::default(), || {
                let (mut engine, mut backend) = (engine(), Null::default());
                let instance =
                    SoundInstance::for_ui("minecraft:block.plain", 1.0, 1.0, LegacyRandom::new(1));
                let id = engine.add(instance, true);
                let result = start(&mut engine, &mut backend, id, |sound| {
                    if at_start {
                        poison(sound);
                    }
                });
                if at_start {
                    assert_eq!(result, PlayResult::NotStarted);
                    assert!(backend.volume.is_empty());
                } else {
                    assert_eq!(result, PlayResult::Started);
                    answer_refreshes(&mut engine, poison);
                    engine.refresh_category_volume(MASTER, &mut backend);
                    assert!(engine.instance(id).unwrap().stopped);
                }
            });
        }
    }
}
