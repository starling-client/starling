use mod_api::{ClientHost, Text};

#[test]
#[ignore = "daily"]
fn visual_read_uses_live_translations_styles_and_bidi_outside_drawing() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        let host: &dyn ClientHost = game;
        let mut argument = Text::literal("Alex");
        argument.style.bold = Some(true);
        argument.style.color = Some("red".into());
        let mut line = Text::translate("fixture.greet", vec![argument]);
        line.style.color = Some("gold".into());
        sunrise_client::lang::scoped(
            sunrise_client::lang::Language::of(
                "test",
                [("fixture.greet".into(), "Hello, %s!".into())],
            ),
            || {
                let output = host.text_visual_order(&line);
                assert_eq!(
                    output
                        .iter()
                        .map(|v| char::from_u32(v.scalar).unwrap())
                        .collect::<String>(),
                    "Hello, Alex!"
                );
                assert_eq!(
                    output.iter().map(|v| v.source_index).collect::<Vec<_>>(),
                    [0, 1, 2, 3, 4, 5, 6, 0, 1, 2, 3, 0]
                );
                assert!(
                    output[7..11].iter().all(|v| v.style.bold == Some(true)
                        && v.style.color.as_deref() == Some("#FF5555"))
                );
                assert!(
                    output[..7]
                        .iter()
                        .chain(&output[11..])
                        .all(|v| v.style.color.as_deref() == Some("#FFAA00"))
                );
            },
        );
        for (input, expected, indices) in [
            ("שלום", "םולש", vec![3, 2, 1, 0]),
            (
                "abc אבג 123",
                "abc 123 גבא",
                vec![0, 1, 2, 3, 0, 1, 2, 3, 2, 1, 0],
            ),
        ] {
            let output = host.text_visual_order(&Text::literal(input));
            assert_eq!(
                output
                    .iter()
                    .map(|v| char::from_u32(v.scalar).unwrap())
                    .collect::<String>(),
                expected
            );
            assert_eq!(
                output.iter().map(|v| v.source_index).collect::<Vec<_>>(),
                indices
            );
        }
        // Independent equal styles form one UTF-16 run; a color or click change starts
        // another. The sidecar must still return each character's own opaque metadata.
        let mut dialog = mod_api::Tag::compound([("leaf", mod_api::Tag::Int(7))]);
        for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
            dialog = mod_api::Tag::compound([("next", dialog)]);
        }
        let mut separate = Text::literal("");
        for (letter, color, marker) in [
            ("A", "red", 1),
            ("B", "#FF5555", 2),
            ("C", "gold", 3),
            ("D", "gold", 4),
        ] {
            let mut child = Text::literal(letter);
            child.style.color = Some(color.into());
            child.style.metadata = mod_api::Tag::compound([("marker", mod_api::Tag::Int(marker))]);
            child.style.click = Some(if marker == 4 {
                mod_api::text::TextClick::CopyToClipboard("different".into())
            } else {
                mod_api::text::TextClick::ShowDialog(sunrise_core::nbt::clone_tag(&dialog))
            });
            separate.children.push(child);
        }
        let output = host.text_visual_order(&separate);
        assert_eq!(
            output.iter().map(|v| v.source_index).collect::<Vec<_>>(),
            [0, 1, 0, 0]
        );
        for (i, output) in output.iter().enumerate() {
            assert_eq!(
                output.style.metadata.get("marker"),
                Some(&mod_api::Tag::Int(i as i32 + 1))
            );
        }
        sunrise_core::nbt::discard_tag(dialog);

        let mut custom = Text::literal("");
        for (letter, id) in [
            ("A", "example"),
            ("B", "minecraft:example"),
            ("C", ":example"),
            ("D", "other:example"),
        ] {
            let mut child = Text::literal(letter);
            child.style.click = Some(mod_api::text::TextClick::Custom {
                id: id.into(),
                payload: Some(mod_api::Tag::Int(7)),
            });
            custom.children.push(child);
        }
        let output = host.text_visual_order(&custom);
        assert_eq!(
            output.iter().map(|v| v.source_index).collect::<Vec<_>>(),
            [0, 1, 2, 0]
        );
        for (character, input) in output.iter().zip(&custom.children) {
            assert_eq!(character.style.click, input.style.click);
        }

        let mut styled = Text::literal("X");
        styled.style.color = Some("#123456".into());
        styled.style.bold = Some(false);
        styled.style.italic = Some(true);
        styled.style.underlined = Some(true);
        styled.style.strikethrough = Some(true);
        styled.style.obfuscated = Some(true);
        styled.style.shadow_color = Some(0x80123456);
        styled.style.font = Some("minecraft:uniform".into());
        styled.style.insertion = Some("insert".into());
        styled.style.click = Some(mod_api::text::TextClick::CopyToClipboard("copy".into()));
        styled.style.hover = Some(mod_api::TextHover::Text(Box::new(Text::literal("hover"))));
        let output = host.text_visual_order(&styled);
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].style, styled.style);
        assert!(host.text_visual_order(&Text::literal("")).is_empty());
        // In-process visual order accepts deep trees (matching vanilla Component.getVisualOrderText).
        let mut deep = Text::literal("A");
        for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
            let mut parent = Text::literal("");
            parent.children.push(deep);
            deep = parent;
        }
        let deep_output = host.text_visual_order(&deep);
        assert_eq!(deep_output.len(), 1);
        assert_eq!(deep_output[0].scalar, 'A' as u32);
    });
}
