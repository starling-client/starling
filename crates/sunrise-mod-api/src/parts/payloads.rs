//! Concrete payload authoring; frozen lookup storage stays in the vocabulary.
use super::*;

#[cfg(test)]
#[path = "payloads/group_tests.rs"]
mod group_tests;
#[cfg(test)]
#[path = "payloads/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "payloads/typed_tests.rs"]
mod typed_tests;

/// Validate an explicit namespaced payload channel ID once and refuse the `minecraft`
/// namespace (NeoForge `NetworkRegistry`) and built-in engine `sunrise:` channels.
fn validate_payload_id(id: &str) -> Result<(), String> {
    if !id.contains(':') {
        return Err("payload ids must be explicitly namespaced".to_owned());
    }
    let Some(parsed) = Identifier::parse_namespaced(id) else {
        return Err(format!("invalid identifier {id:?}"));
    };
    if parsed.namespace() == "minecraft" {
        return Err(
            "the \"minecraft\" namespace is reserved for vanilla protocol channels".to_owned(),
        );
    }
    if sunrise_mod_runtime::engine_channels::is_reserved_engine_channel(id) {
        return Err("channel is reserved for an engine service or transport control".to_owned());
    }
    Ok(())
}

fn indices(protocol: PayloadProtocol) -> &'static [usize] {
    match protocol {
        PayloadProtocol::Configuration => &[0],
        PayloadProtocol::Play => &[1],
        PayloadProtocol::Common => &[0, 1],
    }
}

impl PartRegistrar {
    /// Validate and insert one declaration; for a typed one (`schema`, its value type's
    /// fingerprint), also its slot, whose index is returned. A refusal joins the startup refusal list, naming the mod.
    fn declare_payload(
        &mut self,
        mut declaration: PayloadDeclaration,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
        schema: Option<u64>,
    ) -> Option<(PayloadId, u32)> {
        let refused = (|| {
            validate_payload_id(&declaration.id.0)?;
            if declaration.version.trim().is_empty() {
                return Err("payload version must not be blank".to_owned());
            }
            if declaration.delivery == PayloadDelivery::Droppable
                && declaration.protocol != PayloadProtocol::Play
            {
                return Err("a droppable payload exists in play only".to_owned());
            }
            if declaration
                .version
                .contains(crate::PAYLOAD_DROPPABLE_MARKER)
            {
                return Err(format!(
                    "payload version must not contain {:?}, which marks a droppable delivery",
                    crate::PAYLOAD_DROPPABLE_MARKER
                ));
            }
            if declaration.version.contains(crate::PAYLOAD_SCHEMA_MARKER) {
                return Err(format!(
                    "payload version must not contain {:?}, which marks a typed schema",
                    crate::PAYLOAD_SCHEMA_MARKER
                ));
            }
            if declaration.direction != PayloadDirection::ToClient && server.is_none() {
                return Err("serverbound payload requires a server handler".to_owned());
            }
            if declaration.direction == PayloadDirection::ToClient && server.is_some() {
                return Err("clientbound-only payload has a server handler".to_owned());
            }
            if declaration.direction == PayloadDirection::ToServer && client.is_some() {
                return Err("serverbound-only payload has a client handler".to_owned());
            }
            if let Some(payloads) = &self.parts.payloads {
                if let Some(raw) = payloads.raw.get(&declaration.id.0) {
                    let owners: Vec<_> = raw.iter().map(|owner| owner.owner).collect();
                    return Err(format!(
                        "it collides with the raw channel declared by {}",
                        owners.join(", ")
                    ));
                }
                for &index in indices(declaration.protocol) {
                    if let Some(owner) = payloads.phases[index].get(&declaration.id.0) {
                        return Err(format!(
                            "duplicate payload registration for {} in protocol {index}, already registered by {}",
                            declaration.id.0, owner.owner
                        ));
                    }
                }
                // One id has one value type: a typed id lives in one declaration, whose
                // protocol a token names, so no other protocol may declare it.
                let typed =
                    schema.is_some() || payloads.typed.iter().any(|slot| slot.id == declaration.id);
                if typed {
                    for (index, phase) in payloads.phases.iter().enumerate() {
                        if let Some(owner) = phase.get(&declaration.id.0) {
                            return Err(format!(
                                "a typed payload has one declaration, and {} is already registered in protocol {index} by {}",
                                declaration.id.0, owner.owner
                            ));
                        }
                    }
                }
            }
            Ok(())
        })();
        if let Err(reason) = refused {
            self.refused.push(format!(
                "{} registered payload {}: {reason}",
                self.mod_id, declaration.id.0
            ));
            return None;
        }
        let (owner, owner_index) = (self.mod_id, self.mod_index);
        let payloads = self.payloads_table();
        if declaration.delivery == PayloadDelivery::Droppable {
            // The delivery is negotiated with the version: a peer declaring the other one
            // refuses like another version, never a silent loss on a channel its handler
            // counts on.
            declaration
                .version
                .push_str(crate::PAYLOAD_DROPPABLE_MARKER);
            payloads.drops.entry(declaration.id.0.clone()).or_default();
        }
        let slot = schema.map(|schema| {
            declaration.version = crate::typed_payload_version(&declaration.version, schema);
            payloads.typed.push(TypedSlot {
                id: declaration.id.clone(),
                protocol: declaration.protocol,
                direction: declaration.direction,
                schema,
                owner_index,
            });
            (
                declaration.id.clone(),
                u32::try_from(payloads.typed.len() - 1)
                    .expect("fewer typed payloads than u32::MAX"),
            )
        });
        let server_panicked = Arc::default();
        let client_panicked = Arc::default();
        for &index in indices(declaration.protocol) {
            let mut normalized = declaration.clone();
            normalized.protocol = if index == 0 {
                PayloadProtocol::Configuration
            } else {
                PayloadProtocol::Play
            };
            payloads.phases[index].insert(
                declaration.id.0.clone(),
                Registered {
                    declaration: normalized,
                    owner,
                    owner_index,
                    server: server.clone(),
                    client: client.clone(),
                    client_thread: declaration.thread,
                    server_panicked: Arc::clone(&server_panicked),
                    client_panicked: Arc::clone(&client_panicked),
                },
            );
        }
        slot
    }

    /// This load's payload table, made on first use: the one place a table is created, so
    /// every table has a nonzero load of its own and a token of another load is stale.
    fn payloads_table(&mut self) -> &mut Payloads {
        static NEXT_LOAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Arc::make_mut(self.parts.payloads.get_or_insert_with(|| {
            Arc::new(Payloads {
                load: NEXT_LOAD.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                ..Payloads::default()
            })
        }))
    }
}

impl PayloadsRegistrar for PartRegistrar {
    fn add_payload(
        &mut self,
        declaration: PayloadDeclaration,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
    ) {
        self.declare_payload(declaration, server, client, None);
    }

    fn add_typed_payload(
        &mut self,
        declaration: PayloadDeclaration,
        schema: Option<u64>,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
    ) -> PayloadToken {
        let (protocol, direction) = (declaration.protocol, declaration.direction);
        let requested = declaration.id.clone();
        let Some(schema) = schema else {
            self.refused.push(format!(
                "{} declared typed payload {} with an ordinary-only Wire form",
                self.mod_id, requested.0
            ));
            return PayloadToken::new(requested, u32::MAX, 0, protocol, direction, 0);
        };
        let (id, slot, load) = match self.declare_payload(declaration, server, client, Some(schema))
        {
            Some((id, slot)) => (
                id,
                slot,
                self.parts
                    .payloads
                    .as_ref()
                    .map_or(0, |payloads| payloads.load),
            ),
            None => (requested, u32::MAX, 0),
        };
        PayloadToken::new(id, slot, load, protocol, direction, schema)
    }

    fn add_client_payload_handler(
        &mut self,
        id: &PayloadId,
        thread: HandlerThread,
        handler: Arc<dyn IPayloadHandler>,
    ) {
        let Some(payloads) = self.parts.payloads.as_mut() else {
            self.refused.push(format!(
                "{} supplied a client handler for unknown payload {}",
                self.mod_id, id.0
            ));
            return;
        };
        let mut found = false;
        for phase in &payloads.phases {
            if let Some(entry) = phase.get(&id.0) {
                if entry.owner != self.mod_id {
                    self.refused.push(format!(
                        "{} cannot attach a client handler to {} owned by {}",
                        self.mod_id, id.0, entry.owner
                    ));
                    return;
                }
                if entry.declaration.direction != PayloadDirection::ToServer {
                    found = true;
                    if entry.client.is_some() {
                        self.refused.push(format!(
                            "{} registered a second client payload handler for {}",
                            self.mod_id, id.0
                        ));
                        return;
                    }
                }
            }
        }
        if !found {
            self.refused.push(format!(
                "{} supplied a client handler for unknown or serverbound-only payload {}",
                self.mod_id, id.0
            ));
            return;
        }
        let panicked = Arc::default();
        for phase in &mut Arc::make_mut(payloads).phases {
            if let Some(entry) = phase.get_mut(&id.0)
                && entry.declaration.direction != PayloadDirection::ToServer
            {
                entry.client = Some(handler.clone());
                entry.client_thread = thread;
                entry.client_panicked = Arc::clone(&panicked);
            }
        }
    }

    fn add_raw_payload(&mut self, id: PayloadId, server: Option<Arc<dyn IPayloadHandler>>) {
        let refused = (|| {
            validate_payload_id(&id.0)?;
            let Some(payloads) = &self.parts.payloads else {
                return Ok(());
            };
            for phase in &payloads.phases {
                if let Some(typed) = phase.get(&id.0) {
                    return Err(format!(
                        "it collides with the typed payload registered by {}",
                        typed.owner
                    ));
                }
            }
            if payloads
                .raw
                .get(&id.0)
                .is_some_and(|owners| owners.iter().any(|owner| owner.owner == self.mod_id))
            {
                return Err("it is already declared by this mod".to_owned());
            }
            Ok(())
        })();
        if let Err(reason) = refused {
            self.refused.push(format!(
                "{} declared raw channel {}: {reason}",
                self.mod_id, id.0
            ));
            return;
        }
        let (owner, owner_index) = (self.mod_id, self.mod_index);
        self.payloads_table()
            .raw
            .entry(id.0)
            .or_default()
            .push(RawRegistered {
                owner,
                owner_index,
                server,
                panicked: Arc::default(),
            });
    }
}
