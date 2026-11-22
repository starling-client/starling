#!/usr/bin/env python3
"""Wire damage calls with an owning Game to the existing per-hit death continuation."""

from pathlib import Path

GAME = Path(__file__).resolve().parents[1] / "crates/sunrise-client/src/game"


def replace(file, *edits):
    path = GAME / file
    text = path.read_text()
    for before, after in edits:
        if text.count(before) != 1:
            raise SystemExit(f"{file}: expected one occurrence of {before!r}")
        text = text.replace(before, after)
    path.write_text(text)


replace(
    "entities/lightning.rs",
    ("f32::MAX,\n                        damage,\n                    );\n                    continue;", "f32::MAX,\n                        damage,\n                    );\n                    self.finish_mob_damage_at(i);\n                    continue;"),
    ("5.0,\n                damage,\n            );\n        }", "5.0,\n                damage,\n            );\n            self.finish_mob_damage_at(i);\n        }"),
)
replace(
    "entities/explosion.rs",
    ("self.resolve_player_responsible(victim, source);\n                        }", "self.resolve_player_responsible(victim, source);\n                        }\n                        self.finish_mob_damage_at(i);"),
)
replace(
    "entities/fireworks.rs",
    ("if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {\n                let hurt = m.damage(", "if let Some(index) = self.mob_index.find(&self.mobs, id) {\n                let hurt = self.mobs[index].damage("),
    ("self.remember_mob_player_credit(id);\n                }\n            }", "self.remember_mob_player_credit(id);\n                }\n                self.finish_mob_damage_at(index);\n            }"),
)
replace(
    "mobs/fangs.rs",
    ("if self.mobs[index].damage(", "let hurt = self.mobs[index].damage("),
    ("            ) && owner.is_some()\n            {", "            );\n            self.finish_mob_damage_at(index);\n            if hurt && owner.is_some() {"),
)
replace(
    "blocks/splash_water.rs",
    ("m.extra.counter2 = 64;\n                    }\n                }", "m.extra.counter2 = 64;\n                    }\n                    self.finish_mob_damage_at(index);\n                }"),
)
replace(
    "blocks/block_entity_anims.rs",
    ("let Some(m) = conduit\n            .target\n            .and_then(|id| self.mobs.iter_mut().find(|m| m.id == id))", "let Some(index) = conduit\n            .target\n            .and_then(|id| self.mob_index.find(&self.mobs, id))"),
    ("let at = [m.position.x, m.position.y, m.position.z];\n        m.damage(", "let m = &mut self.mobs[index];\n        let at = [m.position.x, m.position.y, m.position.z];\n        m.damage("),
    ("Damage::of(DamageKind::Magic),\n        );\n        self.audio", "Damage::of(DamageKind::Magic),\n        );\n        self.finish_mob_damage_at(index);\n        self.audio"),
)
replace(
    "mobs/creatures/mod.rs",
    ("if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {\n                        m.damage(", "if let Some(index) = self.mob_index.find(&self.mobs, mob) {\n                        self.mobs[index].damage("),
    ("sunrise_client::mobs::Damage::of(kind),\n                        );", "sunrise_client::mobs::Damage::of(kind),\n                        );\n                        self.finish_mob_damage_at(index);"),
    ("self.remember_mob_player_credit(id);\n                }\n            }\n            // Parrot", "self.remember_mob_player_credit(id);\n                }\n                self.finish_mob_damage_at(index);\n            }\n            // Parrot"),
)
replace(
    "blocks/sculk_fixture.rs",
    ("if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {\n                        m.damage(", "if let Some(index) = self.mob_index.find(&self.mobs, id) {\n                        self.mobs[index].damage("),
    ("                    }\n                    self.remember_mob_player_credit(id);", "                        self.remember_mob_player_credit(id);\n                        self.finish_mob_damage_at(index);\n                    }"),
)
replace(
    "items/potions.rs",
    ("let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {\n                        return;\n                    };\n                    match effect {", "let Some(index) = self.mob_index.find(&self.mobs, id) else {\n                        return;\n                    };\n                    let m = &mut self.mobs[index];\n                    match effect {"),
    ("Damage::of(DamageKind::Magic),\n                                );", "Damage::of(DamageKind::Magic),\n                                );\n                                self.finish_mob_damage_at(index);"),
    ("let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) else {\n                                    return;\n                                };\n                                m.damage_by(", "let Some(index) = self.mob_index.find(&self.mobs, mob) else {\n                                    return;\n                                };\n                                self.mobs[index].damage_by("),
    ('"minecraft:area_effect_cloud",\n                                );', '"minecraft:area_effect_cloud",\n                                );\n                                self.finish_mob_damage_at(index);'),
    ('"minecraft:splash_potion",\n                    );', '"minecraft:splash_potion",\n                    );\n                    self.finish_mob_damage_at(i);'),
)
