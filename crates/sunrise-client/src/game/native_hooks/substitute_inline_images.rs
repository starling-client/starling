use super::Game;
use mod_api::{Answered, HookId, InlineImageReplacement};
use std::collections::BTreeMap;
use sunrise_client::hud::HudAtlas;
use sunrise_client::text::{InlineObject, Text};

impl Game<'_> {
    /// Only a subscribed client lays out substituted images. Atlas, dynamic sprite geometry and
    /// mod toggles can each invalidate layout, but updating a texture's pixels does not.
    pub(in crate::game) fn inline_images_revision(&self) -> Option<[u64; 3]> {
        (self.native_window()
            && mod_api::__private::enabled_listened(HookId::SubstituteInlineImages))
        .then(|| {
            [
                self.gui.atlas.revision,
                self.dynamic_textures.geometry_revision(),
                mod_api::__private::mods_switched(),
            ]
        })
    }

    pub(in crate::game) fn substitute_inline_images(&self, text: &Text) -> Option<Text> {
        substitute(&self.gui.atlas, text, |run, images| {
            self.native_substitute_inline_images(mod_api::__private::SubstituteInlineImagesCall {
                run,
                images,
            });
        })
    }

    /// Ask client mods for replacement atlas images before laying out this text run.
    pub(in crate::game) fn native_substitute_inline_images(
        &self,
        call: mod_api::__private::SubstituteInlineImagesCall<'_>,
    ) {
        self.fire_substitute_inline_images(call);
    }
}

fn substitute(
    atlas: &HudAtlas,
    text: &Text,
    ask: impl Fn(&str, &mut Vec<Answered<InlineImageReplacement>>),
) -> Option<Text> {
    let text = text.resolved();
    let mut images = Vec::new();
    let mut replacement: Option<Vec<_>> = None;
    for (i, span) in text.spans.iter().enumerate() {
        if !span.text.is_empty() && span.style.inline_object.is_none() {
            ask(&span.text, &mut images);
        }
        let mut accepted = BTreeMap::new();
        for entry in images.drain(..) {
            let answer = entry.answer;
            let (start, end) = (answer.start as usize, answer.end as usize);
            if start >= end
                || span.text.get(start..end).is_none_or(|s| s.contains('\n'))
                || accepted
                    .range(..end)
                    .next_back()
                    .is_some_and(|(_, (last_end, _))| start < *last_end)
            {
                continue;
            }
            let sprite = answer.sprite.to_string();
            if atlas.find_inline_sprite("minecraft:gui", &sprite).is_some() {
                accepted.insert(start, (end, sprite));
            }
        }
        if accepted.is_empty() {
            if let Some(spans) = &mut replacement {
                spans.push(span.clone());
            }
            continue;
        }
        let spans = replacement.get_or_insert_with(|| text.spans[..i].to_vec());
        let mut from = 0;
        for (start, (end, sprite)) in accepted {
            if from < start {
                spans.push(span.with_text(&span.text[from..start]));
            }
            let mut image = span.with_text("\u{fffc}");
            image.style.inline_object = Some(InlineObject::Atlas {
                atlas: "minecraft:gui".into(),
                sprite,
                fallback: span.text[start..end].into(),
            });
            spans.push(image);
            from = end;
        }
        if from < span.text.len() {
            spans.push(span.with_text(&span.text[from..]));
        }
    }
    replacement.map(Text::from_spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{ClientHost, Hooks, Identifier, ModEntry, SubstituteInlineImagesEvent};
    use mod_platform::testing::with_mods;
    use sunrise_assets::resource_source::ResourceSource;
    use sunrise_client::{hud::Rect, text};

    struct Propose(&'static [(u32, u32, &'static str)]);

    #[mod_api::__subscriptions]
    impl Hooks for Propose {
        fn substitute_inline_images(
            &self,
            _: &dyn ClientHost,
            event: &mut SubstituteInlineImagesEvent<'_>,
        ) {
            assert_eq!(event.run, "é:ok: !");
            assert!(event.images.is_empty());
            event
                .images
                .extend(
                    self.0
                        .iter()
                        .map(|&(start, end, sprite)| InlineImageReplacement {
                            start,
                            end,
                            sprite: Identifier::parse(sprite).unwrap(),
                        }),
                );
        }
    }

    #[test]
    fn inline_image_ranges_accept_first_available_nonoverlapping_sprites() {
        let mut atlas = HudAtlas::fonts(&ResourceSource::default()).unwrap();
        for name in [
            "demo:first",
            "demo:second",
            sunrise_client::hud::MISSING_SPRITE,
        ] {
            atlas.sprites.insert(
                name.into(),
                Rect {
                    u0: 0.0,
                    v0: 0.0,
                    u1: 1.0,
                    v1: 1.0,
                    w: 1,
                    h: 1,
                },
            );
        }
        let source = Text::translate_with_fallback("demo.token_test", "§aé:ok: !", &[]);
        let mods = vec![
            ModEntry::new(
                "a",
                Box::new(Propose(&[
                    (1, 2, "demo:first"),
                    (0, 2, "demo:missing"),
                    (2, 6, "demo:first"),
                    (6, u32::MAX, "demo:first"),
                ])),
            ),
            ModEntry::new(
                "b",
                Box::new(Propose(&[(0, 2, "demo:second"), (2, 8, "demo:second")])),
            ),
        ];
        with_mods(mods.into(), || {
            let host = mod_api::testing::FakeHost::default();
            let result = substitute(&atlas, &source, |run, images| {
                mod_api::dispatch_substitute_inline_images(
                    &host,
                    mod_api::__private::SubstituteInlineImagesCall { run, images },
                );
            })
            .unwrap();
            assert_eq!(result.plain(), "é:ok: !");
            assert_eq!(result.spans.len(), 3);
            for (span, sprite, fallback) in [
                (&result.spans[0], "demo:second", "é"),
                (&result.spans[1], "demo:first", ":ok:"),
            ] {
                assert_eq!(span.text, "\u{fffc}");
                assert_eq!(span.style.color, Some(text::GREEN));
                assert_eq!(
                    span.style.inline_object,
                    Some(InlineObject::Atlas {
                        atlas: "minecraft:gui".into(),
                        sprite: sprite.into(),
                        fallback: fallback.into(),
                    })
                );
            }
            assert_eq!(result.spans[2].text, " !");
            assert_eq!(source.plain(), "§aé:ok: !");
        });
    }
}
