use super::*;
impl Game<'_> {
    pub(in crate::game) fn text_string(&self, text: &mod_api::Text) -> String {
        sunrise_client_lib::api_text::to_engine_inprocess(text).plain()
    }

    pub(in crate::game) fn text_width(&self, text: &mod_api::Text) -> i32 {
        let engine = sunrise_client_lib::api_text::to_engine_inprocess(text);
        self.gui.atlas.font.width_text(&engine) as i32
    }

    pub(in crate::game) fn language_right_to_left(&self) -> bool {
        sunrise_client::lang::bidirectional()
    }

    pub(in crate::game) fn text_split(
        &self,
        text: &mod_api::Text,
        width: i32,
    ) -> Vec<mod_api::Text> {
        let local = sunrise_client_lib::api_text::to_local_engine(text);
        self.gui
            .atlas
            .font
            .split_formatted(&local.engine().resolved(), width as f32)
            .iter()
            .map(|line| local.text_to_api(line))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::api_values::TryIntoEngine;

    /// `Language.isDefaultRightToLeft` (#5354): false for English, true once a pack's language
    /// marked `bidirectional` is applied, on a game with no logged-in world.
    #[test]
    #[ignore = "daily"]
    fn the_active_languages_direction_reads_outside_a_world() {
        use mod_api::prelude::ClientTextClientHost;
        use sunrise_client::lang::{self, Language};
        crate::game::native_entity_refs::tests::with_game(|game| {
            assert!(game.remote.is_none());
            let english = lang::isolate_publication(Language::of("en_us", []));
            assert!(!ClientTextClientHost::language_right_to_left(&*game));
            drop(english);
            let pack = tempfile::tempdir().unwrap();
            let lang_dir = pack.path().join("assets/minecraft/lang");
            std::fs::create_dir_all(&lang_dir).unwrap();
            std::fs::write(lang_dir.join("ar_sa.json"), "{}").unwrap();
            std::fs::write(
                pack.path().join("pack.mcmeta"),
                r#"{"language":{"ar_sa":{"name":"Arabic","region":"Saudi Arabia","bidirectional":true}}}"#,
            )
            .unwrap();
            let root = pack.path().to_path_buf();
            let arabic = lang::isolate_publication(Language::load(&[&root], "ar_sa"));
            assert!(ClientTextClientHost::language_right_to_left(&*game));
            drop(arabic);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn plain_text_reads_resolve_current_language_arguments_keys_and_siblings() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let key = mod_api::Text::new(mod_api::text::TextContent::Keybind(
                "key.keyboard.space".into(),
            ));
            let mut text =
                mod_api::Text::translate("test.plain", vec![mod_api::Text::literal("Cake"), key]);
            text.style.bold = Some(true);
            let mut sibling = mod_api::Text::literal(" + ");
            sibling
                .children
                .push(mod_api::Text::translate("test.tail", vec![]));
            text.children.push(sibling);
            let host: &dyn mod_api::Host = game;
            for (space, tail, expected) in [
                ("Space", "tail", "Space / Cake + tail"),
                ("Espace", "fin", "Espace / Cake + fin"),
            ] {
                sunrise_client::lang::scoped(
                    sunrise_client::lang::Language::of(
                        "test",
                        [
                            ("test.plain".into(), "%2$s / %1$s".into()),
                            ("key.keyboard.space".into(), space.into()),
                            ("test.tail".into(), tail.into()),
                        ],
                    ),
                    || assert_eq!(host.text_string(&text), expected),
                );
            }
            assert_eq!(host.text_string(&mod_api::Text::literal("§aX")), "§aX");
            assert_eq!(host.text_string(&mod_api::Text::literal("")), "");
            let fallback = mod_api::Text::new(mod_api::text::TextContent::Translatable {
                key: "test.missing.plain".into(),
                fallback: Some("Fallback %s".into()),
                arguments: vec![mod_api::Text::literal("X")],
            });
            assert_eq!(host.text_string(&fallback), "Fallback X");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn styled_text_reads_use_live_font_language_and_wrapping_outside_drawing() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let mut text = mod_api::Text::literal("AA AA");
            text.style.bold = Some(true);
            let host: &dyn mod_api::ClientHost = game;
            assert_eq!(host.text_width(&mod_api::Text::literal("AA")), 12);
            assert_eq!(host.text_width(&mod_api::Text::literal("")), 0);
            let lines = host.text_split(&text, 14);
            let plain: Vec<_> = lines
                .iter()
                .map(|line| {
                    let line: sunrise_client::text::Text = line.try_into_engine(&()).unwrap();
                    assert!(line.chars().all(|(_, style)| style.is_bold()));
                    line.plain()
                })
                .collect();
            assert_eq!(plain, ["AA", "AA"]);
            assert_eq!(host.text_split(&mod_api::Text::literal("AA"), -1).len(), 2);

            let lines = host.text_split(&mod_api::Text::literal("A\nAA"), 100);
            let plain: Vec<_> = lines
                .iter()
                .map(|line| {
                    let line: sunrise_client::text::Text = line.try_into_engine(&()).unwrap();
                    line.plain()
                })
                .collect();
            assert_eq!(plain, ["A", "AA"]);

            // In-process text reads accept deep component trees (matching vanilla
            // Component.getString / Font.width / Font.split), while serialization
            // boundaries still enforce MAX_TEXT_DEPTH.
            let mut deep = mod_api::Text::literal("A");
            for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
                let mut parent = mod_api::Text::literal("");
                parent.children.push(deep);
                deep = parent;
            }
            assert_eq!(host.text_string(&deep), "A");
            assert_eq!(host.text_width(&deep), 6);
            assert_eq!(host.text_split(&deep, 100).len(), 1);
            assert_eq!(
                sunrise_client_lib::api_text::to_serialized_engine(&deep),
                Err(mod_api::TextFormError::TooDeep)
            );

            let translated = mod_api::Text::new(mod_api::text::TextContent::Translatable {
                key: "test.measure".into(),
                fallback: None,
                arguments: vec![],
            });
            for (value, width, lines) in [("A", 6, 1), ("AA", 12, 2)] {
                sunrise_client::lang::scoped(
                    sunrise_client::lang::Language::of(
                        "test",
                        [("test.measure".into(), value.into())],
                    ),
                    || {
                        assert_eq!(host.text_width(&translated), width);
                        assert_eq!(host.text_split(&translated, 6).len(), lines);
                    },
                );
            }
            // A live replacement font uses explicit provider advances, not any
            // metrics cached by a previous host read.
            let source = sunrise_assets::resource_source::ResourceSource::indexed(
                std::collections::BTreeMap::from([(
                    "assets/minecraft/font/default.json".into(),
                    sunrise_assets::resource_source::Resource::bytes(
                        br#"{"providers":[{"type":"space","advances":{"A":4}}]}"#.to_vec(),
                    ),
                )]),
            );
            game.gui.atlas.font = sunrise_client::font::Font::metrics(&source);
            let host: &dyn mod_api::ClientHost = game;
            assert_eq!(host.text_width(&mod_api::Text::literal("AA")), 8);
            assert_eq!(host.text_split(&mod_api::Text::literal("AA"), 8).len(), 1);
        });
    }
}
