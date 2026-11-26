//! Tooltip display visibility and 26.3 tooltip_style sprite companion for tools/test_tooltip_display_parity.py.
use std::{error::Error, fmt::Write};
use sunrise_client::{
    hud::{HudAtlas, HudBatch, Rect},
    item_tooltip::tooltip_display,
    items::ItemStack,
    resource_source::ResourceSource,
    ui::{tooltip_sprites, tooltip_style_sprites},
};
use sunrise_core::{identifier::Identifier, nbt::Tag};

fn gui_sprite_id(gui_id: &str) -> Result<String, Box<dyn Error>> {
    Ok(Identifier::parse(gui_id)?.to_string())
}

pub fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = crate::args().collect();
    let mut output = String::new();
    let mut atlas = HudAtlas::build(&ResourceSource::indexed(Default::default()))?;
    for line in std::fs::read_to_string(&args[1])?.lines() {
        if let Some(name) = line.strip_prefix("name\t") {
            let components = sunrise_client::item_data::command_components(&format!(
                "[minecraft:custom_name={name}]"
            ))?;
            let (_, extra) = sunrise_client::item_data::decode_components(Some(&components));
            let stack = ItemStack {
                extra,
                ..ItemStack::new("minecraft:stick", 1)
            };
            let text = sunrise_client::item_tooltip::styled_name(&stack, || "unused".into());
            output.push_str("name\t");
            for span in &text.spans {
                for c in span.text.chars() {
                    let style = &span.style;
                    write!(
                        output,
                        "{},{},{},{},{},{},{}|",
                        c as u32,
                        style.color.unwrap_or(sunrise_client::text::WHITE),
                        style.is_bold(),
                        style.is_italic(),
                        style.underlined == Some(true),
                        style.strikethrough == Some(true),
                        style.obfuscated == Some(true)
                    )?;
                }
            }
            output.push('\n');
            continue;
        }
        if let Some(rest) = line.strip_prefix("style\t") {
            let fields: Vec<&str> = rest.splitn(5, '\t').collect();
            if fields.len() != 5 {
                return Err(format!("bad style case: {line}").into());
            }
            let x: i32 = fields[0].parse()?;
            let y: i32 = fields[1].parse()?;
            let w: i32 = fields[2].parse()?;
            let h: i32 = fields[3].parse()?;
            let stack = sunrise_client::item_data::parse_stack(fields[4])?
                .ok_or("empty stack in style case")?;
            let style = stack
                .data()
                .and_then(|d| sunrise_client::save::text(d.other("minecraft:tooltip_style")))
                .and_then(|s| Identifier::parse(&s).ok())
                .ok_or("missing tooltip_style")?;
            let sprites = tooltip_style_sprites(style.namespace(), style.path());
            atlas.sprites.clear();
            let rects = [
                Rect {
                    u0: 0.0,
                    v0: 0.0,
                    u1: 0.25,
                    v1: 0.25,
                    w: 16,
                    h: 16,
                },
                Rect {
                    u0: 0.5,
                    v0: 0.0,
                    u1: 0.75,
                    v1: 0.25,
                    w: 16,
                    h: 16,
                },
            ];
            for (key, rect) in sprites.iter().zip(rects) {
                atlas.sprites.insert(key.clone(), rect);
            }
            let mut batch = HudBatch::scratch([320, 240]);
            if !tooltip_sprites(
                &mut batch,
                &atlas,
                [sprites[0].as_str(), sprites[1].as_str()],
                [x as f32, y as f32, w as f32, h as f32],
            ) {
                return Err("tooltip_sprites drew nothing".into());
            }
            let (quads, rest) = batch.vertices.as_chunks::<6>();
            if !rest.is_empty() {
                return Err(format!("non-quad vertex count: {}", batch.vertices.len()).into());
            }
            let mut draws = Vec::new();
            for quad in quads {
                let u0 = quad[0].uv[0].to_bits();
                let key = if u0 == rects[0].u0.to_bits() {
                    &sprites[0]
                } else if u0 == rects[1].u0.to_bits() {
                    &sprites[1]
                } else {
                    return Err(format!("unexpected quad UV: {:?}", quad[0].uv).into());
                };
                let sprite_id = gui_sprite_id(key)?;
                let qx = quad[0].pos[0] as i32;
                let qy = quad[0].pos[1] as i32;
                let qw = (quad[2].pos[0] - quad[0].pos[0]) as i32;
                let qh = (quad[2].pos[1] - quad[0].pos[1]) as i32;
                draws.push(format!("{sprite_id},{qx},{qy},{qw},{qh},{}", quad[0].color));
            }
            writeln!(output, "style\t{}", draws.join("|"))?;
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let hide_tooltip: bool = fields[0].parse()?;
        let creative: bool = fields[1].parse()?;
        let hide_attributes: bool = fields[2].parse()?;
        let mut component = Vec::new();
        if hide_tooltip {
            component.push(("hide_tooltip", Tag::Byte(1)));
        }
        if hide_attributes {
            component.push((
                "hidden_components",
                Tag::List(vec![Tag::String("minecraft:attribute_modifiers".into())]),
            ));
        }
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        if !component.is_empty() {
            stack
                .data_mut()
                .set_other("minecraft:tooltip_display", Some(Tag::compound(component)));
        }
        let display = tooltip_display(&stack);
        writeln!(
            output,
            "{}\t{}",
            display.name_visible(creative),
            display.shows("minecraft:attribute_modifiers")
        )?;
    }
    std::fs::write(&args[2], output)?;
    Ok(())
}
