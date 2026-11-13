//! Starling: a tree a saw felled tips over away from the saw before it breaks, each block
//! breaking where it lands. Create breaks a felled tree where it stands (TreeCutter), its drops
//! thrown away from the saw; this is the fall it leaves to the eye. The saw's block entity keeps
//! the falling tree (`Felling`) for the window and the players near to draw; where the world is
//! simulated, its drops wait for it to land.
use super::Game;
use horizon_client::items::ItemStack;
use horizon_client::mesher::Vertex;
use horizon_core::nbt::Tag;
use horizon_core::position::BlockPos;
use horizon_world::registry::{BlockRegistry, BlockStateId};

/// The ticks a felled tree takes to drop onto the log cut from under it, before it tips.
const DROP: f64 = 5.0;

/// A felled tree's block: where it stood, what it was, what it drops, and whether its break
/// shows (`BlockHelper.destroyBlockAs`'s chance, drawn when it was cut).
pub(super) struct Fallen {
    pub(super) pos: [i32; 3],
    pub(super) state: BlockStateId,
    pub(super) drops: Vec<ItemStack>,
    pub(super) burst: bool,
}

/// A tree falling: its blocks where they stood, the log the saw cut, the way it falls (the saw's
/// facing) and when it was cut (game time).
pub(super) struct FallingTree {
    blocks: Vec<([i32; 3], BlockStateId)>,
    cut: [i32; 3],
    toward: [i32; 3],
    since: i64,
}

/// A felled tree where the world is simulated: the saw that keeps it, and its blocks' drops.
pub(super) struct Felled {
    saw: [i32; 3],
    tree: FallingTree,
    fallen: Vec<Fallen>,
}

impl FallingTree {
    /// The cut log's bottom edge away from the saw, which the tree turns about.
    fn pivot(&self) -> glam::DVec3 {
        let c = self.cut.map(f64::from);
        glam::DVec3::new(c[0] + 0.5, c[1], c[2] + 0.5) + self.way() * 0.5
    }
    fn way(&self) -> glam::DVec3 {
        glam::DVec3::new(f64::from(self.toward[0]), 0.0, f64::from(self.toward[2]))
    }
    /// The ticks it takes to fall (dropping onto the cut first), and how far it turns: until
    /// its lowest block in front meets the ground, at least a third of a turn, at most a quarter.
    fn fall(&self) -> (f64, f64) {
        let (pivot, way) = (self.pivot(), self.way());
        let mut height: f64 = 1.0;
        let mut angle = std::f64::consts::FRAC_PI_2;
        for (pos, _) in &self.blocks {
            let corner = glam::DVec3::from_array(pos.map(f64::from)) - pivot - glam::DVec3::Y;
            // The block's lowest edge farthest ahead: it meets the ground first.
            let ahead = corner.dot(way) + way.x.max(0.0) + way.z.max(0.0);
            height = height.max(corner.y + 1.0);
            if ahead > 0.0 && corner.y >= 1.0 {
                angle = angle.min(corner.y.atan2(ahead));
            }
        }
        // A taller tree takes longer, as a falling pole does (its time grows with the root of
        // its height).
        (
            DROP + 8.0 + 8.0 * height.sqrt(),
            angle.max(std::f64::consts::FRAC_PI_3),
        )
    }

    /// Where it is `t` ticks after it was cut: dropping the block it lost, then turning about
    /// its pivot, up towards where it falls, faster as it goes, as a falling tree.
    fn turn(&self, t: f64) -> glam::DMat4 {
        if t < DROP {
            let f = t / DROP;
            return glam::DMat4::from_translation(glam::DVec3::NEG_Y * f * f);
        }
        let (ticks, angle) = self.fall();
        let f = ((t - DROP) / (ticks - DROP)).clamp(0.0, 1.0);
        let pivot = self.pivot();
        let axis = glam::DVec3::Y.cross(self.way()).normalize();
        glam::DMat4::from_translation(pivot)
            * glam::DMat4::from_axis_angle(axis, angle * f * f)
            * glam::DMat4::from_translation(-pivot - glam::DVec3::Y)
    }

    fn tag(&self, registry: &BlockRegistry) -> Tag {
        let blocks = self
            .blocks
            .iter()
            .filter_map(|(p, s)| {
                Some(Tag::compound([
                    ("X", Tag::Int(p[0])),
                    ("Y", Tag::Int(p[1])),
                    ("Z", Tag::Int(p[2])),
                    ("State", registry.state(*s)?.to_nbt()),
                ]))
            })
            .collect();
        Tag::compound([
            ("Blocks", Tag::List(blocks)),
            ("Cut", Tag::IntArray(self.cut.to_vec())),
            ("Toward", Tag::IntArray(self.toward.to_vec())),
            ("Since", Tag::Long(self.since)),
        ])
    }

    fn read(tag: &Tag, registry: &BlockRegistry) -> Option<Self> {
        let int = |t: &Tag, k: &str| match t.get(k) {
            Some(Tag::Int(v)) => *v,
            _ => 0,
        };
        let three = |k: &str| match tag.get(k) {
            Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
            _ => None,
        };
        let Some(Tag::List(list)) = tag.get("Blocks") else {
            return None;
        };
        let blocks = list
            .iter()
            .filter_map(|b| {
                let state = horizon_world::chunk::BlockState::from_nbt(b.get("State")?).ok()?;
                Some((
                    [int(b, "X"), int(b, "Y"), int(b, "Z")],
                    registry.id(&state)?,
                ))
            })
            .collect();
        Some(FallingTree {
            blocks,
            cut: three("Cut")?,
            toward: three("Toward")?,
            since: match tag.get("Since") {
                Some(Tag::Long(v)) => *v,
                _ => 0,
            },
        })
    }
}

impl Game<'_> {
    /// The tree the saw at `saw` cut at `cut`, falling `toward` its facing: kept in the saw's
    /// block entity, its drops here until it lands.
    pub(super) fn start_falling_tree(
        &mut self,
        saw: [i32; 3],
        cut: [i32; 3],
        toward: [i32; 3],
        fallen: Vec<Fallen>,
    ) {
        let tree = FallingTree {
            blocks: fallen.iter().map(|b| (b.pos, b.state)).collect(),
            cut,
            toward,
            since: self.world.game_time,
        };
        if let Some((_, mut e)) = self.machine_entity(saw) {
            e.felling = Some(tree.tag(self.pack.assets.registry));
            self.set_machine_entity(saw, &e);
        }
        self.machines.falling.push(Felled { saw, tree, fallen });
    }

    /// The trees falling: each lands when its time is up, its blocks breaking where they lie,
    /// their drops there, and its saw lets it go.
    pub(super) fn tick_falling_trees(&mut self) -> super::Result<()> {
        let now = self.world.game_time;
        let (landed, falling) = std::mem::take(&mut self.machines.falling)
            .into_iter()
            .partition::<Vec<_>, _>(|f| (now - f.tree.since) as f64 >= f.tree.fall().0);
        self.machines.falling = falling;
        for felled in landed {
            let turn = felled.tree.turn(f64::INFINITY);
            for b in felled.fallen {
                let center = turn
                    .transform_point3(glam::DVec3::from_array(b.pos.map(|v| f64::from(v) + 0.5)));
                if b.burst {
                    let at = center.floor().as_ivec3().to_array();
                    self.level_event_at(2001, at, b.state.0 as i32);
                }
                for stack in b.drops {
                    self.spawn_item(stack, center.to_array(), [0.0, 0.1, 0.0], 0);
                }
            }
            if let Some((_, mut e)) = self.machine_entity(felled.saw)
                && e.felling.is_some()
            {
                e.felling = None;
                self.set_machine_entity(felled.saw, &e);
            }
        }
        self.process_level_events()
    }

    /// The trees the saws in the window are felling, turned as far as each has fallen, tinted
    /// and lit where they stood.
    pub(super) fn falling_tree_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        let registry = self.pack.assets.registry;
        let now = self.world.game_time as f64 + self.clock.fraction();
        for (pos, (_, part)) in &self.kinetics.parts {
            if part.kind != horizon_client::kinetics::KineticPart::Saw {
                continue;
            }
            let Some(tree) = super::spill::saved_entity(&self.world, *pos)
                .and_then(|tag| FallingTree::read(tag.get("Felling")?, registry))
            else {
                continue;
            };
            let t = now - tree.since as f64;
            // One kept past its fall (a world saved as it fell) is not drawn.
            if t >= tree.fall().0 {
                continue;
            }
            let turn = glam::DMat4::from_translation(-camera) * tree.turn(t);
            for (at, state) in &tree.blocks {
                let corner = glam::DVec3::from_array(at.map(f64::from));
                let pose = (turn * glam::DMat4::from_translation(corner)).as_mat4();
                let light = self.world.light(at[0], at[1], at[2]);
                let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
                let seed = BlockPos::new(at[0], at[1], at[2]);
                let tint = self.particle_sprite(*state, *at).map(|(_, tint)| tint);
                let start = out.len();
                self.block_model_vertices(
                    *state,
                    horizon_core::random::position_seed(seed),
                    pose,
                    packed,
                    false,
                    out,
                );
                if let Some(tint) = tint {
                    let tint = horizon_client::mobs::render::abgr(tint);
                    for vertex in &mut out[start..] {
                        vertex.color = horizon_client::hud_layout::multiply(vertex.color, tint);
                    }
                }
            }
        }
    }
}
