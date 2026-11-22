//! The sound engine's hook points for native mods. They fire from the engine itself,
//! where vanilla's `SoundEngine` does, with the client's [`LoopContext`](mod_api::LoopContext): no game is
//! at hand there, and a sound is played from inside the engine's own loops.
use super::{Attenuation, ChannelId, SoundEngine, SoundInstance};
use mod_api::__private::built;
use mod_api::{ChannelRef, HookId, SoundAttenuation, SoundPlay, SoundSource};

/// Hook point for native mods (docs/modding/src/how-to/native-mods.md, declared in `hooks/`
/// with the game's): a sound is about to play (`PlaySoundEvent`, vanilla's `SoundEngine.play`).
/// `original` is the sound as asked for; `sound` starts as a copy of it, and a mod edits it or
/// sets it `None` to play nothing. It is asked before the engine's own checks (library loaded,
/// sound resolved, volume, channel pool), for every sound the client plays, so answer fast.
#[inline(never)]
pub fn native_play_sound(
    context: &sunrise_world::native_load_context::LoopContext,
    instance: u64,
    starting: bool,
    original: &SoundPlay,
    sound: &mut Option<SoundPlay>,
) {
    if !mod_api::__private::listened(HookId::PlaySound) {
        return;
    }
    mod_api::dispatch_play_sound(
        &crate::api_context::LoopReader(context).context(),
        instance,
        starting,
        original,
        sound,
    );
}

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

// Validate a mod's changed parameters at the playback boundary, leaving vanilla
// inputs alone. Starts and refreshes use the same contract.
fn valid_playback_edit(original: &SoundPlay, edited: &SoundPlay) -> bool {
    (edited.volume.to_bits() == original.volume.to_bits()
        || edited.volume.is_finite() && edited.volume >= 0.0)
        && (edited.pitch.to_bits() == original.pitch.to_bits()
            || edited.pitch.is_finite() && edited.pitch > 0.0)
        && (edited.effects == original.effects || edited.effects.valid())
}

#[derive(Clone, Debug)]
pub(super) struct LiveSettings {
    original: SoundPlay,
    edited: Option<SoundPlay>,
}

impl SoundInstance {
    fn play_settings(&self) -> SoundPlay {
        let mut sound = SoundPlay::ui("");
        self.write_play_settings(&mut sound);
        sound
    }
    fn live_play_settings(&self) -> &std::cell::RefCell<LiveSettings> {
        self.live_settings.get_or_init(|| {
            let original = self.play_settings();
            let edited = Some(original.clone());
            std::cell::RefCell::new(LiveSettings { original, edited })
        })
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

    /// `play_sound`: false when a mod took the sound away. With no listener, or on a
    /// side with no context (a server has no sound), it costs this one check.
    pub(super) fn play_sound_allowed(&mut self, id: super::InstanceId) -> bool {
        let Some(source) = &self.loop_context else {
            return true;
        };
        if !mod_api::__private::listened(HookId::PlaySound) {
            return true;
        }
        let Some(instance) = self.instances.get_mut(&id) else {
            return true;
        };
        let original = instance.play_settings();
        let mut sound = Some(original.clone());
        native_play_sound(source, id, true, &original, &mut sound);
        match sound {
            None => false,
            Some(edited) => {
                if !valid_playback_edit(&original, &edited) {
                    return false;
                }
                if edited != original {
                    instance.apply_play_settings(edited);
                }
                let mut cache = instance.live_play_settings().borrow_mut();
                instance.write_play_settings(&mut cache.original);
                if let Some(edited) = cache.edited.as_mut() {
                    instance.write_play_settings(edited);
                } else {
                    cache.edited = Some(cache.original.clone());
                }
                true
            }
        }
    }

    /// Live volume, pitch and effects refresh, through the single play_sound contract. Other
    /// settings cannot be changed on an already allocated channel.
    pub(super) fn playback_update_parameters(
        &self,
        id: super::InstanceId,
    ) -> Option<(f32, f32, mod_api::SoundEffects)> {
        let instance = self.instances.get(&id)?;
        let Some(source) = self
            .loop_context
            .as_ref()
            .filter(|_| mod_api::__private::listened(HookId::PlaySound))
        else {
            return Some((instance.volume, instance.pitch, instance.effects));
        };
        let mut cache = instance.live_play_settings().borrow_mut();
        let LiveSettings { original, edited } = &mut *cache;
        instance.write_play_settings(original);
        instance.write_play_settings(edited.as_mut()?);
        native_play_sound(source, id, false, original, edited);
        let edited = edited.as_ref()?;
        if !valid_playback_edit(original, edited) {
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
    use crate::sound_events::SoundRegistry;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use sunrise_core::random::LegacyRandom;
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};

    /// What the mod heard, in order.
    type Log = Arc<Mutex<Vec<String>>>;

    /// Mutes `minecraft:block.mute`, turns `minecraft:block.quiet` up and a half-tone down.
    struct Tuner(Log);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Tuner {
        fn play_sound(
            &self,
            _context: &dyn mod_api::LoopContext,
            event: &mut mod_api::PlaySoundEvent<'_>,
        ) {
            self.0.lock().unwrap().push(format!(
                "play {} {}",
                event.original.identifier, event.original.volume
            ));
            match event.original.identifier.as_str() {
                "minecraft:block.mute" => *event.sound = None,
                "minecraft:block.quiet" => {
                    if let Some(sound) = event.sound.as_mut() {
                        sound.volume = 0.5;
                        sound.pitch = 2.0;
                    }
                }
                _ => {}
            }
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

    fn play(
        engine: &mut SoundEngine,
        backend: &mut Null,
        name: &str,
    ) -> crate::sound_engine::PlayResult {
        let instance = SoundInstance::for_ui(name, 1.0, 1.0, LegacyRandom::new(1));
        engine.play_new(instance, false, backend).1
    }

    #[test]
    fn a_mod_mutes_retunes_and_hears_the_sources_start() {
        let log: Log = Log::default();
        let mods: Vec<mod_api::ModEntry> = vec![mod_api::ModEntry::new(
            "tuner",
            Box::new(Tuner(log.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            let (mut engine, mut backend) = (engine(), Null::default());
            engine.notify_loaded();
            assert_eq!(
                play(&mut engine, &mut backend, "minecraft:block.mute"),
                crate::sound_engine::PlayResult::NotStarted
            );
            assert!(backend.volume.is_empty(), "a muted sound takes no channel");
            play(&mut engine, &mut backend, "minecraft:block.quiet");
            assert_eq!(backend.volume.values().copied().collect::<Vec<_>>(), [0.5]);
            play(&mut engine, &mut backend, "minecraft:block.plain");
            play(&mut engine, &mut backend, "minecraft:music.menu");
        });
        assert_eq!(
            *log.lock().unwrap(),
            [
                "load Client",
                "play minecraft:block.mute 1",
                "play minecraft:block.quiet 1",
                "source minecraft:block.quiet 0.5",
                "play minecraft:block.plain 1",
                "source minecraft:block.plain 1",
                "play minecraft:music.menu 1",
                "stream minecraft:music.menu",
            ]
        );
    }

    #[test]
    fn with_no_listener_a_sound_plays_as_asked() {
        let (mut engine, mut backend) = (engine(), Null::default());
        engine.notify_loaded();
        assert_eq!(
            play(&mut engine, &mut backend, "minecraft:block.mute"),
            crate::sound_engine::PlayResult::Started
        );
    }
    #[test]
    fn live_play_sound_edits_reach_the_channel_without_changing_the_instance() {
        struct Live(std::sync::atomic::AtomicUsize, std::sync::atomic::AtomicU64);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Live {
            fn play_sound(
                &self,
                _: &dyn mod_api::LoopContext,
                event: &mut mod_api::PlaySoundEvent<'_>,
            ) {
                let n = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                assert_eq!(event.starting, n == 0 || n == 3);
                if n == 0 {
                    self.1
                        .store(event.instance, std::sync::atomic::Ordering::SeqCst);
                } else {
                    assert_eq!(
                        event.instance,
                        self.1.load(std::sync::atomic::Ordering::SeqCst)
                    );
                }
                if n == 1 {
                    let sound = event.sound.as_mut().unwrap();
                    sound.volume = 0.25;
                    sound.pitch = 2.0;
                } else if n == 2 {
                    *event.sound = None;
                }
            }
        }
        let mods = vec![mod_api::ModEntry::new(
            "live",
            Box::new(Live(0.into(), 0.into())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            let (mut engine, mut backend) = (engine(), Null::default());
            let instance =
                SoundInstance::for_ui("minecraft:block.plain", 1.0, 1.0, LegacyRandom::new(1));
            let (id, result) = engine.play_new(instance, true, &mut backend);
            assert_eq!(result, crate::sound_engine::PlayResult::Started);
            let channel = engine.channel(id).unwrap();
            engine.refresh_category_volume(crate::sound_engine::MASTER, &mut backend);
            assert_eq!(backend.volume[&channel], 0.25);
            assert_eq!(backend.pitch[&channel], 2.0);
            assert_eq!(engine.instance(id).unwrap().volume, 1.0);
            assert_eq!(engine.instance(id).unwrap().pitch, 1.0);
            engine.refresh_category_volume(crate::sound_engine::MASTER, &mut backend);
            assert!(engine.instance(id).unwrap().stopped);
            engine.instance_mut(id).unwrap().stopped = false;
            assert_eq!(
                engine.play(id, &mut backend),
                crate::sound_engine::PlayResult::Started
            );
            engine.refresh_category_volume(crate::sound_engine::MASTER, &mut backend);
            assert!(
                !engine.instance(id).unwrap().stopped,
                "a replay resets a cancelled live edit"
            );
        });
    }
    #[test]
    fn a_mods_effects_reach_the_channel_and_out_of_range_ones_play_nothing() {
        struct Shaper;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Shaper {
            fn play_sound(
                &self,
                _: &dyn mod_api::LoopContext,
                event: &mut mod_api::PlaySoundEvent<'_>,
            ) {
                let sound = event.sound.as_mut().unwrap();
                match event.original.identifier.as_str() {
                    "minecraft:block.quiet" => {
                        sound.effects.lowpass = 600.0;
                        sound.effects.cavern = 0.5;
                    }
                    "minecraft:block.mute" => sound.effects.lowpass = 5.0,
                    _ => {}
                }
            }
        }
        /// Sets one refused value at the start of a sound.
        struct Refused(fn(&mut mod_api::SoundEffects));
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Refused {
            fn play_sound(
                &self,
                _: &dyn mod_api::LoopContext,
                event: &mut mod_api::PlaySoundEvent<'_>,
            ) {
                (self.0)(&mut event.sound.as_mut().unwrap().effects);
            }
        }
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
        for edit in refused {
            let mods = vec![mod_api::ModEntry::new("refused", Box::new(Refused(edit)))];
            mod_platform::testing::with_mods(mods.into(), || {
                let (mut engine, mut backend) = (engine(), Null::default());
                assert_eq!(
                    play(&mut engine, &mut backend, "minecraft:block.plain"),
                    crate::sound_engine::PlayResult::NotStarted
                );
                assert!(backend.effects.is_empty() && backend.volume.is_empty());
            });
        }
        let mods = vec![mod_api::ModEntry::new("shaper", Box::new(Shaper))];
        mod_platform::testing::with_mods(mods.into(), || {
            let (mut engine, mut backend) = (engine(), Null::default());
            assert_eq!(
                play(&mut engine, &mut backend, "minecraft:block.quiet"),
                crate::sound_engine::PlayResult::Started
            );
            let effects = backend.effects.values().copied().collect::<Vec<_>>();
            assert_eq!(effects.len(), 1);
            assert_eq!((effects[0].lowpass, effects[0].cavern), (600.0, 0.5));
            play(&mut engine, &mut backend, "minecraft:block.plain");
            assert!(
                backend
                    .effects
                    .values()
                    .any(|e| *e == mod_api::SoundEffects::NONE)
            );
            assert_eq!(
                play(&mut engine, &mut backend, "minecraft:block.mute"),
                crate::sound_engine::PlayResult::NotStarted
            );
        });
    }
    #[test]
    fn a_ticking_sounds_live_refresh_moves_its_effects_and_a_refused_value_stops_it() {
        /// On each live refresh, sets the low-pass the test asks for next.
        struct Follower(Arc<Mutex<f32>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Follower {
            fn play_sound(
                &self,
                _: &dyn mod_api::LoopContext,
                event: &mut mod_api::PlaySoundEvent<'_>,
            ) {
                if !event.starting {
                    event.sound.as_mut().unwrap().effects.lowpass = *self.0.lock().unwrap();
                }
            }
        }
        for refused in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 10.0, 30_000.0] {
            let next = Arc::new(Mutex::new(900.0));
            let mods = vec![mod_api::ModEntry::new(
                "follower",
                Box::new(Follower(next.clone())),
            )];
            mod_platform::testing::with_mods(mods.into(), || {
                let (mut engine, mut backend) = (engine(), Null::default());
                let mut instance =
                    SoundInstance::for_ui("minecraft:block.plain", 1.0, 1.0, LegacyRandom::new(1));
                instance.ticking = crate::sound_engine::Ticking::Native;
                let (id, result) = engine.play_new(instance, true, &mut backend);
                assert_eq!(result, crate::sound_engine::PlayResult::Started);
                let channel = engine.channel(id).unwrap();
                assert_eq!(backend.effects[&channel], mod_api::SoundEffects::NONE);
                let ctx = crate::sound_engine::TickContext::default();
                engine.tick(false, &ctx, &mut backend);
                assert_eq!(backend.effects[&channel].lowpass, 900.0);
                *next.lock().unwrap() = refused;
                engine.tick(false, &ctx, &mut backend);
                // Refused, not clamped: the channel keeps what it had and stops.
                assert_eq!(backend.effects[&channel].lowpass, 900.0);
                assert!(engine.instance(id).unwrap().stopped);
            });
        }
    }
    #[test]
    fn invalid_play_sound_edits_refuse_starts_and_stop_live_channels() {
        struct Poison(bool);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Poison {
            fn play_sound(
                &self,
                _: &dyn mod_api::LoopContext,
                event: &mut mod_api::PlaySoundEvent<'_>,
            ) {
                if event.starting == self.0 {
                    let sound = event.sound.as_mut().unwrap();
                    sound.volume = f32::NAN;
                    sound.pitch = 0.0;
                }
            }
        }
        for at_start in [true, false] {
            let mods = vec![mod_api::ModEntry::new("poison", Box::new(Poison(at_start)))];
            mod_platform::testing::with_mods(mods.into(), || {
                let (mut engine, mut backend) = (engine(), Null::default());
                let instance =
                    SoundInstance::for_ui("minecraft:block.plain", 1.0, 1.0, LegacyRandom::new(1));
                let (id, result) = engine.play_new(instance, true, &mut backend);
                if at_start {
                    assert_eq!(result, crate::sound_engine::PlayResult::NotStarted);
                    assert!(backend.volume.is_empty());
                } else {
                    assert_eq!(result, crate::sound_engine::PlayResult::Started);
                    engine.refresh_category_volume(crate::sound_engine::MASTER, &mut backend);
                    assert!(engine.instance(id).unwrap().stopped);
                }
            });
        }
    }
}
