# Android control system

Starling integrates Sunrise Android's control engine and visual editor. Open the
control hub to select **MCPE** or **GoyControl**, change the movement mode or edit
a layout. The layout entry in Controls > Touch Controls opens the same visual editor.

The editor supports moving and resizing controls, independent dimensions,
component properties, colors, borders, shapes, labels and event bindings. It has
undo/redo, visibility controls and a save/back confirmation. Presets keep their
names, descriptions and separate MCPE/GoyControl layouts under `controls/`;
`sunrise-touch.json` keeps the active selection.

The control hub includes movement and joystick configuration, aiming and axis
sensitivity, virtual pointer settings, gyroscope settings, button behavior and
inventory controls. GoyControl supports custom static buttons, pass-through,
swiping, hold/toggle behavior and up to three simultaneous events. MCPE supports
joystick/crosshair, joystick/tap and D-pad schemes.

The compact Controls page keeps Starling's Bedrock Inventory option. On the
main title screen, H and `…` sit below the existing account button; in game they
keep their original positions. Back retains its original menu behavior.

**360-degree movement** is available in H → Controls and is off by default.
With it off, touch movement uses Java directional sectors. **Look while
interacting** applies to both attacking and use/place gestures. Vertical swipes
starting on hub sliders scroll the pane; horizontal drags adjust their values.

The original MCPE artwork is downloaded from a pinned manifest with size and
SHA-256 checks. These files are read as a small disk overlay even when the
prepared vanilla resource archive predates their download.

Starling's chat shortcuts (history, completion, cursor and paste) and touch slot
gestures remain available. Inventory Shift/Control include the control system's
latched modifiers and held custom keys. Foreground/focus changes release touch
owners through the existing input paths.

This is independent of account management. It uses the current game's settings
folder; selecting an account does not select a control preset. Existing Starling
`touch-layout.json` files are preserved, but are not imported into the new preset
format automatically.

See `THIRD_PARTY_NOTICES.md` for the MIT source attribution.

Physical D-pad/keypad devices can select **Input profile: D-pad / keypad** in
**Controls > Key Binds**. The profile uses the original Android directional and
numeric aliases, supports rebinding and persists in `sunrise-keypad.json`.
Keyboard is the default profile. Text fields retain ordinary text input.

**Input device** in Controls selects Auto, Touch, Keyboard & Mouse or Controller.
Auto follows the last device used; an explicit selection keeps that mode active.
This preference is saved with the touch settings. When Virtual Mouse is enabled,
**Mouse** beside Back switches menu navigation between direct touch and the
relative pointer without disabling the feature globally.

Inventory follow-up: verify and improve item disposal and creative crafting in
Starling's custom Bedrock Inventory. These custom inventory workflows are not
provided by copying the control layout and must be checked separately from the
vanilla inventory. The Shift modifier routes slot taps through the normal
quick-move handler; scrollbar tracks retain direct dragging.

Text input uses Starling's shared focused-field/IME route. Fields with a
TextMirror adapter retain full text/selection synchronization; other fields
use the native Android
keyboard through focused-widget reports and the regular text event handlers. Custom mod interfaces must
expose text focus and handle text events; painting a box alone does not register
a text input. Back belongs to an open server dialog, including dialogs that
disallow closing, and cannot open the pause menu underneath it.

Android text fields reopen the keyboard on each explicit tap, including floating
keyboards hidden with Back. `EditBox` reports its focused bounds through
`preedit::report_field`; custom widgets can use that same contract and handle
normal screen text events, without adding their screen to a touch allowlist.
The focus report expires every frame. Full TextMirror adapters still provide
selection and composition synchronization for supported fields.

**H > Recommended > Keyboard Button** optionally adds **Keys** beside Mouse/Back in
menus (off by default). It is a manual shortcut; automatic field activation does
not depend on it.
