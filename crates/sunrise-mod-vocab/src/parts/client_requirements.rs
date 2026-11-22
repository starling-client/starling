// exports: none (reached inside the crate only)
//! What a mod's parts and payloads carry that a joining client must have: the engine refuses a
//! `side = "server"` package that has any of it.
use super::*;
use crate::PayloadDirection;

impl Installed {
    /// The first part (in `owner`'s registration order) that `owner` attached and that subscribes
    /// to a client-side hook: the part's kind and the hook's name.
    pub(crate) fn client_part(&self, owner: &str) -> Option<(&'static str, &'static str)> {
        fn mine<'a, P: ?Sized + 'a>(
            owner: &'a str,
            parts: impl Iterator<Item = &'a Attached<P>> + 'a,
            subscribed: impl Fn(&P) -> &'static [Option<HookId>] + 'a,
        ) -> impl Iterator<Item = (usize, &'static str, &'static [Option<HookId>])> + 'a {
            parts
                .filter(move |attached| attached.mod_id == owner)
                .map(move |attached| {
                    let hooks = subscribed(&*attached.part);
                    (attached.order, attached.type_name, hooks)
                })
        }
        let (keys, tags) = (&self.keys, &self.tags);
        let blocks = keys.blocks.values().flatten();
        let tagged_blocks = tags.blocks.iter().map(|(_, attached)| attached);
        let items = keys.items.values().flatten();
        let tagged_items = tags.items.iter().map(|(_, attached)| attached);
        let subscriptions = mine(owner, blocks.chain(tagged_blocks), |p| p.block_subscribed())
            .chain(mine(owner, keys.block_entities.values().flatten(), |p| {
                p.block_entity_subscribed()
            }))
            .chain(mine(owner, items.chain(tagged_items), |p| {
                p.item_subscribed()
            }))
            .chain(mine(owner, keys.menus.values().flatten(), |p| {
                p.menu_subscribed()
            }))
            .chain(mine(owner, keys.effects.values().flatten(), |p| {
                p.effect_subscribed()
            }))
            .chain(mine(owner, keys.fluids.values().flatten(), |p| {
                p.fluid_subscribed()
            }))
            .chain(mine(owner, keys.entities.values().flatten(), |p| {
                p.entity_subscribed()
            }));
        subscriptions
            .filter_map(|(order, kind, hooks)| {
                crate::runtime::client_hook_name(hooks).map(|hook| (order, kind, hook))
            })
            .min_by_key(|(order, ..)| *order)
            .map(|(_, kind, hook)| (kind, hook))
    }

    /// The first synced attachment declared by `owner` (in sorted `namespace:name` order).
    pub(crate) fn first_synced_attachment(&self, owner: &str) -> Option<String> {
        self.synced_attachments(owner).into_iter().next()
    }

    /// All synced attachments declared by `owner`, in sorted `namespace:name` order.
    pub(crate) fn synced_attachments(&self, owner: &str) -> Vec<String> {
        let mut attachments = Vec::new();
        for (namespace, types) in &self.attachment_types {
            for (name, &sync) in types {
                if !sync {
                    continue;
                }
                let declarer = self
                    .attachment_owners
                    .get(namespace)
                    .and_then(|owners| owners.get(name))
                    .copied()
                    .unwrap_or(namespace.as_str());
                if declarer == owner {
                    attachments.push(format!("{namespace}:{name}"));
                }
            }
        }
        attachments.sort_unstable();
        attachments
    }

    /// Every client-required declaration registered by `owner` during `Hooks::register`,
    /// paired with its display kind. Both `side = "server"` declarers and `side = "both"`
    /// hosts declaring into a `side = "server"` served package's namespace are checked
    /// against this single list.
    pub(crate) fn client_required_declarations(&self, owner: &str) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        for declaration in &self.general_entity_types {
            if declaration.mod_id == owner {
                out.push(("entity type", declaration.id.clone()));
            }
        }
        for declaration in &self.attributes {
            if declaration.mod_id == owner {
                out.push(("attribute", declaration.id.clone()));
            }
        }
        for widening in &self.attribute_widenings {
            if widening.mod_id == owner {
                out.push(("attribute", widening.id.clone()));
            }
        }
        for (id, declarer) in self.custom_stats.iter().zip(&self.custom_stat_owners) {
            if *declarer == owner {
                out.push(("custom stat", id.clone()));
            }
        }
        for attachment in self.synced_attachments(owner) {
            out.push(("synced attachment", attachment));
        }
        for declaration in &self.datapack_registries {
            if declaration.mod_id == owner {
                out.push(("datapack registry", declaration.key.clone()));
            }
        }
        if let Some(payloads) = self.keys.payloads.as_ref() {
            for channel in payloads.client_bound_channels(owner) {
                out.push(("payload", channel.to_owned()));
            }
        }
        out
    }
}

impl Payloads {
    /// The first required channel `owner` declared that the server sends to clients, in
    /// channel order: a client without the mod would be refused at join for it.
    pub(crate) fn client_bound(&self, owner: &str) -> Option<&str> {
        self.phases
            .iter()
            .flat_map(|phase| phase.iter())
            .filter(|(_, entry)| {
                let declaration = &entry.declaration;
                entry.owner == owner
                    && !declaration.optional
                    && declaration.direction != PayloadDirection::ToServer
            })
            .map(|(channel, _)| channel.as_str())
            .min()
    }

    /// All required channels `owner` declared that the server sends to clients, in channel order.
    pub(crate) fn client_bound_channels(&self, owner: &str) -> Vec<&str> {
        let mut channels: Vec<&str> = self
            .phases
            .iter()
            .flat_map(|phase| phase.iter())
            .filter(|(_, entry)| {
                let declaration = &entry.declaration;
                entry.owner == owner
                    && !declaration.optional
                    && declaration.direction != PayloadDirection::ToServer
            })
            .map(|(channel, _)| channel.as_str())
            .collect();
        channels.sort_unstable();
        channels
    }
}
