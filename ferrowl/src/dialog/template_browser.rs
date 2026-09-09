//! The template-browser overlay of the script dialog (UI-R-053): the templates bundled for the
//! dialog's script context on the left, a read-only preview of the selected one's Lua code on the
//! right. `Enter` inserts the selection into the dialog's script list; `Esc`/`q` closes it.

use crossterm::event::{KeyCode, KeyModifiers};
use ferrowl_syntax::Language;
use ferrowl_ui::{
    Border, COLOR_SCHEME,
    state::{CodeInputFieldState, CodeInputFieldStateBuilder, TableState, TableStateBuilder},
    style::{InputFieldStyleBuilder, TableStyleBuilder},
    traits::{HandleEvents, SetFocus},
    widgets::{CodeInputField, CodeInputFieldBuilder, Table, TableBuilder, Widget},
};
use ferrowl_ui_derive::TableEntry;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, HorizontalAlignment, Layout, Margin, Rect},
    widgets::{Block, Clear, StatefulWidget, Widget as UiWidget},
};

use crate::dialog::lua_help::ScriptContext;
use crate::script_template::{ScriptTemplate, templates};
use crate::view::border_style;

#[derive(Clone, Debug, Default, TableEntry)]
#[table_entry(header = TemplateHeader)]
pub(crate) struct TemplateRow {
    #[column(name = "Template", min = 12, max = 24)]
    name: String,
    #[column(name = "Description", min = 20, max = 60)]
    description: String,
}

type TemplateTable = Widget<TableState<TemplateRow, 2>, Table<TemplateRow, TemplateHeader, 2>>;

/// Which pane of the overlay has focus. The preview is focusable so a long template can be scrolled
/// with the editor's own motions; it stays disabled, so it can never be edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BrowserFocus {
    List,
    Preview,
}

/// What the host dialog should do after feeding a key to an open browser.
#[derive(Debug, PartialEq, Eq)]
pub enum TemplateBrowserEvent {
    /// Key eaten, the overlay stays open.
    Consumed,
    /// `Esc`/`q`: drop the overlay, change nothing.
    Close,
    /// `Enter`: insert this template, then drop the overlay.
    Insert(&'static ScriptTemplate),
}

/// Outcome of [`route_template_browser`], mirroring [`route_lua_help`](super::lua_help::route_lua_help).
#[derive(Debug, PartialEq, Eq)]
pub enum TemplateBrowserOutcome {
    /// No overlay was open; the caller should route the key itself.
    NotActive,
    /// The overlay captured the key (closing itself if applicable).
    Consumed,
    /// The user picked a template; the caller should insert it. The overlay is already cleared.
    Insert(&'static ScriptTemplate),
}

pub struct TemplateBrowser {
    templates: Vec<&'static ScriptTemplate>,
    table: TemplateTable,
    preview: Widget<CodeInputFieldState, CodeInputField>,
    focus: BrowserFocus,
}

impl TemplateBrowser {
    /// Build the browser for `ctx`, listing only that context's templates (UI-R-053).
    pub fn new(ctx: ScriptContext) -> Self {
        let templates = templates(ctx);
        let mut browser = Self {
            table: template_table(rows(&templates)),
            preview: preview_editor(),
            templates,
            focus: BrowserFocus::List,
        };
        browser.sync_preview();
        browser
    }

    fn selected(&self) -> Option<&'static ScriptTemplate> {
        let index = self.table.state.table_state().selected()?;
        self.templates.get(index).copied()
    }

    /// Load the selected template's code into the (read-only) preview.
    fn sync_preview(&mut self) {
        let code = self.selected().map(|t| t.code).unwrap_or_default();
        self.preview.state.set_content(code);
    }

    /// Feed one key while the overlay is open.
    pub fn handle_key(&mut self, modifiers: KeyModifiers, code: KeyCode) -> TemplateBrowserEvent {
        match (modifiers, code) {
            (KeyModifiers::NONE, KeyCode::Esc) => return TemplateBrowserEvent::Close,
            (KeyModifiers::NONE, KeyCode::Enter) => {
                if let Some(template) = self.selected() {
                    return TemplateBrowserEvent::Insert(template);
                }
                return TemplateBrowserEvent::Consumed;
            }
            (KeyModifiers::NONE, KeyCode::Tab)
            | (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::BackTab) => {
                self.focus = match self.focus {
                    BrowserFocus::List => BrowserFocus::Preview,
                    BrowserFocus::Preview => BrowserFocus::List,
                };
                return TemplateBrowserEvent::Consumed;
            }
            _ => {}
        }

        match self.focus {
            BrowserFocus::List => {
                // `q` closes only from the list: in the preview it is a (harmless) editor motion
                // key, and the editor is where a stray `q` is most likely to be typed.
                if let (KeyModifiers::NONE, KeyCode::Char('q')) = (modifiers, code) {
                    return TemplateBrowserEvent::Close;
                }
                let _ = self.table.state.handle_events(modifiers, code);
                self.sync_preview();
            }
            BrowserFocus::Preview => {
                // The preview is a disabled editor: motions work, edits are refused (UI-R-036).
                let _ = self.preview.state.handle_events(modifiers, code);
            }
        }
        TemplateBrowserEvent::Consumed
    }

    /// Render the overlay as a centered popup over `area`.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let [_, hc, _] = Layout::horizontal([
            Constraint::Percentage(8),
            Constraint::Percentage(84),
            Constraint::Percentage(8),
        ])
        .areas(area);
        let [_, vc, _] = Layout::vertical([
            Constraint::Percentage(8),
            Constraint::Percentage(84),
            Constraint::Percentage(8),
        ])
        .areas(hc);

        UiWidget::render(&Clear, vc, buf);
        let block = Block::bordered()
            .style(
                ratatui::prelude::Style::default()
                    .fg(COLOR_SCHEME.hi)
                    .bg(COLOR_SCHEME.bg),
            )
            .title_alignment(HorizontalAlignment::Center)
            .title(" Templates (Enter: insert, Tab: preview, Esc/q: close) ");
        let block_inner = block.inner(vc);
        block.render(vc, buf);
        let inner = block_inner.inner(Margin {
            vertical: 1,
            horizontal: 2,
        });

        let [list_area, preview_area] =
            Layout::horizontal([Constraint::Max(50), Constraint::Min(1)]).areas(inner);

        self.table
            .state
            .set_focused(self.focus == BrowserFocus::List);
        self.preview
            .state
            .set_focused(self.focus == BrowserFocus::Preview);

        StatefulWidget::render(&self.table.widget, list_area, buf, &mut self.table.state);
        StatefulWidget::render(
            &self.preview.widget,
            preview_area,
            buf,
            &mut self.preview.state,
        );
    }
}

/// Feed one key through `browser`, if the template browser is open. Clears `*browser` on close and
/// on insert (the overlay is single-use, like the other script-dialog popups).
pub fn route_template_browser(
    browser: &mut Option<TemplateBrowser>,
    modifiers: KeyModifiers,
    code: KeyCode,
) -> TemplateBrowserOutcome {
    let Some(b) = browser.as_mut() else {
        return TemplateBrowserOutcome::NotActive;
    };
    match b.handle_key(modifiers, code) {
        TemplateBrowserEvent::Consumed => TemplateBrowserOutcome::Consumed,
        TemplateBrowserEvent::Close => {
            *browser = None;
            TemplateBrowserOutcome::Consumed
        }
        TemplateBrowserEvent::Insert(template) => {
            *browser = None;
            TemplateBrowserOutcome::Insert(template)
        }
    }
}

fn rows(templates: &[&'static ScriptTemplate]) -> Vec<TemplateRow> {
    templates
        .iter()
        .map(|t| TemplateRow {
            name: t.name.to_string(),
            description: t.description.to_string(),
        })
        .collect()
}

fn template_table(rows: Vec<TemplateRow>) -> TemplateTable {
    Widget {
        state: TableStateBuilder::default()
            .values(rows)
            .build()
            .expect("all required builder fields are set"),
        widget: TableBuilder::default()
            .border(Border::Full(Margin::new(1, 0)))
            .title(Some("Templates".into()))
            .style(
                TableStyleBuilder::default()
                    .build()
                    .expect("all required builder fields are set"),
            )
            .row_margin(Margin {
                vertical: 0,
                horizontal: 0,
            })
            .build()
            .expect("all required builder fields are set"),
    }
}

/// The preview pane: a Lua code editor kept `disabled`, i.e. read-only (UI-R-036).
fn preview_editor() -> Widget<CodeInputFieldState, CodeInputField> {
    Widget {
        state: CodeInputFieldStateBuilder::default()
            .focused(false)
            .disabled(true)
            .placeholder(Some("-- no template selected".to_string()))
            .language(Some(Language::Lua))
            .build()
            .expect("all required builder fields are set"),
        widget: CodeInputFieldBuilder::default()
            .border(Border::Full(Margin::new(1, 0)))
            .title(Some("Preview".into()))
            .style(
                InputFieldStyleBuilder::default()
                    .border(border_style())
                    .build()
                    .expect("all required builder fields are set"),
            )
            .margin(Margin {
                vertical: 0,
                horizontal: 0,
            })
            .build()
            .expect("all required builder fields are set"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrowl_ui::state::VimMode;

    #[test]
    /// UI-R-053 — the browser lists only the templates of its own script context.
    fn ut_lists_only_context_templates() {
        let browser = TemplateBrowser::new(ScriptContext::Modbus);
        assert!(!browser.templates.is_empty());
        for template in &browser.templates {
            assert!(
                template
                    .contexts
                    .iter()
                    .any(|&c| ScriptContext::from(c) == ScriptContext::Modbus),
                "'{}' is not a modbus template",
                template.name
            );
        }
    }

    #[test]
    /// UI-R-053 — the preview shows the selected template's code and cannot be edited.
    fn ut_preview_is_read_only_and_follows_selection() {
        let mut browser = TemplateBrowser::new(ScriptContext::Modbus);
        let first = browser.selected().unwrap();
        assert_eq!(browser.preview.state.content().trim(), first.code.trim());
        assert!(browser.preview.state.disabled());

        // Typing into the focused preview must not change it.
        browser.focus = BrowserFocus::Preview;
        let before = browser.preview.state.content();
        browser.handle_key(KeyModifiers::NONE, KeyCode::Char('i'));
        browser.handle_key(KeyModifiers::NONE, KeyCode::Char('x'));
        assert_eq!(browser.preview.state.content(), before);

        // Moving the selection re-loads the preview.
        browser.focus = BrowserFocus::List;
        browser.handle_key(KeyModifiers::NONE, KeyCode::Down);
        let second = browser.selected().unwrap();
        assert_ne!(second.name, first.name);
        assert_eq!(browser.preview.state.content().trim(), second.code.trim());
    }

    #[test]
    /// UI-R-200 — `Esc` and `q` close the overlay without picking anything.
    fn ut_esc_and_q_close() {
        let mut browser = TemplateBrowser::new(ScriptContext::Session);
        assert_eq!(
            browser.handle_key(KeyModifiers::NONE, KeyCode::Esc),
            TemplateBrowserEvent::Close
        );
        assert_eq!(
            browser.handle_key(KeyModifiers::NONE, KeyCode::Char('q')),
            TemplateBrowserEvent::Close
        );
    }

    #[test]
    /// UI-R-054 — `Enter` yields the selected template for insertion.
    fn ut_enter_yields_selected_template() {
        let mut browser = TemplateBrowser::new(ScriptContext::Session);
        let selected = browser.selected().unwrap();
        assert_eq!(
            browser.handle_key(KeyModifiers::NONE, KeyCode::Enter),
            TemplateBrowserEvent::Insert(selected)
        );
    }

    #[test]
    /// Tab cycles focus between the template list and its preview.
    fn ut_tab_cycles_list_and_preview() {
        let mut browser = TemplateBrowser::new(ScriptContext::Modbus);
        assert_eq!(browser.focus, BrowserFocus::List);
        browser.handle_key(KeyModifiers::NONE, KeyCode::Tab);
        assert_eq!(browser.focus, BrowserFocus::Preview);
        browser.handle_key(KeyModifiers::NONE, KeyCode::Tab);
        assert_eq!(browser.focus, BrowserFocus::List);
    }

    #[test]
    /// Routing reports NotActive when the browser is closed.
    fn ut_route_not_active_when_none() {
        let mut browser: Option<TemplateBrowser> = None;
        assert_eq!(
            route_template_browser(&mut browser, KeyModifiers::NONE, KeyCode::Enter),
            TemplateBrowserOutcome::NotActive
        );
    }

    #[test]
    /// UI-R-054 — confirming a template through the router closes the browser overlay.
    fn ut_route_insert_clears_overlay() {
        let mut browser = Some(TemplateBrowser::new(ScriptContext::Session));
        let outcome = route_template_browser(&mut browser, KeyModifiers::NONE, KeyCode::Enter);
        assert!(matches!(outcome, TemplateBrowserOutcome::Insert(_)));
        assert!(browser.is_none());
    }

    #[test]
    /// UI-R-200 — a close key clears the browser overlay.
    fn ut_route_close_clears_overlay() {
        let mut browser = Some(TemplateBrowser::new(ScriptContext::Session));
        assert_eq!(
            route_template_browser(&mut browser, KeyModifiers::NONE, KeyCode::Esc),
            TemplateBrowserOutcome::Consumed
        );
        assert!(browser.is_none());
    }

    #[test]
    fn ut_render_does_not_panic() {
        let mut browser = TemplateBrowser::new(ScriptContext::OcppServer);
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        browser.render(area, &mut buf);
    }

    #[test]
    /// UI-R-053 — the read-only template preview cannot be edited.
    fn ut_preview_stays_in_normal_mode() {
        // Sanity: a disabled vim editor never leaves Normal mode, so `i` cannot start typing.
        let mut browser = TemplateBrowser::new(ScriptContext::Modbus);
        browser.focus = BrowserFocus::Preview;
        browser.handle_key(KeyModifiers::NONE, KeyCode::Char('i'));
        assert_eq!(browser.preview.state.vim_mode(), VimMode::Normal);
    }
}
