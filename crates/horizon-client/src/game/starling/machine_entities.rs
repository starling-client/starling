//! Content mods' machines' block entities (Starling; `KineticTable::keeps_entity`): what a depot,
//! a basin or a millstone holds and how far its work is, kept as the block entity tag the chunk
//! saves for blocks Horizon does not model (spill.rs), its id the block's, as Create's block
//! entities keep theirs. A change goes to the players near as vanilla's block entity updates do
//! (`mark_block_entity_updated`), and the window reads the same tag to draw it (kinetics.rs).
//! Breaking the block drops what it holds.
use super::{Game, spill};
use horizon_client::items::ItemStack;
use horizon_client::kinetics::{KineticPart, Machine};
use horizon_core::nbt::Tag;

/// A machine's block entity, as its tag keeps it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct MachineEntity {
    /// Its slots, laid out by its kind ([`slots`]): `Items`.
    pub(super) items: Vec<Option<ItemStack>>,
    /// Its kind's clock (`Timer`): a millstone's ticks to go, a press's run.
    pub(super) timer: i32,
    /// What its kind is doing (`Running`): a press's mode while it presses (0: idle).
    pub(super) running: i32,
    /// The game time it started what it is doing (`Since`), for the window's drawing.
    pub(super) since: i64,
    /// A second clock (`Work`): a mixer's processing ticks.
    pub(super) work: i32,
    /// Its tanks, laid out by its kind ([`Kind::tanks`]): a fluid and how many millibuckets
    /// (`Fluids`).
    pub(super) fluids: Vec<Option<(String, i32)>>,
    /// A belt's items, on its start (`Belt`; belts.rs).
    pub(super) belt: Vec<super::belts::Transported>,
    /// Which of its recipes for the item it holds it took last (`RecipeIndex`): a saw's.
    pub(super) recipe: i32,
    /// The blocks it moves (`Contraption`): a bearing's (bearings.rs).
    pub(super) contraption: Option<Tag>,
    /// An arm's points and work (`Arm`; arms.rs).
    pub(super) arm: Option<Tag>,
    /// The tree a saw felled, falling (`Felling`; falling_trees.rs).
    pub(super) felling: Option<Tag>,
}

/// What a machine is, from its block's state: a declared machine or a kinetic part with work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Depot,
    Basin,
    Belt,
    Burner,
    Millstone,
    Press,
    Mixer,
    Saw,
    Funnel,
    Chute,
    Bearing,
    Arm,
}

impl Kind {
    pub(super) fn of(machine: Option<Machine>, part: Option<KineticPart>) -> Option<Self> {
        Some(match (machine, part) {
            (Some(Machine::Depot), _) => Kind::Depot,
            (Some(Machine::Basin), _) => Kind::Basin,
            (Some(Machine::Belt), _) => Kind::Belt,
            (Some(Machine::Burner), _) => Kind::Burner,
            (Some(Machine::Funnel), _) => Kind::Funnel,
            (Some(Machine::Chute), _) => Kind::Chute,
            (_, Some(KineticPart::Millstone)) => Kind::Millstone,
            (_, Some(KineticPart::Press)) => Kind::Press,
            (_, Some(KineticPart::Mixer)) => Kind::Mixer,
            (_, Some(KineticPart::Saw)) => Kind::Saw,
            (_, Some(KineticPart::Bearing)) => Kind::Bearing,
            (_, Some(KineticPart::Arm)) => Kind::Arm,
            _ => return None,
        })
    }
    /// Its slots: a depot's held stack then its 8 outputs (DepotBehaviour's processing output
    /// buffer), a basin's 9 inputs then 9 outputs, a millstone's input then its 9 outputs, a
    /// saw's input then what it was cut into (ProcessingInventory).
    pub(super) fn slots(self) -> usize {
        match self {
            Kind::Depot => 9,
            Kind::Basin => 18,
            Kind::Millstone => 10,
            Kind::Saw => super::saws::SLOTS,
            Kind::Chute | Kind::Arm => 1,
            Kind::Press
            | Kind::Mixer
            | Kind::Belt
            | Kind::Burner
            | Kind::Funnel
            | Kind::Bearing => 0,
        }
    }
    /// Its tanks: a basin's two inputs then two outputs (its SmartFluidTankBehaviours).
    pub(super) fn tanks(self) -> usize {
        match self {
            Kind::Basin => 4,
            _ => 0,
        }
    }
}

impl MachineEntity {
    /// A fresh one of `kind`, empty.
    pub(super) fn empty(kind: Kind) -> Self {
        MachineEntity {
            items: vec![None; kind.slots()],
            fluids: vec![None; kind.tanks()],
            ..MachineEntity::default()
        }
    }
    fn read(tag: &Tag, slots: usize, tanks: usize) -> Self {
        let int = |t: &Tag, k: &str| match t.get(k) {
            Some(Tag::Int(v)) => i64::from(*v),
            Some(Tag::Long(v)) => *v,
            _ => 0,
        };
        let size = (int(tag, "Size") as usize).max(slots);
        let mut fluids = vec![None; tanks];
        if let Some(Tag::List(list)) = tag.get("Fluids") {
            for t in list {
                let (tank, amount) = (int(t, "Tank") as usize, int(t, "Amount") as i32);
                if let (Some(slot), Some(Tag::String(id))) = (fluids.get_mut(tank), t.get("Fluid"))
                    && amount > 0
                {
                    *slot = Some((id.to_string_lossy(), amount));
                }
            }
        }
        MachineEntity {
            items: horizon_client::save::decode_items(tag.get("Items"), size),
            timer: int(tag, "Timer") as i32,
            running: int(tag, "Running") as i32,
            since: int(tag, "Since"),
            work: int(tag, "Work") as i32,
            fluids,
            belt: super::belts::read_items(tag),
            recipe: int(tag, "RecipeIndex") as i32,
            contraption: tag.get("Contraption").cloned(),
            arm: tag.get("Arm").cloned(),
            felling: tag.get("Felling").cloned(),
        }
    }
    fn tag(&self, id: &str, pos: [i32; 3]) -> Tag {
        let fluids = self
            .fluids
            .iter()
            .enumerate()
            .filter_map(|(tank, f)| {
                let (fluid, amount) = f.as_ref()?;
                Some(Tag::compound([
                    ("Tank", Tag::Int(tank as i32)),
                    ("Fluid", Tag::String(fluid.as_str().into())),
                    ("Amount", Tag::Int(*amount)),
                ]))
            })
            .collect();
        let mut entries = vec![
            ("id", Tag::String(id.into())),
            ("x", Tag::Int(pos[0])),
            ("y", Tag::Int(pos[1])),
            ("z", Tag::Int(pos[2])),
            ("Items", horizon_client::save::encode_items(&self.items)),
            ("Size", Tag::Int(self.items.len() as i32)),
            ("Timer", Tag::Int(self.timer)),
            ("Running", Tag::Int(self.running)),
            ("Since", Tag::Long(self.since)),
            ("Work", Tag::Int(self.work)),
            ("Fluids", Tag::List(fluids)),
            ("Belt", super::belts::items_tag(&self.belt)),
            ("RecipeIndex", Tag::Int(self.recipe)),
        ];
        if let Some(c) = &self.contraption {
            entries.push(("Contraption", c.clone()));
        }
        if let Some(a) = &self.arm {
            entries.push(("Arm", a.clone()));
        }
        if let Some(f) = &self.felling {
            entries.push(("Felling", f.clone()));
        }
        Tag::compound(entries)
    }
    /// What it holds, slot by slot, the empty ones left out.
    pub(super) fn stacks(&self) -> impl Iterator<Item = &ItemStack> {
        self.items.iter().flatten()
    }
}

/// The machine entity of `kind` a world keeps at `pos` (a fresh one without).
pub(super) fn read(
    world: &horizon_client::world::World,
    pos: [i32; 3],
    kind: Kind,
) -> MachineEntity {
    match spill::saved_entity(world, pos) {
        Some(tag) => MachineEntity::read(&tag, kind.slots(), kind.tanks()),
        None => MachineEntity::empty(kind),
    }
}

impl Game<'_> {
    /// The machine at `pos`, when one is there.
    pub(in crate::game) fn machine_kind(&self, pos: [i32; 3]) -> Option<Kind> {
        let table = self.res.kinetics();
        let state = self.world.block(pos[0], pos[1], pos[2]);
        Kind::of(table.machine(state), table.get(state).map(|p| p.kind))
    }

    /// The block entity of the machine at `pos` and its kind, when a machine is there, with the
    /// timer it runs on since its tag was last kept (processing.rs `keep_machine_timer`).
    pub(super) fn machine_entity(&self, pos: [i32; 3]) -> Option<(Kind, MachineEntity)> {
        let kind = self.machine_kind(pos)?;
        let mut entity = read(&self.world, pos, kind);
        if let Some(timer) = self.machines.timers.get(&pos) {
            entity.timer = *timer;
        }
        if let Some(work) = self.machines.works.get(&pos) {
            entity.work = *work;
        }
        Some((kind, entity))
    }

    /// Keeps `entity` as the block entity of the machine at `pos` (nothing when the machine is
    /// gone), saved with its chunk and sent to the players near.
    pub(super) fn set_machine_entity(&mut self, pos: [i32; 3], entity: &MachineEntity) {
        self.machines.timers.remove(&pos);
        self.machines.works.remove(&pos);
        if self.machine_kind(pos).is_none() {
            return;
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let id = self.res.facts().get(state).name.clone();
        spill::put_saved_entity(&mut self.world, pos, entity.tag(&id, pos));
        self.mark_block_entity_updated((pos[0], pos[1], pos[2]));
    }
}

/// Whether a saved block entity is a content mod's machine's (its id is no vanilla one).
pub(super) fn is_machine_tag(tag: &Tag) -> bool {
    matches!(tag.get("id"), Some(Tag::String(id)) if !id.to_string_lossy().starts_with("minecraft:"))
}

/// What a machine's saved block entity holds, every slot: dropped when its block breaks
/// (`ItemHelper.dropContents`).
pub(super) fn held_items(tag: &Tag) -> Vec<Option<ItemStack>> {
    let size = match tag.get("Size") {
        Some(Tag::Int(v)) => (*v).max(0) as usize,
        _ => 0,
    };
    let mut items = horizon_client::save::decode_items(tag.get("Items"), size);
    // A belt's items too.
    items.extend(
        super::belts::read_items(tag)
            .into_iter()
            .map(|i| Some(i.stack)),
    );
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_entity_comes_back_from_its_tag() {
        let entity = MachineEntity {
            items: vec![
                Some(ItemStack::new("minecraft:iron_ingot", 5)),
                None,
                Some(ItemStack::new("create:iron_sheet", 2)),
            ],
            timer: 120,
            running: 2,
            since: 9_000_000_000,
            work: 7,
            fluids: vec![None, Some(("minecraft:water".into(), 250))],
            belt: Vec::new(),
            recipe: 3,
            contraption: Some(Tag::compound([("Angle", Tag::Float(12.5f32.to_bits()))])),
            arm: None,
            felling: Some(Tag::compound([("Since", Tag::Long(42))])),
        };
        let tag = entity.tag("create:depot", [1, 64, -3]);
        assert!(is_machine_tag(&tag));
        assert_eq!(MachineEntity::read(&tag, 3, 2), entity);
        // Read for more slots, the rest are empty; what it holds drops whole.
        assert_eq!(MachineEntity::read(&tag, 9, 4).items.len(), 9);
        assert_eq!(held_items(&tag).iter().flatten().count(), 2);
        let pot = Tag::compound([("id", Tag::String("minecraft:decorated_pot".into()))]);
        assert!(!is_machine_tag(&pot));
    }
}
