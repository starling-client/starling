//! Light reads against what vanilla's light storage answers: stored sections, the padding sections one
//! past the build range, sections between stored ones, and the columns a client or server no longer holds.
//! `tools/light_storage_scenario.txt` is replayed on a `World`; `tools/light_storage_witness.txt` is what the
//! real 26.3 `LevelLightEngine` answered for it (`tools/test_light_storage_parity.py` records it again).

use crate::chunk::Chunk;
use crate::world::World;
use sunrise_world::registry::BlockStateId;

const SCENARIO: &str = include_str!("../../../tools/light_storage_scenario.txt");
const WITNESS: &str = include_str!("../../../tools/light_storage_witness.txt");

/// The first light section of the Overworld's column.
const FIRST_LIGHT_SECTION: i32 = -5;

/// A scenario layer: `U<n>` is `n` everywhere, `R<k>` is `(x + 2z + 3y + k) & 15` in the section.
fn layer(spec: &str) -> Vec<u8> {
    let n: i32 = spec[1..].parse().unwrap();
    let mut bytes = vec![0u8; 2048];
    for i in 0..4096i32 {
        let (x, z, y) = (i & 15, i >> 4 & 15, i >> 8);
        let v = if spec.starts_with('U') {
            n
        } else {
            (x + 2 * z + 3 * y + n) & 15
        };
        bytes[(i >> 1) as usize] |= (v as u8) << ((i & 1) * 4);
    }
    bytes
}

/// Replays `scenario` as vanilla's oracle does, one line per read.
fn replay(scenario: &str) -> String {
    let mut out = Vec::new();
    let mut world = World::new(BlockStateId(0), 0);
    for line in scenario
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let f: Vec<&str> = line.split(' ').collect();
        let n = |i: usize| f[i].parse::<i32>().unwrap();
        match f[0] {
            "case" => {
                world = World::new(BlockStateId(0), 0);
                world.shape.sky_light = f.get(2) != Some(&"nosky");
                out.push(format!("== {}", f[1]));
            }
            "column" => {
                let mut chunk = Chunk::empty(n(1), n(2));
                for s in f[3].trim_start_matches("nonair=").split(',') {
                    if let Ok(s) = s.parse::<i32>() {
                        chunk.set(0, s * 16, 0, 1);
                        chunk.add_light_support(chunk.section_index(s * 16).unwrap());
                    }
                }
                let mut layers = vec![(None, None); 26];
                for stored in f[4].trim_start_matches("stored=").split(',') {
                    let (y, kind) = stored.split_once(':').unwrap();
                    let (sky, block) = kind.split_once('/').unwrap();
                    let y: i32 = y.parse().unwrap();
                    layers[(y - FIRST_LIGHT_SECTION) as usize] =
                        (Some(layer(sky)), Some(layer(block)));
                }
                crate::light::light_from_layers(&mut chunk, &layers, world.shape.sky_light);
                world.install_light_column(chunk);
            }
            "drop" => {
                world.unload_chunk((n(1), n(2)));
            }
            "read" => {
                let light = world.light(n(1), n(2), n(3));
                out.push(format!("{line} -> {} {}", light >> 4, light & 15));
            }
            other => panic!("unknown scenario line {other}"),
        }
    }
    out.join("\n") + "\n"
}

#[test]
fn light_reads_follow_vanilla_storage() {
    assert_eq!(replay(SCENARIO), WITNESS);
}
