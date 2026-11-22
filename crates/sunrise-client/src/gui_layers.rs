//! The HUD's layer order (NeoForge's `GuiLayerManager`): vanilla's layers in the order Sunrise
//! draws them, with native layers inserted and bodies installed once at client startup.

use mod_api::{
    __private::game::{CallbackOwner, discard_registration},
    GuiLayerDraw, GuiLayerPlacement, GuiLayerRegistrar, GuiLayerWrapper, HookId, Identifier,
    content::crash::current_native_mod,
};
use std::sync::OnceLock;

/// Vanilla HUD anchors used by the engine's HUD pass (no camera overlays or debug screen).
#[derive(Clone, Copy)]
pub enum VanillaGuiLayer {
    Crosshair,
    Hotbar,
    Effects,
    BossOverlay,
    PlayerHealth,
    ArmorLevel,
    FoodLevel,
    VehicleHealth,
    AirLevel,
    ContextualInfoBar,
    SelectedItemName,
    SleepOverlay,
    OverlayMessage,
    Title,
    ScoreboardSidebar,
    SubtitleOverlay,
    Chat,
    TabList,
}

impl VanillaGuiLayer {
    /// Vanilla anchors in the layer registrar's bottom-to-top order.
    pub const ALL: [Self; 18] = [
        Self::Crosshair,
        Self::Hotbar,
        Self::Effects,
        Self::BossOverlay,
        Self::PlayerHealth,
        Self::ArmorLevel,
        Self::FoodLevel,
        Self::VehicleHealth,
        Self::AirLevel,
        Self::ContextualInfoBar,
        Self::SelectedItemName,
        Self::SleepOverlay,
        Self::OverlayMessage,
        Self::Title,
        Self::ScoreboardSidebar,
        Self::SubtitleOverlay,
        Self::Chat,
        Self::TabList,
    ];

    /// The vanilla layer ID reported to HUD observers.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Crosshair => "minecraft:crosshair",
            Self::Hotbar => "minecraft:hotbar",
            Self::Effects => "minecraft:effects",
            Self::BossOverlay => "minecraft:boss_overlay",
            Self::PlayerHealth => "minecraft:player_health",
            Self::ArmorLevel => "minecraft:armor_level",
            Self::FoodLevel => "minecraft:food_level",
            Self::VehicleHealth => "minecraft:vehicle_health",
            Self::AirLevel => "minecraft:air_level",
            Self::ContextualInfoBar => "minecraft:contextual_info_bar",
            Self::SelectedItemName => "minecraft:selected_item_name",
            Self::SleepOverlay => "minecraft:sleep_overlay",
            Self::OverlayMessage => "minecraft:overlay_message",
            Self::Title => "minecraft:title",
            Self::ScoreboardSidebar => "minecraft:scoreboard_sidebar",
            Self::SubtitleOverlay => "minecraft:subtitle_overlay",
            Self::Chat => "minecraft:chat",
            Self::TabList => "minecraft:tab_list",
        }
    }
}

// no wire: slots own process-local layer bodies.
/// An installed HUD layer, with an ID and anchor independent of its body.
pub struct GuiLayerSlot {
    id: GuiLayerId,
    pub body: GuiLayerBody,
}

enum GuiLayerId {
    Vanilla(VanillaGuiLayer),
    Native(String),
}

// no wire: native bodies own process-local draw callbacks.
/// The current implementation of one HUD slot.
pub enum GuiLayerBody {
    Vanilla,
    Native {
        owner: Option<CallbackOwner>,
        draw: Box<GuiLayerDraw>,
    },
    Wrapped {
        owner: Option<CallbackOwner>,
        wrap: Box<GuiLayerWrapper>,
        previous: Box<GuiLayerBody>,
    },
}

impl GuiLayerSlot {
    /// The stable slot ID reported by Pre and Post.
    pub fn id(&self) -> &str {
        match &self.id {
            GuiLayerId::Vanilla(layer) => layer.id(),
            GuiLayerId::Native(id) => id,
        }
    }

    /// The engine paint site for a vanilla slot, even when its body is replaced or wrapped.
    pub fn vanilla_anchor(&self) -> Option<VanillaGuiLayer> {
        match &self.id {
            GuiLayerId::Vanilla(layer) => Some(*layer),
            GuiLayerId::Native(_) => None,
        }
    }
}

/// The client startup registrar, seeded with vanilla's layers.
pub struct GuiLayers {
    slots: Vec<GuiLayerSlot>,
}

impl Default for GuiLayers {
    fn default() -> Self {
        Self {
            slots: VanillaGuiLayer::ALL
                .into_iter()
                .map(|layer| GuiLayerSlot {
                    id: GuiLayerId::Vanilla(layer),
                    body: GuiLayerBody::Vanilla,
                })
                .collect(),
        }
    }
}

impl GuiLayerBody {
    /// Drop each callback separately. Detach the tail before running a wrapper's destructor so
    /// another mod destructor cannot run as part of its unwind.
    fn dispose(&mut self, in_listener: bool) {
        loop {
            let (owner, draw, wrap) = match std::mem::replace(self, Self::Vanilla) {
                Self::Vanilla => return,
                Self::Native { owner, draw } => (owner, Some(draw), None),
                Self::Wrapped {
                    owner,
                    wrap,
                    previous,
                } => {
                    *self = *previous;
                    (owner, None, Some(wrap))
                }
            };
            if in_listener {
                // A superseded callback from this batch is still the current listener's work.
                drop((draw, wrap));
            } else {
                // Slots are already published or rejected; cleanup cannot undo another batch.
                discard_registration(
                    (HookId::RegisterGuiLayers, "register_gui_layers"),
                    owner.as_ref().map(CallbackOwner::id),
                    (draw, wrap),
                );
            }
        }
    }
}

enum StagedLayerId {
    Existing(usize),
    Native(String),
}

struct StagedLayer {
    id: StagedLayerId,
    // A Vanilla tail on an existing slot stands for its original body, still in `existing`.
    body: GuiLayerBody,
    // The position of that original body in `retired`, assigned on the first replacement.
    retirement: Option<usize>,
}

struct GuiLayerEdits<'a> {
    existing: &'a [GuiLayerSlot],
    slots: Vec<StagedLayer>,
    obsolete: Vec<GuiLayerBody>,
    retired: Vec<GuiLayerBody>,
}

impl GuiLayerEdits<'_> {
    /// Resolve against the order at this call, including this listener's earlier insertions.
    fn position(&self, id: &str) -> Option<usize> {
        self.slots.iter().position(|slot| match &slot.id {
            StagedLayerId::Existing(at) => self.existing[*at].id() == id,
            StagedLayerId::Native(key) => key == id,
        })
    }

    fn edit_position(&self, id: &str, owner: Option<&str>, action: &str) -> Option<usize> {
        let at = self.position(id);
        if at.is_none() {
            log::warn!(
                "{} cannot {action} HUD layer {id}: it is not registered",
                owner.unwrap_or("unknown mod")
            );
        }
        at
    }
}

impl GuiLayerRegistrar for GuiLayerEdits<'_> {
    fn register(
        &mut self,
        id: String,
        placement: GuiLayerPlacement,
        draw: Box<GuiLayerDraw>,
    ) -> bool {
        if Identifier::parse_namespaced(&id).is_none() {
            log::warn!("HUD layer {id:?} must be <namespace>:<path>");
            return false;
        }
        if self.position(&id).is_some() {
            log::warn!("HUD layer {id} is registered twice; the first is kept");
            return false;
        }
        let at = match &placement {
            GuiLayerPlacement::BelowAll => Some(0),
            GuiLayerPlacement::AboveAll => Some(self.slots.len()),
            GuiLayerPlacement::Below(other) => self.position(other),
            GuiLayerPlacement::Above(other) => self.position(other).map(|i| i + 1),
        };
        let owner = current_native_mod();
        let Some(at) = at else {
            log::warn!(
                "{} cannot register HUD layer {id}: no registered anchor for {placement:?}",
                owner.unwrap_or("unknown mod")
            );
            return false;
        };
        self.slots.insert(
            at,
            StagedLayer {
                id: StagedLayerId::Native(id),
                body: GuiLayerBody::Native {
                    owner: owner.map(CallbackOwner::for_mod),
                    draw,
                },
                retirement: None,
            },
        );
        true
    }

    fn replace_layer(&mut self, id: &str, draw: Box<GuiLayerDraw>) -> bool {
        let owner = current_native_mod();
        let Some(at) = self.edit_position(id, owner, "replace") else {
            return false;
        };
        let slot = &mut self.slots[at];
        if matches!(slot.id, StagedLayerId::Existing(_)) && slot.retirement.is_none() {
            slot.retirement = Some(self.retired.len());
            self.retired.push(GuiLayerBody::Vanilla);
        }
        let old = std::mem::replace(
            &mut slot.body,
            GuiLayerBody::Native {
                owner: owner.map(CallbackOwner::for_mod),
                draw,
            },
        );
        if !matches!(old, GuiLayerBody::Vanilla) {
            self.obsolete.push(old);
        }
        // Keep both the new callback and any undropped tail in the batch if a destructor unwinds.
        while let Some(body) = self.obsolete.last_mut() {
            body.dispose(true);
            self.obsolete.pop();
        }
        true
    }

    fn wrap_layer(&mut self, id: &str, wrap: Box<GuiLayerWrapper>) -> bool {
        let owner = current_native_mod();
        let Some(at) = self.edit_position(id, owner, "wrap") else {
            return false;
        };
        let body = &mut self.slots[at].body;
        let previous = Box::new(std::mem::replace(body, GuiLayerBody::Vanilla));
        *body = GuiLayerBody::Wrapped {
            owner: owner.map(CallbackOwner::for_mod),
            wrap,
            previous,
        };
        true
    }
}

/// The frozen draw order, with a direct index for each vanilla anchor.
pub struct GuiLayerOrder {
    slots: Vec<GuiLayerSlot>,
    positions: [usize; VanillaGuiLayer::ALL.len()],
    has_native_bodies: bool,
}

impl GuiLayers {
    /// Stage one listener's edits and publish them in call order only if it succeeds.
    pub fn register_listener(&mut self, listen: impl FnOnce(&mut dyn GuiLayerRegistrar) -> bool) {
        let mut pending = GuiLayerEdits {
            existing: &self.slots,
            slots: (0..self.slots.len())
                .map(|at| StagedLayer {
                    id: StagedLayerId::Existing(at),
                    body: GuiLayerBody::Vanilla,
                    retirement: None,
                })
                .collect(),
            obsolete: Vec::new(),
            retired: Vec::new(),
        };
        let accepted = listen(&mut pending);
        let GuiLayerEdits {
            slots,
            obsolete,
            mut retired,
            ..
        } = pending;
        if accepted {
            // Insertions never reorder existing slots. Publish the slots without destructors,
            // then retire their old bodies in the order of the listener's first replacements.
            let mut existing = std::mem::take(&mut self.slots).into_iter();
            let mut published = Vec::with_capacity(slots.len());
            for StagedLayer {
                id,
                mut body,
                retirement,
            } in slots
            {
                let id = match id {
                    StagedLayerId::Native(id) => GuiLayerId::Native(id),
                    StagedLayerId::Existing(_) => {
                        let slot = existing.next().expect("staged existing HUD slot");
                        if let Some(retirement) = retirement {
                            retired[retirement] = slot.body;
                        } else {
                            let mut tail = &mut body;
                            while let GuiLayerBody::Wrapped { previous, .. } = tail {
                                tail = previous;
                            }
                            *tail = slot.body;
                        }
                        slot.id
                    }
                };
                published.push(GuiLayerSlot { id, body });
            }
            self.slots = published;
        } else {
            for mut slot in slots {
                slot.body.dispose(false);
            }
        }
        for mut body in obsolete.into_iter().chain(retired) {
            body.dispose(false);
        }
    }

    fn into_order(self) -> GuiLayerOrder {
        let mut positions = [0; VanillaGuiLayer::ALL.len()];
        let mut has_native_bodies = false;
        for (at, slot) in self.slots.iter().enumerate() {
            if let Some(layer) = slot.vanilla_anchor() {
                positions[layer as usize] = at;
            }
            has_native_bodies |= !matches!(slot.body, GuiLayerBody::Vanilla);
        }
        GuiLayerOrder {
            slots: self.slots,
            positions,
            has_native_bodies,
        }
    }
}

impl GuiLayerOrder {
    /// The position of a vanilla anchor, including anchors skipped this frame.
    pub fn position(&self, layer: VanillaGuiLayer) -> usize {
        self.positions[layer as usize]
    }

    /// Every layer, bottom first.
    pub fn slots(&self) -> &[GuiLayerSlot] {
        &self.slots
    }
}

static ORDER: OnceLock<GuiLayerOrder> = OnceLock::new();

/// Publish the client startup registration once, before any HUD draws.
pub fn freeze(layers: GuiLayers) {
    if let Err(order) = ORDER.set(layers.into_order()) {
        log::warn!("HUD layers were registered already; these are left out");
        for mut slot in order.slots {
            slot.body.dispose(false);
        }
    }
}

/// The installed order, if it has a mod's own layer, replacement or wrapper.
pub fn order() -> Option<&'static GuiLayerOrder> {
    ORDER.get().filter(|order| order.has_native_bodies)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_go_where_register_gui_layers_event_inserts_them() {
        use GuiLayerPlacement::*;
        let mut layers = GuiLayers::default();
        layers.register_listener(|layers| {
            for (id, placement) in [
                ("compass:bar", Above("minecraft:hotbar".into())),
                // Below the compass, so between the hotbar and it.
                ("compass:shadow", Below("compass:bar".into())),
                ("compass:first", BelowAll),
                ("compass:last", AboveAll),
                // A second `BelowAll` goes under the first.
                ("compass:very_first", BelowAll),
                ("compass:over_chat", Above("minecraft:chat".into())),
            ] {
                assert!(layers.register(id.into(), placement, Box::new(|_, _| {})));
            }
            true
        });
        let order = layers.into_order();
        let ids: Vec<&str> = order.slots().iter().map(GuiLayerSlot::id).collect();
        assert_eq!(
            ids,
            [
                "compass:very_first",
                "compass:first",
                "minecraft:crosshair",
                "minecraft:hotbar",
                "compass:shadow",
                "compass:bar",
                "minecraft:effects",
                "minecraft:boss_overlay",
                "minecraft:player_health",
                "minecraft:armor_level",
                "minecraft:food_level",
                "minecraft:vehicle_health",
                "minecraft:air_level",
                "minecraft:contextual_info_bar",
                "minecraft:selected_item_name",
                "minecraft:sleep_overlay",
                "minecraft:overlay_message",
                "minecraft:title",
                "minecraft:scoreboard_sidebar",
                "minecraft:subtitle_overlay",
                "minecraft:chat",
                "compass:over_chat",
                "minecraft:tab_list",
                "compass:last",
            ]
        );
        for layer in VanillaGuiLayer::ALL {
            assert_eq!(ids[order.position(layer)], layer.id());
        }
    }

    #[test]
    fn a_layer_that_cannot_be_placed_is_left_out() {
        use GuiLayerPlacement::*;
        let mut layers = GuiLayers::default();
        layers.register_listener(|layers| {
            for (id, placement, accepted) in [
                ("minecraft:hotbar", AboveAll, false),
                ("minecraft:mine", AboveAll, true),
                ("no_namespace", AboveAll, false),
                ("compass:bad space", AboveAll, false),
                ("compass:bar", AboveAll, true),
                ("compass:bar", BelowAll, false),
                // An unknown anchor does not reserve the ID: it can be registered again later.
                ("compass:early", Below("compass:late".into()), false),
                ("compass:late", AboveAll, true),
                ("compass:early", Below("compass:late".into()), true),
                (
                    "compass:lost",
                    Above("minecraft:no_such_layer".into()),
                    false,
                ),
            ] {
                assert_eq!(
                    layers.register(id.into(), placement, Box::new(|_, _| {})),
                    accepted,
                    "registration of {id}"
                );
            }
            true
        });
        let order = layers.into_order();
        let mods: Vec<&str> = order
            .slots()
            .iter()
            .filter(|slot| slot.vanilla_anchor().is_none())
            .map(GuiLayerSlot::id)
            .collect();
        assert_eq!(
            mods,
            [
                "minecraft:mine",
                "compass:bar",
                "compass:early",
                "compass:late"
            ]
        );
        assert_eq!(
            order.slots()[VanillaGuiLayer::ALL.len()].id(),
            "minecraft:mine"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_panicking_hud_registration_cannot_displace_vanilla_or_later_layers() {
        const TEST: &str = "gui_layers::tests::a_panicking_hud_registration_cannot_displace_vanilla_or_later_layers";
        const CHILD: &str = "SUNRISE_HUD_REGISTRAR_DIAGNOSTIC_TEST";
        if !std::env::var_os(CHILD).is_some_and(|value| value == TEST) {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([TEST, "--exact", "--ignored"])
                .env(CHILD, TEST)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("test result: ok. 1 passed;"),
                "{TEST} did not pass in its own process:\n{stdout}{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        use mod_api::__private::game::RegistrationContextNew;
        use mod_api::__private::{HookPanicReport, take_hook_panics};
        use mod_api::{Hooks, LoadedMods, ModEntry, RegisterGuiLayersEvent, RegistrationContext};
        use std::sync::{Arc, Mutex};

        type Trace = Arc<Mutex<Vec<(&'static str, Option<&'static str>)>>>;

        struct PanicOnDrop(&'static str, Trace);
        impl Drop for PanicOnDrop {
            fn drop(&mut self) {
                self.1.lock().unwrap().push((self.0, current_native_mod()));
                panic!("{}", self.0);
            }
        }

        struct ReadOnDrop(LoadedMods, Trace);
        impl Drop for ReadOnDrop {
            fn drop(&mut self) {
                let label = if self.0.get("failing").is_some() {
                    "typed capture"
                } else {
                    "missing package from registration context"
                };
                self.1.lock().unwrap().push((label, current_native_mod()));
            }
        }

        struct Register {
            mod_id: &'static str,
            trace: Trace,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Register {
            fn register_gui_layers(
                &self,
                context: &RegistrationContext,
                event: &mut RegisterGuiLayersEvent<'_>,
            ) {
                use GuiLayerPlacement::{Above, AboveAll};
                let layers = &mut *event.layers;
                let panicking_draw = |label| -> Box<GuiLayerDraw> {
                    let capture = PanicOnDrop(label, self.trace.clone());
                    Box::new(move |_, _| {
                        std::hint::black_box(&capture);
                    })
                };
                match self.mod_id {
                    "retired" => {
                        assert!(layers.register(
                            "hud:earlier".into(),
                            AboveAll,
                            panicking_draw("retired body"),
                        ));
                    }
                    "failing" => {
                        assert!(layers.replace_layer("minecraft:hotbar", Box::new(|_, _| {})));
                        assert!(layers.wrap_layer("minecraft:hotbar", Box::new(|_| {})));
                        assert!(layers.register(
                            "hud:discarded".into(),
                            AboveAll,
                            panicking_draw("discarded inner draw"),
                        ));
                        let wrapper = PanicOnDrop("superseded wrapper", self.trace.clone());
                        assert!(layers.wrap_layer(
                            "hud:discarded",
                            Box::new(move |_| {
                                std::hint::black_box(&wrapper);
                            }),
                        ));
                        let panic = PanicOnDrop("discarded replacement", self.trace.clone());
                        // This capture has only the owned startup metadata, not a Host.
                        let metadata = ReadOnDrop(context.mods().clone(), self.trace.clone());
                        // Dropping the superseded wrapper rejects the batch. Its inner body and
                        // this replacement (both captures) must still be cleaned up separately.
                        let _ = layers.replace_layer(
                            "hud:discarded",
                            Box::new(move |_, _| {
                                std::hint::black_box(&panic);
                                std::hint::black_box(&metadata);
                            }),
                        );
                        self.trace
                            .lock()
                            .unwrap()
                            .push(("replacement returned", current_native_mod()));
                    }
                    "healthy" => {
                        self.trace
                            .lock()
                            .unwrap()
                            .push(("next listener", current_native_mod()));
                        assert!(!layers.register(
                            "hud:stale-anchor".into(),
                            Above("hud:discarded".into()),
                            Box::new(|_, _| {})
                        ));
                        assert!(layers.replace_layer("hud:earlier", Box::new(|_, _| {})));
                        assert!(layers.register(
                            "hud:next".into(),
                            Above("minecraft:hotbar".into()),
                            Box::new(|_, _| {}),
                        ));
                        assert!(layers.register(
                            "hud:after".into(),
                            Above("hud:next".into()),
                            Box::new(|_, _| {})
                        ));
                    }
                    _ => unreachable!(),
                }
            }
        }

        let trace = Trace::default();
        let mods = ["retired", "failing", "healthy"].map(|mod_id| {
            ModEntry::new(
                mod_id,
                Box::new(Register {
                    mod_id,
                    trace: trace.clone(),
                }),
            )
        });
        mod_platform::testing::with_mods(Vec::from(mods).into(), || {
            mod_platform::__private::with_test_packages(&["failing"], || {
                let mut layers = GuiLayers::default();
                mod_api::dispatch_register_gui_layers(&RegistrationContext::new(), |listen| {
                    layers.register_listener(listen);
                });
                let order = layers.into_order();
                let hotbar = order.position(VanillaGuiLayer::Hotbar);
                assert!(matches!(order.slots()[hotbar].body, GuiLayerBody::Vanilla));
                assert_eq!(order.slots()[hotbar + 1].id(), "hud:next");
                assert_eq!(order.slots()[hotbar + 2].id(), "hud:after");
                assert!(
                    !order
                        .slots()
                        .iter()
                        .any(|slot| slot.id() == "hud:discarded")
                );
                let replaced = order.slots().last().unwrap();
                assert_eq!(replaced.id(), "hud:earlier");
                assert!(
                    matches!(&replaced.body, GuiLayerBody::Native { owner: Some(owner), .. }
                    if owner.id() == "healthy")
                );
                assert_eq!(
                    take_hook_panics(),
                    [
                        HookPanicReport {
                            mod_id: "failing",
                            hook: "register_gui_layers"
                        },
                        HookPanicReport {
                            mod_id: "retired",
                            hook: "register_gui_layers"
                        },
                    ]
                );
            });
        });
        let trace = trace.lock().unwrap();
        assert_eq!(trace[0], ("superseded wrapper", Some("failing")));
        // Rust does not specify the relative destruction order of closure captures.
        let mut captures = [trace[1], trace[2]];
        captures.sort_unstable();
        assert_eq!(
            captures,
            [
                ("discarded replacement", Some("failing")),
                ("typed capture", Some("failing")),
            ]
        );
        assert_eq!(
            &trace[3..],
            [
                ("discarded inner draw", Some("failing")),
                ("next listener", Some("healthy")),
                ("retired body", Some("retired")),
            ]
        );
    }

    #[test]
    fn earlier_hud_bodies_retire_in_first_replacement_order() {
        use std::sync::{Arc, Mutex};

        struct RecordDrop(&'static str, Arc<Mutex<Vec<&'static str>>>);
        impl Drop for RecordDrop {
            fn drop(&mut self) {
                self.1.lock().unwrap().push(self.0);
            }
        }

        let drops = Arc::new(Mutex::new(Vec::new()));
        let mut layers = GuiLayers::default();
        layers.register_listener(|layers| {
            for id in ["hud:a", "hud:b"] {
                let capture = RecordDrop(id, Arc::clone(&drops));
                assert!(layers.register(
                    id.into(),
                    GuiLayerPlacement::AboveAll,
                    Box::new(move |_, _| {
                        std::hint::black_box(&capture);
                    }),
                ));
            }
            true
        });
        layers.register_listener(|layers| {
            for id in ["hud:b", "hud:a", "hud:b"] {
                assert!(layers.replace_layer(id, Box::new(|_, _| {})));
            }
            assert!(drops.lock().unwrap().is_empty());
            true
        });
        assert_eq!(*drops.lock().unwrap(), ["hud:b", "hud:a"]);
        let order = layers.into_order();
        let slots = &order.slots()[VanillaGuiLayer::ALL.len()..];
        assert_eq!(
            slots.iter().map(GuiLayerSlot::id).collect::<Vec<_>>(),
            ["hud:a", "hud:b"]
        );
    }

    #[test]
    fn no_registration_leaves_vanilla_alone() {
        let order = GuiLayers::default().into_order();
        assert!(
            order
                .slots()
                .iter()
                .all(|slot| matches!(slot.body, GuiLayerBody::Vanilla))
        );
        for (at, layer) in VanillaGuiLayer::ALL.into_iter().enumerate() {
            assert_eq!(order.position(layer), at);
            assert_eq!(order.slots()[at].id(), layer.id());
        }
    }

    #[test]
    fn only_existing_vanilla_layers_are_registration_anchors() {
        let staged_ids = [
            "camera_overlays",
            "after_camera_decorations",
            "contextual_info_bar_background",
            "experience_level",
            "spectator_tooltip",
            "demo_overlay",
        ];
        let mut layers = GuiLayers::default();
        layers.register_listener(|layers| {
            for id in staged_ids {
                assert!(!layers.register(
                    "compass:bar".into(),
                    GuiLayerPlacement::Above(format!("minecraft:{id}")),
                    Box::new(|_, _| {}),
                ));
            }
            true
        });
        let order = layers.into_order();
        assert_eq!(order.slots().len(), VanillaGuiLayer::ALL.len());
        assert!(
            order
                .slots()
                .iter()
                .all(|slot| slot.vanilla_anchor().is_some())
        );
    }
}
