/// `ChatType.Bound.decorate` and `decorateNarration`: a private message
/// shows as "Steve whispers to you: hi" and is narrated "Steve says hi";
/// a chat type the registry lacks is `minecraft:chat`'s.
#[test]
fn chat_is_narrated_with_its_chat_types_narration_decoration() {
    use super::super::chat::chat_type_decorate;
    use super::super::{Decoration, play};
    use serde_json::json;
    use sunrise_core::nbt::{JavaString, Tag, parse_snbt};
    let bound = play::ChatTypeBound {
        chat_type: 0,
        name: Tag::String(JavaString::from("Steve")),
        target: None,
    };
    let content = json!("hi");
    let (incoming, _) = parse_snbt(
        r#"{chat:{translation_key:"commands.message.display.incoming",parameters:["sender","content"],style:{color:"gray",italic:1b}},narration:{translation_key:"chat.type.text.narrate",parameters:["sender","content"]}}"#,
    )
    .unwrap();
    let chat = chat_type_decorate(Some(&incoming), Decoration::Chat, &bound, &content);
    assert_eq!(chat["translate"], "commands.message.display.incoming");
    assert_eq!(chat["color"], "gray");
    assert_eq!(chat["with"], json!(["Steve", "hi"]));
    assert_eq!(
        chat_type_decorate(Some(&incoming), Decoration::Narration, &bound, &content),
        json!({"translate": "chat.type.text.narrate", "with": ["Steve", "hi"]})
    );
    assert_eq!(
        chat_type_decorate(None, Decoration::Narration, &bound, &content),
        json!({"translate": "chat.type.text.narrate", "with": ["Steve", "hi"]})
    );
    assert_eq!(
        chat_type_decorate(None, Decoration::Chat, &bound, &content)["translate"],
        "chat.type.text"
    );
}
