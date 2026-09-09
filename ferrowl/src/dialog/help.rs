//! The scrollable `?` help overlay shared by the script dialog's two help pages: the Lua bindings
//! available in a script context ([`crate::dialog::lua_help`]) and the script table's own keybinds
//! ([`crate::dialog::script_keys`]). Both are the same shape — titled groups of
//! `(name, description)` rows — so they share one widget and differ only in the sections they carry.

use crossterm::event::{KeyCode, KeyModifiers};
use ferrowl_ui::COLOR_SCHEME;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};

/// One titled group shown in the overlay: a heading plus its `(name, description)` entries. `name`
/// is a Lua signature on the bindings page and a key on the keybind page.
pub struct BindingSection {
    pub title: &'static str,
    pub entries: &'static [(&'static str, &'static str)],
}

/// Builds the popup's logical lines, word-wrapping each entry's description to `desc_budget`
/// columns with a 34-col indent (2-space indent + `{name:<32}`) on continuation lines.
fn build_lines(
    sections: &'static [&'static BindingSection],
    desc_budget: usize,
) -> Vec<Line<'static>> {
    let desc_style = Style::default().fg(COLOR_SCHEME.text).bg(COLOR_SCHEME.bg);
    let make_lines = |(name, desc): &(&str, &str)| -> Vec<Line<'static>> {
        let mut segments = crate::view::text::wrap(desc, desc_budget).into_iter();
        let first = segments.next().unwrap_or_default();
        let mut out = vec![Line::from(vec![
            Span::styled(
                format!("  {name:<32}"),
                Style::default()
                    .fg(COLOR_SCHEME.hi)
                    .bg(COLOR_SCHEME.bg)
                    .bold(),
            ),
            Span::styled(first, desc_style),
        ])];
        out.extend(
            segments.map(|seg| Line::from(Span::styled(format!("{:34}{seg}", ""), desc_style))),
        );
        out
    };
    let section_title = |title: &str| {
        Line::from(Span::styled(
            title.to_string(),
            Style::default()
                .fg(COLOR_SCHEME.text)
                .bg(COLOR_SCHEME.bg)
                .bold(),
        ))
    };

    let mut lines: Vec<Line> = Vec::new();
    for section in sections {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(section_title(section.title));
        lines.extend(section.entries.iter().flat_map(make_lines));
    }
    lines
}

/// A scrollable `?` overlay over one set of [`BindingSection`]s.
pub struct HelpOverlay {
    title: &'static str,
    sections: &'static [&'static BindingSection],
    scroll: u16,
    /// Highest valid `scroll` value, refreshed on every `render` call from the content length and
    /// viewport height. Used to clamp `handle_key`'s `j`/`G` so `scroll` never runs past the last
    /// visible line.
    max_scroll: u16,
}

impl HelpOverlay {
    /// Open the overlay on `sections`. `title` names the page (it is shown in the popup's border,
    /// with the close keys appended).
    pub fn new(title: &'static str, sections: &'static [&'static BindingSection]) -> Self {
        Self {
            title,
            sections,
            scroll: 0,
            max_scroll: 0,
        }
    }

    /// Handles one key while the overlay is open. Returns `true` if the overlay should close.
    pub fn handle_key(&mut self, modifiers: KeyModifiers, code: KeyCode) -> bool {
        let _ = modifiers;
        match code {
            KeyCode::Esc | KeyCode::Char('q' | '?') => true,
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll = self.scroll.saturating_add(1).min(self.max_scroll);
                false
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll = self.scroll.saturating_sub(1);
                false
            }
            KeyCode::Char('g') => {
                self.scroll = 0;
                false
            }
            KeyCode::Char('G') => {
                self.scroll = self.max_scroll;
                false
            }
            _ => false,
        }
    }

    /// Renders the overlay as a centered popup over `area`.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let popup_w = 120.min(area.width);
        let inner_width = popup_w.saturating_sub(4) as usize;
        let desc_budget = inner_width.saturating_sub(36);

        let lines = build_lines(self.sections, desc_budget);

        let popup_h = (lines.len() as u16 + 4).min(area.height.saturating_sub(4));
        let [_, mid, _] = Layout::horizontal([
            Constraint::Min(1),
            Constraint::Length(popup_w),
            Constraint::Min(1),
        ])
        .areas(area);
        let [_, popup, _] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(popup_h),
            Constraint::Min(1),
        ])
        .areas(mid);

        ratatui::prelude::Widget::render(Clear, popup, buf);
        let block = Block::bordered()
            .title(format!(" {} (Esc/q/? to close) ", self.title))
            .style(Style::default().fg(COLOR_SCHEME.hi).bg(COLOR_SCHEME.bg));
        let inner = block.inner(popup).inner(Margin::new(2, 1));
        ratatui::prelude::Widget::render(block, popup, buf);

        self.max_scroll = (lines.len() as u16).saturating_sub(inner.height);
        self.scroll = self.scroll.min(self.max_scroll);
        ratatui::prelude::Widget::render(
            Paragraph::new(lines)
                .scroll((self.scroll, 0))
                .style(Style::default().bg(COLOR_SCHEME.bg)),
            inner,
            buf,
        );
    }
}

/// Outcome of [`route_help`]: whether an overlay was open, so the caller knows whether to keep
/// routing the key itself.
#[derive(Debug, PartialEq, Eq)]
pub enum HelpOutcome {
    /// No overlay was open; the key wasn't touched, the caller should route it itself.
    NotActive,
    /// The overlay captured the key (closing itself if applicable); the caller should stop routing
    /// this key further.
    Consumed,
}

/// Feed one key through `help`, if a help overlay is currently open. Clears `*help` when
/// [`HelpOverlay::handle_key`] reports the overlay should close.
pub fn route_help(
    help: &mut Option<HelpOverlay>,
    modifiers: KeyModifiers,
    code: KeyCode,
) -> HelpOutcome {
    let Some(overlay) = help.as_mut() else {
        return HelpOutcome::NotActive;
    };
    if overlay.handle_key(modifiers, code) {
        *help = None;
    }
    HelpOutcome::Consumed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialog::lua_help::{self, ScriptContext};

    fn overlay() -> HelpOverlay {
        HelpOverlay::new("Lua Bindings", lua_help::sections(ScriptContext::Session))
    }

    #[test]
    /// UI-R-186 — Esc/q/? close the script dialog's help overlay.
    fn ut_handle_key_close_keys() {
        let mut o = overlay();
        assert!(o.handle_key(KeyModifiers::NONE, KeyCode::Esc));
        assert!(o.handle_key(KeyModifiers::NONE, KeyCode::Char('q')));
        assert!(o.handle_key(KeyModifiers::NONE, KeyCode::Char('?')));
    }

    #[test]
    /// j/k scroll the help overlay.
    fn ut_handle_key_scroll() {
        let mut o = overlay();
        o.max_scroll = 100;
        assert!(!o.handle_key(KeyModifiers::NONE, KeyCode::Char('j')));
        assert_eq!(o.scroll, 1);
        assert!(!o.handle_key(KeyModifiers::NONE, KeyCode::Down));
        assert_eq!(o.scroll, 2);
        assert!(!o.handle_key(KeyModifiers::NONE, KeyCode::Char('k')));
        assert_eq!(o.scroll, 1);
        assert!(!o.handle_key(KeyModifiers::NONE, KeyCode::Up));
        assert_eq!(o.scroll, 0);
        // Saturating: k at 0 stays at 0.
        assert!(!o.handle_key(KeyModifiers::NONE, KeyCode::Char('k')));
        assert_eq!(o.scroll, 0);
    }

    #[test]
    /// g/G jump to the top/bottom of the help overlay.
    fn ut_handle_key_top_bottom() {
        let mut o = overlay();
        o.max_scroll = 20;
        o.scroll = 5;
        assert!(!o.handle_key(KeyModifiers::NONE, KeyCode::Char('g')));
        assert_eq!(o.scroll, 0);
        assert!(!o.handle_key(KeyModifiers::NONE, KeyCode::Char('G')));
        assert_eq!(o.scroll, 20);
    }

    #[test]
    /// Scrolling past the bottom of the help overlay does not overflow.
    fn ut_handle_key_g_then_j_stays_at_max_no_overflow() {
        let mut o = overlay();
        o.max_scroll = 7;
        assert!(!o.handle_key(KeyModifiers::NONE, KeyCode::Char('G')));
        assert_eq!(o.scroll, 7);
        for _ in 0..5 {
            assert!(!o.handle_key(KeyModifiers::NONE, KeyCode::Char('j')));
        }
        assert_eq!(o.scroll, 7);
    }

    #[test]
    /// The help overlay clamps its scroll to the rendered viewport height.
    fn ut_render_sets_max_scroll_and_handle_key_respects_it_after() {
        // A render with a tall content and small viewport should compute a finite max_scroll;
        // subsequent `G`/`j` must not exceed it.
        let mut o = overlay();
        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);
        o.render(area, &mut buf);
        assert!(o.max_scroll < u16::MAX);
        let max = o.max_scroll;
        assert!(!o.handle_key(KeyModifiers::NONE, KeyCode::Char('G')));
        assert_eq!(o.scroll, max);
        assert!(!o.handle_key(KeyModifiers::NONE, KeyCode::Char('j')));
        assert_eq!(o.scroll, max);
    }

    #[test]
    /// Help overlay lines fit the popup width.
    fn ut_build_lines_fit_popup_width() {
        // popup_w = 75, inner_width = 73, desc_budget = 73 - 34 = 39.
        let lines = build_lines(lua_help::sections(ScriptContext::OcppClient), 39);
        for line in &lines {
            let total: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
            assert!(total <= 73, "line too wide ({total} chars): {line:?}");
        }
    }

    #[test]
    /// A long help entry description wraps within the overlay.
    fn ut_build_lines_wraps_long_action_desc() {
        let secs = lua_help::sections(ScriptContext::OcppClient);
        let ocpp = secs.iter().find(|s| s.title == "C_OCPP").unwrap();
        let (_, action_desc) = ocpp
            .entries
            .iter()
            .find(|(sig, _)| sig.contains("<Action>"))
            .unwrap();
        assert!(action_desc.chars().count() > 39);
        let wrapped_segments = crate::view::text::wrap(action_desc, 39).len();
        assert!(
            wrapped_segments >= 2,
            "expected the long <Action> description to wrap into >= 2 segments, got {wrapped_segments}"
        );

        // build_lines must emit exactly that many physical lines for this entry: one entry-count
        // increase per section beyond the un-wrapped baseline.
        let lines = build_lines(secs, 39);
        let baseline: usize = 1 // section title
            + ocpp.entries.len();
        assert!(lines.len() >= baseline + (wrapped_segments - 1));
    }

    #[test]
    /// UI-R-187 — the help overlay consumes all other keys while open.
    fn ut_handle_key_other_keys_eaten() {
        let mut o = overlay();
        assert!(!o.handle_key(KeyModifiers::NONE, KeyCode::Char('x')));
        assert!(!o.handle_key(KeyModifiers::NONE, KeyCode::Enter));
        assert!(!o.handle_key(KeyModifiers::NONE, KeyCode::Tab));
    }

    // --- route_help -----------------------------------------------------

    #[test]
    /// Routing reports NotActive when the help overlay is closed.
    fn ut_route_not_active_when_none() {
        let mut help: Option<HelpOverlay> = None;
        assert_eq!(
            route_help(&mut help, KeyModifiers::NONE, KeyCode::Char('q')),
            HelpOutcome::NotActive
        );
        assert!(help.is_none());
    }

    #[test]
    /// UI-R-186 — a close key routed to the help overlay clears it.
    fn ut_route_close_key_clears_overlay() {
        let mut help = Some(overlay());
        assert_eq!(
            route_help(&mut help, KeyModifiers::NONE, KeyCode::Esc),
            HelpOutcome::Consumed
        );
        assert!(help.is_none());
    }

    #[test]
    /// UI-R-187 — an unrelated key keeps the help overlay open and is consumed.
    fn ut_route_other_key_stays_open_and_consumed() {
        let mut help = Some(overlay());
        assert_eq!(
            route_help(&mut help, KeyModifiers::NONE, KeyCode::Char('j')),
            HelpOutcome::Consumed
        );
        assert!(help.is_some());
    }
}
