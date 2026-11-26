//! A mipmapped dynamic texture drawn in the GUI always shows its level 0, however far it is
//! minified: `GuiCanvas::texture` has no mip range (the book page `dynamic-textures.md`).
use super::coverage_tests::Gpu;
use super::dynamic_textures::Command;
use super::{Blend, GuiScale, HudBatch, rgba};

#[test]
#[ignore = "daily"]
fn a_mipmapped_dynamic_texture_minified_in_the_gui_shows_level_zero() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(mut gpu) = Gpu::new() else {
        return;
    };
    // 256 x 256 in 4 levels, one solid colour each; handle 2 is the same texture with level 0 only.
    let colors: [[u8; 4]; 4] = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 0, 255],
    ];
    let level_pixels = |level: u32| colors[level as usize].repeat(((256 >> level) as usize).pow(2));
    let mut commands = vec![
        Command::Create {
            handle: 1,
            size: [256, 256],
            mip_levels: 4,
            rgba: level_pixels(0),
        },
        Command::Create {
            handle: 2,
            size: [256, 256],
            mip_levels: 1,
            rgba: level_pixels(0),
        },
    ];
    for level in 1..4 {
        commands.push(Command::Write {
            handle: 1,
            mip_level: level,
            origin: [0, 0],
            size: [256 >> level; 2],
            rgba: level_pixels(level),
        });
    }
    let mut uploads = crate::gfx::upload::Uploads::new(&gpu.device);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    gpu.hud
        .dynamic
        .apply(&gpu.device, &mut uploads, &mut encoder, None, &commands);
    uploads.finish(&encoder);
    gpu.queue.submit([encoder.finish()]);

    // The whole texture on 4 x 2 pixels: 64 texels a pixel, so a mip range would reach level 3.
    let mut drawn = |handle: u64| {
        let mut batch = HudBatch::new(
            [4, 2],
            GuiScale {
                requested: 1,
                unicode: false,
            },
        );
        batch.dynamic_texture(
            handle,
            [256, 256],
            [0.0, 0.0, 4.0, 2.0],
            [0.0, 0.0],
            [[256, 256], [256, 256]],
            rgba(255, 255, 255, 255),
        );
        assert!(matches!(batch.blends[0].1, Blend::Dynamic(h) if h == handle));
        let mut uploads = crate::gfx::upload::Uploads::new(&gpu.device);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        gpu.hud
            .prepare(&gpu.device, &mut uploads, &mut encoder, &batch, [4, 2]);
        uploads.finish(&encoder);
        gpu.queue.submit([encoder.finish()]);
        gpu.draw(&batch, 0.0, 0.0, false).0
    };
    let level_zero_only = drawn(2);
    assert!(level_zero_only.iter().any(|&b| b != 0), "nothing was drawn");
    assert_eq!(drawn(1), level_zero_only);
}
