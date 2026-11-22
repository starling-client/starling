use super::*;
use crate::game::native_entity_refs::tests::with_client_game;
use mod_api::{AttachmentCodec, AttachmentType, Host};
use sunrise_mod_runtime::attachments::AttachmentSync;
use sunrise_protocol::attachment_sync::Message;

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "mirror",
    name: "value",
    default: || 73,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |c| match c.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(mod_api::AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: true,
};
fn values(game: &Game<'_>) -> Vec<i32> {
    let host: &dyn Host = game;
    let query = host
        .attachment_query("mirror", "value")
        .unwrap()
        .query(&VALUE)
        .unwrap();
    let mut rows = query.iter();
    std::iter::from_fn(|| rows.try_next().unwrap())
        .map(|row| {
            row.with_value(game.attachment_frame_borrow(), |value| *value)
                .unwrap()
        })
        .collect()
}
fn client<'a>(game: &'a mut Game<'_>) -> &'a mut Remote {
    game.remote.as_mut().unwrap()
}
fn sync(game: &mut Game<'_>, holder: Holder, value: i32) {
    game.remote_attachment_sync(Message {
        holder,
        replace: true,
        changes: vec![AttachmentSync {
            namespace: "mirror".into(),
            name: "value".into(),
            value: Some([("n".into(), Tag::Int(value))].into()),
        }],
    });
}
#[test]
#[ignore = "daily"]
fn synced_client_queries_follow_replacement_retirement_and_level_changes() {
    with_client_game(|game| {
        let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "QueryMirror").unwrap();
        let mut spawn = sunrise_protocol::packets::play::SpawnInfo {
            dimension_type: 0,
            dimension: "test:pocket".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(
            &spawn,
            RemoteDimensionEntry::Login,
            sunrise_client::level_loading::Reason::Other,
        );
        client(game).logged_in = true;
        assert!(values(game).is_empty());
        let host: &mut dyn Host = game;
        VALUE.set(host.attachments(), Holder::Level, 73).unwrap();
        assert_eq!(values(game), [73]);
        client(game).attachments.clear();
        let host: &mut dyn Host = game;
        assert_eq!(VALUE.get(host.attachments(), Holder::Level), Ok(73));
        assert_eq!(values(game), [73]);
        client(game).attachments.clear();
        sync(game, Holder::Level, 41);
        assert_eq!(values(game), [41]);
        let captured = game.attachment_query("mirror", "value").unwrap();
        let query = captured.query(&VALUE).unwrap();
        let row = query.iter().try_next().unwrap().unwrap();
        assert_eq!(
            row.holder,
            mod_api::AttachmentHolder::Level(game.level("test:pocket").unwrap())
        );
        let token = row.write_token();
        sync(game, Holder::Level, 42);
        assert_eq!(values(game), [42]);
        assert_eq!(
            row.with_value(game.attachment_frame_borrow(), |value| *value)
                .unwrap(),
            41
        );
        client(game).attachments.remove(&Holder::Level);
        assert!(values(game).is_empty());
        assert_eq!(
            game.attachment_validate_write(token),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        sync(game, Holder::Chunk([1, 2]), 51);
        assert_eq!(values(game), [51]);
        client(game).attachments.retain(|_, _| false);
        assert!(values(game).is_empty());
        sync(game, Holder::Level, 61);
        client(game).attachments.clear();
        assert!(values(game).is_empty());
        sync(game, Holder::Level, 71);
        spawn.dimension = "minecraft:the_nether".into();
        game.enter_remote_dimension(
            &spawn,
            RemoteDimensionEntry::Login,
            sunrise_client::level_loading::Reason::Other,
        );
        assert!(values(game).is_empty());
        assert!(game.remote.as_ref().unwrap().attachments.is_empty());
        sync(game, Holder::Level, 81);
        assert_eq!(values(game), [81]);
    });
}
