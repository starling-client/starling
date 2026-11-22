# Sending vanilla play packets

`ClientHost::send_packet` submits an owned `ServerboundPlayPacket`. The game validates the
body and reserves outgoing credit. Publication waits for the current root transaction to
commit; rollback, disconnect or a protocol change cancels it. Submission applies no local
prediction or inventory effects, and the server still checks reach, permissions and state.

Build common packet bodies with these portable associated constructors. Each returns
`Result<ServerboundPlayPacket, PacketBuildError>`:

| Constructor | Values |
| --- | --- |
| `use_item` | hand (0 main, 1 off), sequence, yaw, pitch |
| `use_item_on` | hand, block position, face (0 down, 1 up, 2 north, 3 south, 4 west, 5 east), relative cursor, inside, world border, sequence |
| `player_action` | action, block position, direction, sequence |
| `interact` | entity network ID, hand, hit location, sneaking |
| `attack` | entity network ID; vanilla 26.3 uses a separate attack packet |
| `pick_item_from_block` | block position, include data |
| `pick_item_from_entity` | entity network ID, include data |
| `sign_update` | block position, front side, four lines |
| `move_player_pos` | position, on ground, horizontal collision |
| `move_player_pos_rot` | position, yaw, pitch, on ground, horizontal collision |
| `move_player_rot` | yaw, pitch, on ground, horizontal collision |
| `move_player_status_only` | on ground, horizontal collision |

Player actions follow vanilla 26.3: 0 start destroy, 1 change destroy direction, 2 abort,
3 stop destroy, 4 drop all, 5 drop one, 6 release use, 7 swap offhand, 8 stab.
Entity IDs are network IDs, not Sunrise entity handles. Angles are degrees. Sequence
numbers and the corresponding client state remain the caller's responsibility.

The builders refuse non-finite positions, cursors and angles, unknown action/hand/direction
values, unrepresentable packed block positions and sign lines longer than 384 UTF-16 code
units. `PacketBuildError` identifies the packet and field. Interaction locations use vanilla's
lossy compressed-vector codec, including its clamping of large finite values.

`ClientHost::container_click(&ContainerClickRequest)` builds a container click from plain
carried/changed-slot `ItemStack`s. It needs the host because the packet carries hashed stacks.
The host uses the connection's item/component registry and the game's existing component
codecs and hashes. Submit the returned packet with `send_packet`; construction applies no
local menu effects. The request contains container/state IDs, slot, button, click type,
changed slots and carried stack. Click types are 0 pickup, 1 quick move, 2 swap, 3 clone,
4 throw, 5 quick craft and 6 pickup all.

Container limits are signed 32-bit container/state IDs, signed 16-bit slots, a signed-byte
button, at most 128 distinct changed slots, and the hashed component codec's count bounds.
The host refuses components it cannot encode or hash rather than silently omitting them.
No menu, reach, state or editor checks are replicated. Without a client connection, construction
returns `PacketBuildError::Unavailable`. `validate_container_click_limits` is also available
for preflighting the ID, slot and changed-slot-count wire limits.

For chat and commands, call `send_chat`; the game owns signing. There are no typed builders
for chat, signed commands, chat sessions, login/configuration, keep-alive, pong or custom
payload packets. Registered and engine-reserved custom-payload channels use their payload
or service API.
