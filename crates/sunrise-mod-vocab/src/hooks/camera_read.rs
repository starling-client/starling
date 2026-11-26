pub trait Notes {
    #[notes = r#"## Camera entity

`ClientHost::camera_entity()` returns an `EntityRef` for the entity the view follows
(`Minecraft.getCameraEntity`): the local player, unless a server attached the camera to another
entity. It is `None` on the title screen and outside a world. Together with `camera()` and
`camera_type()` it lets a minimap or freecam mod tell its own view from a spectated one.

`ClientHost::set_camera_entity(entity)` is `Minecraft.setCameraEntity`: it moves the view onto a
mob (or back onto the player with the player's own ref) and returns whether it did. It selects
the entity only; the camera angle stays the entity's, and `compute_camera_angles` and
`camera_position` are the hooks that override angles and render-only camera position.

## Client heights

`LevelRef::visit_surface_heights(host, columns, heightmap, visit)` is
`LevelReader.getHeight(type, x, z)` on any open level (server active or parked, or the client's
current open level): each column's height from loaded chunks, `None` for a column whose chunk is
not held, streamed with no column-count cap and early exit via `ControlFlow::Break(())`. A minimap
or a tracker reads many columns in one call instead of scanning blocks; on a client it reads the
client's own view and follows server block changes as they arrive."#]
    fn camera_read_note0();
}
