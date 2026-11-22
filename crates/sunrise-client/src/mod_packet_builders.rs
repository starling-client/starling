//! Host-owned container packet construction; the protocol owns stack codecs and hashes.
use mod_api::{ContainerClickRequest, PacketBuildError, ServerboundPlayPacket};
use std::cell::RefCell;
use sunrise_core::vanilla_bytes::Writer;
use sunrise_protocol::{
    buf::Codec,
    item::{Hashed, Stack},
    packets::play::ContainerClick,
    registry::{Builtin, Dynamic, SyncedNames},
};

fn component_error(field: impl Into<String>) -> PacketBuildError {
    PacketBuildError::ComponentCannotEncode {
        packet: "container_click".into(),
        field: field.into(),
    }
}
fn range(field: impl Into<String>) -> PacketBuildError {
    PacketBuildError::OutOfRange {
        packet: "container_click".into(),
        field: field.into(),
    }
}

fn validate(request: &ContainerClickRequest) -> Result<(), PacketBuildError> {
    ServerboundPlayPacket::validate_container_click_limits(
        request.container_id,
        request.state_id,
        request.slot,
        request.changed_slots.len() as u64,
    )?;
    i8::try_from(request.button).map_err(|_| range("button"))?;
    if !(0..=6).contains(&request.click_type) {
        return Err(PacketBuildError::UnknownAction {
            packet: "container_click".into(),
            field: "click_type".into(),
        });
    }
    let mut slots = std::collections::HashSet::new();
    for (i, (slot, _)) in request.changed_slots.iter().enumerate() {
        i16::try_from(*slot).map_err(|_| range(format!("changed_slots[{i}].slot")))?;
        if !slots.insert(slot) {
            return Err(range(format!("changed_slots[{i}].slot")));
        }
    }
    Ok(())
}

/// Encode a client mod container click with the connection registries and the game's
/// API-to-engine stack conversion. Uses the ordinary item codec and Hashed::create;
/// refuses any lost component or unresolved registry reference.
pub fn encode_container_click(
    request: &ContainerClickRequest,
    builtin: &Builtin,
    dynamic: &Dynamic,
    convert: impl Fn(&mod_api::ItemStack) -> Option<sunrise_items::items::ItemStack>,
) -> Result<ServerboundPlayPacket, PacketBuildError> {
    validate(request)?;
    let names = SyncedNames { builtin, dynamic };
    let stack = |value: &Option<mod_api::ItemStack>,
                 field: &str|
     -> Result<Option<Hashed>, PacketBuildError> {
        let Some(value) = value else {
            return Ok(None);
        };
        i32::try_from(value.count()).map_err(|_| range(format!("{field}.count")))?;
        if let Some(damage) = value.damage_component() {
            i32::try_from(damage)
                .map_err(|_| component_error(format!("{field}.components.minecraft:damage")))?;
        }
        let engine = convert(value).ok_or_else(|| component_error(field))?;
        let missing = RefCell::new(None);
        let lookup = |registry: &str, name: &str| {
            let id = builtin
                .get(registry)
                .and_then(|ids| ids.id(name))
                .or_else(|| dynamic.id(registry, name))
                .or_else(|| {
                    (dynamic.custom_components && registry == "minecraft:data_component_type")
                        .then(|| sunrise_protocol::item::custom::id(name))
                        .flatten()
                });
            if id.is_none() {
                missing
                    .borrow_mut()
                    .get_or_insert_with(|| format!("{field}.components.{name}"));
            }
            id
        };
        let item_id = lookup("minecraft:item", &engine.item)
            .ok_or_else(|| component_error(format!("{field}.item")))?;
        let mut network = sunrise_client::lan::guest::stack_to_network(
            |name| lookup("minecraft:item", name).unwrap_or(item_id),
            lookup,
            &Some(engine),
        )
        .ok_or_else(|| component_error(field))?;
        if let Some(field) = missing.into_inner() {
            return Err(component_error(field));
        }
        // The saved-component converter can report a built-in removal through both
        // its generic and specialised branches. A component patch is a set.
        let mut removed = std::collections::HashSet::new();
        network.removed.retain(|id| removed.insert(*id));
        if value.damage_component().is_some()
            && !network
                .added
                .iter()
                .any(|(id, _)| *id == sunrise_protocol::item::component::DAMAGE)
        {
            return Err(component_error(format!(
                "{field}.components.minecraft:damage"
            )));
        }
        // The existing codec deliberately drops components it cannot encode. A builder must refuse them.
        if let Some(patch) = value.components() {
            for key in patch.as_nbt().keys() {
                let key = key.to_string().map_err(|_| component_error(field))?;
                let (removed, name) = key
                    .strip_prefix('!')
                    .map_or((false, key.as_str()), |name| (true, name));
                let id = builtin
                    .get("minecraft:data_component_type")
                    .and_then(|ids| ids.id(name))
                    .or_else(|| {
                        dynamic
                            .custom_components
                            .then(|| sunrise_protocol::item::custom::id(name))
                            .flatten()
                    })
                    .ok_or_else(|| component_error(format!("{field}.components.{key}")))?;
                let represented = if removed {
                    network.removed.contains(&id)
                } else {
                    network.added.iter().any(|(kind, _)| *kind == id)
                };
                if !represented {
                    return Err(component_error(format!("{field}.components.{key}")));
                }
            }
        }
        checked_hash(&network, &names, field).map(Some)
    };
    let mut changed = Vec::with_capacity(request.changed_slots.len());
    for (i, (slot, value)) in request.changed_slots.iter().enumerate() {
        changed.push((
            i16::try_from(*slot).map_err(|_| range(format!("changed_slots[{i}].slot")))?,
            stack(value, &format!("changed_slots[{i}].stack"))?,
        ));
    }
    let packet = ContainerClick {
        container: i32::try_from(request.container_id).map_err(|_| range("container_id"))?,
        state: i32::try_from(request.state_id).map_err(|_| range("state_id"))?,
        slot: i16::try_from(request.slot).map_err(|_| range("slot"))?,
        button: i8::try_from(request.button).map_err(|_| range("button"))?,
        input: request.click_type,
        changed,
        carried: stack(&request.carried, "carried")?,
    };
    let mut writer = Writer::new();
    ContainerClick::write(&packet, &mut writer);
    let id =
        mod_api::Identifier::from_parts("minecraft", "container_click").map_err(|_| range("id"))?;
    Ok(ServerboundPlayPacket::new(id, writer.data))
}

fn checked_hash(
    stack: &Stack,
    names: &dyn sunrise_protocol::item::Names,
    field: &str,
) -> Result<Hashed, PacketBuildError> {
    if stack.added.len() > 256 || stack.removed.len() > 256 {
        return Err(range(format!("{field}.components")));
    }
    Hashed::create(stack, names).map_err(|error| {
        use sunrise_protocol::item::StackHashError;
        component_error(match error {
            StackHashError::Component(id) => format!("{field}.components.{id}"),
            _ => format!("{field}.components"),
        })
    })
}

#[cfg(test)]
#[path = "mod_packet_builders_tests.rs"]
mod tests;
