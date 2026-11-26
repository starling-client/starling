use super::*;
use crate::{buf::NBT_QUOTA, packets::configuration};

fn large_component() -> Tag {
    // Enough short text fragments to exceed the generic byte quota without
    // exceeding the individual NBT string limit.
    let fragment = "x".repeat(16 * 1024);
    Tag::compound([
        ("text", Tag::String("".into())),
        (
            "extra",
            Tag::List(vec![
                Tag::String(fragment.as_str().into());
                NBT_QUOTA / fragment.len()
            ]),
        ),
    ])
}

fn assert_trusted_limits<P: Codec<P> + std::fmt::Debug + PartialEq>(packet: impl Fn(Tag) -> P) {
    assert_trusted_limits_for(large_component(), packet);
}

fn assert_trusted_limits_for<P: Codec<P> + std::fmt::Debug + PartialEq>(
    large: Tag,
    packet: impl Fn(Tag) -> P,
) {
    let expected = packet(large);
    let mut w = Writer::new();
    P::write(&expected, &mut w);
    assert!(w.data.len() > NBT_QUOTA);
    let mut r = Reader::new(&w.data);
    assert_eq!(P::read(&mut r).unwrap(), expected);
    assert_eq!(r.remaining(), 0);

    // Encode a packet with a unique placeholder, then replace just that NBT
    // field: the normal writer cannot produce a tag past the depth limit.
    let mut marker = Writer::new();
    let placeholder = Tag::String("trusted NBT fixture".into());
    marker.nbt(&placeholder);
    let mut w = Writer::new();
    P::write(&packet(placeholder), &mut w);
    let start = w
        .data
        .windows(marker.data.len())
        .position(|bytes| bytes == marker.data)
        .unwrap();
    // A compound containing 512 unnamed compound children nested in order:
    // root type, 512 (compound type, empty name) entries, 513 End markers.
    let mut deep = vec![10];
    for _ in 0..512 {
        deep.extend([10, 0, 0]);
    }
    deep.extend(std::iter::repeat_n(0, 513));
    w.data.splice(start..start + marker.data.len(), deep);
    let err = P::read(&mut Reader::new(&w.data)).unwrap_err();
    assert!(err.to_string().contains("depth limit exceeded"), "{err}");
}

/// A chat line or styled number format whose `show_item` hover carries a registered mod data
/// component decodes (#6083: it disconnected the client with "unknown component type"); an
/// unregistered one is refused.
#[test]
fn a_chat_hover_item_with_a_registered_mod_component_decodes() {
    // This test binary installs no other custom types; ids far past the built-in ones.
    crate::item::custom::install(1_000_000, vec!["sunrise_test:hover_marker".into()]).unwrap();
    let hover_event = |component: &str| {
        Tag::compound([
            ("action", Tag::String("show_item".into())),
            ("id", Tag::String("minecraft:stone".into())),
            (
                "components",
                Tag::compound([(component, Tag::String("example:zombie".into()))]),
            ),
        ])
    };
    let hover = |component: &str| {
        Tag::compound([
            ("text", Tag::String("[Data Model]".into())),
            ("hover_event", hover_event(component)),
        ])
    };
    let style = |component: &str| Tag::compound([("hover_event", hover_event(component))]);
    // The connection's synchronized names: the hovered item is a known one.
    let mut names = crate::registry::Dynamic::default();
    names.registries.insert(
        "minecraft:item".into(),
        vec![("minecraft:stone".into(), None)],
    );
    let decode = |content: Tag| {
        let packet = SystemChat {
            content,
            overlay: false,
        };
        let mut w = Writer::new();
        SystemChat::write(&packet, &mut w);
        SystemChat::read(&mut Reader::new(&w.data).with_names(&names))
    };
    let registered = hover("sunrise_test:hover_marker");
    assert_eq!(
        decode(registered.clone()).map(|packet| packet.content),
        Ok(registered),
        "a registered mod component in a hover item decodes"
    );
    let err = decode(hover("sunrise_test:unregistered")).unwrap_err();
    assert!(
        err.to_string()
            .contains("unknown component type sunrise_test:unregistered"),
        "{err}"
    );

    let decode_style = |tag: Tag| {
        let format = NumberFormat::Styled(tag);
        let mut w = Writer::new();
        NumberFormat::write(&format, &mut w);
        NumberFormat::read(&mut Reader::new(&w.data).with_names(&names))
    };
    let registered_style = style("sunrise_test:hover_marker");
    assert_eq!(
        decode_style(registered_style.clone()),
        Ok(NumberFormat::Styled(registered_style)),
        "a registered mod component in a styled number format hover item decodes"
    );
    let err = decode_style(style("sunrise_test:unregistered")).unwrap_err();
    assert!(
        err.to_string()
            .contains("unknown component type sunrise_test:unregistered"),
        "{err}"
    );
}

#[test]
fn chat_uses_trusted_nbt() {
    assert_trusted_limits(|content| SystemChat {
        content,
        overlay: false,
    });
}

#[test]
fn title_subtitle_and_action_bar_use_the_shared_trusted_text_codec() {
    assert_trusted_limits(|text| Text { text });
}

#[test]
fn title_packet_rejects_nbt_that_is_not_a_component() {
    let packet = Clientbound::SetTitleText(Text { text: Tag::Int(7) });
    let bytes = packet.encode();
    let mut r = Reader::new(&bytes);
    let id = r.varint().unwrap();
    let err = Clientbound::decode(id, r.rest()).unwrap_err();
    assert!(err.to_string().starts_with("Failed to decode:"), "{err}");
}

#[test]
fn tab_list_uses_trusted_nbt() {
    assert_trusted_limits(|header| TabList {
        header,
        footer: Tag::String("footer".into()),
    });
}

#[test]
fn teams_and_scores_use_trusted_nbt() {
    assert_trusted_limits(|prefix| SetPlayerTeam {
        name: "red".into(),
        method: 2,
        parameters: Some(TeamParameters {
            display_name: Tag::String("Red".into()),
            prefix,
            suffix: Tag::String("".into()),
            name_tag_visibility: 0,
            collision_rule: 0,
            color: None,
            options: 0,
        }),
        players: vec![],
    });
    assert_trusted_limits(|display| SetScore {
        owner: "player".into(),
        objective: "score".into(),
        score: 1,
        display: Some(display),
        number_format: None,
    });
}

#[test]
fn boss_bar_uses_trusted_nbt() {
    assert_trusted_limits(|name| BossEvent {
        id: 1,
        action: BossAction::UpdateName { name },
    });
}

#[test]
fn block_entity_data_uses_trusted_nbt() {
    let data = Tag::compound([("payload", Tag::ByteArray(vec![0; NBT_QUOTA]))]);
    assert_trusted_limits_for(data, |tag| BlockEntityData {
        pos: [1, 2, 3],
        kind: 0,
        tag,
    });
}

#[test]
fn block_entity_data_requires_a_non_null_compound() {
    for (tag, accepted) in [
        (Tag::Compound(Default::default()), true),
        (Tag::End, false),
        (Tag::List(vec![Tag::Int(1)]), false),
    ] {
        let packet = BlockEntityData {
            pos: [1, 2, 3],
            kind: 0,
            tag,
        };
        let mut w = Writer::new();
        BlockEntityData::write(&packet, &mut w);
        let read = BlockEntityData::read(&mut Reader::new(&w.data));
        if accepted {
            assert_eq!(read.unwrap(), packet);
        } else {
            assert!(read.is_err());
        }
    }
}

#[test]
fn styled_number_format_decodes_a_style_map_not_a_component() {
    let large = Tag::compound([
        ("color", Tag::String("gold".into())),
        (
            "hover_event",
            Tag::compound([
                ("action", Tag::String("show_text".into())),
                ("value", large_component()),
            ]),
        ),
    ]);
    assert_trusted_limits_for(large, NumberFormat::Styled);
    for tag in [
        Tag::String("not a style".into()),
        Tag::compound([("bold", Tag::String("yes".into()))]),
    ] {
        let mut w = Writer::new();
        NumberFormat::write(&NumberFormat::Styled(tag), &mut w);
        assert!(NumberFormat::read(&mut Reader::new(&w.data)).is_err());
    }
}

#[test]
fn common_disconnect_uses_trusted_nbt() {
    assert_trusted_limits(|reason| Disconnect { reason });
}

#[test]
fn configuration_and_inline_play_dialogs_use_trusted_nbt() {
    let notice = Tag::compound([
        ("type", Tag::String("minecraft:notice".into())),
        ("title", large_component()),
    ]);
    assert_trusted_limits_for(notice.clone(), |dialog| configuration::ShowDialog {
        dialog,
    });
    assert_trusted_limits_for(notice, |dialog| ShowDialog {
        dialog: DialogHolder::Direct(dialog),
    });
}

#[test]
fn player_info_uses_trusted_nbt() {
    assert_trusted_limits(|name| PlayerInfoUpdate {
        actions: 1 << 5,
        entries: vec![PlayerInfo {
            uuid: 1,
            display_name: Some(Some(name)),
            ..PlayerInfo::default()
        }],
    });
}

#[test]
fn command_suggestion_tooltips_use_trusted_nbt() {
    assert_trusted_limits(|tooltip| CommandSuggestions {
        id: 1,
        start: 0,
        length: 1,
        suggestions: vec![CommandSuggestion {
            text: "help".into(),
            tooltip: Some(tooltip),
        }],
    });
}

#[test]
fn command_suggestion_packet_rejects_nbt_that_is_not_a_tooltip_component() {
    let packet = Clientbound::CommandSuggestions(CommandSuggestions {
        id: 1,
        start: 0,
        length: 1,
        suggestions: vec![CommandSuggestion {
            text: "help".into(),
            tooltip: Some(Tag::Int(7)),
        }],
    });
    let bytes = packet.encode();
    let mut r = Reader::new(&bytes);
    let id = r.varint().unwrap();
    let err = Clientbound::decode(id, r.rest()).unwrap_err();
    assert!(err.to_string().starts_with("Failed to decode:"), "{err}");
}

#[test]
fn server_data_and_open_screen_use_trusted_nbt() {
    assert_trusted_limits(|motd| ServerData { motd, icon: None });
    assert_trusted_limits(|title| OpenScreen {
        container: 1,
        menu: 0,
        title,
    });
}

#[test]
fn advancement_display_uses_trusted_nbt() {
    assert_trusted_limits(|description| UpdateAdvancements {
        reset: false,
        added: vec![Advancement {
            id: "test:advancement".into(),
            parent: None,
            display: Some(AdvancementDisplay {
                title: Tag::String("title".into()),
                description,
                icon: Stack::new(1, 1),
                frame: 0,
                show_toast: false,
                hidden: false,
                background: None,
            }),
            requirements: vec![],
            telemetry: false,
            x: 0.0,
            y: 0.0,
        }],
        removed: vec![],
        progress: vec![],
        show: false,
    });
}

#[test]
fn resource_pack_prompt_uses_trusted_nbt() {
    assert_trusted_limits(|prompt| ResourcePackPush {
        id: 1,
        url: "https://example.invalid/pack.zip".into(),
        hash: "".into(),
        required: false,
        prompt: Some(prompt),
    });
}

#[test]
fn custom_server_link_label_uses_trusted_nbt() {
    assert_trusted_limits(|label| ServerLinks {
        links: vec![configuration::ServerLink {
            kind: configuration::ServerLinkKind::Custom(label),
            link: "https://example.invalid/".into(),
        }],
    });
}
