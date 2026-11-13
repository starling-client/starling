//! Measures the production Text boundary; each attempt must run in a separate process.
use serde_json::{Value, json};
use std::{hint::black_box, path::Path};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let bytes: usize = args[1].parse().unwrap();
    let language: Value =
        serde_json::from_slice(&std::fs::read(Path::new(&args[2])).unwrap()).unwrap();
    let relation: usize = args[3].parse().unwrap();
    std::thread::Builder::new()
        .stack_size(bytes)
        .spawn(move || {
            let mut source = json!({"translate":"block.minecraft.stone"});
            for _ in 1..512 {
                source = match relation {
                    0 => json!({"text":"", "extra":[source]}),
                    1 => json!({"translate":"chat.type.text", "with":[source]}),
                    2 => json!({"text":"", "hover_event":{"action":"show_text", "value":source}}),
                    3 => json!({"selector":"@s", "separator":source}),
                    4 => json!({"object":"atlas", "sprite":"minecraft:block/stone", "fallback":source}),
                    5 => json!({"text":"", "hover_event":{"action":"show_entity", "id":"minecraft:pig", "uuid":[0,0,0,1], "name":source}}),
                    6 => json!({"nbt":"name", "storage":"test:data", "interpret":true, "separator":source}),
                    7 => json!({"object":"player", "player":{"name":"Test"}, "fallback":source}),
                    _ => panic!("unknown relation"),
                };
            }
            let engine = horizon_client_core::component::to_text(&source, &mut |key| {
                horizon_client_core::text::Text::literal(language[key].as_str().unwrap_or(key))
            });
            eprintln!("built");
            text_boundary_tests::measure_host_text(black_box(engine));
            drop(source);
            eprintln!("complete");
        })
        .unwrap()
        .join()
        .unwrap();
}
