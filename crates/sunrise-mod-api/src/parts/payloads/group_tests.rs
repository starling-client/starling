//! `ToClient`'s group sends against a host double that keeps the recipient lists the engine
//! would give and records every one-player send.
use super::*;
use crate::{
    Backend, BackendMut, ChunkRef, DimensionKey, EntityRef, Host, IPayloadContext, LevelReadError,
    LevelRef, PayloadAccepted, PayloadBytes, PayloadSendError, PayloadToken, PlayerRef, Players,
    ServerHost, ToClient, Wire, WorldEpoch,
};
use core::ops::ControlFlow;
use std::collections::HashMap;

#[derive(Debug, PartialEq, Wire)]
struct Counter {
    counter: u8,
}

fn player(id: u32) -> PlayerRef {
    crate::__private::game::player_ref(EntityRef::new(WorldEpoch::new(1), id), id.into())
}

fn overworld() -> LevelRef {
    LevelRef::new(WorldEpoch::new(1), DimensionKey::OVERWORLD)
}

fn nether() -> LevelRef {
    LevelRef::new(WorldEpoch::new(1), DimensionKey::NETHER)
}

struct Online(Vec<PlayerRef>);
impl Players for Online {
    fn server_players(&self) -> Vec<PlayerRef> {
        self.0.clone()
    }
}

struct Double {
    table: mod_vocab::__private::game::Payloads,
    everyone: Online,
    levels: HashMap<LevelRef, Vec<PlayerRef>>,
    positions: HashMap<u32, [f64; 3]>,
    watchers: Vec<PlayerRef>,
    /// Entity id to the players it is sent to.
    trackers: HashMap<u32, Vec<PlayerRef>>,
    /// Players whose send is refused, as one who left after the list was read.
    refused: Vec<PlayerRef>,
    sent: Vec<(u32, Vec<u8>)>,
}

impl Host for Double {
    fn backend(&self) -> Option<crate::BackendRef<'_>> {
        Some(mod_vocab::__private::parts_installation::backend_ref(self))
    }
}
impl crate::prelude::PayloadTokensHost for Double {
    fn check_payload_token(&self, token: &PayloadToken) -> Result<(), PayloadSendError> {
        self.table.check_token(token)
    }
}
crate::impl_host_areas!(Double, except(payload_tokens));
impl ServerHost for Double {
    fn backend_mut(&mut self) -> Option<crate::BackendMutRef<'_>> {
        Some(mod_vocab::__private::parts_installation::backend_mut_ref(
            self,
        ))
    }
}
impl crate::prelude::ServicesServerHost for Double {
    fn players(&self) -> &dyn Players {
        &self.everyone
    }
}
crate::impl_server_host_areas!(Double, except(services, payloads));
impl Backend for Double {}
impl crate::prelude::LevelEntitiesBackend for Double {
    fn level_visit_online_players(
        &self,
        level: LevelRef,
        visit: &mut dyn FnMut(PlayerRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, LevelReadError> {
        Ok(self
            .levels
            .get(&level)
            .ok_or(LevelReadError::Stale)?
            .iter()
            .copied()
            .try_for_each(visit))
    }
}
impl crate::prelude::LevelChunksBackend for Double {
    fn chunk_watchers(&self, _: ChunkRef) -> Option<Vec<PlayerRef>> {
        Some(self.watchers.clone())
    }
}
impl crate::prelude::EntityIdentityBackend for Double {
    fn entity_trackers(&self, entity: EntityRef) -> Option<Vec<PlayerRef>> {
        self.trackers.get(&entity.id()).cloned()
    }
}
impl crate::prelude::EntityPositionBackend for Double {
    fn entity_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        self.positions.get(&entity.id()).copied()
    }
}
crate::impl_backend_areas!(
    Double,
    except(
        level_entities,
        level_chunks,
        entity_identity,
        entity_position
    )
);
impl BackendMut for Double {}
impl crate::prelude::PayloadsBackendMut for Double {
    fn player_send_payload(
        &mut self,
        player: PlayerRef,
        _: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        if self.refused.contains(&player) {
            return Err(PayloadSendError::StaleEpoch);
        }
        self.sent.push((player.entity().id(), bytes.0));
        Ok(PayloadAccepted::Queued)
    }
}
crate::impl_backend_mut_areas!(Double, except(payloads));
impl crate::prelude::PayloadsServerHost for Double {
    fn players_send_payload(
        &mut self,
        players: &[PlayerRef],
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> u32 {
        players
            .iter()
            .filter(|player| {
                crate::prelude::PayloadsBackendMut::player_send_payload(
                    self,
                    **player,
                    id,
                    bytes.clone(),
                )
                .is_ok()
            })
            .count() as u32
    }
}

fn setup() -> (Double, ToClient<Counter>, ToClient<Counter>) {
    let mut registrar = PartRegistrar::new("test");
    let (down, up_only) = {
        let r = &mut registrar as &mut dyn Registrar;
        let mut payloads = r.payload_handlers();
        let mut view = payloads.registrar("1");
        let down = view.play_to_client_typed(
            PayloadId("test:down".into()),
            |_: Counter, _: &mut dyn IPayloadContext| Ok(()),
        );
        let stale = view.play_to_client_typed(
            PayloadId("test:stale".into()),
            |_: Counter, _: &mut dyn IPayloadContext| Ok(()),
        );
        (down, stale)
    };
    let table: mod_vocab::__private::game::Payloads = registrar
        .parts
        .payloads
        .as_ref()
        .unwrap()
        .as_ref()
        .clone()
        .into();
    // A token of another load: the same claim, which this table never handed out.
    let mut claim = up_only.token().clone();
    claim.load += 1;
    let mut bytes = crate::wire::Writer::new();
    crate::wire::Wire::put(&claim, &mut bytes);
    let forged = crate::wire::Reader::decode(&bytes.into_bytes()).unwrap();
    let (a, b, c, d) = (player(1), player(2), player(3), player(4));
    let double = Double {
        table,
        everyone: Online(vec![a, b, c, d]),
        levels: HashMap::from([(overworld(), vec![a, b, c]), (nether(), vec![d])]),
        positions: HashMap::from([
            (1, [0.0, 64.0, 0.0]),
            (2, [3.0, 64.0, 4.0]),
            (3, [10.0, 64.0, 0.0]),
            (4, [1.0, 64.0, 0.0]),
        ]),
        watchers: vec![b, d],
        trackers: HashMap::from([(2, vec![a, c]), (9, vec![a, b, c])]),
        refused: Vec::new(),
        sent: Vec::new(),
    };
    (double, down, forged)
}

fn recipients(host: &Double) -> Vec<u32> {
    host.sent.iter().map(|(id, _)| *id).collect()
}

#[test]
fn every_group_reaches_its_list_with_the_same_body() {
    let (mut host, down, _) = setup();
    let value = Counter { counter: 7 };
    assert_eq!(down.send_to_all(&mut host, &value), Ok(4));
    assert_eq!(recipients(&host), [1, 2, 3, 4]);
    assert!(host.sent.iter().all(|(_, body)| *body == [7]));
    host.sent.clear();
    assert_eq!(down.send_in_level(&mut host, overworld(), &value), Ok(3));
    assert_eq!(recipients(&host), [1, 2, 3]);
    host.sent.clear();
    // The watchers list is used as given, wherever the players stand.
    let chunk = ChunkRef::new(overworld(), [0, 0]);
    assert_eq!(down.send_tracking_chunk(&mut host, chunk, &value), Ok(2));
    assert_eq!(recipients(&host), [2, 4]);
}

#[test]
fn a_stale_level_has_no_recipients() {
    let (mut host, down, _) = setup();
    let stale = LevelRef::new(WorldEpoch::new(2), DimensionKey::OVERWORLD);
    let value = Counter { counter: 1 };
    assert_eq!(down.send_in_level(&mut host, stale, &value), Ok(0));
    assert_eq!(
        down.send_near(&mut host, stale, [0.0, 64.0, 0.0], 100.0, None, &value),
        Ok(0)
    );
    assert!(host.sent.is_empty());
}

#[test]
fn near_is_strictly_inside_the_radius_in_that_level_and_skips_the_excluded() {
    let (mut host, down, _) = setup();
    let value = Counter { counter: 1 };
    let at = [0.0, 64.0, 0.0];
    // Distances: player 1 is 0, player 2 is 5 (3-4-5), player 3 is 10; player 4 is close
    // but in the nether.
    assert_eq!(
        down.send_near(&mut host, overworld(), at, 5.5, None, &value),
        Ok(2)
    );
    assert_eq!(recipients(&host), [1, 2]);
    host.sent.clear();
    // Exactly on the radius is outside.
    assert_eq!(
        down.send_near(&mut host, overworld(), at, 5.0, None, &value),
        Ok(1)
    );
    assert_eq!(recipients(&host), [1]);
    host.sent.clear();
    assert_eq!(
        down.send_near(&mut host, overworld(), at, 5.5, Some(player(1)), &value),
        Ok(1)
    );
    assert_eq!(recipients(&host), [2]);
}

#[test]
fn a_refused_recipient_is_skipped_and_not_counted() {
    let (mut host, down, _) = setup();
    let value = Counter { counter: 1 };
    host.refused = vec![player(2), player(9)];
    host.everyone.0.push(player(9));
    assert_eq!(down.send_to_all(&mut host, &value), Ok(3));
    assert_eq!(recipients(&host), [1, 3, 4]);
}

#[test]
fn a_stale_token_is_refused_before_anything_is_sent() {
    let (mut host, _, stale) = setup();
    let value = Counter { counter: 1 };
    let chunk = ChunkRef::new(overworld(), [0, 0]);
    let at = [0.0; 3];
    let id = PayloadId("test:stale".into());
    let refused = Err(PayloadSendError::StaleToken(id));
    assert_eq!(stale.send_to_all(&mut host, &value), refused);
    assert_eq!(stale.send_in_level(&mut host, overworld(), &value), refused);
    assert_eq!(
        stale.send_near(&mut host, overworld(), at, 9.0, None, &value),
        refused
    );
    assert_eq!(stale.send_tracking_chunk(&mut host, chunk, &value), refused);
    let entity = EntityRef::new(WorldEpoch::new(1), 9);
    assert_eq!(
        stale.send_tracking_entity(&mut host, entity, true, &value),
        refused
    );
    assert!(host.sent.is_empty());
}

#[test]
fn tracking_an_entity_reaches_its_trackers_and_itself_once_when_asked() {
    let (mut host, down, _) = setup();
    let value = Counter { counter: 1 };
    let entity = |id| EntityRef::new(WorldEpoch::new(1), id);
    // A zombie (entity 9) seen by players 1, 2 and 3.
    assert_eq!(
        down.send_tracking_entity(&mut host, entity(9), false, &value),
        Ok(3)
    );
    assert_eq!(recipients(&host), [1, 2, 3]);
    host.sent.clear();
    // Not a player: and_self adds no one.
    assert_eq!(
        down.send_tracking_entity(&mut host, entity(9), true, &value),
        Ok(3)
    );
    host.sent.clear();
    // Player 2 is tracked by 1 and 3 and is itself added, once, last.
    assert_eq!(
        down.send_tracking_entity(&mut host, entity(2), true, &value),
        Ok(3)
    );
    assert_eq!(recipients(&host), [1, 3, 2]);
    host.sent.clear();
    // A stale entity has no trackers.
    assert_eq!(
        down.send_tracking_entity(&mut host, entity(77), false, &value),
        Ok(0)
    );
    assert!(host.sent.is_empty());
}
