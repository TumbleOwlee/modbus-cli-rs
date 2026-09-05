# TUI — API Contract

Operator-facing surface: exhaustive `:` command list, every keybinding table by context/mode, code-editor mode/command set. Names, aliases, argument shapes, key mappings are contract and shall not change without a spec change.

**Generic vs protocol-specific.** **Generic (app-level)** commands are parsed and executed by the application (``## Generic `:` commands (app-level)``); behavior owned here. **Module (protocol-specific)** commands are forwarded verbatim to the active view (``## Module (protocol-specific) `:` commands``); this document lists name and syntax, the effect on protocol state is the owning area (`modbus/`, `ocpp/`) — "→ modbus" / "→ ocpp" points there.

Dispatch: first token matched against the generic set; not in the set → forwarded to the active view. A generic name always wins over a same-named module command.

---

## Generic `:` commands (app-level)

| Command | Aliases | Arguments | Effect (owned here) | Req |
|---|---|---|---|---|
| `:quit` | `:q`, `:q!` | — | Stop and close the active tab; quit the app if it was the last | UI-R-019 |
| `:qall` | `:qa`, `:qa!` | — | Quit the whole app immediately | UI-R-019 |
| `:new` | `:n` | — | Open the new-module type selector | UI-R-008, UI-R-024 |
| `:load [path]` | `:l` | optional device-config path | Open the Modbus create dialog, config-path field pre-filled with `path` | UI-R-017 |
| `:write [path]` | `:w`, `:s`, `:save` | optional output path (default `session.toml`) | Save all module instances plus session scripts/interval as a session file; format from extension (`.toml`/`.json`) | UI-R-017 |
| `:swap <i> <j>` | — | two tab indices | Swap tabs `i` and `j` (no-op if equal or out of range); non-numeric rejected | UI-R-017 |
| `:session` | — | — | Open the session-level Lua scripts + sim-interval dialog | UI-R-017 |
| `:script copy <idx>` | — | source tab index | Replace the active tab's script list with tab `idx`'s; errors if `idx` missing, out of range, equals the active tab, or either tab lacks script support | UI-R-017 |
| `:log clear` | — | the literal `clear` | Clear the active tab's on-screen log ring | UI-R-017, UI-R-045 |

- `:log` with **any argument other than `clear`** (including a path), and **bare `:log`**, are *not* generic — forwarded to the active view (``## Module (protocol-specific) `:` commands``), where `:log <file>` sets/clears the module's file sink.
- Bare `:script` (without `copy`) is *not* generic — forwarded; the Modbus view opens its script dialog (UI-R-018).
- Any unrecognized first token is forwarded; if the view also does not recognize it, app logs `Unknown command ':<input>'` (Warning) (UI-R-018).

## Module (protocol-specific) `:` commands

Forwarded to the active view; **semantics owned by the protocol area**. Each view advertises exactly its own list in the command-help popup.

### Modbus module

| Command | Arguments | Purpose (→ modbus) | Req |
|---|---|---|---|
| `:edit` / `:e` | — | Open the module setup dialog | UI-R-018 |
| `:add` / `:a` | — | Open the add-register dialog | UI-R-018, UI-R-024 |
| `:start` | — | Start the module (connect/bind) | UI-R-018 |
| `:stop` | — | Stop the module | UI-R-018 |
| `:restart` | — | Stop then start | UI-R-018 |
| `:reload` | — | Reload the device config from disk and restart | UI-R-018 |
| `:compact` | — | Toggle compact table rows | UI-R-018 |
| `:set <register> <value>` | register name, value (name may be `"quoted"` to allow spaces) | Write a value into a register → **modbus** owns type/range/codec semantics and store-vs-wire behavior | UI-R-018 |
| `:write-device` / `:wd` `[path]` | optional path (default: configured device path) | Save the device config file | UI-R-018 |
| `:log <file>` | file base path | Set the module's log-file sink base | UI-R-018 |
| `:script` | — | Open the Lua script manager dialog | UI-R-018 |
| `:order [col] [asc\|desc]` | optional column, optional direction (default `asc`) | Sort the register table by column; bare `:order` clears | UI-R-018 |

Modbus monitor module (`role = monitor`, MB-R-140–145, MB-R-191–198):

| Command | Arguments | Purpose (→ modbus) | Req |
|---|---|---|---|
| `:edit` / `:e` | — | Open the module setup dialog | UI-R-018 |
| `:add` / `:a` | — | Open the add-register-interpretation dialog (UI-R-061) | UI-R-018, UI-R-061 |
| `:start` | — | Start the module (open the serial port receive-only) | UI-R-018 |
| `:stop` | — | Stop the module | UI-R-018 |
| `:restart` | — | Stop then start | UI-R-018 |
| `:reload` | — | Reload the device config from disk and restart | UI-R-018 |
| `:compact` | — | Toggle compact table rows | UI-R-018 |
| `:write-device` / `:wd` `[path]` | optional path (default: configured device path) | Save the device config file | UI-R-018 |
| `:log <file>` | file base path | Set the module's log-file sink base | UI-R-018 |
| `:order [col] [asc\|desc]` | optional column, optional direction (default `asc`) | Sort the resolved-registers table; bare `:order` clears | UI-R-018 |

`:set` (nothing to write) and `:script` (no Lua surface) are omitted for this role — unrecognized commands on a monitor view, not errors.

### OCPP client module

| Command | Arguments | Purpose (→ ocpp) | Req |
|---|---|---|---|
| `:edit` / `:e` | — | Open the module setup dialog | UI-R-018 |
| `:start` | — | Connect to the CSMS | UI-R-018 |
| `:stop` | — | Disconnect | UI-R-018 |
| `:restart` | — | Reconnect | UI-R-018 |
| `:compact` | — | Toggle compact rows | UI-R-018 |
| `:write-device` / `:wd` `[path]` | optional path | Save the device config | UI-R-018 |
| `:log [file]` | optional file path | Set the file sink; bare `:log` or empty path disables | UI-R-018 |

### OCPP server (CSMS) module

| Command | Arguments | Purpose (→ ocpp) | Req |
|---|---|---|---|
| `:start` | — | Bind the CSMS listener | UI-R-018 |
| `:stop` | — | Unbind (clears connected-station entries) | UI-R-018 |
| `:restart` | — | Rebind (clears entries) | UI-R-018 |
| `:edit` / `:e` | — | Open the module setup dialog | UI-R-018 |
| `:write-device` / `:wd` `[path]` | optional path | Save the device config | UI-R-018 |
| `:compact` | — | Toggle compact rows | UI-R-018 |
| `:log [file]` | optional file path | Set the file sink; bare `:log` or empty path disables | UI-R-018 |
| `:rfid [add\|del <tag> \| clear]` | subcommand + tag | Manage the CSMS RFID accept-list; bare `:rfid` prints it | UI-R-018, OC-R-074, OC-R-075 |

**OCPP action send is not a `:` command.** Composing and sending an action goes through the action dialog opened by `Enter` on the message table / action control. Action set and payload semantics: `ocpp/`.

## Global keybindings

| Key | Context | Action | Req |
|---|---|---|---|
| `:` | content focused, no view overlay | Enter command mode | UI-R-014 |
| `?` | content focused, no view overlay | Open the keybind-help dialog | UI-R-005 |
| `Ctrl+w` then `j`/`k`/`Down`/`Up` | content focused | Toggle focus between content view and log pane | UI-R-009 |
| `Ctrl+t` then `l` | content focused | Next tab (wraps) | UI-R-010 |
| `Ctrl+t` then `h` | content focused | Previous tab (wraps) | UI-R-010 |
| `Ctrl+t` then digit(s) | content focused | Jump to tab by index; waits up to 800 ms for a 2nd digit if one could form a valid 2-digit index (UI-R-011) | UI-R-011 |

`:` and `?` are suppressed while the active view has an overlay open; they type into it instead.

## Context keybinding tables

### Command mode

| Key | Action | Req |
|---|---|---|
| `Esc` | Cancel (discard buffer) | UI-R-015 |
| `Enter` | Run the command | UI-R-015 |
| printable / `Left`/`Right` / `Home`/`End` / `Backspace`/`Delete` | Edit the buffer | UI-R-015 |

### Dialogs (generic)

| Key | Action | Req |
|---|---|---|
| `Tab` | Next field (skips disabled) | UI-R-022, UI-R-078 |
| `Shift+Tab` / `BackTab` | Previous field | UI-R-022 |
| `Enter` | Confirm | UI-R-079 |
| `Esc` | Request close (close-confirm popup if edits may be lost) | UI-R-023 |

Applied only when the focused widget did not consume the key.

### Close-confirm / yes-no popup

| Key | Action | Req |
|---|---|---|
| `Enter` / `Space` | Confirm (close / delete) | UI-R-023 |
| `Esc` | Dismiss (back to editing) | UI-R-023 |
| `Tab` / `Shift+Tab` | Move between confirm/cancel (delete confirm) | UI-R-023 |

Focus defaults to the safe (cancel) choice; confirming requires that choice focused.

### Tables and selection lists (when focused)

| Key | Action | Req |
|---|---|---|
| `j` / `Down` | Row down | UI-R-013 |
| `k` / `Up` | Row up | UI-R-013 |
| `h` / `Left` | Column left (or previous item) | UI-R-013 |
| `l` / `Right` | Column right (or next item) | UI-R-013 |
| `g` | First row | UI-R-013 |
| `G` | Last row | UI-R-013 |
| `0` / `Home` | First column | UI-R-013 |
| `$` / `End` | Last column | UI-R-013 |

Selection clamps at the ends (no wrap).

### Suggestion-completion popup (open)

| Key | Action | Req |
|---|---|---|
| `Up` / `Down` | Move highlight | UI-R-026 |
| `Enter` | Accept highlight (partial → keep open and re-query; else close) | UI-R-026, UI-R-082 |
| `Tab` | Never consumed by the popup, even open; moves focus to the dialog's next field | UI-R-081 |
| `Esc` | Dismiss | UI-R-026 |

### Single-line text input (focused)

| Key | Action | Req |
|---|---|---|
| printable (with `Shift`) | Insert character (rejected chars still consumed) | UI-R-048, UI-R-086 |
| `Left` / `Right` | Move cursor | UI-R-048 |
| `Home` / `End` | Line start / end | UI-R-048 |
| `Backspace` / `Delete` | Delete before / at cursor | UI-R-048 |
| `Ctrl+F` | Autofill from placeholder (only when empty) | UI-R-048 |
| `Ctrl+D` | Clear | UI-R-048 |

### Keybind-help dialog (`?`)

| Key | Action | Req |
|---|---|---|
| `Esc` / `q` / `?` | Close | UI-R-005 |
| `j` / `Down` | Scroll down | UI-R-005 |
| `k` / `Up` | Scroll up | UI-R-005 |
| `g` | Top | UI-R-005 |
| `G` | Bottom | UI-R-005 |

### Lua-bindings help overlay (`?` in a script editor, Normal mode)

Same navigation as ``### Keybind-help dialog (`?`)``. Reachable only from the code editor in Normal mode; in Insert/Visual `?` is literal text.

### Script-manager dialog

| Key | Context | Action | Req |
|---|---|---|---|
| `Tab` / `Shift+Tab` | dialog | Cycle focus (script table → name input → Templates button → code editor → interval → log); code editor skipped while no script selected | UI-R-058 |
| `Esc` | dialog | Open close-confirm | UI-R-023 |
| `t` | script table focused | Toggle the selected script's enabled flag | UI-R-091 |
| `d` | script table focused | Delete the selected script (opens confirm) | UI-R-092 |
| `c` | script table focused | Toggle compact rows | UI-R-093 |
| `e` | script table focused | Execute the selected script once (current editor content, enabled or not) | UI-R-051 |
| `Enter` | script table focused | Open the rename prompt | UI-R-055 |
| `Enter` | name input focused | Create a new script with the typed name | UI-R-058 |
| `Enter` / `Space` | Templates button focused | Open the template-browser overlay | UI-R-052 |
| `?` | script table focused | Open the script-table keybind-help overlay (`Esc`/`q`/`?` closes) | UI-R-056 |
| `?` | code editor, Normal mode | Open the Lua-bindings help overlay | UI-R-056 |

### Template-browser overlay (Templates button in the script-manager dialog)

| Key | Context | Action | Req |
|---|---|---|---|
| `j` / `k` / `Up` / `Down` | template list | Move selection; preview follows | UI-R-053 |
| `Tab` / `Shift+Tab` | overlay | Cycle focus between list and (read-only) preview | UI-R-053 |
| `Enter` | overlay | Insert the selected template as a new enabled script, close | UI-R-054 |
| `Esc` / `q` | overlay | Close, changing nothing | UI-R-053 |

### Script rename prompt (`Enter` on the script table)

| Key | Context | Action | Req |
|---|---|---|---|
| `Enter` | prompt | Commit; empty or duplicate name refused, prompt stays open | UI-R-055, UI-R-089 |
| `Esc` | prompt | Cancel, name unchanged | UI-R-055 |

### Modbus monitor view overlays

| Key | Context | Action | Req |
|---|---|---|---|
| `Esc` | monitor setup-edit / add-interpretation / edit-interpretation dialog | Open that dialog's close-confirm popup | UI-R-112, UI-R-113 |
| `Enter` / `Space` | that close-confirm popup | Confirm — overlay closes, edits discarded | UI-R-023, UI-R-112 |
| `Esc` | that close-confirm popup | Dismiss — back to editing | UI-R-023, UI-R-112 |

Focus defaults to the safe (cancel) choice, per `### Close-confirm / yes-no popup`.

## Code editor — modes and commands

Vim-modal editor (default for the Lua-script editor). Modes: `NORMAL`, `INSERT`, `VISUAL` (charwise), `V-LINE` (linewise).

### Mode transitions

| Key | From | Action | Req |
|---|---|---|---|
| `i` | Normal | Insert at cursor | UI-R-028 |
| `a` | Normal | Insert after cursor | UI-R-028 |
| `I` | Normal | Insert at first non-blank | UI-R-028 |
| `A` | Normal | Insert at end of line | UI-R-028 |
| `o` | Normal | Open (auto-indented) line below, Insert | UI-R-028 |
| `O` | Normal | Open line above (copying indent), Insert | UI-R-028 |
| `v` | Normal | Charwise Visual | UI-R-028 |
| `V` | Normal | Linewise Visual | UI-R-028 |
| `Esc` | Insert / Visual | Back to Normal (Insert also steps cursor back one, vim-style) | UI-R-029, UI-R-028 |
| `Esc` | Normal | Left unhandled → reaches dialog (opens close-confirm) | UI-R-028 |
| `v` | Visual | Back to Normal | UI-R-028 |
| `V` | Visual (charwise) | Switch to linewise Visual | UI-R-028 |

### Motions (Normal and Visual)

| Key | Motion | Req |
|---|---|---|
| `h` / `l` | Left / right one char (no line wrap) | UI-R-029 |
| `j` / `k` | Down / up one line | UI-R-028 |
| `Left`/`Right`/`Up`/`Down` | Arrow move (wraps to adjacent line) | UI-R-029 |
| `0` | First column | UI-R-028 |
| `$` | Last column | UI-R-028 |
| `w` | Start of next word (crosses lines; punctuation runs are their own word) | UI-R-028 |
| `b` | Start of previous word | UI-R-028 |
| `e` | End of current/next word | UI-R-028 |
| `gg` | First line, first column | UI-R-028 |
| `G` | Last line | UI-R-028 |

### Edits (Normal)

| Key | Action | Req |
|---|---|---|
| `x` | Delete char under cursor (to register, charwise) | UI-R-030 |
| `dd` | Delete current line (to register, linewise) | UI-R-030 |
| `yy` | Yank current line (to register, linewise) | UI-R-030 |
| `p` | Paste register after cursor / below line | UI-R-030 |
| `P` | Paste register before cursor / above line | UI-R-030 |
| `u` | Undo last change (again to redo) | UI-R-031 |

### Edits (Visual)

| Key | Action | Req |
|---|---|---|
| `y` | Yank selection, return to Normal at selection start | UI-R-030 |
| `d` / `x` | Delete selection, return to Normal | UI-R-030 |

### Insert-mode keys

| Key | Action | Req |
|---|---|---|
| printable / `Enter` / `Backspace` / `Delete` / arrows | Standard editing (auto-indent on `Enter` when a language is set) | UI-R-032, UI-R-035 |
| `Tab` | Insert four spaces | UI-R-034 |
| `Shift+Tab` / `BackTab` | Remove up to four leading spaces | UI-R-034 |

### Plain (non-vim) editor

Printable keys insert; `Enter` splits with auto-indent (when a language is set); `Backspace`/`Delete` edit; arrows navigate with line wrap; `Home`/`End` and character-based editing as `### Single-line text input (focused)`. Two space presses at the same position within ~300 ms expand to a four-space indent (an intervening key cancels).

Yank/delete also copy to the system clipboard via OSC 52. A `language` setting drives syntax highlighting and format-on-blur (JSON may decline invalid input; Lua always reformats). The Diff language colors whole lines and has no formatter, so a Diff field never reformats on blur. (UI-R-170, UI-R-171)

## Markdown input field — modes and commands

The widget uses the mode transitions, motions, edits and Insert-mode keys of the code editor tables above (UI-R-181).

| Key | Mode | Action | Req |
|---|---|---|---|
| `gj` | Normal / Visual | Down one display row | UI-R-135 |
| `gk` | Normal / Visual | Up one display row | UI-R-135 |
| `gg` | Normal / Visual / read-only | First source line, first column | UI-R-139 |
| `G` | Normal / Visual / read-only | Last source line, first column | UI-R-139 |
| `Ctrl+D` | Normal / Visual / read-only | Half a screen of display rows down, cursor moved the same number of rows | UI-R-136, UI-R-139 |
| `Ctrl+U` | Normal / Visual / read-only | Half a screen of display rows up, cursor moved the same number of rows | UI-R-136, UI-R-139 |
| `j` / `k` | Normal / Visual / read-only | Down / up one source line, wrapping ignored | UI-R-134 |
| `yy` | Normal / read-only | Yank current source line | UI-R-134, UI-R-139 |
| `h` / `l` / `0` / `$` / `w` / `b` / `e` | read-only | Consumed, no movement | UI-E-089 |
| mutating keys, `i` / `a` / `I` / `A` / `o` / `O` / `v` / `V` | read-only | Ignored, reported unhandled | UI-R-155 |

Public surface: content get and set (UI-R-181), read-only toggle (UI-R-184, UI-R-155), focus set and query (UI-R-182, UI-R-184), current vim mode and its display label (UI-R-181), builder options for the markdown theme (UI-R-141), the syntax theme (UI-R-129) and the line-number gutter, default off (UI-R-140).

## Code editor and syntax public surface

| Item | Meaning | Req |
|---|---|---|
| Syntax language `Diff` | whole-line diff classification, no formatter | UI-R-037, UI-R-156, UI-R-170, UI-R-171 |
| Highlight kinds `Added`, `Removed`, `Meta` | diff line kinds added to the fixed kind enumeration | UI-R-039 |
| Syntax theme styles `added`, `removed`, `meta` | per-kind styles for the diff kinds, foreground-only defaults | UI-R-162, UI-R-163 |
| Code-editor state `gutter_labels: Option<Vec<String>>` | per-line gutter text replacing the line index; builder-settable and settable after construction | UI-R-164, UI-R-165, UI-R-168 |
