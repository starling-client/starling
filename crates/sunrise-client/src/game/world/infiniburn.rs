//! Fire sources and golem heads are resolved when a World receives its dimension facts or new tags.
use super::*;
use sunrise_world::level::data::BlockSet;

impl Game<'_> {
    /// World data-pack reloads rebuild `Items.block_tags`, not the shared `Behaviours`
    /// tables. Both server and client therefore resolve from the items snapshot they play
    /// with, translating block names through the shared state table only at resolution.
    pub(super) fn resolve_level_infiniburn(
        &self,
        facts: &sunrise_world::dimension_type::DimensionFacts,
    ) -> Arc<BlockSet> {
        let name = facts
            .infiniburn
            .strip_prefix('#')
            .unwrap_or(&facts.infiniburn);
        self.resolve_block_tag(name)
    }

    /// `sunrise:golem_heads`, the blocks `CarvedPumpkinBlock.onPlace` also answers for, resolved the
    /// same way: a level reads it per placement, so the set is built once per tag install.
    pub(super) fn resolve_golem_heads(&self) -> Arc<BlockSet> {
        self.resolve_block_tag("sunrise:golem_heads")
    }

    fn resolve_block_tag(&self, name: &str) -> Arc<BlockSet> {
        let mut set = BlockSet::default();
        if let Some(members) = self.items().block_tags.get(name) {
            for member in members {
                if let Some(block) = self.res.behaviours().table.block_by_name(member) {
                    set.insert(block);
                }
            }
        }
        Arc::new(set)
    }

    /// Refreshes every level's fire sources and golem heads, the parked levels included.
    pub(super) fn refresh_infiniburn(&mut self) {
        let golem_heads = self.resolve_golem_heads();
        self.world.golem_heads = Some(golem_heads.clone());
        self.world.infiniburn = Some(self.resolve_level_infiniburn(self.dimension_type()));
        let parked: Vec<_> = self
            .levels
            .parked
            .keys()
            .copied()
            .map(|dim| {
                (
                    dim,
                    self.resolve_level_infiniburn(self.dimension_type_of(dim)),
                )
            })
            .collect();
        for (dim, set) in parked {
            let world = &mut self.levels.parked.get_mut(&dim).unwrap().world;
            world.infiniburn = Some(set);
            world.golem_heads = Some(golem_heads.clone());
        }
    }

    /// An opened world's level becomes current: its `World` was installed after the tables
    /// were built, so its fire sources resolve now, against the tags just installed.
    pub(in crate::game) fn enter_opened_dim(&mut self, dim: Dim) {
        self.dim = dim;
        self.refresh_infiniburn();
    }

    /// The new remote World and its connection's dimension facts are now both installed.
    pub(super) fn finish_remote_level(&mut self) {
        self.refresh_infiniburn();
        self.apply_settings();
    }
}
