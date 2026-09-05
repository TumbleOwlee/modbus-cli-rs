# TUI — Edge Cases and Known Limitations

Boundary behavior, error semantics, intentional or known constraints. The known-limitations section below (`## Known limitations and stated constraints`) is working as implemented; recorded so it is not "fixed".

---

## Command line

| ID | Condition | Behavior |
|---|---|---|
| **UI-E-001** | Unknown first token (`:bogus`) | forwarded to the active view; if the view also does not recognize it, app logs `Unknown command ':bogus'` at Warning |
| **UI-E-002** | Empty submission (`:` then `Enter`) | no-op; command mode exits |
| **UI-E-003** | Extra whitespace between tokens | collapsed on split; `:  swap   0    1` = `:swap 0 1` |
| **UI-E-004** | `:swap` with a non-numeric or missing index | rejected (parsed as unknown-`swap`); no swap |
| **UI-E-005** | `:swap i j` with `i == j` or either out of range | silent no-op |
| **UI-E-006** | `:script copy` with no index | error logged: `usage: :script copy <tab-index>` |
| **UI-E-007** | `:script copy <idx>` out of range | error logged: `no tab [idx] (0..=max)` |
| **UI-E-008** | `:script copy <active-index>` | error logged: `cannot copy from the active tab` |
| **UI-E-009** | `:script copy <idx>` where source or active tab lacks script support | warning logged: `... has no script support` |
| **UI-E-010** | `:quit` on the last tab | quits the application |
| **UI-E-011** | `:log` bare, or `:log <path>` (path ≠ `clear`) | not app-level; forwarded to the view as a module command |
| **UI-E-012** | Generic name shadows a module name | generic always wins; module commands reached only for unrecognized tokens |

## Navigation and tab jumps

| ID | Condition | Behavior |
|---|---|---|
| **UI-E-013** | `Ctrl+t` + digit uniquely indexing a tab (or `0`) | jumps immediately |
| **UI-E-014** | `Ctrl+t` + first digit that could start a 2-digit index | waits up to 800 ms for a second digit |
| **UI-E-015** | Second digit forms an out-of-range index | falls back to the first digit's tab |
| **UI-E-016** | 800 ms elapses with no second digit | pending first-digit jump commits |
| **UI-E-017** | Non-digit pressed while a first digit is pending | commits the pending jump, then processes the key |
| **UI-E-018** | Jump to out-of-range or already-active index | silent no-op |
| **UI-E-019** | Tab switch with 0 or 1 tabs | safe no-op |

## Dialogs and overlays

| ID | Condition | Behavior |
|---|---|---|
| **UI-E-020** | `Esc` on a dialog with edits | close-confirm popup; `Enter`/`Space` confirms close, `Esc` returns to editing |
| **UI-E-021** | Creating a tab whose name collides | refused; Warning to the active tab's log; setup dialog stays open |
| **UI-E-022** | Startup new-module selector cancelled before any tab exists | application exits (UI-R-057) |
| **UI-E-023** | Rename/session-load produces a duplicate tab name | later duplicate auto-suffixed; Warning into the renamed tab's log |
| **UI-E-024** | Focus cycle reaches a field whose enabling condition is false | skipped in `Tab`/`Shift+Tab` |
| **UI-E-025** | `:` or `?` pressed while a view overlay is open | not global; delivered to the overlay (`?` types into a Lua editor, `:` into a text field) |
| **UI-E-026** | Key with no binding in the current dialog/field | left unhandled; generic defaults (`Enter`/`Esc`/`Tab`) apply only if no widget consumed it |
| **UI-E-027** | Suggestion popup closed, `Up`/`Down`/`Enter`/`Tab`/`Esc` pressed | passed through to the dialog |
| **UI-E-028** | Inserting a template whose name is already used | inserted as `<name>-2` (then `-3`, …); never refused |
| **UI-E-029** | Template browser preview pane | a disabled code editor: vim motions and visual-yank work, edits do not |
| **UI-E-030** | `?` in the script dialog | focus decides: on the script table → keybind help; in the code editor's Normal mode → Lua-bindings help (Insert/Visual: literal text) |
| **UI-E-031** | Renaming a script to an empty or duplicate name | refused silently; prompt stays open (same rule as creating) |
| **UI-E-032** | Renaming a script to its current name | accepted; no-op |
| **UI-E-033** | `Esc` while the rename prompt is open | cancels the prompt; does not reach the dialog's close-confirm |
| **UI-E-034** | A rename is an edit | restarts the sim thread when the dialog closes (SC-R-024): the Lua context is keyed by script name |
| **UI-E-061** | `Esc` on a monitor view overlay (UI-R-112) with nothing typed into it yet | close-confirm popup opens anyway — monitor overlays track no dirty flag, so the confirmation is unconditional |
| **UI-E-062** | `Esc` on a monitor add/edit-interpretation dialog while one of its own sub-popups is open (the delete confirmation, the predefined-register picker) | dismisses only that sub-popup; does not reach the dialog's close-confirm (UI-R-112), same rule as UI-E-033 |

## Code editor

| ID | Condition | Behavior |
|---|---|---|
| **UI-E-035** | `Esc` in Normal mode | left unhandled, reaches the dialog, opens close-confirm (two `Esc` from Insert: first to Normal, second toward closing) |
| **UI-E-036** | Unrecognized printable key in Normal mode (e.g. `z`, `q`) | consumed and ignored |
| **UI-E-037** | `?` in Insert or Visual mode | literal text; the Lua-bindings overlay opens only from Normal |
| **UI-E-038** | Disabled editor | mutating keys ignored and reported unhandled; navigation works; never reformats on blur |
| **UI-E-039** | Format-on-blur with invalid JSON | formatter declines; buffer left as typed |
| **UI-E-040** | Format-on-blur with Lua | always reformats (never declines) |
| **UI-E-041** | `h`/`l` at a line edge | do not wrap; arrows do wrap to the adjacent line |
| **UI-E-042** | Multi-byte UTF-8 | edited character by character; cursor columns count characters, never bytes |
| **UI-E-043** | `u` pressed twice | first undoes, second redoes (single-level) |
| **UI-E-044** | `gg`/`dd`/`yy` first press | held pending; any non-matching key cancels the chord before doing its own action |
| **UI-E-045** | Yank/delete with no clipboard-capable terminal | OSC 52 best-effort; failure ignored; internal register still holds the text |
| **UI-E-079** | Mutating edit on an enabled field that has gutter labels (UI-R-164) | labels are never resynced by the widget: an inserted, split or deleted line shifts rows out from under the labels and leaves them stale; labels are intended for disabled, read-only use, and keeping them in sync is the consumer's job |
| **UI-E-080** | Gutter-label list longer than the buffer | the surplus labels are never rendered, but still count toward the gutter width of UI-R-167 |
| **UI-E-083** | Gutter labels wider than the field's whole area (UI-R-172) | the gutter takes the full area width and the text content is left zero columns; no minimum content width is reserved and no label is dropped, matching the app's no-minimum-size stance (UI-E-047, UI-E-055) |

## Syntax highlighting

| ID | Condition | Behavior |
|---|---|---|
| **UI-E-081** | Diff line `---` or `+++` (UI-R-159) | classified diff meta, not diff removed or diff added, because the meta prefixes are matched first |
| **UI-E-082** | Diff line consisting of a lone `+` or `-` | yields a one-character span of the diff added or diff removed kind (UI-R-156 spans the whole line, which is one character) |

## Markdown input field

| ID | Condition | Behavior |
|---|---|---|
| **UI-E-086** | Width too narrow for a list item's or block quote's hanging indent (UI-R-132) | the hanging indent is dropped and continuation rows start at column zero |
| **UI-E-087** | Single word longer than the available width (UI-R-131) | broken mid-word at a character boundary; never truncated, never overflowed |
| **UI-E-088** | Cursor line revealed as source in `Normal` (UI-R-182) | cursor column is a source column; no mapping between rendered and source columns is kept, so the reveal is the only place the cursor is positioned against markup |
| **UI-E-089** | `h`, `l`, `0`, `$`, `w`, `b`, `e` in a read-only markdown input field | consumed and ignored; only line/display-row navigation (`j`, `k`, `gg`, `G`, `Ctrl+D`, `Ctrl+U`) and yank act (UI-R-139) |
| **UI-E-090** | Fence opened and never closed before the end of the buffer (UI-R-177) | every following line stays fence body to the last line of the buffer |
| **UI-E-076** | `Ctrl+D` / `Ctrl+U` near the first or last display row (UI-R-136) | movement clamps to the first/last row; no wrap-around |

## Rendering and terminal size

| ID | Condition | Behavior |
|---|---|---|
| **UI-E-046** | Terminal resize | next tick re-lays out; content, log, command rows reflow |
| **UI-E-047** | Very small terminal | no app-level minimum-size guard; content area squeezed, content clips. Popups skip drawing when their area is zero-sized |
| **UI-E-048** | Log line longer than the per-line cap | truncated before storage |
| **UI-E-049** | Table cell wider than the column | reachable via horizontal scroll tied to the selected column |
| **UI-E-050** | Tabs overflow the tab bar's width | the bar scrolls horizontally per the tab widget's centering scroll (UI-R-117, UI-R-118), keeping the active tab visible |
| **UI-E-051** | No input for one redraw interval (~100 ms) | UI redraws anyway |
| **UI-E-063** | Tab widget drawn into an area of zero width or zero height | skips drawing; because no window is computed the recorded scroll offset (UI-R-175) keeps the value the previous render left, rather than being recomputed (UI-E-047) |
| **UI-E-064** | Tab widget drawn into an area larger across its layout direction than its rendered extent (UI-R-121) | draws into the first `1 + 2c` lines from the near edge only — leftmost columns under `Vertical`, topmost rows under `Horizontal` — and leaves the rest untouched |
| **UI-E-065** | Tab widget with an empty tab list | nothing drawn; scroll offset reset to zero |
| **UI-E-066** | Tab widget active index out of range (UI-R-119) | no cell takes the active style; with no in-range active block to centre on, no new window is computed and the recorded scroll offset (UI-R-175) keeps the value the previous render left, clamped per UI-E-085 if it is now past the last cell the current tabs occupy; never panics |
| **UI-E-067** | Tab widget title that is the empty string (UI-R-115, UI-R-174) | occupies no character cells, only its twice-the-along-direction-padding cells (UI-R-120), which still take the active style when it is the active tab; with that padding count 0 the tab occupies no cells at all and is invisible |
| **UI-E-068** | Tab widget under `Vertical` layout with a double-width title character (CJK, emoji) | drawn as-is into the one-column character column and clipped there; no substitution or fallback glyph. Intentional |
| **UI-E-069** | Tab widget drawn into an area smaller across its layout direction than its rendered extent (UI-R-121) | the rendered lines are clipped at the far edge of the area; no reflow, no padding reduction, never panics |
| **UI-E-070** | Tab widget active tab whose block is longer along the layout direction than the area (UI-R-118) | there is no leftover extent to centre in, so the computed window starts at the block's first cell, placing it at the near edge; the rest of the block is clipped |
| **UI-E-071** | Tab widget holding a single tab with spare extent (UI-R-122) | that one tab takes every spare cell and covers the whole area |
| **UI-E-072** | Tab widget with fewer spare cells than tabs (UI-R-123) | the first `s` tabs from the start of the layout direction gain one cell each and the rest gain none; the area is still covered to its last cell |
| **UI-E-073** | Tab widget empty title with along-direction padding 0 (UI-E-067) while there is spare extent | takes part in the division (UI-R-123) like any other tab, so a tab of zero natural extent becomes visible as its share of spare cells |
| **UI-E-074** | Tab widget tab gaining exactly one cell under `Center` (UI-R-127) | the gained cell goes after the tab's trailing padding cell; the title sits one cell before centre |
| **UI-E-075** | Tab widget tab that gains no cells (UI-R-123) while others do | its own render is identical under all three alignments; only the stretched tabs move |
| **UI-E-084** | Tab widget under `Horizontal` layout with a double-width title character (CJK, emoji) | counted as one cell when extents are computed (UI-R-174, UI-R-122) and drawn as-is, so the drawn tab covers more terminal cells than its computed extent and the fill is off by one cell per such character; no substitution or fallback glyph. Intentional |
| **UI-E-085** | Tab widget whose recorded scroll offset (UI-R-175) is past the last cell the current tabs occupy, because the active index is out of range (UI-E-066) and no new window was computed | the offset is clamped to the largest window start that still shows tabs, so the widget draws tabs rather than a blank area; never panics |

## Known limitations and stated constraints

### Single compile-time color scheme

**UI-E-052** — Build-time feature-selected constant; no runtime theme switch. Changing themes requires rebuilding. Intentional.

### Single-level undo only

**UI-E-053** — Exactly one undo snapshot: `u` toggles between the current buffer and the last pre-edit state. No multi-step history, no separate redo stack.

### Editor consumes unmapped Normal-mode keys

**UI-E-054** — In Normal mode any printable key that is not a recognized motion/operator is consumed and discarded. Keeps stray keystrokes from leaking into the enclosing dialog, at the cost of silently swallowing them.

### No minimum terminal size

**UI-E-055** — No refusal or "terminal too small" message; lays out as best it can and lets content clip. Rendering stays panic-safe (zero-sized popups skipped); usability on a tiny terminal not guaranteed.

### Protocol-command results depend on the view

**UI-E-056** — A forwarded `:` command produces whatever `(level, message)` the view chooses; the TUI area does not standardize per-module result text or severities beyond requiring the level be chosen explicitly (never derived from text). The forwarded commands a view accepts are that module's contract, listed in its command-help popup.

### OSC 52 clipboard is best-effort

**UI-E-057** — Yank/delete emit an OSC 52 escape. Terminals without OSC 52 (or with it disabled) do not receive the copy; failure silent; in-app register still works for `p`/`P`. No fallback clipboard.

### Command help lists a fixed generic set

**UI-E-058** — The command-help popup lists a fixed generic set plus the active view's list. Generic aliases beyond those shown (`:q!`, `:save`, `:write`) are still accepted by the parser though the popup shows one spelling.

### Module commands match on the exact first token

**UI-E-059** — A forwarded command is recognized by its exact first whitespace-delimited token: `:setfoo` is unknown, not a malformed `:set`. Argument validation applies only after the token matches (`:set` alone still reports the usage warning).

### Terminal-restore paths are not unit-tested

**UI-E-060** — UI-R-001 requires terminal restore on normal exit, error exit, and from a panic hook. None of the three — `AlternateScreen`'s `Drop` impl (`ferrowl-ui/src/screen.rs`), `AlternateScreen::release()` from the error-exit branch (`ferrowl/src/main.rs`, after `app.run()` returns `Err`), or the same `release()` in the panic hook (`main.rs`, before `runtime.block_on`) — is exercised by an automated test. All three mutate the real terminal's raw-mode/alternate-screen state; doing so inside the test harness's process would corrupt its terminal (the panic-hook path also requires actually panicking), so this is left to manual verification (`cargo run -- --demo`, then exit normally, force an error exit, trigger a panic, checking the prompt is intact each time).

### Markdown rendering covers a fixed construct set

**UI-E-091** — Tables, raw HTML, footnotes, reference links, setext headings and autolinks render as plain text (UI-R-180). Rendering is line-preserving (UI-R-142), so a table is never laid out into columns and adjacent lines are never reflowed into one paragraph.

### Nested inline emphasis is best-effort

**UI-E-092** — Nested inline markers are resolved best-effort rather than by a full CommonMark inline parser: `***x***` yields bold and italic together, but unusual or ambiguous nestings may leave a marker visible or drop a style.

### Intraword underscore never opens italic

**UI-E-078** — A `_` adjacent to a word character (letter, digit or `_`) on the side that would make it a marker never opens or closes italic: preceded by one it cannot open, followed by one it cannot close, so `snake_case_word` and `_snake_case_name` keep their underscores visible instead of hiding a spurious pair as markers.

### Markdown input field has no consumer in the application

**UI-E-077** — The markdown input field is a library widget in the TUI crate with no use in any application view; it is exercised only by its runnable example and by automated buffer-render tests. Absence of a consumer is deliberate, not an oversight.
