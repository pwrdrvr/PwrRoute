# Accessibility mechanics

The app pins gpui-kit 0.6.6, gpui-base 0.6.6 and gpui-pre 0.3.6. This
implementation was checked against those crates' source, including
`elements/div.rs`, `elements/text.rs`, `window/a11y.rs` and Base's
`popover.rs` and `reduce_motion.rs`.

Custom controls use the same Return/Space click path as pointer clicks. Shared
buttons name themselves; other buttons derive their accessible names from real
AccessKit text descendants, with explicit names for icon controls. Switches,
radio choices, menus, dialogs, input values, headings and row summaries carry
semantics in the app's widget layer. Plain text now uses GPUI's identified
`Text` elements; tracked headings expose their whole phrase instead of one
node per glyph.

Menus keep their own focus scope. Up/Down wrap, Home/End select the ends, and
Return or Space activates the focused item. Tab stays inside an open menu or
sheet. Esc first collapses a text selection, then dismisses the innermost
surface. Closing a menu returns focus to its trigger; canceling Add a rule
returns to the initiating control or Activity list. The custom context is
`PwrRoutePopover`: Base already binds Enter/Space/Esc in `Popover`, which
would consume custom controls' keys. Settings' native Select and the app menu
bar keep their existing action handlers.

The main screens have Command+1 through Command+4 on macOS, Ctrl+1 through
Ctrl+4 elsewhere, in View order. Tab moves among Overview controls; Up/Down
still move through its map. Document windows close with Esc and scroll with
Page Up/Down and Home/End.

`accessibility_prefs::current` exposes live Reduce Motion and Increase
Contrast. Native macOS and Windows settings are read every two seconds,
including with no window open. Linux contrast uses the desktop settings
portal's initial value and change stream; Base owns its live motion stream.
Unavailable settings leave the last known value alone. Base preserves an
application motion override, including deterministic tests. Contrast calls
`theme::set_increase_contrast` on the GPUI thread and refreshes windows,
using the merged contrast palette. No color values are defined by this change.

## Validation boundaries

GPUI exposes these roles and names to its platform AccessKit adapters; there
is no missing role/label API in this pin. The pinned test platform does not
implement `PlatformWindow::a11y_init`, so window-harness tests cannot activate
its adapter or obtain a populated `debug_a11y_tree_json` snapshot. This blocks
a deterministic native-tree assertion in that harness. The private,
environment-gated E2E socket offers `{"cmd":"accessibility"}` for inspection
with an assistive tool attached: `active` reports the adapter state and `tree`
is null until it has built a frame. The keyboard regressions use real focus
and key events, including key-up activation.

The custom text fields expose a TextInput role, name and current value. They
do not yet expose editable-text ranges, selection offsets or a SetValue
accessibility action; adding that requires connecting their app-owned editing
model to GPUI's native input/accessibility action layer. This is an app input
integration gap, rather than a limitation of the role/label API. Metadata alone does not provide native screen-reader text editing.

Manual VoiceOver, Narrator/NVDA and Orca operation and live OS-setting changes
still require platform testing. Linux sandbox checks do not establish macOS
or Windows behavior. A successful fixture screen render checks layout, not
the accessibility tree. The keyboard-only tray changes are integrated with
the merged Hold redesign.
