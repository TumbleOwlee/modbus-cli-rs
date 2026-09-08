# TUI — Requirements

Application shell and tab model, focus model, keyboard navigation, `:` command line mechanism, modal dialog/overlay mechanism, in-TUI vim-modal Lua/JSON code editor, syntax highlighting, reusable widget set, live value/log rendering.

IDs stable, append-only (`UI-R-nnn`). See [`../README.md`](../README.md). Companions: [`api-contract.md`](./api-contract.md) (exhaustive `:` command list, every keybinding table, code-editor mode/command set), [`edge-cases.md`](./edge-cases.md).

**Area boundaries.** This area owns the *mechanism*: how `:` commands parse and dispatch, generic (app-level) commands, keybindings, vim + arrow navigation, dialogs as a mechanism, the code editor, syntax highlighting. It does **not** own protocol-specific command *semantics*: a command forwarded to a module view (Modbus `:set`, `:reload`, OCPP `:rfid`, `:start`/`:stop`) is *listed* here with general syntax; its effect on protocol state is `modbus/` or `ocpp/`. Which config fields a dialog exposes and their valid ranges belong to the protocol / `config-session/` areas; the dialog *mechanism* is owned here. The process command line (`ferrowl run`, CLI flags) is `cli-headless/`; only the in-TUI `:` line is owned here.

---

## App shell, tabs & focus model

**UI-R-001** — The application presents a full-screen terminal UI in the alternate screen buffer with raw mode enabled, and restores the terminal (leave alternate screen, disable raw mode) on normal exit, on the error exit path, and from a panic hook.

**UI-R-002** — Screen layout top-to-bottom: the tab bar, a tab widget (UI-R-114) in `Horizontal` layout at its rendered extent across that direction (UI-R-121), one row under the default padding, flexible module content area, fixed-height log pane, one-row command line. The content area absorbs remaining height.

**UI-R-003** — The application owns an ordered list of tabs and one active index. Each tab pairs one module content view with its own log pane. Exactly one tab is active and rendered; the others keep running in the background (UI-R-030).

**UI-R-004** — Every tab has a unique display name.

**UI-R-069** — When an operation (in-dialog rename, session load) would make two tabs share a display name (UI-R-004), the later duplicate(s) are auto-suffixed.

**UI-R-070** — When a tab's display name is auto-suffixed (UI-R-069), a warning is logged into the renamed tab's own log.

**UI-R-005** — Input is routed by a single modal layer selector with precedence: keybind-help dialog (topmost) → app-level creation/session dialog → active tab's open overlay → command line → active tab's content/log panes.

**UI-R-071** — An open modal layer (UI-R-005) consumes the keys its lower layers would otherwise receive.

**UI-R-006** — Keyboard focus within the active tab is the content view or the log pane, never both.

**UI-R-072** — The `:` command line and any dialog remove keyboard focus from the active tab's panes (UI-R-006) while open and restore it on close.

**UI-R-073** — Every keyboard-focus transition within the active tab (UI-R-006) routes through a single choke point.

**UI-R-007** — Only key **press** events are acted upon; release/repeat kinds and non-key terminal events are ignored for command/navigation.

**UI-R-008** — Starting with no tabs configured opens the new-module type selector immediately.

**UI-R-057** — Whenever the application holds zero tabs and no modal layer is open (no app-level creation/type-select overlay, session dialog, or keybind-help dialog), it exits through the normal terminal-restoring path (UI-R-001). Cancelling the startup selector (UI-R-008) before any tab exists therefore quits. Independent of `:quit`/`:qall` (UI-R-019).

## Navigation & tab switching

**UI-R-009** — `Ctrl+w` begins a window-switch chord; a following `j`, `k`, `Down`, or `Up` toggles focus between the active tab's content view and log pane.

**UI-R-010** — `Ctrl+t` begins a tab-switch chord: `l` next tab, `h` previous (both wrap), a digit begins a by-index jump.

**UI-R-011** — A `Ctrl+t` digit jump: if the first digit already uniquely identifies a tab (no in-range two-digit index starts with it, or it is `0`), jump immediately.

**UI-R-074** — A `Ctrl+t` first digit that does not uniquely identify a tab (UI-R-011) waits up to 800 ms for a second digit; a second digit forming an in-range two-digit index jumps there.

**UI-R-075** — A second digit after a pending `Ctrl+t` first digit (UI-R-074) that forms an out-of-range two-digit combination falls back to the first digit's tab.

**UI-R-076** — A `Ctrl+t` second-digit wait (UI-R-074) that times out with no second digit commits the pending jump.

**UI-R-077** — Any non-digit while a `Ctrl+t` first digit is pending (UI-R-074) commits that jump and is then processed normally.

**UI-R-012** — A jump to an out-of-range or already-active index is a silent no-op. Tab-switch operations are safe with zero or one tabs.

**UI-R-013** — In a focused table or selection list, `j`/`Down` and `k`/`Up` move the row selection; `h`/`Left` and `l`/`Right` move the column selection (tables) or item (horizontal selection); `g` first row, `G` last, `0`/`Home` first column, `$`/`End` last. Selection clamps at the ends.

## Command line mechanism

**UI-R-014** — `:` while the content panes are focused and no view overlay is open enters command mode: focus moves to the command line, buffer cleared, printable keys type into it. With a view overlay open, `:` types into the overlay.

**UI-R-015** — In command mode, `Esc` cancels (discard buffer, restore content focus); `Enter` submits the trimmed buffer and restores content focus. Empty submission is a no-op.

**UI-R-016** — The command line is parsed by a pure, state-independent parser into a fixed set of app-level commands ([`api-contract.md`](./api-contract.md) ``## Generic `:` commands (app-level)``); leading/trailing and inter-token whitespace collapsed. Any first token not recognized at the app level is forwarded verbatim to the active view.

**UI-R-017** — App-level commands are dispatched by the application: tab lifecycle (`:quit`, `:qall`, `:new`, `:load`), session persistence (`:write`), tab reordering (`:swap`), session-script management (`:session`, `:script copy`), log-ring clear (`:log clear`). Exact syntax and aliases: [`api-contract.md`](./api-contract.md).

**UI-R-018** — A command not handled at the app level is forwarded to the active tab's view. If handled, any `(level, message)` returned is appended to the tab's log; if unhandled, the application logs `Unknown command ':<input>'` at Warning. The level of a result message is chosen by the producer, never re-derived from message text.

**UI-R-019** — `:quit` closes the active tab, stopping its module first, and quits the application only when it was the last tab. `:qall` quits immediately regardless of tab count.

**UI-R-020** — While the command line is focused, a help popup lists available commands: generic app-level commands plus whatever the active view advertises for its module type.

**UI-R-189** — A command-line widget state carries an open flag, a single-line input state holding the typed text and its cursor, an optional error message, an optional notice message and a hint string.

**UI-R-190** — Opening the command line (UI-R-189) sets it open, clears the input text and focuses the input.

**UI-R-191** — While the command line is open, `Enter` reports a submit outcome carrying the trimmed input text and closes the line.

**UI-R-192** — While the command line is open, `Esc` reports a cancel outcome and closes the line.

**UI-R-193** — While the command line is open, every key other than `Enter` and `Esc` is offered to the input state and the event is reported consumed.

**UI-R-194** — The command line renders, in this order of precedence, the `:` prompt followed by the input text and cursor while open, otherwise the error message in the theme's error style, otherwise the notice message, otherwise the hint.

**UI-R-195** — The command-line widget never clears its error or notice message on its own; both persist until the consumer clears them.

**UI-R-196** — The command-line widget builder takes a help list of usage and description pairs and, while the line is open and the list is non-empty, renders a bordered help box directly above the line, anchored to the bottom of the frame, one row per pair, with the usage column bold in the theme's highlight style.

**UI-R-197** — An empty help list (UI-R-196) renders no help box, and the command line occupies its single row alone.

**UI-R-198** — The command-line widget parses nothing: the submit outcome of UI-R-191 carries the raw trimmed string and the widget derives no command from it.

## Dialogs & overlays mechanism

**UI-R-021** — A dialog/overlay is a modal layer rendered over the content and log panes, consuming keyboard input while open. Overlays paint back-to-front (module overlays, command help popup, app-level dialog, keybind-help dialog on top).

**UI-R-022** — Within a dialog, `Tab` advances focus to the next field and `Shift+Tab`/`BackTab` retreats, cycling.

**UI-R-078** — A dialog's `Tab`/`Shift+Tab` focus cycle (UI-R-022) skips fields whose enabling condition is false.

**UI-R-079** — Within a dialog, `Enter` confirms; `Esc` requests close.

**UI-R-080** — The dialog key defaults (UI-R-022, UI-R-078, UI-R-079) apply only when the focused widget did not consume the key.

**UI-R-067** — A setup dialog opens with exactly one field focused, the first in its `Tab` cycle (UI-R-022), and its focus cursor names that same field. Every other field opens unfocused, nested sections included. Where the focused field is a text input, it is the dialog's only field painting a text cursor.

**UI-R-068** — A single-line input's border is styled by validation first and focus second: text failing validation paints the error style focused or not; only a field whose text validates, or has no validator, paints the focused style when focused and the normal border otherwise. A disabled single-line input never paints the focused style.

**UI-R-110** — The multi-line code editor styles its border from focus only and keeps the focused border while disabled, since a read-only viewer still holds focus for scrolling.

**UI-R-111** — As a consequence of UI-R-068, a fresh setup dialog shows a focused field with an error border (empty name input against a non-empty requirement); a dialog opened by `:edit` prefills the name and shows the focused border.

**UI-R-023** — `Esc` on a dialog that may hold unsaved edits opens a close-confirmation popup; confirming (`Enter` or `Space`) closes, dismissing (`Esc`) returns to editing. A yes/no box defaults focus to the safe (cancel) choice.

**UI-R-024** — The new-module flow is two staged overlays: module-type selector, then the chosen type's setup dialog. Confirming the selector swaps in the setup dialog; confirming a valid setup dialog creates and starts the tab. A dialog failing validation stays open.

**UI-R-025** — Creating a tab whose name collides with an existing tab is refused with a warning in the active tab's log, dialog left open.

**UI-R-026** — A field-completion popup (suggestion input), while open, consumes `Up`/`Down` to move the highlight, `Enter` to accept, `Esc` to dismiss. While closed, `Up`/`Down`/`Enter`/`Esc` pass through to the dialog.

**UI-R-081** — `Tab` is never consumed by a field-completion popup (UI-R-026), so it always moves focus to the dialog's next field.

**UI-R-082** — Accepting a field-completion popup suggestion (UI-R-026) marked *partial* keeps the popup open and re-queries (e.g. descending a directory); accepting a non-partial one closes it.

**UI-R-199** — An editor-dialog widget renders a centered bordered box over the frame, clearing the cells beneath it, with a caller-supplied title on its border.

**UI-R-200** — The editor dialog's box takes a percentage of the frame's width and height, both builder-settable, defaulting to 60 percent of the width and 50 percent of the height, and is never smaller than its builder-settable minimum of 40 columns by 8 rows.

**UI-R-201** — The editor dialog holds one editable markdown input field in the vim-modal profile, sized to the box's inner area, opened with empty text in `Normal` mode.

**UI-R-202** — `Enter` in `Normal` mode with text that is not blank reports a confirmed outcome carrying the field's text and closes the editor dialog.

**UI-R-203** — `Enter` in `Normal` mode with blank text — empty or whitespace only — leaves the editor dialog open and reports the event consumed.

**UI-R-204** — `Esc` in `Normal` mode reports a cancelled outcome and closes the editor dialog.

**UI-R-205** — Every key the editor dialog does not act on itself (UI-R-202 through UI-R-204) is offered to its markdown field, so `Esc` in `Insert` or a Visual mode only returns the field to `Normal` (UI-R-028) and leaves the dialog open.

**UI-R-206** — The editor dialog's border title shows the field's current vim mode label next to the caller-supplied title (UI-R-199), in the same form the code editor's border uses.

## Script-manager dialog

**UI-R-051** — In the script-manager dialog, while the script table is focused, `e` executes the selected script exactly once, using the script's current editor content (including unapplied edits) regardless of its enabled flag. No selection → no-op. Execution semantics: SC-R-035.

**UI-R-088** — An on-demand script run (UI-R-051) leaves the script-manager dialog open; the run's `print`/`C_Log` output and any error appear in the dialog's log pane.

**UI-R-052** — The script-manager dialog carries a *Templates* button in its focus cycle, after the new-script name input. `Enter` or `Space` on it opens the template-browser overlay.

**UI-R-053** — The template-browser overlay lists only templates applicable to the dialog's script context (SC-R-036), each with name and description, plus a read-only syntax-highlighted preview of the selected template's code. `Esc` or `q` closes without changing the script list. While open it takes precedence over all other dialog keys.

**UI-R-054** — Confirming a template appends it to the dialog's working script list as a new enabled script whose code copies the template body, selects it, closes the overlay, leaves the dialog open. The script takes the template's name; if taken, the first free `<name>-<n>` (n from 2); insertion is never refused for a name collision.

**UI-R-055** — In the script-manager dialog, while the script table is focused, `Enter` on a selected script opens a rename prompt pre-filled with its name. `Enter` renames; `Esc` dismisses unchanged. Renaming changes only the name.

**UI-R-089** — In the script rename prompt (UI-R-055), an empty (after trimming) or already-used name is refused and the prompt stays open.

**UI-R-090** — In the script-manager dialog, `Enter` on the script table with no selection (UI-R-055) is a no-op.

**UI-R-056** — In the script-manager dialog, while the script table is focused, `?` opens a keybind-help overlay listing the table's bindings (rename, run once, toggle enabled, delete, compact) with a one-line description each. `Esc`, `q`, or `?` closes. While open it takes precedence over all other dialog keys. The table's title advertises only this overlay.

**UI-R-058** — In the script-manager dialog, while the script table is focused, the table supports editing its working list: a non-empty name in the new-script input plus confirm adds a new enabled, empty script (empty or already-used name refused, per UI-R-089).

**UI-R-091** — In the script-manager dialog, while the script table is focused, `t` toggles the selected script enabled/disabled in the table's working list (UI-R-058); `t` is a no-op with no selection.

**UI-R-092** — In the script-manager dialog, while the script table is focused, `d` deletes the selected script from the table's working list (UI-R-058) after a yes/no confirmation (UI-R-023); `d` is a no-op with no selection.

**UI-R-093** — In the script-manager dialog, while the script table is focused, `c` toggles compact/normal rows.

**UI-R-094** — Script-table edits (UI-R-058, UI-R-091, UI-R-092) change the working list only and take effect on the owner when the dialog is applied (SC-R-024); UI-R-058, UI-R-091, UI-R-092 and UI-R-093 are the bindings advertised by UI-R-056.

## OCPP setup dialog

**UI-R-059** — In the OCPP setup dialog, a client-role instance shows two always-visible new-header inputs (name, value) backing `extra_headers`, plus the headers table once the list is non-empty; both hidden for the server role (the `#[focus(when = …)]` role-conditional pattern).

**UI-R-095** — In the OCPP setup dialog, while the headers table (UI-R-059) is focused, a name and value in the new-header inputs plus confirm adds a row, refused inline if name/value fails OC-R-118's grammar or OC-R-153's reserved-name check.

**UI-R-096** — In the OCPP setup dialog, while the headers table (UI-R-059) is focused, `Enter` on a selected row opens an edit prompt pre-filled with its name and value, UI-R-095's validation applied on confirm, `Esc` dismissing unchanged; `Enter` is a no-op with no selection.

**UI-R-097** — In the OCPP setup dialog, while the headers table (UI-R-059) is focused, `d` deletes the selected row after a yes/no confirmation (UI-R-023); `d` is a no-op with no selection.

**UI-R-098** — Rows of the OCPP setup dialog's headers table (UI-R-059) keep insertion order (OC-R-117).

**UI-R-099** — Edits to the OCPP setup dialog's headers table (UI-R-095, UI-R-096, UI-R-097) change the working list only and take effect when the dialog is applied.

## Code editor (vim-modal)

**UI-R-027** — The multi-line code editor supports two profiles: plain single-mode (printable keys insert, `Enter` splits, `Backspace`/`Delete` edit, arrows navigate with wrap) and vim-modal (`Normal`/`Insert`/`Visual`). Vim-modal is default for the Lua-script editor.

**UI-R-028** — Vim-modal: `Normal` provides motions and operators; `i`/`a`/`I`/`A`/`o`/`O` enter `Insert` at the documented position; `v`/`V` enter charwise/linewise `Visual`; `Esc` from `Insert` or `Visual` returns to `Normal`; `Esc` in `Normal` is left unhandled so it reaches the dialog. Exact motion/operator set: [`api-contract.md`](./api-contract.md) `## Code editor — modes and commands`.

**UI-R-029** — The editor keeps the vim block-cursor invariant in `Normal` (cursor rests on a character, clamping to the last column); `Insert` allows one past the last character. `h`/`l` do not wrap across lines; arrows wrap to the adjacent line.

**UI-R-030** — Yank (`y`, `yy`) and delete (`d`, `dd`, `x`) write the removed/copied text into an internal register (linewise or charwise) used by paste (`p`/`P`).

**UI-R-083** — Yank and delete (UI-R-030) also emit the removed/copied text to the system clipboard via an OSC 52 escape, best-effort, never failing the edit.

**UI-R-031** — Single-level undo (`u`): each mutating operation snapshots the buffer before applying; `u` swaps current buffer with the snapshot (pressing again redoes). Motions and mode changes do not consume the slot.

**UI-R-032** — With a language set, the editor auto-indents on newline and `o`/`O`: the new line inherits the current line's leading indentation adjusted by the language's per-line block-balance delta (four spaces per level), floored at zero. Without a language, no automatic indent.

**UI-R-033** — With a language set and the field enabled, losing focus reformats the buffer through the language formatter; if the formatter declines (e.g. invalid JSON), the buffer is unchanged. A disabled field never reformats; gaining focus never reformats.

**UI-R-034** — In `Insert`, `Tab` inserts four spaces; `Shift+Tab` removes up to four leading spaces from the current line.

**UI-R-084** — In the plain editor, two space presses at the same cursor position within a short bound (default 300 ms) expand to a four-space indent; an intervening key cancels.

**UI-R-035** — All cursor movement, insertion, deletion are character-based, not byte-based, so multi-byte UTF-8 is never split or miscounted.

**UI-R-036** — A disabled code editor ignores all mutating keys (insert, delete, paste, mode entry that would edit) while permitting navigation, and reports such keys unhandled.

**UI-R-164** — The multi-line code editor's state carries an optional list of gutter labels, one entry per buffer line, settable both when the state is built and afterwards; the default is no labels.

**UI-R-165** — With gutter labels set (UI-R-164), the gutter cell of buffer row `i` renders the label at index `i` in place of that row's line index.

**UI-R-166** — An empty gutter label renders as a blank gutter cell of the full gutter width, so a filler row shows no number.

**UI-R-167** — With gutter labels set, gutter contents are right-aligned within a width of one separator space plus the widest of every entry in the label list (surplus entries past the end of the buffer included, UI-E-080) and, whenever at least one row falls back to its line index (UI-R-168), the widest such fallback index, the whole subject to the clamp of UI-R-172.

**UI-R-168** — A buffer row with no entry in the gutter-label list, because the list is shorter than the buffer, falls back to rendering that row's line index.

**UI-R-169** — Gutter styling is independent of gutter content: a row rendering a gutter label is styled exactly as the same row rendering its line index would be, for the active row and every other row alike.

**UI-R-172** — The gutter width of UI-R-167 is clamped to the field's area width, never exceeding it and never wrapping, so the gutter is always drawn inside the widget's area.

**UI-R-173** — The multi-line code editor's state remembers the visible height in rows of its last render, and reports one row before the first render.

**UI-R-174** — `PageDown` and `PageUp` move the code editor's active line down or up by the remembered visible height (UI-R-173), clamped to the first and last buffer line.

**UI-R-175** — `Ctrl+D` and `Ctrl+U` move the code editor's active line down or up by half the remembered visible height (UI-R-173), rounded down and never less than one line, clamped to the first and last buffer line.

**UI-R-176** — While the code editor is disabled, `h`, `l`, `Left` and `Right` scroll the viewport one column left or right instead of moving the cursor within the line, and the cursor column follows the scroll so the keep-the-cursor-visible logic never scrolls the view back.

**UI-R-177** — The horizontal scroll of UI-R-176 is clamped to zero at the left and, at the right, to the last column of the widest line in the buffer.

**UI-R-178** — While the code editor is disabled, `0` sets the horizontal scroll to the first column.

**UI-R-179** — While the code editor is disabled, `$` sets the horizontal scroll to the smallest offset that brings the active line's last column into view.

**UI-R-180** — A vertical move in a disabled code editor leaves the horizontal scroll unchanged, whatever the length of the line moved onto.

**UI-R-181** — The paging of UI-R-174 and UI-R-175 and the read-only horizontal scrolling of UI-R-176 through UI-R-180 are available in both the plain and the vim-modal editor profiles (UI-R-027).

**UI-R-182** — A disabled vim-modal code editor keeps `gg`, `G`, `v`, `V` and `Esc` at their enabled meanings (UI-R-028), so a read-only field can still enter Visual mode and hold a charwise or linewise selection.

## Markdown input field

**UI-R-181** — The markdown input field is a multi-line widget composing the vim-modal code-editor state (UI-R-027 through UI-R-036): buffer, `Normal`/`Insert`/`Visual` modes, motions and operators, registers, single-level undo and the disabled flag, which the markdown widget surfaces as read-only.

**UI-R-182** — Focused, editable, in `Normal` mode: every source line is drawn in its rendered form except the line holding the cursor, which is drawn as styled source, revealing that line's markup for editing.

**UI-R-183** — Focused, editable, in `Insert` or `Visual` mode: every source line is drawn as styled source, with no rendered lines.

**UI-R-184** — Unfocused, or read-only in any state, every source line is drawn in its rendered form, including the cursor line; no line reveals its source.

**UI-R-129** — The styled-source view of UI-R-182 and UI-R-183 is produced by the Markdown language of UI-R-037: source characters are drawn unchanged, styled by highlight span.

**UI-R-130** — Line wrapping is always on and cannot be disabled: content never overflows the widget width horizontally and the widget never scrolls horizontally.

**UI-R-131** — A line too long for the available width breaks at a word boundary; a single word longer than the available width breaks at a character boundary instead.

**UI-R-132** — Continuation rows of a wrapped list item or block quote are indented to the content start of that line (the column after its marker), so wrapped text aligns under the first row's text.

**UI-R-133** — A fence delimiter or fence-body line wraps at a character boundary with no hanging indent.

**UI-R-134** — `j`, `k`, their count prefixes, `dd`, `yy` and gutter line numbers address source lines, not display rows: `j` from a wrapped line moves to the next source line.

**UI-R-135** — `gj` and `gk` move the cursor one display row down or up, staying inside a wrapped source line when it spans several rows.

**UI-R-136** — `Ctrl+D` and `Ctrl+U` scroll by half the visible height measured in display rows and move the cursor by the same number of display rows.

**UI-R-137** — Scrolling is measured in display rows and the viewport always keeps the cursor's display row visible.

**UI-R-138** — Read-only, the display row range of the cursor's source line is drawn in the theme's highlighted-row style, so line navigation is visible without a text cursor.

**UI-R-139** — Read-only, `j`, `k`, their count prefixes, `yy` (with its count prefix), `gg`, `G`, `Ctrl+D` and `Ctrl+U` remain available, so a reader can navigate lines and display rows and yank them.

**UI-R-140** — An optional line-number gutter, off by default and selected on the widget builder, prints the source line number on the first display row of each source line and leaves continuation rows blank.

**UI-R-141** — Markdown rendering styles come from a markdown theme with compile-time defaults, injected on the widget builder like the syntax theme, holding per-level heading styles, per-level quote-bar styles, and link, inline-code, bullet, horizontal-rule, image and read-only highlighted-row styles.

**UI-R-142** — Rendering is line-preserving: one source line produces exactly one rendered line, wrapped over one or more display rows; lines are never joined into paragraphs and no construct is laid out across lines.

**UI-R-143** — A heading line hides its `#` markers and the space after them and draws the remaining text bold in the theme's color for that heading level.

**UI-R-144** — An unordered list item replaces its `-`, `*` or `+` marker with `•` in the theme's bullet style, keeping the leading indentation that expresses nesting depth.

**UI-R-145** — An ordered list item keeps its number and delimiter as written, unchanged.

**UI-R-146** — A task item replaces `- [ ]` with `☐` and `- [x]` with `☑`, keeping the item's text.

**UI-R-147** — A block-quote line replaces each `>` marker with a `▎` bar in the theme's quote-bar color for that nesting depth and draws the quoted text dimmed and italic.

**UI-R-148** — A horizontal rule (`---` or `***`) is drawn as a rule line spanning the full text width, after the gutter when enabled, in the theme's rule style.

**UI-R-149** — A fence delimiter line hides its backticks and info string, rendering as an empty line in the theme's code style, so the one-line-per-source-line invariant of UI-R-142 holds.

**UI-R-150** — A fence-body line is drawn verbatim, character for character, in the theme's code style.

**UI-R-151** — When the fence's info string is `lua` or `json`, its body lines are additionally highlighted through the syntax highlighter for that language (UI-R-037) with the carry-over line state threaded across the block; any other info string, or none, leaves the body in plain code style.

**UI-R-152** — Inline `**bold**`, `*italic*`/`_italic_`, `` `code` `` and `~~strike~~` hide their markers and draw the content bold, italic, in the theme's code style, and crossed out respectively.

**UI-R-153** — `[text](url)` renders as `text` in the theme's link style, underlined; the brackets, parentheses and URL are hidden.

**UI-R-154** — `![alt](url)` renders as `alt` in the theme's image style; the `!`, brackets, parentheses and URL are hidden.

**UI-R-155** — A read-only markdown input field ignores every mutating key and every mode-entry key, reporting them unhandled as the disabled code editor does (UI-R-036), so the widget never leaves `Normal` mode.

**UI-R-188** — The markdown input field widget measures text without rendering it: given a text and an available width, it reports the number of display rows that text would occupy if drawn by that widget at that width, applying the same wrapping, hanging-indent and gutter rules as a render (UI-R-130 through UI-R-133, UI-R-140, UI-R-142), and mutating no state.

## Diff widget

**UI-R-207** — The diff widget takes a unified diff text and, optionally, the full new-side file text, and parses the diff into hunks, each hunk's header line supplying the old-side and new-side starting line numbers of its body.

**UI-R-208** — The diff widget classifies each parsed body line by its first character — space as context, `+` as added, `-` as removed — and keeps the remainder of the line as that line's text; every other line of the input is a meta line kept verbatim.

**UI-R-209** — The diff widget aligns the diff into rows holding an optional old-side entry and an optional new-side entry: a context line occupies both entries of one row, a run of removed lines pairs positionwise with the run of added lines that follows it, a surplus line on either side occupies a row whose other entry is a filler, and, with the full new-side text supplied (UI-R-207), every file line between and around the hunks occupies a further row holding that line on both sides.

**UI-R-210** — A meta line (UI-R-208), file header and hunk header included, occupies a row of its own drawn in the diff widget's own meta row style (UI-R-276) across the full width of the widget, with a blank gutter on every side.

**UI-R-211** — In the split layout the diff widget draws two panes of equal width side by side, the old side left and the new side right, rendering one screen row per aligned row (UI-R-209) so corresponding old and new lines always sit on the same screen row.

**UI-R-212** — A filler entry (UI-R-209) renders as an empty text area with a blank gutter cell of the full gutter width, so the row carries no line number and no text on that side.

**UI-R-213** — In the unified layout the diff widget draws one pane, rendering each aligned row (UI-R-209) as its old-side entry followed by its new-side entry when both are present and differ, and as a single screen row otherwise, so the widget's selection and active row stay addressed in aligned rows in either layout.

**UI-R-214** — The diff widget's layout is a builder option defaulting to split (UI-R-211).

**UI-R-215** — `Ctrl+T` toggles the diff widget between the split and unified layouts at runtime.

**UI-R-216** — A rendered entry carries no marker column: its text starts in the cell immediately after its gutter, and the row's kind is signalled by its row style alone (UI-R-219, UI-R-276, UI-R-278).

**UI-R-217** — An entry's gutter cell holds that side's file line number, counted from the hunk header's starting line for that side (UI-R-207).

**UI-R-218** — The diff widget takes an optional list of gutter labels per side, settable when built and afterwards, whose entry for a row replaces that side's line number in the gutter, following the gutter rules of the code editor (UI-R-165 through UI-R-169, UI-R-172).

**UI-R-219** — A row's text is styled by its diff kind, using the diff widget's own added, removed and meta row styles (UI-R-276); a context line takes the theme's normal text style.

**UI-R-220** — The diff widget takes one optional syntax language applying to both sides; with a language set, every entry's text is highlighted by that language (UI-R-037) and the highlight spans supply the foreground, the diff-kind style of UI-R-219 supplying every other attribute.

**UI-R-221** — The syntax language of UI-R-220 defaults to none, in which case the diff-kind style of UI-R-219 alone styles the text.

**UI-R-222** — The diff widget is read-only: it holds no editable buffer and reports every mutating and Insert-entering key unhandled, as the disabled code editor does (UI-R-036).

**UI-R-223** — The diff widget has two modes, `Normal` and `Visual`: `v` and `V` enter Visual from Normal, `Esc` in Visual returns to Normal, `Esc` in Normal is left unhandled so it reaches the enclosing layer (UI-R-028), and no Insert mode exists.

**UI-R-224** — The active row is drawn in the theme's read-only highlighted-row style on every pane at once (UI-R-138), so the reader sees the same row marked on both sides.

**UI-R-225** — In Visual mode every row from the selection anchor to the active row inclusive is drawn in the selection style on every pane at once.

**UI-R-226** — The diff widget reports its selected rows as the active row alone in `Normal` mode and as the inclusive range between the selection anchor row and the active row, ordered ascending, in Visual mode.

**UI-R-227** — The diff widget answers, for any of its rows, that row's diff kind and its old-side and new-side file line numbers, each absent where that side holds a filler, so a consumer maps a selected row range (UI-R-226) to file lines without parsing the diff itself.

**UI-R-228** — The diff widget carries a focused side, settable and queryable, whose pane paints the focused border style while the other paints the normal one.

**UI-R-229** — `yy` in `Normal` and `y` in Visual copy the focused side's text of the selected rows (UI-R-226) into the register and to the system clipboard, as the code editor's yank does (UI-R-030, UI-R-083), skipping rows whose focused side holds a filler.

**UI-R-230** — `j`, `k`, their count prefixes, `gg` and `G` move the active row within the widget's aligned rows, and the single vertical scroll offset keeps the active row visible, so both panes always show the same row range.

**UI-R-231** — `PageDown`, `PageUp`, `Ctrl+D` and `Ctrl+U` move the diff widget's active row by as many display rows as the visible height or half of it, landing on the logical row holding the display row reached, with the remembered-height and clamping semantics of UI-R-173 through UI-R-175.

**UI-R-232** — With wrapping off (UI-R-260), `h`, `l`, `Left`, `Right`, `0` and `$` scroll the diff widget horizontally with the semantics of UI-R-176 through UI-R-179, applying one horizontal offset to every pane at once.

**UI-R-233** — `]c` moves the active row to the first row of the next hunk and `[c` to the first row of the previous hunk, each clamping at the last and first hunk.

**UI-R-253** — With the full new-side text supplied (UI-R-207), each row's new-side entry is the corresponding line of that text, the old side is reconstructed from it — a context line repeated with its old-side number offset by the cumulative line delta of the preceding hunks — and removed lines come from the patch; the widget runs no diff algorithm of its own.

**UI-R-254** — The aligned row list (UI-R-209) is built once per input and is the same list in either display mode, the full-file rows being a superset of the hunk rows.

**UI-R-255** — Hunk-only mode is that same row list (UI-R-254) carrying fold ranges over the unchanged spans outside the hunks, so switching modes flips folds instead of rebuilding and a row keeps its index across the switch.

**UI-R-256** — A folded span (UI-R-255) contributes no display rows: its rows are neither drawn nor reachable by navigation, while their indices remain valid.

**UI-R-257** — The diff widget's display mode is a builder option defaulting to hunk-only.

**UI-R-258** — `Ctrl+F` toggles the diff widget between hunk-only and full-file display at runtime.

**UI-R-259** — Constructing the diff widget from a unified diff text alone (UI-R-207) yields hunk-only display with no full-file mode to switch to.

**UI-R-260** — The diff widget takes a line-wrap option, builder-set and defaulting to off; with it on, a row's text too wide for its pane continues on further display rows, breaking at a whitespace boundary and never inside a word.

**UI-R-261** — A continuation display row of a wrapped row (UI-R-260) carries a blank gutter cell and starts at the same column as the first display row's text.

**UI-R-262** — In the split layout a logical row occupies as many display rows as the taller of its two sides needs when wrapped (UI-R-260), the shorter side padded with blank display rows, so corresponding old and new lines keep starting on the same display row.

**UI-R-263** — With wrapping on (UI-R-260) the horizontal offset is fixed at zero and `h`, `l`, `Left`, `Right`, `0` and `$` are consumed without moving the view.

**UI-R-264** — The active row, the selection anchor and the selected-row query (UI-R-226) address logical rows, so a wrapped row or an annotated row is selected whole.

**UI-R-265** — The vertical scroll offset, the paging of UI-R-231 and the keep-the-active-row-visible settle count display rows, so wrapping and annotations never make part of a row unreachable.

**UI-R-266** — The diff widget takes a list of marked ranges, each naming a side, a file line range on that side and a colour, settable when built and afterwards.

**UI-R-267** — For every row a marked range covers (UI-R-266), the widget fills that side's gutter cell with the range's colour, so a marked span reads as one continuous block down the gutter.

**UI-R-268** — Marked ranges and gutter labels coexist on the same row: the label supplies the gutter's text (UI-R-218) and the range its colour (UI-R-267).

**UI-R-269** — The diff widget takes a list of annotations, each naming a side, a file line range on that side and a markdown text, settable when built and afterwards.

**UI-R-270** — An annotation renders as a bordered block directly beneath the last row of its range, spanning the full width of the widget in the unified layout and both panes in the split layout.

**UI-R-271** — An annotation's text is rendered by a read-only markdown input field, so its body shows the rendered markdown of UI-R-128 rather than its source.

**UI-R-272** — An annotation block's height is the display rows its text measures at the block's inner width (UI-R-188) plus its border rows.

**UI-R-273** — Annotations are display-only: no annotation is ever the active row, none is reachable by navigation, and none appears in the selected-row query (UI-R-226).

**UI-R-274** — Annotations are shown when the widget is built.

**UI-R-275** — `Ctrl+A` toggles every annotation between shown and hidden at once, hidden annotations contributing no display rows.

**UI-R-276** — The diff widget's added, removed and meta row styles are builder-settable and default to white on the color scheme's success color darkened toward black, white on its error color darkened toward black, and the theme's meta style respectively (UI-R-219).

**UI-R-278** — The added and removed row styles (UI-R-219, UI-R-276) paint every cell of that entry's row across the full width of its pane — gutter cell, text cells and the blank cells past the end of the text alike — in either layout (UI-R-211, UI-R-213), so the row reads as one uninterrupted band.

**UI-R-279** — Every continuation display row of a wrapped added or removed row (UI-R-260, UI-R-261) is painted across its pane's full width in that row's style, as UI-R-278 paints the first display row.

**UI-R-280** — Within a hunk, the removed rows of a run and the added rows of the run immediately following it are paired in order, the n-th removed row with the n-th added row.

**UI-R-281** — For each pair of UI-R-280 the widget diffs the two texts over word tokens — one token being a run of word characters (letter, digit or `_`) and one being a run of any other characters — and the tokens present on only one side form that side's word-diff spans.

**UI-R-282** — The diff widget carries an added-word and a removed-word emphasis style beside its row styles (UI-R-276), builder-settable, defaulting to a background lighter than the added and the removed row style's background respectively.

**UI-R-283** — A row's word-diff spans (UI-R-281) are painted in the added-word emphasis style on an added row and in the removed-word emphasis style on a removed row (UI-R-282), every other cell of the row keeping the full-width band of UI-R-278.

**UI-R-284** — Word-diff emphasis (UI-R-283) sets the background only: a span's foreground stays whatever UI-R-219 through UI-R-221 give it, so syntax highlighting survives inside the emphasised words.

## File tree widget

**UI-R-234** — The file tree widget's state is built from a list of file paths and derives the directory nodes from the paths' components, so a caller supplies paths alone and never assembles a tree.

**UI-R-235** — Every directory node is expanded when the tree is built, and the state can expand all directories or collapse all of them in one call.

**UI-R-236** — A collapsed directory's subtree contributes no rows, so the widget's visible rows are the root's children plus, recursively, the children of every expanded directory.

**UI-R-237** — Sibling nodes are ordered directories first and files second, each group ordered by name, so the same path list always renders in the same order.

**UI-R-238** — Each row is drawn indented by its depth, a directory carrying an expansion marker — `▾` expanded, `▸` collapsed — and a file carrying none.

**UI-R-239** — `j`, `Down`, `k` and `Up` move the selection one visible row, and `gg` and `G` move it to the first and last visible row, clamping at the ends (UI-R-013).

**UI-R-240** — `l` and `Right` expand the selected directory when it is collapsed, move the selection to its first child when it is already expanded, and do nothing on a file.

**UI-R-241** — `h` and `Left` collapse the selected directory when it is expanded, and otherwise move the selection to the node's parent directory.

**UI-R-242** — `Enter` toggles the selected directory's expansion, and on a file reports an activation outcome carrying that file's full path.

**UI-R-243** — The file tree widget answers the selected node's full path and whether that node is a directory.

**UI-R-244** — A file node may carry a change status of added, removed or modified, drawn as a leading `+`, `-` or `~` marker and styling that row with the syntax theme's added, removed and meta styles (UI-R-162, UI-R-163); a node with no status takes the normal text style.

**UI-R-245** — The file tree's viewport scrolls vertically to keep the selected row visible, and `PageDown`, `PageUp`, `Ctrl+D` and `Ctrl+U` move the selection with the remembered-height and clamping semantics of UI-R-173 through UI-R-175.

**UI-R-246** — The file tree paints the focused border style while focused and the normal border otherwise (UI-R-110).

**UI-R-252** — The file tree draws its selected row in the theme's highlighted-row style across the widget's full width, as the diff widget draws its active row (UI-R-224), the row's change-status styling (UI-R-244) supplying the foreground.

## Diff-review example

**UI-R-247** — The TUI crate ships a runnable diff-review example whose layout is a top row of two selection inputs, labelled as base and branch, above a file browser on the left and a diff widget on the right.

**UI-R-248** — The example's two selection inputs offer the local repository's git branches, and the example shows the real diff between the selected base and branch.

**UI-R-249** — `Tab` cycles focus through the example's four panes — base input, branch input, file browser, diff viewer — in that order and wrapping, and `Shift+Tab` cycles in reverse.

**UI-R-250** — The example's file browser holds the paths changed between the selected base and branch, each carrying its change status (UI-R-244).

**UI-R-251** — Activating a file in the example's browser (UI-R-242) shows that file's diff in the diff widget, and changing either selection rebuilds both the browser's paths and the shown diff.

**UI-R-277** — The diff-review example supplies the selected file's full new-side text from the selected branch (UI-R-207), so its viewer shows the whole file with the changes marked in place.

## Syntax highlighting

**UI-R-037** — Syntax highlighting is pure text-to-span computation: for a language and one line of source (plus a carry-over line state for multi-line constructs) it returns `(start_char, end_char, kind)` spans, sorted by start, non-overlapping, character indices. Four languages: Lua, JSON, Markdown and Diff.

**UI-R-038** — The carry-over state lets multi-line constructs (Lua long strings and long comments) highlight correctly across lines when highlighted in order.

**UI-R-039** — Highlight kinds are a fixed enumeration (keyword, identifier, number, string, comment, punctuation, JSON key, literal, object identifier, function identifier, diff added, diff removed, diff meta); the consumer maps kind to colors. Highlighting never mutates the source.

**UI-R-156** — The Diff language classifies whole lines only: highlighting one line yields at most one span, and that span covers the entire line from the first to the last character.

**UI-R-157** — A Diff line whose first character is `+` yields a span of the diff added kind.

**UI-R-158** — A Diff line whose first character is `-` yields a span of the diff removed kind.

**UI-R-159** — A Diff line starting with `@@`, `---`, `+++`, `diff ` or `index ` yields a span of the diff meta kind; this classification is tested before UI-R-157 and UI-R-158, so `+++` and `---` are meta, not added or removed.

**UI-R-160** — A Diff line matching neither UI-R-157, UI-R-158 nor UI-R-159 (a space-prefixed context line, an empty line, any other text) yields no span at all and is therefore rendered in the field's general style.

**UI-R-161** — Diff highlighting is line-independent: it neither reads nor changes the carry-over line state (UI-R-038), so a Diff line highlights identically whatever precedes it.

**UI-R-162** — The syntax theme's kind-to-style lookup returns the theme's diff added, diff removed and diff meta styles for the corresponding kinds of UI-R-039.

**UI-R-163** — The default diff styles set foreground only, leaving background and modifiers unset: diff added takes the color scheme's success color, diff removed its error color, diff meta its placeholder color.

**UI-R-170** — The Diff language provides no formatter, so a field set to Diff is left byte-for-byte unchanged on blur (UI-R-033).

**UI-R-171** — The Diff language's per-line block-balance delta is always zero, so auto-indent on newline (UI-R-032) in a Diff field only inherits the current line's leading indentation.

**UI-R-176** — The Markdown language exposes, beside the span path of UI-R-037, a per-line block-model entry point: given one source line and a carried state it returns that line's block kind — paragraph, heading with level 1–6, unordered list item with nesting depth and marker character, ordered list item with nesting depth, task item with checked state, block quote with nesting depth, horizontal rule, fence delimiter with info string, or fence body — together with the inline spans of UI-R-178.

**UI-R-177** — The block-model carried state of UI-R-176 records whether the parser is inside a fenced code block and that fence's info string; while inside a fence every line is classified as fence body regardless of its own content, until a matching closing fence delimiter line.

**UI-R-178** — Inline parsing reports, over source character columns of one line, spans for `**bold**`, `*italic*`, `_italic_`, `` `code` ``, `~~strike~~`, `[text](url)` and `![alt](url)`, each distinguishing its marker columns (delimiters, brackets, URL) from its content columns, so a consumer can hide markers without recomputing positions.

**UI-R-179** — A character preceded by a backslash is literal: the backslash is a marker column and the escaped character never opens or closes an inline construct.

**UI-R-180** — Constructs the block model does not recognize — tables, raw HTML, footnotes, reference links, setext headings, autolinks — are reported as paragraph text with no inline spans over them.

## Tables, live updates & logging

**UI-R-040** — On every UI tick the application polls **all** tabs' views to refresh state, not only the active, so background modules keep sending/receiving and their values and logs stay current. Refreshes are polled concurrently, so tick latency is bounded by the slowest tab, not their sum.

**UI-R-041** — The UI redraws on input and on a periodic timeout (≈100 ms) with no input.

**UI-R-042** — A view may request replacement by a different view (e.g. an OCPP role switch); the application applies it on the next tick, carries over the tab's log channel and focus, rebuilds the session-module registry.

**UI-R-043** — Each tab owns a bounded ring log of timestamped, severity-tagged lines (Info/Warning/Error). The log pane shows the most recent lines and auto-follows the tail unless the user has focused that tab's log pane, in which case scroll position holds.

**UI-R-044** — A log line longer than the per-line cap is truncated to it. A monotonic total-written counter lets a consumer holding only a bounded snapshot compute how many lines are new since its last read, across ring eviction.

**UI-R-045** — `:log clear` clears the active tab's on-screen ring.

**UI-R-085** — File-sink logging (`:log <file>`) is module-forwarded, semantics in the module's area; with a file sink configured, buffered lines flush to disk once per UI tick (and on sink teardown), not per line.

**UI-R-046** — A table cell wider than the visible width is reachable by horizontal scroll tied to the selected column. Live-updated cells can be highlighted briefly after they change. The tab bar's own overflow scroll is UI-R-117.

## Widget & focus-derive contract

**UI-R-047** — Reusable widgets follow one event contract: offered a `(modifiers, code)` key event, each returns *consumed* or *unhandled* (carrying the original key back). Unhandled propagates to the enclosing layer; consumed stops propagation.

**UI-R-048** — A single-line text input supports `Home`/`End`, `Left`/`Right`, `Backspace`/`Delete`, printable insertion (including `Shift` for capitals and symbols), `Ctrl+F` autofill from placeholder (only when empty), `Ctrl+D` clear.

**UI-R-086** — A focused single-line text input (UI-R-048) consumes printable keys even when a per-field filter rejects the character, so disallowed characters never leak to app-level shortcuts.

**UI-R-087** — Editing in a single-line text input (UI-R-048) is character-based.

**UI-R-049** — A view is composable as a focusable node: set/query focus and next/previous focus stepping, so the owning tab treats "switch content↔log" as one focus step and toggles the whole tab's focus recursively into whichever pane is active. A focusable container's cycle skips fields whose enabling condition is false.

**UI-R-050** — The color scheme is a single compile-time constant selected by build feature; no runtime switch.

**UI-R-114** — A tab widget renders an ordered list of tab titles laid out one after another in list order along its layout direction (UI-R-173), each tab occupying as many cells along that direction as its title has characters, plus its padding cells (UI-R-120).

**UI-R-115** — Under `Vertical` layout (UI-R-173) a tab's title in the tab widget (UI-R-114) is written one character per row, top to bottom in title order, so an n-character title occupies n consecutive character rows.

**UI-R-116** — The tab widget (UI-R-114) renders every cell belonging to the active tab — its character cells, its padding cells along and across the layout direction (UI-R-120, UI-R-121) and the cells it gains from filling the area (UI-R-122) alike — in the active style, and every cell of every other tab in the inactive style, in either layout direction.

**UI-R-117** — When the tabs of the tab widget (UI-R-114) need more cells along the layout direction (UI-R-173) than the area offers, the widget scrolls along that direction so the active tab's block of cells stays visible.

**UI-R-118** — The tab widget's scroll (UI-R-117) places the active tab's block as near the middle of the area as fits: the cells before the block along the layout direction take at most half the leftover extent and the cells after it take the remainder, and extent one side cannot use never rolls over to the other.

**UI-R-119** — The tab widget (UI-R-114) derives no selection of its own: the caller owns the tab list and the active index in the widget's state and updates them before each render, and the only field the widget itself maintains is the scroll offset (UI-R-117), in either layout direction.

**UI-R-120** — The tab widget (UI-R-114) takes a padding of a horizontal count H and a vertical count V, both default 0, and renders the count running along its layout direction (UI-R-173) — V under `Vertical`, H under `Horizontal` — as that many blank cells before and that many after every tab's character cells (UI-R-115, UI-R-174).

**UI-R-121** — The padding count running across the tab widget's layout direction (UI-R-120) — H under `Vertical`, V under `Horizontal` — renders that many blank lines on either side of the character line, so the widget's rendered extent across the direction is `1 + 2c` cells for that count `c` and is never fixed at one; with H = 1, V = 1 and title `Tab` the `Vertical` widget renders the rows `"   "`, `" T "`, `" a "`, `" b "`, `"   "` and the `Horizontal` widget the rows `"     "`, `" Tab "`, `"     "`, each from top to bottom.

**UI-R-122** — When the displayed tabs' natural extent along the layout direction — character cells plus padding cells (UI-R-115, UI-R-120, UI-R-174) — is less than the area's extent along that direction, the tab widget (UI-R-114) divides the spare cells among those tabs so that the tabs together cover every cell of the area along that direction.

**UI-R-123** — The spare cells of UI-R-122 are divided evenly: with `s` spare cells over `n` displayed tabs every tab gains `s / n` cells and the first `s % n` tabs, counted from the start of the layout direction, gain one cell more.

**UI-R-124** — The cells a tab gains under UI-R-122 are added outside that tab's padding cells, leaving its character cells and its padding cells contiguous and in place.

**UI-R-125** — The tab widget (UI-R-114) stretches no tab when the tabs' natural extent along the layout direction is at least the area's extent along that direction; the centering scroll (UI-R-117, UI-R-118, UI-R-175) governs that case instead.

**UI-R-126** — The tab widget (UI-R-114) takes an alignment of `Start`, `Center` or `End`, default `Center`, which places each tab's character cells together with its padding cells (UI-R-115, UI-R-120, UI-R-174) at the start of, in the middle of, or at the end of that tab's stretched extent along the layout direction (UI-R-122), the gained cells filling the remainder.

**UI-R-127** — Under `Center` alignment (UI-R-126) an odd number of gained cells splits with the smaller half before the tab: a tab gaining `g` cells takes `g / 2` before its leading padding cell and the rest after its trailing padding cell.

**UI-R-128** — Alignment (UI-R-126) changes nothing when no stretching applies (UI-R-125): with the tabs' natural extent at or above the area's extent along the layout direction, `Start`, `Center` and `End` render identically.

**UI-R-173** — The tab widget (UI-R-114) takes a layout direction of `Horizontal` or `Vertical`, default `Horizontal`, which lays the tabs out left to right or top to bottom and thereby fixes which axis every along-the-direction rule (UI-R-114, UI-R-117, UI-R-118, UI-R-120, UI-R-122, UI-R-126) applies to.

**UI-R-174** — Under `Horizontal` layout (UI-R-173) a tab's title in the tab widget (UI-R-114) is written one character per column, left to right in title order, so an n-character title occupies n consecutive character columns.

**UI-R-175** — The tab widget recomputes its scroll offset from the active tab's block on every render (UI-R-118) and stores it in its state as the record of the last computed window start, so the stored offset is an output of a render and never an input carried into the next one.

## Modbus monitor view

**UI-R-060** — A Modbus monitor module's content view has a left panel listing every unit id observed (updated live) and, on the right, sections scoped to the selected unit id: a message table (MB-R-146 records), a memory layout (MB-R-144's observed-value table, grouped by table kind), and a resolved-registers table (MB-R-145 interpretations applied to that unit id's memory).

**UI-R-100** — The Modbus monitor content view's resolved-registers table (UI-R-060) is hidden entirely when no interpretation exists for the selected unit id.

**UI-R-101** — A Modbus monitor module's own log occupies the bottom log pane of its tab per UI-R-003 (UI-R-060).

**UI-R-061** — On a monitor view, `:add`/`:a` opens a dialog to add a register interpretation (MB-R-145) scoped to the selected unit id, mirroring the client/server `:add` dialog (`api-contract.md`). The resolved-registers table reflects it immediately. With no unit id discovered yet, `:add` is rejected with a Warning-level message instead of opening.

**UI-R-062** — The message table (UI-R-060) renders one row per MB-R-146 record for the selected unit id, most recent first, columns Time, Status, Slave, Operation, Address, Quantity, Values/Payload.

**UI-R-102** — The message table's Values/Payload column (UI-R-062) renders `[XXXX XXXX ...]` (4-digit lowercase hex per 16-bit word, space-separated, matching the modbus module's raw-value formatting) for register-shaped operations, `[0 1 0 ...]` (one digit per bit) for coil/discrete-input-shaped, empty for any operation MB-R-146 carries no address/quantity/value for.

**UI-R-103** — Horizontal overflow of the message table (UI-R-062) scrolls rather than truncating or wrapping.

**UI-R-063** — The memory layout panel (UI-R-060) renders each of MB-R-144's populated table kinds for the selected unit id as an independent hex-editor-style block. Holding/Input Registers address by register, 8 per line; Coils/Discrete Inputs pack 8 bits per byte (MSB-first), address by byte, 16 per line.

**UI-R-104** — Each memory layout line (UI-R-063) shows, in order: table kind (the modbus module's `Kind` naming), starting address, hex byte/word values, character representation.

**UI-R-105** — A memory layout line (UI-R-063) with no observed value in its range is omitted; within a rendered line, an unobserved byte/word renders as a dim placeholder (`··`), not a decoded zero.

**UI-R-106** — Each memory layout byte/word (UI-R-063) is colorized by value class (zero in the placeholder color, printable ASCII in normal text color, other non-zero in a distinguishing highlight).

**UI-R-107** — Independently of its value-class color (UI-R-106), a memory layout byte/word is painted with the change-highlight color while an MB-R-147 recency marker is active for its address.

**UI-R-064** — The resolved-registers table (UI-R-060, MB-R-145) renders columns Name, Description, Address, Kind, Format, Length, Resolution, Value, Raw Value: the modbus module's register table's set (`ferrowl::module::modbus::table::TableHeader`) less Slave ID and Access.

**UI-R-108** — Enter on a selected resolved-registers table row (UI-R-064) opens an edit dialog prefilled from that interpretation, offering Confirm (MB-R-148 edit) and Delete (MB-R-148 remove), mirroring the modbus edit-register dialog.

**UI-R-065** — Tab/Shift+Tab on a monitor content view cycles focus across panels: Units, Messages, Memory layout, Resolved registers, back to Units, skipping Resolved registers whenever hidden (UI-R-100). Each panel keeps its own selection/scroll position; Up/Down/Left/Right and Enter act on the focused panel.

**UI-R-066** — The shared `Table` widget's optional selection-marker gutter reserves zero width whenever no row is selected, otherwise exactly the highlight symbol's rendered width, for every table regardless of whether the marker is shown.

**UI-R-109** — A `Table` widget's selected row's background is painted with its highlight style in every case except an unfocused table with the selection marker (UI-R-066) shown, where the marker glyph alone is the cue.

**UI-R-112** — `Esc` on a Modbus monitor view overlay — the monitor setup-edit dialog, the add-interpretation dialog (UI-R-061), the edit-interpretation dialog (UI-R-108) — opens that overlay's own close-confirmation popup per UI-R-023 and never closes the overlay directly.

**UI-R-113** — The add-interpretation (UI-R-061) and edit-interpretation (UI-R-108) dialogs each carry the close-confirmation popup state UI-R-023 defines, so UI-R-112 has a popup to open in every monitor overlay.
