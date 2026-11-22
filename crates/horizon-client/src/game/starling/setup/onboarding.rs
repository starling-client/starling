//! One-time preferences after the Android installer has prepared the game files.
use super::{Action, Loading, Setup};
use crate::ha_launcher::{
    backend,
    catalog::{Config, OnboardingState},
};
use std::path::Path;
use winit::{
    dpi::PhysicalPosition,
    event::{ElementState, MouseButton, WindowEvent},
    keyboard::{KeyCode, PhysicalKey},
};

pub(super) struct Onboarding {
    pub(super) visible: bool,
    pub(super) chosen: bool,
    pub(super) directly: bool,
    pub(super) error: Option<String>,
    cursor: PhysicalPosition<f64>,
}

impl Setup {
    pub(in crate::game) fn for_launch(resources: &Path) -> Self {
        let home = backend::data_dir();
        let mut config = Config::load(&home);
        let pending = match config.onboarding {
            OnboardingState::Complete => false,
            OnboardingState::Pending | OnboardingState::Chosen => true,
            OnboardingState::Unseen if !resources.join("horizon-resources.json").is_file() => {
                config.onboarding = OnboardingState::Pending;
                if let Err(error) = config.save(&home) {
                    log::warn!("Starling: could not save pending first-run setup: {error}");
                }
                true
            }
            OnboardingState::Unseen => false,
        };
        Self {
            onboarding: pending.then_some(Onboarding {
                visible: resources.join("horizon-resources.json").is_file(),
                chosen: config.onboarding == OnboardingState::Chosen,
                directly: config.open_directly,
                error: None,
                cursor: PhysicalPosition::new(-1.0, -1.0),
            }),
            ..Self::default()
        }
    }
}

impl Loading {
    pub(in crate::game) fn show_onboarding(&mut self) -> bool {
        let Some(onboarding) = &mut self.setup.onboarding else {
            return false;
        };
        if !onboarding.visible {
            onboarding.visible = true;
            log::info!("Starling: showing first-run preferences");
            self.setup.buttons.set([None; 5]);
            self.setup.pond.set(None);
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        true
    }

    pub(in crate::game) fn onboarding_input(&mut self, event: &WindowEvent) -> bool {
        let Some(onboarding) = self.setup.onboarding.as_mut().filter(|setup| setup.visible) else {
            return false;
        };
        match event {
            WindowEvent::Touch(touch) => self.setup_touch(touch),
            WindowEvent::CursorMoved { position, .. } => onboarding.cursor = *position,
            WindowEvent::CursorLeft { .. } => {
                onboarding.cursor = PhysicalPosition::new(-1.0, -1.0);
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                let cursor = onboarding.cursor;
                if let Some(window) = &self.window {
                    let size = window.inner_size();
                    let density = super::setup_density(size.width, size.height);
                    let (x, y) = (cursor.x as f32 / density, cursor.y as f32 / density);
                    self.setup_action_at(&|[left, top, width, height]| {
                        x >= left && x <= left + width && y >= top && y <= top + height
                    });
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state.is_pressed() && !event.repeat =>
            {
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::ArrowLeft) => onboarding.directly = false,
                    PhysicalKey::Code(KeyCode::ArrowRight) => onboarding.directly = true,
                    PhysicalKey::Code(KeyCode::Tab) => onboarding.directly = !onboarding.directly,
                    PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) => {
                        let directly = onboarding.directly;
                        self.setup_action(Action::FinishOnboarding(directly));
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseInput { .. }
            | WindowEvent::KeyboardInput { .. }
            | WindowEvent::MouseWheel { .. } => {}
            _ => return false,
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        true
    }

    pub(in crate::game) fn finish_saved_onboarding(&mut self) -> bool {
        let Some(onboarding) = self.setup.onboarding.as_mut().filter(|setup| setup.chosen) else {
            return false;
        };
        onboarding.chosen = false;
        let directly = onboarding.directly;
        self.finish_onboarding(directly);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        true
    }

    pub(super) fn finish_onboarding(&mut self, directly: bool) {
        let Some(onboarding) = &mut self.setup.onboarding else {
            return;
        };
        let home = backend::data_dir();
        let mut config = Config::load(&home);
        config.open_directly = directly;
        config.onboarding = OnboardingState::Complete;
        if let Err(error) = config.save(&home) {
            onboarding.error = Some(format!(
                "Could not save your choice. Tap again to retry. {error}"
            ));
            return;
        }
        self.setup.onboarding = None;
        if directly {
            self.setup.starting = false;
            if let Err(error) = self.retry() {
                self.failure = Some(error.to_string());
            }
        } else {
            crate::ha_launcher::back_to_launcher();
            horizon_client::preparer::drop_downloads(&self.vanilla_root);
        }
    }
}
