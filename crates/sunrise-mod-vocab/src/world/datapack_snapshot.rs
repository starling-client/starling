//! The registries a server sends its clients: a snapshot of the synchronized registries of one published set, encoded with the
//! wire format, and the checks a client makes before it activates anything. Negotiating which
//! registries a peer gets and carrying the bytes are the game's.
use super::datapack_registry::{
    DatapackEntry, DatapackField, DatapackRegistryDeclaration, DatapackRegistrySync, DatapackValue,
    unbind_vanilla_values, valid_namespaced,
};
use super::datapack_registry_set::{
    DatapackCandidate, DatapackLoadError, DatapackRegistrySet, resolve_references,
};
use crate::wire::{Reader, Wire, Writer};
use std::collections::{BTreeMap, BTreeSet};
use sunrise_core::vanilla_bytes::SERVERBOUND_CUSTOM_PAYLOAD_LIMIT as SERVERBOUND_LIMIT;

/// The most encoded bytes one split snapshot may carry: the 128 MiB default of NeoForge's
/// `NeoForgeLocalConfig.splitPacketMaxSizeLimitMb`, applied in `GenericPacketSplitter.receivedPacket`.
pub const MAX_SNAPSHOT_BYTES: usize = 128 << 20;

/// One entry of a snapshot: its id and one value per field of the registry, in schema order.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a plain pair that the snapshot builds and reads
#[derive(sunrise_mod_api_macros::Wire)]
pub struct DatapackSnapshotEntry {
    /// The entry's id (`addon:fire`).
    pub id: String,
    /// The values, one per field of the registry's schema.
    pub values: Vec<DatapackValue>,
}

/// One registry of a snapshot: its key, the schema the server decoded it with, and its entries
/// sorted by id.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a plain record that the snapshot builds and reads
#[derive(sunrise_mod_api_macros::Wire)]
pub struct DatapackSnapshotRegistry {
    /// The registry's key (`example:spell`).
    pub key: String,
    /// The schema, which the client compares with its own declaration.
    pub fields: Vec<DatapackField>,
    /// The entries, sorted by id.
    pub entries: Vec<DatapackSnapshotEntry>,
    /// Fully resolved tag names and entry identifiers, including known empty tags.
    pub tags: BTreeMap<String, Vec<String>>,
}

/// The synchronized registries of a published set that a peer negotiated.
/// A local commit does not guarantee that this projection fits the transport;
/// failing to send it does not roll back the local set.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a plain record that the snapshot builds and reads
#[derive(sunrise_mod_api_macros::Wire)]
pub struct DatapackSnapshot {
    /// The registries, sorted by key.
    pub registries: Vec<DatapackSnapshotRegistry>,
}

/// Why a snapshot was refused. A refused snapshot activates nothing.
#[derive(Clone, Debug, PartialEq)]
// no wire: the game reports it and disconnects
#[non_exhaustive]
pub enum DatapackSnapshotError {
    /// The bytes are not a valid Wire snapshot, or have bytes left over.
    Malformed,
    /// The received bytes exceed [`MAX_SNAPSHOT_BYTES`].
    TooLarge,
    /// The snapshot has a registry this side did not declare.
    UnknownRegistry {
        /// The registry's key.
        registry: String,
    },
    /// This side declared the registry with a different schema, or the snapshot lists it twice.
    SchemaMismatch {
        /// The registry's key.
        registry: String,
    },
    /// This side declared the registry as required and the snapshot has none of it.
    MissingRegistry {
        /// The registry's key.
        registry: String,
    },
    /// An entry is not a valid id, is listed twice, or has a value its field refuses.
    InvalidEntry {
        /// The registry's key.
        registry: String,
        /// The entry's id.
        entry: String,
    },
    /// A reference names an entry the snapshot does not have.
    Unresolved(Box<DatapackLoadError>),
}

/// Why a server refuses a datapack registry offer: it cannot read the offer, or the
/// client did not offer a required registry with the same schema.
#[derive(Clone, Debug, PartialEq, Eq)]
// no wire: the game turns it into a disconnect reason before any payload
#[non_exhaustive]
pub enum DatapackNegotiationError {
    /// The client did not offer the required registry, or offered it with another schema.
    RequiredRegistry {
        /// The registry's key.
        registry: String,
        /// The mod that declared it.
        mod_id: String,
    },
    /// The client's offer is not readable or exceeds the serverbound custom payload limit.
    UnreadableOffer,
}

impl DatapackRegistrySet {
    /// The snapshot of this set a peer receives: the registries `declarations` list as
    /// synchronized and `accepts` lets through (the game passes what the peer negotiated).
    /// Server-only registries are never in it.
    pub fn snapshot(
        &self,
        declarations: &[DatapackRegistryDeclaration],
        accepts: impl Fn(&DatapackRegistryDeclaration) -> bool,
    ) -> DatapackSnapshot {
        let mut registries: Vec<_> = declarations
            .iter()
            .filter(|declaration| declaration.sync != DatapackRegistrySync::ServerOnly)
            .filter(|declaration| accepts(declaration))
            .filter_map(|declaration| {
                let entries = self.registries.get(&declaration.key)?;
                Some(DatapackSnapshotRegistry {
                    key: declaration.key.clone(),
                    fields: declaration.fields.clone(),
                    tags: self.tags.get(&declaration.key).cloned().unwrap_or_default(),
                    entries: entries
                        .iter()
                        .map(|(id, entry)| DatapackSnapshotEntry {
                            id: id.clone(),
                            values: entry
                                .values
                                .iter()
                                .map(|(_, v)| {
                                    let mut wire_value = v.clone();
                                    unbind_vanilla_values(&mut wire_value);
                                    wire_value
                                })
                                .collect(),
                        })
                        .collect(),
                })
            })
            .collect();
        registries.sort_by(|a, b| a.key.cmp(&b.key));
        DatapackSnapshot { registries }
    }

    /// The snapshot for a client that `offered` these registries (`offer`): a registry goes
    /// out when the client offered the same key with the same schema. A required registry the
    /// client did not offer so refuses it before any payload; an optional one it did not offer
    /// is left out, with nothing sent in its place.
    pub fn snapshot_for(
        &self,
        declarations: &[DatapackRegistryDeclaration],
        offered: &[DatapackRegistryDeclaration],
    ) -> Result<DatapackSnapshot, DatapackNegotiationError> {
        let matches = |declaration: &DatapackRegistryDeclaration| {
            offered
                .iter()
                .any(|o| o.key == declaration.key && o.fields == declaration.fields)
        };
        if let Some(missing) = declarations.iter().find(|declaration| {
            declaration.sync == DatapackRegistrySync::Required && !matches(declaration)
        }) {
            return Err(DatapackNegotiationError::RequiredRegistry {
                registry: missing.key.clone(),
                mod_id: missing.mod_id.clone(),
            });
        }
        Ok(self.snapshot(declarations, matches))
    }

    /// Publishes a snapshot's candidate on a client.
    pub fn received(candidate: DatapackCandidate) -> Self {
        Self {
            registries: candidate.registries,
            tags: candidate.tags,
            received_registries: Some(candidate.synchronized),
        }
    }
}

impl DatapackSnapshot {
    /// Encodes the synchronized registries and their schemas in one serverbound custom payload.
    /// Returns `None` if the offer cannot be encoded within vanilla's
    /// `ServerboundCustomPayloadPacket.MAX_PAYLOAD_SIZE` (32,767 bytes).
    pub fn encode_offer(declarations: &[DatapackRegistryDeclaration]) -> Option<Vec<u8>> {
        let offer = declarations
            .iter()
            .filter(|declaration| declaration.sync != DatapackRegistrySync::ServerOnly);
        let mut writer = Writer::new();
        writer.limit(SERVERBOUND_LIMIT);
        writer.put_len(offer.clone().count());
        for declaration in offer {
            writer.put(declaration);
        }
        writer.error().is_none().then(|| writer.into_bytes())
    }

    /// Reads one serverbound custom payload as an offer, refusing malformed bytes or a payload
    /// larger than vanilla's `ServerboundCustomPayloadPacket.MAX_PAYLOAD_SIZE` (32,767 bytes).
    pub fn decode_offer(
        bytes: &[u8],
    ) -> Result<Vec<DatapackRegistryDeclaration>, DatapackNegotiationError> {
        if bytes.len() > SERVERBOUND_LIMIT {
            return Err(DatapackNegotiationError::UnreadableOffer);
        }
        Reader::decode(bytes).map_err(|_| DatapackNegotiationError::UnreadableOffer)
    }

    /// The Wire bytes, or `None` if the snapshot cannot be encoded within [`MAX_SNAPSHOT_BYTES`].
    pub fn encode(&self) -> Option<Vec<u8>> {
        let mut writer = Writer::new();
        writer.limit(MAX_SNAPSHOT_BYTES);
        self.put(&mut writer);
        writer.error().is_none().then(|| writer.into_bytes())
    }

    /// Reads Wire bytes: excess bytes yield [`DatapackSnapshotError::TooLarge`]; invalid
    /// encodings and trailing bytes yield [`DatapackSnapshotError::Malformed`].
    pub fn decode(bytes: &[u8]) -> Result<Self, DatapackSnapshotError> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(DatapackSnapshotError::TooLarge);
        }
        Reader::decode(bytes).map_err(|_| DatapackSnapshotError::Malformed)
    }

    /// Checks the snapshot against this side's `declarations` and builds the candidate to
    /// activate: every registry must be one this side declared with the same schema, every
    /// value must fit its field, ids must be valid and unique, references and tag members must
    /// resolve, and a registry declared `Required` must be present. Declared registries omitted
    /// from the snapshot (server-only, or optional and not negotiated) are unavailable to client
    /// reads. Encoding and transport limits do not enter into this validation; use [`Self::decode`]
    /// on received bytes first. Nothing is built unless everything passes.
    pub fn candidate(
        &self,
        declarations: &[DatapackRegistryDeclaration],
    ) -> Result<DatapackCandidate, DatapackSnapshotError> {
        let mut registries: BTreeMap<String, BTreeMap<String, DatapackEntry>> = declarations
            .iter()
            .map(|declaration| (declaration.key.clone(), BTreeMap::new()))
            .collect();
        let mut seen = BTreeSet::new();
        for sent in &self.registries {
            let registry = sent.key.clone();
            let declaration = declarations
                .iter()
                .find(|declaration| declaration.key == sent.key)
                .ok_or_else(|| DatapackSnapshotError::UnknownRegistry {
                    registry: registry.clone(),
                })?;
            if !seen.insert(sent.key.as_str())
                || declaration.fields != sent.fields
                || declaration.sync == DatapackRegistrySync::ServerOnly
            {
                return Err(DatapackSnapshotError::SchemaMismatch { registry });
            }
            let entries = registries.get_mut(&sent.key).expect("declared");
            for entry in &sent.entries {
                let invalid = || DatapackSnapshotError::InvalidEntry {
                    registry: registry.clone(),
                    entry: entry.id.clone(),
                };
                if !valid_namespaced(&entry.id)
                    || entry.values.len() != declaration.fields.len()
                    || !declaration
                        .fields
                        .iter()
                        .zip(&entry.values)
                        .all(|(field, value)| field.kind.accepts(value))
                {
                    return Err(invalid());
                }
                let mut values: Vec<(String, DatapackValue)> = declaration
                    .fields
                    .iter()
                    .map(|field| field.name.clone())
                    .zip(entry.values.iter().cloned())
                    .collect();
                for (_, value) in &mut values {
                    unbind_vanilla_values(value);
                }
                if entries
                    .insert(entry.id.clone(), DatapackEntry { values })
                    .is_some()
                {
                    return Err(invalid());
                }
            }
        }
        if let Some(missing) = declarations.iter().find(|declaration| {
            declaration.sync == DatapackRegistrySync::Required
                && !seen.contains(declaration.key.as_str())
        }) {
            return Err(DatapackSnapshotError::MissingRegistry {
                registry: missing.key.clone(),
            });
        }
        resolve_references(declarations, &registries).map_err(DatapackSnapshotError::Unresolved)?;
        let mut candidate = DatapackCandidate {
            registries,
            tags: BTreeMap::new(),
            synchronized: seen.into_iter().map(str::to_owned).collect(),
        };
        for sent in &self.registries {
            candidate
                .bind_tags(&sent.key, sent.tags.clone())
                .map_err(DatapackSnapshotError::Unresolved)?;
        }
        Ok(candidate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::datapack_registry::{DatapackFieldKind, NoVanillaValueDecoder};

    fn spell(sync: DatapackRegistrySync) -> DatapackRegistryDeclaration {
        DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:spell".into(),
            fields: vec![
                DatapackField::required("power", DatapackFieldKind::Int { min: 0, max: 100 }),
                DatapackField::with_default(
                    "next",
                    DatapackFieldKind::Reference {
                        registry: "example:spell".into(),
                    },
                    DatapackValue::Text("addon:fire".into()),
                ),
            ],
            sync,
        }
    }

    fn set(declaration: &DatapackRegistryDeclaration) -> DatapackRegistrySet {
        let files = vec![
            (
                "data/addon/example/spell/fire.json".to_owned(),
                vec![("pack".to_owned(), br#"{"power": 7}"#.to_vec())],
            ),
            (
                "data/addon/example/spell/ice.json".to_owned(),
                vec![(
                    "pack".to_owned(),
                    br#"{"power": 4, "next": "addon:fire"}"#.to_vec(),
                )],
            ),
        ];
        let mut candidate = DatapackCandidate::build(
            std::slice::from_ref(declaration),
            &files,
            &mut NoVanillaValueDecoder,
        )
        .unwrap();
        candidate
            .bind_tags(
                "example:spell",
                BTreeMap::from([
                    ("example:hot".to_owned(), vec!["addon:fire".to_owned()]),
                    ("example:empty".to_owned(), Vec::new()),
                ]),
            )
            .unwrap();
        DatapackRegistrySet::from_candidate(candidate)
    }

    #[test]
    fn resolved_tags_roundtrip_and_refuse_missing_members_atomically() {
        let declaration = spell(DatapackRegistrySync::Required);
        let server = set(&declaration);
        let snapshot = server
            .snapshot_for(
                std::slice::from_ref(&declaration),
                std::slice::from_ref(&declaration),
            )
            .unwrap();
        let received = DatapackSnapshot::decode(&snapshot.encode().unwrap()).unwrap();
        let mut candidate = received
            .candidate(std::slice::from_ref(&declaration))
            .unwrap();
        assert_eq!(
            candidate.entries("example:spell").unwrap(),
            ["addon:fire", "addon:ice"]
        );
        let before = candidate.clone();
        assert!(
            candidate
                .bind_tags(
                    "example:spell",
                    BTreeMap::from([("example:hot".to_owned(), vec!["addon:missing".to_owned()]),])
                )
                .is_err()
        );
        assert_eq!(candidate, before);
        let client = DatapackRegistrySet::received(candidate);
        for set in [&server, &client] {
            assert_eq!(
                set.tag_contains("example:spell", "addon:fire", "example:hot"),
                Some(true)
            );
            assert_eq!(
                set.tag_contains("example:spell", "addon:ice", "example:hot"),
                Some(false)
            );
            assert_eq!(
                set.tag_contains("example:spell", "addon:fire", "example:empty"),
                Some(false)
            );
            assert_eq!(
                set.tag_contains("example:spell", "addon:fire", "example:absent"),
                Some(false)
            );
            assert_eq!(
                set.tag_contains("example:spell", "addon:missing", "example:hot"),
                None
            );
            assert_eq!(
                set.tag_contains("example:absent", "addon:fire", "example:hot"),
                None
            );
        }
        let mut invalid = received;
        invalid.registries[0]
            .tags
            .insert("example:hot".to_owned(), vec!["addon:missing".to_owned()]);
        assert!(
            invalid
                .candidate(std::slice::from_ref(&declaration))
                .is_err()
        );
    }

    #[test]
    fn a_client_reads_what_the_server_sent() {
        let declaration = spell(DatapackRegistrySync::Required);
        let server = DatapackRegistrySet::from_candidate(
            DatapackCandidate::build(
                std::slice::from_ref(&declaration),
                &[(
                    "data/addon/example/spell/fire.json".to_owned(),
                    vec![("pack".to_owned(), br#"{"power": 9}"#.to_vec())],
                )],
                &mut NoVanillaValueDecoder,
            )
            .unwrap(),
        );
        let snapshot = server.snapshot(std::slice::from_ref(&declaration), |_| true);
        let bytes = snapshot.encode().unwrap();
        let received = DatapackSnapshot::decode(&bytes).unwrap();
        assert_eq!(received, snapshot);
        let candidate = received
            .candidate(std::slice::from_ref(&declaration))
            .unwrap();
        let client = DatapackRegistrySet::received(candidate);
        assert_eq!(
            client
                .get("example:spell", "addon:fire")
                .unwrap()
                .get("power"),
            Some(&DatapackValue::Int(9))
        );
    }

    #[test]
    fn server_only_and_unaccepted_registries_are_not_sent() {
        let only = spell(DatapackRegistrySync::ServerOnly);
        assert!(
            set(&only)
                .snapshot(std::slice::from_ref(&only), |_| true)
                .registries
                .is_empty()
        );
        let optional = spell(DatapackRegistrySync::Optional);
        assert!(
            set(&optional)
                .snapshot(std::slice::from_ref(&optional), |_| false)
                .registries
                .is_empty()
        );
    }

    #[test]
    fn a_client_refuses_a_snapshot_that_does_not_match_its_declarations() {
        let declaration = spell(DatapackRegistrySync::Required);
        let snapshot = set(&declaration).snapshot(std::slice::from_ref(&declaration), |_| true);
        // A registry this side never declared.
        assert_eq!(
            snapshot.candidate(&[]).unwrap_err(),
            DatapackSnapshotError::UnknownRegistry {
                registry: "example:spell".into()
            }
        );
        // A different schema.
        let mut other = declaration.clone();
        other.fields.pop();
        assert_eq!(
            snapshot.candidate(&[other]).unwrap_err(),
            DatapackSnapshotError::SchemaMismatch {
                registry: "example:spell".into()
            }
        );
        // A required registry the snapshot lacks.
        let empty = DatapackSnapshot {
            registries: Vec::new(),
        };
        assert_eq!(
            empty
                .candidate(std::slice::from_ref(&declaration))
                .unwrap_err(),
            DatapackSnapshotError::MissingRegistry {
                registry: "example:spell".into()
            }
        );
        // An optional one it lacks stays empty.
        let optional = spell(DatapackRegistrySync::Optional);
        let candidate = empty.candidate(std::slice::from_ref(&optional)).unwrap();
        let client = DatapackRegistrySet::received(candidate);
        assert_eq!(client.entries("example:spell").unwrap().next(), None);
    }

    #[test]
    fn a_client_refuses_bad_entries_and_dangling_references() {
        let declaration = spell(DatapackRegistrySync::Required);
        let good = set(&declaration).snapshot(std::slice::from_ref(&declaration), |_| true);
        let refuses = |change: &dyn Fn(&mut DatapackSnapshot)| {
            let mut snapshot = good.clone();
            change(&mut snapshot);
            snapshot
                .candidate(std::slice::from_ref(&declaration))
                .unwrap_err()
        };
        // A value past its field's bound.
        assert!(matches!(
            refuses(&|s| s.registries[0].entries[0].values[0] = DatapackValue::Int(101)),
            DatapackSnapshotError::InvalidEntry { .. }
        ));
        // The wrong number of values.
        assert!(matches!(
            refuses(&|s| {
                s.registries[0].entries[0].values.pop();
            }),
            DatapackSnapshotError::InvalidEntry { .. }
        ));
        // A bad id, and an id listed twice.
        assert!(matches!(
            refuses(&|s| s.registries[0].entries[0].id = "NoColon".into()),
            DatapackSnapshotError::InvalidEntry { .. }
        ));
        assert!(matches!(
            refuses(&|s| {
                let first = s.registries[0].entries[0].clone();
                s.registries[0].entries.push(first);
            }),
            DatapackSnapshotError::InvalidEntry { .. }
        ));
        // A reference to an entry the snapshot does not have.
        assert!(matches!(
            refuses(&|s| {
                s.registries[0].entries[1].values[1] = DatapackValue::Text("addon:gone".into())
            }),
            DatapackSnapshotError::Unresolved(_)
        ));
        // The same registry twice.
        assert!(matches!(
            refuses(&|s| {
                let first = s.registries[0].clone();
                s.registries.push(first);
            }),
            DatapackSnapshotError::SchemaMismatch { .. }
        ));
    }

    #[test]
    fn a_server_sends_a_client_only_what_it_offered() {
        let required = spell(DatapackRegistrySync::Required);
        let server = set(&required);
        let declarations = std::slice::from_ref(&required);
        let offer = DatapackSnapshot::encode_offer(declarations).unwrap();
        let offered = DatapackSnapshot::decode_offer(&offer).unwrap();
        assert_eq!(offered, declarations);
        let snapshot = server.snapshot_for(declarations, &offered).unwrap();
        assert_eq!(snapshot.registries.len(), 1);
        // A client without the mod, or with another schema, is refused before any payload.
        let refused = DatapackNegotiationError::RequiredRegistry {
            registry: "example:spell".into(),
            mod_id: "example".into(),
        };
        assert_eq!(server.snapshot_for(declarations, &[]), Err(refused.clone()));
        let mut other = required.clone();
        other.fields.pop();
        assert_eq!(server.snapshot_for(declarations, &[other]), Err(refused));
        // An optional registry the client lacks is left out; the join goes on.
        let optional = spell(DatapackRegistrySync::Optional);
        let snapshot = set(&optional)
            .snapshot_for(std::slice::from_ref(&optional), &[])
            .unwrap();
        assert!(snapshot.registries.is_empty());
        // A server-only registry is not in a client's offer.
        let only = spell(DatapackRegistrySync::ServerOnly);
        let offer = DatapackSnapshot::encode_offer(std::slice::from_ref(&only)).unwrap();
        assert!(DatapackSnapshot::decode_offer(&offer).unwrap().is_empty());
    }

    #[test]
    fn an_offer_that_is_not_one_is_refused() {
        let required = spell(DatapackRegistrySync::Required);
        let mut bytes = DatapackSnapshot::encode_offer(std::slice::from_ref(&required)).unwrap();
        bytes.push(0);
        assert_eq!(
            DatapackSnapshot::decode_offer(&bytes),
            Err(DatapackNegotiationError::UnreadableOffer)
        );
        // An offer can exceed the old registry count and still fit one custom payload.
        let many: Vec<_> = (0..257)
            .map(|n| DatapackRegistryDeclaration {
                key: format!("example:r{n}"),
                fields: Vec::new(),
                ..required.clone()
            })
            .collect();
        let bytes = DatapackSnapshot::encode_offer(&many).unwrap();
        assert_eq!(DatapackSnapshot::decode_offer(&bytes), Ok(many));

        let with_default = |len| DatapackRegistryDeclaration {
            fields: vec![DatapackField::with_default(
                "body",
                DatapackFieldKind::Text { max_len: 32_768 },
                DatapackValue::Text("x".repeat(len)),
            )],
            ..required.clone()
        };
        let overhead = DatapackSnapshot::encode_offer(&[with_default(0)])
            .unwrap()
            .len();
        let exact = [with_default(32_767 - overhead)];
        let bytes = DatapackSnapshot::encode_offer(&exact).unwrap();
        assert_eq!(bytes.len(), 32_767);
        assert_eq!(DatapackSnapshot::decode_offer(&bytes).unwrap(), exact);

        // This is a complete, valid Wire offer, one byte past the vanilla payload limit.
        let oversized = [with_default(32_768 - overhead)];
        assert!(DatapackSnapshot::encode_offer(&oversized).is_none());
        let mut writer = Writer::new();
        writer.put(oversized.as_slice());
        assert_eq!(writer.bytes().len(), 32_768);
        assert_eq!(
            DatapackSnapshot::decode_offer(writer.bytes()),
            Err(DatapackNegotiationError::UnreadableOffer)
        );
    }

    #[test]
    fn bytes_that_are_not_a_snapshot_are_refused() {
        let declaration = spell(DatapackRegistrySync::Required);
        let snapshot = set(&declaration).snapshot(std::slice::from_ref(&declaration), |_| true);
        let mut bytes = snapshot.encode().unwrap();
        bytes.push(0);
        assert_eq!(
            DatapackSnapshot::decode(&bytes),
            Err(DatapackSnapshotError::Malformed)
        );
        bytes.truncate(bytes.len() - 3);
        assert_eq!(
            DatapackSnapshot::decode(&bytes),
            Err(DatapackSnapshotError::Malformed)
        );
        let oversized = vec![0; (128 << 20) + 1];
        assert_eq!(
            DatapackSnapshot::decode(&oversized),
            Err(DatapackSnapshotError::TooLarge)
        );
    }

    #[test]
    fn snapshots_accept_counts_past_the_old_limits() {
        let declarations: Vec<_> = (0..257)
            .map(|n| DatapackRegistryDeclaration {
                mod_id: "example".into(),
                key: format!("example:r{n:03}"),
                fields: Vec::new(),
                sync: DatapackRegistrySync::Required,
            })
            .collect();
        let mut snapshot = DatapackSnapshot {
            registries: declarations
                .iter()
                .map(|declaration| DatapackSnapshotRegistry {
                    key: declaration.key.clone(),
                    fields: Vec::new(),
                    entries: Vec::new(),
                    tags: BTreeMap::new(),
                })
                .collect(),
        };
        let first = &mut snapshot.registries[0];
        first.entries = (0..65_537)
            .map(|n| DatapackSnapshotEntry {
                id: format!("example:e{n:05}"),
                values: Vec::new(),
            })
            .collect();
        first.tags = (0..65_537)
            .map(|n| (format!("example:t{n:05}"), Vec::new()))
            .collect();
        first.tags.insert(
            "example:t00000".into(),
            first.entries.iter().map(|entry| entry.id.clone()).collect(),
        );

        let bytes = snapshot.encode().unwrap();
        let received = DatapackSnapshot::decode(&bytes).unwrap();
        assert_eq!(received, snapshot);
        drop(snapshot);
        let client = DatapackRegistrySet::received(received.candidate(&declarations).unwrap());
        assert!(client.registry_available("example:r256"));
        assert_eq!(client.entries("example:r000").unwrap().len(), 65_537);
        assert_eq!(
            client.tag_contains("example:r000", "example:e00000", "example:t00000"),
            Some(true)
        );
    }

    #[test]
    fn received_registry_availability_distinguishes_omitted_and_empty() {
        let required = spell(DatapackRegistrySync::Required);
        let empty = DatapackRegistryDeclaration {
            key: "example:empty".into(),
            sync: DatapackRegistrySync::Optional,
            ..required.clone()
        };
        let omitted = DatapackRegistryDeclaration {
            key: "example:omitted".into(),
            ..empty.clone()
        };
        let secret = DatapackRegistryDeclaration {
            key: "example:secret".into(),
            sync: DatapackRegistrySync::ServerOnly,
            ..required.clone()
        };
        let mut snapshot = set(&required).snapshot(std::slice::from_ref(&required), |_| true);
        snapshot.registries.push(DatapackSnapshotRegistry {
            key: empty.key.clone(),
            fields: empty.fields.clone(),
            entries: Vec::new(),
            tags: BTreeMap::new(),
        });
        let declarations = [required, empty, omitted, secret];
        let received = DatapackSnapshot::decode(&snapshot.encode().unwrap()).unwrap();
        let client = DatapackRegistrySet::received(received.candidate(&declarations).unwrap());
        assert!(client.registry_available("example:spell"));
        assert!(client.registry_available("example:empty"));
        for registry in ["example:omitted", "example:secret", "example:unknown"] {
            assert!(!client.registry_available(registry));
        }
        for registry in ["example:empty", "example:omitted", "example:secret"] {
            assert_eq!(client.entries(registry).unwrap().len(), 0);
        }
        snapshot
            .registries
            .retain(|registry| registry.key != "example:empty");
        let next = DatapackRegistrySet::received(snapshot.candidate(&declarations).unwrap());
        assert!(!next.registry_available("example:empty"));
        assert!(next.registry_available("example:spell"));
        assert_eq!(next.entries("example:empty").unwrap().next(), None);
        assert!(
            client.registry_available("example:empty"),
            "the previous publication remains intact"
        );
    }

    #[test]
    fn omitted_optional_defaults_do_not_reduce_received_snapshot_capacity() {
        const ENTRY_BYTES: usize = 1 << 16;
        let payload = DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:payload".into(),
            fields: vec![DatapackField::required(
                "body",
                DatapackFieldKind::Text {
                    max_len: ENTRY_BYTES as u32,
                },
            )],
            sync: DatapackRegistrySync::Required,
        };
        let optional = DatapackRegistryDeclaration {
            mod_id: "client".into(),
            key: "client:optional".into(),
            fields: vec![DatapackField::with_default(
                "local_default",
                DatapackFieldKind::Text { max_len: 1 << 14 },
                DatapackValue::Text("d".repeat(1 << 14)),
            )],
            sync: DatapackRegistrySync::Optional,
        };
        let declarations = [payload.clone(), optional];
        assert!(DatapackSnapshot::encode_offer(&declarations).is_some());
        let snapshot = DatapackSnapshot {
            registries: vec![DatapackSnapshotRegistry {
                key: payload.key,
                fields: payload.fields,
                entries: vec![DatapackSnapshotEntry {
                    id: "example:body".into(),
                    values: vec![DatapackValue::Text("x".repeat(ENTRY_BYTES))],
                }],
                tags: BTreeMap::from([("example:hot".into(), vec!["example:body".into()])]),
            }],
        };
        let bytes = snapshot.encode().unwrap();
        assert!(bytes.len() > 32_767, "a snapshot is not an offer");
        let received = DatapackSnapshot::decode(&bytes).unwrap();
        let candidate = received.candidate(&declarations).unwrap();
        let client = DatapackRegistrySet::received(candidate);
        assert!(client.registry_available("example:payload"));
        assert!(!client.registry_available("client:optional"));
        assert_eq!(client.entries("client:optional").unwrap().next(), None);
        let Some(DatapackValue::Text(body)) = client
            .get("example:payload", "example:body")
            .unwrap()
            .get("body")
        else {
            panic!("the accepted payload must be readable");
        };
        assert_eq!(body.len(), ENTRY_BYTES);
        assert!(body.bytes().all(|byte| byte == b'x'));
        assert_eq!(
            client.tag_contains("example:payload", "example:body", "example:hot"),
            Some(true)
        );
    }

    /// A synced registry delivers the nested tree to a client: lists, maps, records and an
    /// absent optional field arrive as the server decoded them, and a client refuses a nested
    /// value its schema does not accept.
    #[test]
    fn a_synced_registry_delivers_the_nested_tree() {
        let declaration = crate::world::datapack_registry::tests::model();
        let server = DatapackRegistrySet::from_candidate(
            DatapackCandidate::build(
                std::slice::from_ref(&declaration),
                &[(
                    "data/addon/example/model/zombie.json".to_owned(),
                    vec![(
                        "pack".to_owned(),
                        br#"{"neoforge:conditions": [], "entity": "zombie",
                            "drops": [{"item": "rotten_flesh", "count": 2}],
                            "tiers": {"elite": {"data_per_kill": 4}, "basic": {"data_per_kill": 1}}}"#
                            .to_vec(),
                    )],
                )],
&mut NoVanillaValueDecoder,
)
            .unwrap(),
        );
        let snapshot = server.snapshot(std::slice::from_ref(&declaration), |_| true);
        let received = DatapackSnapshot::decode(&snapshot.encode().unwrap()).unwrap();
        assert_eq!(received, snapshot);
        let candidate = received
            .candidate(std::slice::from_ref(&declaration))
            .unwrap();
        let client = DatapackRegistrySet::received(candidate);
        assert_eq!(
            client.get("example:model", "addon:zombie").unwrap(),
            server.get("example:model", "addon:zombie").unwrap()
        );
        let entry = client.get("example:model", "addon:zombie").unwrap();
        let drops = entry.get("drops").and_then(DatapackValue::as_list).unwrap();
        assert_eq!(
            drops[0].get("count").and_then(DatapackValue::as_int),
            Some(2)
        );
        assert_eq!(entry.get("color"), None);
        // The map's pairs reach the client in the file's order, past the skipped conditions.
        let tiers: Vec<_> = entry
            .get("tiers")
            .and_then(DatapackValue::as_map)
            .unwrap()
            .iter()
            .map(|(key, _)| key.as_str())
            .collect();
        assert_eq!(tiers, ["elite", "basic"]);
        // A received map naming one key twice is refused.
        let tiers_at = declaration
            .fields
            .iter()
            .position(|field| field.name == "tiers")
            .unwrap();
        let mut twice = received.clone();
        let DatapackValue::Map(pairs) = &mut twice.registries[0].entries[0].values[tiers_at] else {
            panic!("tiers");
        };
        pairs[1].0 = pairs[0].0.clone();
        assert!(matches!(
            twice.candidate(std::slice::from_ref(&declaration)),
            Err(DatapackSnapshotError::InvalidEntry { .. })
        ));
        // A nested count out of its bounds is refused, as a top-level one is.
        let mut bad = received.clone();
        let DatapackValue::List(items) = &mut bad.registries[0].entries[0].values[1] else {
            panic!("drops");
        };
        let DatapackValue::Record(fields) = &mut items[0] else {
            panic!("a drop");
        };
        fields[1].1 = DatapackValue::Int(65);
        assert!(matches!(
            bad.candidate(std::slice::from_ref(&declaration)),
            Err(DatapackSnapshotError::InvalidEntry { .. })
        ));
    }
}
