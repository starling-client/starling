//! Entity state flags, custom colliders and package data file reads.

pub mod living_flags {
    use mod_api::{EntityRef, ServerHost};

    pub fn pause_mob_ai(host: &mut dyn ServerHost, entity: EntityRef) {
        // ANCHOR: pause_mob_ai
        // A spawner's movable no-AI mob: paused once, and still paused after a reload.
        if let Some(mob) = host.mob(entity) {
            let _ = mob.set_ai_paused(host, true);
        }
        // ANCHOR_END: pause_mob_ai
    }
}

pub mod custom_collider {
    use mod_api::{Collider, ColliderBox, ColliderError, ColliderPose, EntityRef, ServerHost};

    pub fn configure_collider(
        entity: EntityRef,
        host: &mut dyn ServerHost,
        turned: [f32; 4],
    ) -> Result<(), ColliderError> {
        // ANCHOR: custom_collider
        let pivot = [0.0, 0.0, 0.0];
        entity.set_collider(
            host,
            Some(
                Collider::new(vec![ColliderBox {
                    min: [-1.5, -1.0, -10.0],
                    max: [1.5, 0.0, 10.0],
                }])
                .with_pivot(pivot),
            ),
        )?;
        // Update the pose (rotation + pivot) without resending the boxes:
        entity.set_collider_pose(host, ColliderPose::new(turned, pivot))?;
        // ANCHOR_END: custom_collider
        Ok(())
    }
}

pub mod data_files {
    // ANCHOR: read_scripts
    pub fn scripts(
        cx: &mod_api::RegistrationContext,
    ) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error>> {
        let files = cx.package_files("lumen_demo")?;
        let mut scripts = Vec::new();
        for path in files.list("data/lumen_demo/lumen/") {
            scripts.push(files.read(path)?);
        }
        Ok(scripts)
    }
    // ANCHOR_END: read_scripts
}
