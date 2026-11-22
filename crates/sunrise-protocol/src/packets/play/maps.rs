//! The map item's data: `ClientboundMapItemDataPacket`, its decorations and the
//! patch of map colours it carries.
use crate::{
    Result,
    buf::{Bool, Byte, Codec, List, Nbt, Opt, Reader, VarInt},
    packet,
};
use sunrise_core::nbt::Tag;
use sunrise_core::vanilla_bytes::Writer;

packet!(
/// MapDecoration: a registry holder, its map position and rotation, and an
/// optional name.
MapDecoration {
    kind: i32 => VarInt,
    x: i8 => Byte,
    y: i8 => Byte,
    rot: i8 => Byte,
    name: Option<Tag> => Opt<Nbt>,
}
);
/// MapItemSavedData.MapPatch: an empty patch is a zero width.
#[derive(Clone, Debug, PartialEq)]
pub struct MapPatch {
    pub start_x: i32,
    pub start_y: i32,
    pub width: i32,
    pub height: i32,
    pub colors: Vec<u8>,
}
pub struct MapPatchOpt;
impl Codec<Option<MapPatch>> for MapPatchOpt {
    fn read(r: &mut Reader) -> Result<Option<MapPatch>> {
        let width = i32::from(r.u8()?);
        if width == 0 {
            return Ok(None);
        }
        let height = i32::from(r.u8()?);
        let start_x = i32::from(r.u8()?);
        let start_y = i32::from(r.u8()?);
        let n = r.count(r.remaining())?;
        let colors = r.bytes(n)?.to_vec();
        Ok(Some(MapPatch {
            start_x,
            start_y,
            width,
            height,
            colors,
        }))
    }
    fn write(patch: &Option<MapPatch>, w: &mut Writer) {
        let Some(patch) = patch else {
            w.u8(0);
            return;
        };
        w.u8(patch.width as u8);
        w.u8(patch.height as u8);
        w.u8(patch.start_x as u8);
        w.u8(patch.start_y as u8);
        w.varint(patch.colors.len() as i32);
        w.bytes(&patch.colors);
    }
}

packet!(MapItemData {
    map_id: i32 => VarInt,
    scale: i8 => Byte,
    locked: bool => Bool,
    decorations: Option<Vec<MapDecoration>> => Opt<List<MapDecoration>>,
    patch: Option<MapPatch> => MapPatchOpt,
});
