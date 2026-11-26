//! Parity probe comparing Sunrise's section visibility against a vanilla 26.3 witness JSON
//! (`tools/test_visibility_parity.py`).
use super::{
    Frustum,
    culling_control::occlusion_radius,
    frustum,
    gpu_culling::{FrameInputs, Record, UNBOUNDED, keeps, rounded_box, tables},
    occlusion::Occlusion,
    resident::{Resident, ResidentSize, origin as section_origin},
    view_sections::view_sections,
    world_projection,
};
use glam::{DVec3, Vec3, Vec4};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

struct ProbeSlot;

impl ResidentSize for ProbeSlot {
    fn census_bytes(&self) -> (u64, usize) {
        (0, 0)
    }
}

fn parse_f64_bits(hex: &str) -> Result<f64, String> {
    let stripped = hex
        .strip_prefix("0x")
        .or_else(|| hex.strip_prefix("0X"))
        .unwrap_or(hex);
    u64::from_str_radix(stripped, 16)
        .map(f64::from_bits)
        .map_err(|e| format!("invalid f64 hex {hex:?}: {e}"))
}

fn parse_camera_bits(arr: &[Value]) -> Result<DVec3, String> {
    Ok(DVec3::new(
        parse_f64_bits(arr[0].as_str().ok_or("camera_bits[0]")?)?,
        parse_f64_bits(arr[1].as_str().ok_or("camera_bits[1]")?)?,
        parse_f64_bits(arr[2].as_str().ok_or("camera_bits[2]")?)?,
    ))
}

fn parse_sections(arr: &[Value]) -> Vec<[i32; 3]> {
    let mut out: Vec<[i32; 3]> = arr
        .iter()
        .map(|s| {
            let a = s.as_array().expect("section triple");
            [
                a[0].as_i64().expect("sx") as i32,
                a[1].as_i64().expect("sy") as i32,
                a[2].as_i64().expect("sz") as i32,
            ]
        })
        .collect();
    out.sort_unstable();
    out
}

fn parse_excluded_chunks(case: &Value) -> HashSet<(i32, i32)> {
    case.get("excluded_loaded_chunks")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|pair| {
                    let p = pair.as_array().expect("chunk pair");
                    (p[0].as_i64().unwrap() as i32, p[1].as_i64().unwrap() as i32)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_tight_bounds(case: &Value, min_sec_y: i32) -> HashMap<(i32, i32, i32), [Vec3; 2]> {
    let mut out = HashMap::new();
    if let Some(entries) = case.get("tight_bounds").and_then(Value::as_array) {
        for entry in entries {
            let s = entry["section"].as_array().expect("tight_bounds section");
            let lo = entry["min"].as_array().expect("tight_bounds min");
            let hi = entry["max"].as_array().expect("tight_bounds max");
            let key = (
                s[0].as_i64().unwrap() as i32,
                s[1].as_i64().unwrap() as i32 - min_sec_y,
                s[2].as_i64().unwrap() as i32,
            );
            out.insert(
                key,
                [
                    Vec3::new(
                        lo[0].as_f64().unwrap() as f32,
                        lo[1].as_f64().unwrap() as f32,
                        lo[2].as_f64().unwrap() as f32,
                    ),
                    Vec3::new(
                        hi[0].as_f64().unwrap() as f32,
                        hi[1].as_f64().unwrap() as f32,
                        hi[2].as_f64().unwrap() as f32,
                    ),
                ],
            );
        }
    }
    out
}

fn loaded_bounds(case: &Value, cam_cx: i32, cam_cz: i32, rd: i32) -> (i32, i32, i32, i32) {
    if let Some(r) = case.get("loaded_chunk_range").and_then(Value::as_array) {
        (
            r[0].as_i64().unwrap() as i32,
            r[1].as_i64().unwrap() as i32,
            r[2].as_i64().unwrap() as i32,
            r[3].as_i64().unwrap() as i32,
        )
    } else {
        (
            cam_cx - rd - 1,
            cam_cx + rd + 1,
            cam_cz - rd - 1,
            cam_cz + rd + 1,
        )
    }
}

fn build_frustum(f: &serde_json::Map<String, Value>, rd: i32) -> (Frustum, f32, f32, f32, f32) {
    let yaw = f["yaw"].as_f64().expect("yaw") as f32;
    let pitch = f["pitch"].as_f64().expect("pitch") as f32;
    let fov = f["fov"].as_f64().expect("fov") as f32;
    let aspect = f["aspect"].as_f64().expect("aspect") as f32;
    let far = f
        .get("far")
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .unwrap_or_else(|| ((rd * 16 * 4) as f32).max(12.0 * 16.0));
    let proj = world_projection(fov, aspect, far);
    let view = crate::camera::view_matrix(yaw, pitch);
    (
        Frustum::from_planes(frustum(proj * view)),
        yaw,
        pitch,
        fov,
        far,
    )
}

/// Verifies every witness case across `Occlusion`, `Resident`, `gpu_culling`, `culling_control`,
/// and `view_sections`, returning the number of cases checked.
pub fn verify_witness(path: &Path) -> Result<usize, String> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "failed to read section visibility witness {}: {e}",
            path.display()
        )
    })?;
    let witness: Value =
        serde_json::from_str(&text).map_err(|e| format!("invalid witness JSON: {e}"))?;
    if witness["meta"]["game_version"].as_str() != Some("26.3") {
        return Err(format!(
            "unexpected witness game_version: {:?}",
            witness["meta"]["game_version"]
        ));
    }
    let cases = witness["cases"]
        .as_array()
        .ok_or("missing cases array in witness")?;
    let all_frustum = Frustum::from_planes([Vec4::W; 6]);
    let mut words = Vec::new();
    let mut checked = 0usize;

    for case in cases {
        let name = case["name"].as_str().ok_or("case name")?;
        let min_y = case["min_y"].as_i64().ok_or("min_y")? as i32;
        let sections = case["sections"].as_i64().ok_or("sections")? as i32;
        let min_sec_y = case["min_section_y"].as_i64().ok_or("min_section_y")? as i32;
        let rd = case["render_distance"].as_i64().ok_or("render_distance")? as i32;
        let smart_cull = case["smart_cull"].as_bool().ok_or("smart_cull")?;
        let camera = parse_camera_bits(case["camera_bits"].as_array().ok_or("camera_bits")?)?;
        let cam_sec = case["camera_section"].as_array().ok_or("camera_section")?;
        let cam_cx = cam_sec[0].as_i64().ok_or("cam_cx")? as i32;
        let cam_cz = cam_sec[2].as_i64().ok_or("cam_cz")? as i32;
        let excluded = parse_excluded_chunks(case);
        let (lx0, lx1, lz0, lz1) = loaded_bounds(case, cam_cx, cam_cz, rd);
        let non_empty: Option<HashSet<i32>> = case
            .get("non_empty_world_sy")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|v| v.as_i64().unwrap() as i32).collect());

        // 1. culling_control::occlusion_radius matches vanilla ViewArea.getViewDistance().
        if occlusion_radius((rd * 16) as f32) != rd {
            return Err(format!(
                "case {name}: occlusion_radius mismatch for rd={rd}"
            ));
        }

        // 2. Occlusion graph matches vanilla (both smart_cull true & false, plus unloaded stripes).
        let mut o = Occlusion::new(min_y, sections);
        for x in lx0..=lx1 {
            for z in lz0..=lz1 {
                if !excluded.contains(&(x, z)) {
                    o.load_column(x, z);
                }
            }
        }
        if let Some(overrides) = case.get("vis_overrides").and_then(Value::as_array) {
            for ov in overrides {
                let s = ov["section"].as_array().expect("override section");
                let sx = s[0].as_i64().unwrap() as i32;
                let sy = s[1].as_i64().unwrap() as i32;
                let sz = s[2].as_i64().unwrap() as i32;
                o.set((sx, sy - min_sec_y, sz), ov["mask"].as_u64().unwrap());
            }
        }
        o.settle(camera, rd, smart_cull);
        let reach = o
            .reach()
            .ok_or_else(|| format!("case {name}: missing Reach"))?;
        let expected_octree = parse_sections(
            case["octree_all_sections"]
                .as_array()
                .ok_or("octree_all_sections")?,
        );
        let mut actual_occ = Vec::new();
        for x in cam_cx - rd - 1..=cam_cx + rd + 1 {
            for z in cam_cz - rd - 1..=cam_cz + rd + 1 {
                for y in -1..=sections {
                    let key = (x, y, z);
                    let got = o.contains(key);
                    if reach.contains(key) != got {
                        return Err(format!(
                            "case {name}: Reach::contains disagrees with Occlusion::contains at {key:?}"
                        ));
                    }
                    let sy = y + min_sec_y;
                    if got && non_empty.as_ref().is_none_or(|set| set.contains(&sy)) {
                        actual_occ.push([x, sy, z]);
                    }
                }
            }
        }
        actual_occ.sort_unstable();
        if actual_occ != expected_octree {
            return Err(format!(
                "case {name}: Occlusion/Reach mismatch (got {} vs expected {})",
                actual_occ.len(),
                expected_octree.len()
            ));
        }

        // Check visible_for_camera fail-open below vertical band on banded interior cases.
        if matches!(
            name,
            "genpvp_menu_level" | "cave_floor_with_shaft_and_vertical_clamp"
        ) {
            let cs = o.section_of(camera);
            let below_y = cs.1 - rd - 1;
            if below_y < 0 || o.visible_for_camera((cs.0, below_y, cs.2), cs).is_some() {
                return Err(format!(
                    "case {name}: visible_for_camera must return None below vertical clamp"
                ));
            }
        }

        // 3. Partial propagation step (column loads and/or shifted eye before full search lands).
        if let Some(ps) = case.get("partial_step").and_then(Value::as_object) {
            let next_cam = if let Some(pa) = ps.get("camera").and_then(Value::as_array) {
                DVec3::new(
                    pa[0].as_f64().unwrap(),
                    pa[1].as_f64().unwrap(),
                    pa[2].as_f64().unwrap(),
                )
            } else {
                camera
            };
            if let Some(added) = ps.get("add_loaded_chunks").and_then(Value::as_array) {
                for pair in added {
                    let p = pair.as_array().unwrap();
                    o.load_column(p[0].as_i64().unwrap() as i32, p[1].as_i64().unwrap() as i32);
                }
            }
            if let Some(overrides) = ps.get("vis_overrides").and_then(Value::as_array) {
                for ov in overrides {
                    let s = ov["section"].as_array().unwrap();
                    o.set(
                        (
                            s[0].as_i64().unwrap() as i32,
                            s[1].as_i64().unwrap() as i32 - min_sec_y,
                            s[2].as_i64().unwrap() as i32,
                        ),
                        ov["mask"].as_u64().unwrap(),
                    );
                }
            }
            o.propagate_partial_in_flight(next_cam, rd, smart_cull);
            let next_reach = o
                .reach()
                .ok_or_else(|| format!("case {name}: missing Reach after partial step"))?;
            let expected_after = parse_sections(
                case["after_partial_octree_all_sections"]
                    .as_array()
                    .ok_or("after_partial_octree_all_sections")?,
            );
            let next_cs = o.section_of(next_cam);
            let mut actual_after = Vec::new();
            for x in (cam_cx.min(next_cs.0) - rd - 1)..=(cam_cx.max(next_cs.0) + rd + 1) {
                for z in (cam_cz.min(next_cs.2) - rd - 1)..=(cam_cz.max(next_cs.2) + rd + 1) {
                    for y in 0..sections {
                        let key = (x, y, z);
                        let got = o.contains(key);
                        if next_reach.contains(key) != got {
                            return Err(format!(
                                "case {name}: Reach disagrees after partial step at {key:?}"
                            ));
                        }
                        let sy = y + min_sec_y;
                        if got && non_empty.as_ref().is_none_or(|set| set.contains(&sy)) {
                            actual_after.push([x, sy, z]);
                        }
                    }
                }
            }
            actual_after.sort_unstable();
            if actual_after != expected_after {
                return Err(format!(
                    "case {name}: after partial step mismatch (got {} vs expected {})",
                    actual_after.len(),
                    expected_after.len()
                ));
            }
        }

        // 4. Resident::visible & gpu_culling tables/keeps (open-world cases with full loaded grid),
        // tested independently of the occlusion graph and with out-of-height sentinel rows.
        if case.get("vis_overrides").is_none()
            && excluded.is_empty()
            && case.get("partial_step").is_none()
        {
            let tight_bounds = parse_tight_bounds(case, min_sec_y);
            let mut r = Resident::default();
            r.set_min_y(min_y);
            r.set_sections(sections);
            for x in cam_cx - rd - 1..=cam_cx + rd + 1 {
                for z in cam_cz - rd - 1..=cam_cz + rd + 1 {
                    for y in -1..=sections {
                        let sy = y + min_sec_y;
                        let out_of_height = y < 0 || y >= sections;
                        if out_of_height || non_empty.as_ref().is_none_or(|set| set.contains(&sy)) {
                            let bounds = tight_bounds.get(&(x, y, z)).copied();
                            r.insert_marked((x, y, z), ProbeSlot, bounds, false);
                        }
                    }
                }
            }

            let mut out = Vec::new();
            r.visible(camera, rd, &all_frustum, |_| true, |_, _| false, &mut out);
            let mut actual_res: Vec<[i32; 3]> = out
                .iter()
                .map(|v| [v.key.0, v.key.1 + min_sec_y, v.key.2])
                .collect();
            actual_res.sort_unstable();
            if actual_res != expected_octree {
                return Err(format!(
                    "case {name}: Resident::visible all-frustum mismatch (got {} vs expected {})",
                    actual_res.len(),
                    expected_octree.len()
                ));
            }

            let input = FrameInputs::new(camera, min_y, sections, rd, &all_frustum);
            let mut params = tables(&input, &mut words)
                .ok_or_else(|| format!("case {name}: gpu_culling::tables returned None"))?;
            params.pages = 1;
            let mut actual_gpu = Vec::new();
            for x in cam_cx - rd - 1..=cam_cx + rd + 1 {
                for z in cam_cz - rd - 1..=cam_cz + rd + 1 {
                    for y in -1..=sections {
                        let sy = y + min_sec_y;
                        let out_of_height = y < 0 || y >= sections;
                        if !out_of_height
                            && non_empty.as_ref().is_some_and(|set| !set.contains(&sy))
                        {
                            continue;
                        }
                        let record = Record::new((x, y, z), 0, 0, 1, UNBOUNDED);
                        if keeps(&params, &words, &record) {
                            actual_gpu.push([x, sy, z]);
                        }
                    }
                }
            }
            actual_gpu.sort_unstable();
            if actual_gpu != expected_octree {
                return Err(format!(
                    "case {name}: gpu_culling all-frustum mismatch (got {} vs expected {})",
                    actual_gpu.len(),
                    expected_octree.len()
                ));
            }

            // Exact frustum-visible keys on single-direction frustum cases (including outside-height octree).
            if let Some(f) = case.get("frustum").and_then(Value::as_object) {
                let (view_frustum, yaw, pitch, fov, far) = build_frustum(f, rd);
                let expected_frustum = parse_sections(
                    case["frustum_visible_sections"]
                        .as_array()
                        .ok_or("frustum_visible_sections")?,
                );
                let expected_set: HashSet<[i32; 3]> = expected_frustum.iter().copied().collect();

                out.clear();
                r.visible(camera, rd, &view_frustum, |_| true, |_, _| false, &mut out);
                let mut res_vis: Vec<[i32; 3]> = out
                    .iter()
                    .map(|v| [v.key.0, v.key.1 + min_sec_y, v.key.2])
                    .collect();
                res_vis.sort_unstable();

                let input = FrameInputs::new(camera, min_y, sections, rd, &view_frustum);
                let mut params = tables(&input, &mut words)
                    .ok_or_else(|| format!("case {name}: frustum tables returned None"))?;
                params.pages = 1;
                let mut gpu_vis = Vec::new();
                for x in cam_cx - rd - 1..=cam_cx + rd + 1 {
                    for z in cam_cz - rd - 1..=cam_cz + rd + 1 {
                        for y in -1..=sections {
                            let sy = y + min_sec_y;
                            let out_of_height = y < 0 || y >= sections;
                            if !out_of_height
                                && non_empty.as_ref().is_some_and(|set| !set.contains(&sy))
                            {
                                continue;
                            }
                            let bounds = tight_bounds.get(&(x, y, z)).copied();
                            let record = Record::new((x, y, z), 0, 0, 1, rounded_box(bounds));
                            if keeps(&params, &words, &record) {
                                gpu_vis.push([x, sy, z]);
                            }
                        }
                    }
                }
                gpu_vis.sort_unstable();

                if name.starts_with("outside_height_octree_") || expected_frustum.is_empty() {
                    if res_vis != expected_frustum {
                        return Err(format!(
                            "case {name}: Resident::visible frustum keys mismatch\n  actual:   {res_vis:?}\n  expected: {expected_frustum:?}"
                        ));
                    }
                    if gpu_vis != expected_frustum {
                        return Err(format!(
                            "case {name}: gpu_culling frustum keys mismatch\n  actual:   {gpu_vis:?}\n  expected: {expected_frustum:?}"
                        ));
                    }
                } else {
                    // For in-world frusta without camera-cube pull-back, every Resident kept key
                    // must be accepted by gpu_culling as well.
                    let gpu_set: HashSet<[i32; 3]> = gpu_vis.iter().copied().collect();
                    for k in &res_vis {
                        if !gpu_set.contains(k) {
                            return Err(format!(
                                "case {name}: gpu_culling dropped section {k:?} kept by Resident::visible"
                            ));
                        }
                    }
                }
                if name == "outside_height_octree_above_rd2_square70"
                    && (!expected_set.contains(&[0, 19, 0]) || expected_set.contains(&[2, 19, 0]))
                {
                    return Err(format!(
                        "case {name}: witness sanity check failed: expected (0,19,0) in and (2,19,0) out"
                    ));
                }
                if name == "outside_height_octree_below_rd2_square35"
                    && (!expected_set.contains(&[0, -4, 0]) || expected_set.contains(&[2, -4, 0]))
                {
                    return Err(format!(
                        "case {name}: witness sanity check failed: expected (0,-4,0) in and (2,-4,0) out"
                    ));
                }
                if name == "outside_height_octree_below_rd2_square39_tight_bounds" {
                    let offset = view_frustum.offset_to_fully_include_camera_cube(camera, 8);
                    let sec_min = section_origin((2, 0, 0), min_y, camera).as_vec3();
                    let [bmin, bmax] = tight_bounds
                        .get(&(2, 0, 0))
                        .copied()
                        .ok_or("missing tight_bounds for (2, -4, 0)")?;
                    if !expected_set.contains(&[2, -4, 0])
                        || offset.intersects(sec_min + bmin, sec_min + bmax)
                    {
                        return Err(format!(
                            "case {name}: witness sanity check failed: (2,-4,0) must be kept while X=46..47 misses"
                        ));
                    }
                }
                if name == "genpvp_menu_looking_down" {
                    let view = crate::render_views::WorldView::new(
                        1,
                        camera,
                        yaw,
                        pitch,
                        fov,
                        [1280, 800],
                    );
                    out.clear();
                    view_sections(&r, &view, camera, far, rd, &mut out);
                    if out.len() != expected_frustum.len() {
                        return Err(format!(
                            "case {name}: view_sections kept {} vs expected {}",
                            out.len(),
                            expected_frustum.len()
                        ));
                    }
                }
            }
        }
        checked += 1;
    }
    if checked < 31 {
        return Err(format!(
            "expected at least 31 visibility cases, got {checked}"
        ));
    }
    Ok(checked)
}
