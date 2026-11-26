#!/usr/bin/env python3
"""Replay P3-51's mechanical setter preparation on 99c5682ffe6."""

from pathlib import Path


def main() -> None:
    crates = Path(__file__).resolve().parents[1] / "crates"
    border = crates / "sunrise-border/src"
    client = crates / "sunrise-client"
    game = client / "src/game"
    updates: dict[Path, str] = {}

    def replace(path: Path, old: str, new: str) -> None:
        source = updates[path] if path in updates else path.read_text()
        start = source.find(old)
        if start == -1 or source.find(old, start + 1) != -1:
            raise SystemExit(f"{path.relative_to(crates)}: expected exactly one copy of {old!r}")
        updates[path] = source[:start] + new + source[start + len(old):]

    worldborder = border / "worldborder.rs"
    replace(
        worldborder,
        "use sunrise_core::geometry::border::BorderGeometry;\n"
        "pub use sunrise_core::geometry::border::BorderStatus;\n",
        "pub use sunrise_core::geometry::border::BorderStatus;\n"
        "use sunrise_core::geometry::border::{BorderError, BorderGeometry};\n",
    )
    replace(
        worldborder,
        """    /// `setCenter` (non-finite centres are ignored).
    pub fn set_center(&mut self, center: [f64; 2]) {
        let _ = self.geometry.set_center(center);
    }
""",
        """    /// `setCenter` (rejects non-finite centres).
    pub fn set_center(&mut self, center: [f64; 2]) -> Result<(), BorderError> {
        self.geometry.set_center(center)
    }
""",
    )
    replace(
        worldborder,
        """    pub fn set_size(&mut self, size: f64) {
        let _ = self.geometry.set_size(size.clamp(1.0, MAX_SIZE));
    }
    /// `lerpSizeBetween(from, to, ticks)` (what the server and the border
    /// packets do).
    pub fn lerp_between(&mut self, from: f64, to: f64, ticks: i64) {
        let _ = self.geometry.lerp_size_between(from, to, ticks);
    }
    /// A size change from the current size (`/worldborder set|add` with a time).
    pub fn lerp_to(&mut self, to: f64, ticks: i64) {
        if ticks <= 0 {
            self.set_size(to);
        } else {
            self.lerp_between(self.size(), to.clamp(1.0, MAX_SIZE), ticks);
        }
    }
""",
        """    pub fn set_size(&mut self, size: f64) -> Result<(), BorderError> {
        self.geometry.set_size(size.clamp(1.0, MAX_SIZE))
    }
    /// `lerpSizeBetween(from, to, ticks)` (what the server and the border
    /// packets do).
    pub fn lerp_between(&mut self, from: f64, to: f64, ticks: i64) -> Result<(), BorderError> {
        self.geometry.lerp_size_between(from, to, ticks)
    }
    /// A size change from the current size (`/worldborder set|add` with a time).
    pub fn lerp_to(&mut self, to: f64, ticks: i64) -> Result<(), BorderError> {
        if ticks <= 0 {
            self.set_size(to)
        } else {
            self.lerp_between(self.size(), to.clamp(1.0, MAX_SIZE), ticks)
        }
    }
""",
    )

    terrain = game / "administration/commands/terrain_cmds.rs"
    native = game / "modding/native_level_border.rs"
    packets = game / "network/remote/packets_world.rs"
    # Keep callers that used to discard the geometry error doing so for this step.
    for path, calls in {
        worldborder: (
            '        b.set_center([d("center_x", 0.0), d("center_z", 0.0)]);',
            "        b.set_size(size);",
            "            b.lerp_between(size, to.clamp(1.0, MAX_SIZE), *t);",
        ),
        native: (
            "        self.border.set_center(center);",
            "        self.border.lerp_to(size, ticks);",
        ),
        terrain: (
            "                    self.border.lerp_to(distance, ticks);",
            "                    self.border.set_size(distance);",
            "                self.border.set_center([x, z]);",
        ),
        packets: (
            "        b.set_center([p.center_x, p.center_z]);",
            "            b.lerp_between(p.old_size, p.new_size, p.lerp_time);",
            "            b.set_size(p.new_size);",
        ),
    }.items():
        for call in calls:
            statement = call.lstrip()
            indent = call[: len(call) - len(statement)]
            replace(path, f"\n{call}\n", f"\n{indent}let _ = {statement}\n")
    replace(
        packets,
        """        self.border
            .lerp_between(p.old_size, p.new_size, p.lerp_time);
""",
        """        let _ = self
            .border
            .lerp_between(p.old_size, p.new_size, p.lerp_time);
""",
    )
    remote = game / "network/remote/mod.rs"
    for arm, call in (
        ("SetBorderCenter", "set_center([p.x, p.z])"),
        ("SetBorderSize", "set_size(p.size)"),
    ):
        replace(
            remote,
            f"            C::{arm}(p) => self.border.{call},\n",
            f"            C::{arm}(p) => {{\n                let _ = self.border.{call};\n            }}\n",
        )

    fixture = client / "examples/probe/world_border_render_fixture.rs"
    replace(
        fixture,
        '        border.set_center([f(&c["centerX"]), f(&c["centerZ"])]);\n'
        '        border.set_size(f(&c["size"]));\n',
        '        border.set_center([f(&c["centerX"]), f(&c["centerZ"])])?;\n'
        '        border.set_size(f(&c["size"]))?;\n',
    )
    replace(
        fixture,
        """                "lerp" => border.lerp_between(
                    f(&op["from"]),
                    f(&op["to"]),
                    op["ticks"].as_i64().unwrap_or(0),
                ),
                "size" => border.set_size(f(&op["size"])),
                "center" => border.set_center([f(&op["x"]), f(&op["z"])]),
""",
        """                "lerp" => border.lerp_between(
                    f(&op["from"]),
                    f(&op["to"]),
                    op["ticks"].as_i64().unwrap_or(0),
                )?,
                "size" => border.set_size(f(&op["size"]))?,
                "center" => border.set_center([f(&op["x"]), f(&op["z"])])?,
""",
    )

    # Trusted fixture setup must succeed. Include neighboring lines where a
    # statement is repeated, so every replacement still checks a unique block.
    spawning = game / "mobs/natural_spawning.rs"
    for path, blocks in {
        worldborder: (
            "        b.set_size(100.0);",
            "        b.lerp_to(200.0, 10);",
            "        b.lerp_to(10.0, 0);",
            "        b.set_center([10.0, 0.0]);",
            "        b.set_size(500.0);\n        b.set_center([3.5, -7.0]);",
        ),
        border / "border_render.rs": (
            "        border.set_size(100.0);",
            "        border.set_size(10_000.0);",
        ),
        client / "src/lan/border.rs": (
            "        b.set_center([4.0, -2.0]);\n        b.set_size(10.0);",
            "        b.lerp_to(20.0, 3);",
        ),
        game / "entities/ahead.rs": (
            "        game.border.set_center([8.0, 8.0]);\n        game.border.set_size(16.0);",
        ),
        game / "entities/extinguish/extinguish_tests.rs": (
            "    game.border.set_center([1000.0, 1000.0]);\n    game.border.set_size(100.0);",
        ),
        game / "modding/native_level_refs.rs": (
            "            game.border.set_center([12.0, -8.0]);\n            game.border.set_size(40.0);",
            "            game.border.set_center([0.0, 0.0]);",
        ),
        spawning: (
            "            game.border.set_center([8.0, 8.0]);\n            game.border.set_size(16.0);",
            "            game.border.set_size(15.5);",
            "            game.border.set_size(16.5);",
            "            game.border.set_center([8.0, 8.0]);\n            game.border.set_size(2.0);",
            "            game.border.set_size(64.0);",
        ),
    }.items():
        for block in blocks:
            asserted = "\n".join(f"{line[:-1]}.unwrap();" for line in block.splitlines())
            replace(path, f"\n{block}\n", f"\n{asserted}\n")
    replace(
        spawning,
        """            game.world.inhabited.clear();
            game.border.set_size(2.0);
""",
        """            game.world.inhabited.clear();
            game.border.set_size(2.0).unwrap();
""",
    )

    methods = """    /// Every tick: the size change, the wall the player cannot pass and
    /// damage outside the border.
    /// `BorderChangeListener.DelegateBorderChangeListener`'s packets: what
    /// changed on the border since the guests were last told of it.
    pub(in crate::game) fn lan_sync_border(&mut self) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let now = sunrise_client::lan::border::Sent::of(&self.border);
        if let Some(sent) = host.border_sent.replace(now) {
            for packet in sent.changes(&self.border) {
                // PlayerList$1: only the players of the border's own level.
                host.hub.broadcast_level(&packet);
            }
        }
    }
    pub(in crate::game) fn tick_worldborder(&mut self) {
        let remaining = self.border.remaining_ticks();
        self.border.tick();
        if remaining > 0
            && self.border.remaining_ticks() == 0
            && (self.serving.is_some() || self.options.dedicated.is_some())
        {
            self.native_world_border_bounds_change_finish();
        }
        self.player_border();
        self.border_damage();
    }
    /// The player's movement collides with the border as it is now
    /// (`CollisionGetter.borderCollision`): from inside, its walls stop it.
    pub(in crate::game) fn player_border(&mut self) {
        if let Some(p) = self.player.as_mut() {
            p.border = Some(self.border.geometry().clone());
        }
    }
    /// `LivingEntity.baseTick`: a player outside the border, further than
    /// its safe zone, is hurt by the blocks beyond it. The game's player, or
    /// a guest's in their turn (`as_guest`).
    pub(in crate::game) fn border_damage(&mut self) {
        let Some((px, pz, half)) = self.player.as_ref().map(|p| {
            (
                p.position.x,
                p.position.z,
                f64::from(p.dimensions.width) / 2.0,
            )
        }) else {
            return;
        };
        let inside = self.border.contains(px - half, pz - half)
            && self.border.contains(px + half, pz + half);
        if inside {
            return;
        }
        let dist = self.border.distance(px, pz) + self.border.safe_zone;
        if dist < 0.0 && self.border.damage_per_block > 0.0 && self.ticks.is_multiple_of(10) {
            let amount = (-dist * self.border.damage_per_block).floor().max(1.0) as f32;
            self.hurt_player_by("minecraft:outside_border", amount);
        }
    }
"""
    replace(terrain, methods, "")
    replace(
        client / "src/game.rs",
        '#[path = "game/world/world_entry.rs"]\nmod world_entry;\n',
        '#[path = "game/world/world_border.rs"]\nmod world_border;\n'
        '#[path = "game/world/world_entry.rs"]\nmod world_entry;\n',
    )
    new_module = game / "world/world_border.rs"
    with new_module.open("x") as output:
        output.write(
            "//! The world border's tick, player collision and damage, and guest synchronization.\n"
            "use super::Game;\n\n"
            "impl Game<'_> {\n" + methods + "}\n"
        )
    for path, source in updates.items():
        path.write_text(source)


if __name__ == "__main__":
    main()
