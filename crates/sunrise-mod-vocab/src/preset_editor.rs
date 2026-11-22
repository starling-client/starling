//! World-preset editor registration (`register_preset_editors`, NeoForge's
//! `RegisterPresetEditorsEvent` and vanilla's `PresetEditor`).
use crate::{Compound, DrawContext, Identifier, RegisterPresetEditorsEvent};
use std::sync::Arc;

/// Read-only context handed to a registered preset editor factory when the player opens Customize
/// on Create World (`register_preset_editors`).
// no wire: borrowed open-time context for a process-local preset editor factory.
// exhaustive: private fields let the game construct this view while mods only read it.
pub struct PresetEditorContext<'a> {
    preset: &'a Identifier,
    dimensions: &'a Compound,
    seed: &'a str,
}

impl PresetEditorContext<'_> {
    /// The selected world-preset registry identifier (`example:islands` or `minecraft:flat`).
    pub fn preset(&self) -> &Identifier {
        self.preset
    }

    /// The starting dimension stems for this world creation.
    pub fn dimensions(&self) -> &Compound {
        self.dimensions
    }

    /// The world seed text currently entered in Create World.
    pub fn seed(&self) -> &str {
        self.seed
    }
}

/// Mutable working copy handed to a live [`PresetEditor`] during render and input callbacks.
///
/// Changes stay in this draft until the player clicks Done (which commits them to world creation)
/// or Cancel / Escape (which discards them).
// no wire: borrowed mutable draft for a live process-local preset editor.
// exhaustive: private fields keep validation on the engine boundary.
pub struct PresetEditorDraft<'a> {
    preset: &'a Identifier,
    dimensions: &'a mut Compound,
    validate: &'a dyn Fn(&Compound) -> bool,
}

impl PresetEditorDraft<'_> {
    /// The world-preset registry identifier being edited.
    pub fn preset(&self) -> &Identifier {
        self.preset
    }

    /// The current working dimension stems.
    pub fn dimensions(&self) -> &Compound {
        self.dimensions
    }

    /// Replace the working dimension stems after structural validation (requires an overworld,
    /// valid dimension ids, and valid type/generator declarations). Returns `false` and leaves the
    /// draft unchanged if malformed.
    pub fn set_dimensions(&mut self, dimensions: Compound) -> bool {
        if !(self.validate)(&dimensions) {
            return false;
        }
        *self.dimensions = dimensions;
        true
    }
}

/// A live client world-preset editor screen instance created by a factory registered with
/// [`RegisterPresetEditorsEvent::register`] (vanilla's `PresetEditor`).
///
/// The game renders the menu background, header title, and Done/Cancel footer buttons, then
/// forwards drawing and input events to this editor while `world_preset_editor` is open. Keep
/// custom drawing above the bottom 33 GUI pixels reserved for Done and Cancel.
// no wire: live process-local client screen instance.
pub trait PresetEditor: Send + 'static {
    /// Optional title drawn at the top of the screen; `None` falls back to the preset's localized
    /// generator label.
    fn title(&self) -> Option<String> {
        None
    }

    /// Draw the editor's custom UI inside `rect` (`[x, y, width, height]` in scaled GUI pixels),
    /// after the stock editor frame and before `screen_render_post`. If the call fails, its paint
    /// and draft edits are discarded; the stock frame and later render phases are kept. A panic
    /// disables the bound editor callbacks for the session and is reported with the mod's name.
    fn render(
        &mut self,
        _draw: &mut dyn DrawContext,
        _rect: [f32; 4],
        _draft: &mut PresetEditorDraft<'_>,
    ) {
    }

    /// Handle a press at GUI coordinates `(x, y)` after mouse button Pre allows it and the stock
    /// Done and Cancel buttons have had first chance at a left click. `button` is the same GLFW
    /// index as in the screen hooks, including platform button numbers beyond the usual buttons.
    /// `double_click` is true when the last handled real press was of this button on this screen
    /// instance less than 250 ms ago; synthetic clicks pass `false`.
    ///
    /// Return `true` to report the click handled. The mouse button Post gets this original answer
    /// even when it is `true` and may override it for subsequent input handling.
    fn mouse_clicked(
        &mut self,
        _x: f32,
        _y: f32,
        _button: i32,
        _double_click: bool,
        _draft: &mut PresetEditorDraft<'_>,
    ) -> bool {
        false
    }

    /// Handle a key press (`key`: vanilla `InputConstants` code, `scancode`: platform scan code,
    /// `modifiers`: GLFW bits: shift 1, control 2, alt 4, super 8). Return `true` to consume the
    /// key. Keys consumed by the input method during composition do not reach this callback.
    fn key_pressed(
        &mut self,
        _key: u32,
        _scancode: u32,
        _modifiers: u32,
        _draft: &mut PresetEditorDraft<'_>,
    ) -> bool {
        false
    }

    /// Handle a typed character, including text committed by the input method (`modifiers`:
    /// GLFW bits). Return `true` to consume the character.
    fn char_typed(
        &mut self,
        _ch: char,
        _modifiers: u32,
        _draft: &mut PresetEditorDraft<'_>,
    ) -> bool {
        false
    }
}

/// A retained preset editor factory bound at `register_preset_editors`.
// no wire: retained process-local preset editor factory callback.
pub type PresetEditorFactoryFn =
    dyn Fn(&PresetEditorContext<'_>) -> Result<Box<dyn PresetEditor>, String> + Send + Sync;

impl RegisterPresetEditorsEvent<'_> {
    /// Register a preset editor factory for world preset `preset` (NeoForge's
    /// `RegisterPresetEditorsEvent::register`).
    ///
    /// Any valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted. A later
    /// registration for the same preset replaces the earlier one (last-writer-wins), and
    /// registering a vanilla preset (`minecraft:flat`, `minecraft:single_biome_surface`) overrides
    /// its built-in editor.
    pub fn register<F>(&mut self, preset: Identifier, factory: F)
    where
        F: Fn(&PresetEditorContext<'_>) -> Result<Box<dyn PresetEditor>, String>
            + Send
            + Sync
            + 'static,
    {
        self.editors.push(PresetEditorFactoryRegistration {
            preset,
            factory: Arc::new(factory),
        });
    }
}

/// A staged preset editor factory; dispatch attributes it to the registering mod.
// no wire: retained process-local preset editor factory callback.
// hidden: retained registration staging belongs to the game
#[doc(hidden)]
#[non_exhaustive]
pub struct PresetEditorFactoryRegistration {
    /// The world-preset registry identifier (`example:islands` or `minecraft:flat`).
    pub preset: Identifier,
    /// The factory invoked when the player opens Customize for this preset.
    pub factory: Arc<PresetEditorFactoryFn>,
}

/// Construct a [`PresetEditorContext`] for one factory invocation (engine only).
// hidden: the game owns the state borrowed by the factory
#[doc(hidden)]
pub fn new_preset_editor_context<'a>(
    preset: &'a Identifier,
    dimensions: &'a Compound,
    seed: &'a str,
) -> PresetEditorContext<'a> {
    PresetEditorContext {
        preset,
        dimensions,
        seed,
    }
}

/// Construct a [`PresetEditorDraft`] for one editor callback invocation (engine only).
// hidden: the game owns the state borrowed by the editor
#[doc(hidden)]
pub fn new_preset_editor_draft<'a>(
    preset: &'a Identifier,
    dimensions: &'a mut Compound,
    validate: &'a dyn Fn(&Compound) -> bool,
) -> PresetEditorDraft<'a> {
    PresetEditorDraft {
        preset,
        dimensions,
        validate,
    }
}

#[cfg(test)]
mod tests {
    use super::new_preset_editor_context;
    use crate::*;

    struct TestEditor {
        title: String,
    }

    impl PresetEditor for TestEditor {
        fn title(&self) -> Option<String> {
            Some(self.title.clone())
        }
    }

    struct EditorMod;
    #[crate::__subscriptions]
    impl Hooks for EditorMod {
        fn register_preset_editors(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterPresetEditorsEvent<'_>,
        ) {
            event.register(Identifier::parse("example:islands").unwrap(), |ctx| {
                Ok(Box::new(TestEditor {
                    title: format!("{} ({})", ctx.preset(), ctx.seed()),
                }))
            });
        }
    }

    #[test]
    fn preset_editor_registration_uses_the_generated_dispatch() {
        testing::with_dispatch(
            vec![ModEntry::new("example", Box::new(EditorMod))].into(),
            || {
                let mut editors = Vec::new();
                dispatch_register_preset_editors(&RegistrationContext::new(), &mut editors);
                assert_eq!(editors.len(), 1);
                assert_eq!(editors[0].mod_id, "example");
                assert_eq!(
                    editors[0].answer.preset,
                    Identifier::parse("example:islands").unwrap()
                );
                let dimensions = Compound::new();
                let ctx = new_preset_editor_context(&editors[0].answer.preset, &dimensions, "42");
                let editor = (editors[0].answer.factory)(&ctx).unwrap();
                assert_eq!(editor.title().as_deref(), Some("example:islands (42)"));
            },
        );
        testing::with_dispatch(Vec::new().into(), || {
            let mut editors = Vec::new();
            dispatch_register_preset_editors(&RegistrationContext::new(), &mut editors);
            assert!(editors.is_empty());
        });
    }
}
