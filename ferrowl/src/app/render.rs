//! Whole-frame rendering: tab bar, module view, log pane, command line and overlay.

use ferrowl_ui::COLOR_SCHEME;
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Constraint, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, StatefulWidget},
};

use crate::dialog::scripts::ScriptDialog;
use crate::module::view::CommandDescriptor;
use crate::view::command::CommandLine;
use crate::view::tabs::{render_tabs, tab_bar_height};

use super::{Focus, Overlay, Tab, help};

#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    frame: &mut Frame,
    tabs: &mut [Tab],
    active: usize,
    focus: Focus,
    command: &mut CommandLine,
    overlay: Option<&mut Overlay>,
    session_dialog: Option<&mut ScriptDialog>,
    help_open: bool,
    help_scroll: &mut u16,
    tab_scroll: &mut usize,
) {
    let area = frame.area();
    let [tabs_area, view_area, log_area, cmd_area] = Layout::vertical([
        Constraint::Length(tab_bar_height()),
        Constraint::Min(1),
        Constraint::Length(10),
        Constraint::Length(1),
    ])
    .areas(area);

    // Phase 1: background and tab bar.
    {
        let buf = frame.buffer_mut();
        buf.set_style(area, Style::default().bg(COLOR_SCHEME.bg));

        let names: Vec<String> = tabs
            .iter()
            .enumerate()
            .map(|(i, t)| format!(" [{i}] {} ", t.name))
            .collect();
        render_tabs(&names, active, tabs_area, buf, tab_scroll);
    }

    // Phase 2: module content view (includes its own status bar). Focus is carried by the view's
    // own stored state (set at focus-change time), not recomputed here.
    if let Some(tab) = tabs.get_mut(active) {
        tab.view.render(frame, view_area);
    }

    // Phase 3: log pane and command line.
    {
        let buf = frame.buffer_mut();
        if let Some(tab) = tabs.get_mut(active) {
            StatefulWidget::render(&tab.log_view.widget, log_area, buf, &mut tab.log_view.state);
        }
        render_command(command, focus, cmd_area, buf);
    }

    // Phase 4: overlays, painted on top of content and log (bottom-to-top z-order).
    // 1. Module dialogs. Drawn first so command help and the app dialog sit above them. The view's
    //    own match no-ops when no overlay is open, so this is called unconditionally.
    if let Some(tab) = tabs.get_mut(active) {
        tab.view.render_overlay(frame, view_area);
    }
    // 2. Command help popup and 3. app-level modal dialog. Both draw to the buffer; the module
    //    overlay above needed `&mut Frame`, so these go in a separate, sequential borrow.
    {
        let buf = frame.buffer_mut();
        if focus == Focus::Command {
            let module_cmds: &[CommandDescriptor] =
                tabs.get(active).map_or(&[], |t| t.view.commands());
            render_command_help(cmd_area, buf, module_cmds);
        }
        if let Some(dialog) = overlay {
            dialog.render(area, buf);
        }
        if let Some(dialog) = session_dialog {
            dialog.render(area, buf);
        }
        // 4. Keybind help dialog, always topmost.
        if help_open {
            let module = tabs
                .get(active)
                .map(|t| (t.name.as_str(), t.view.keybinds()));
            render_help(area, buf, module, help_scroll);
        }
    }
}

fn render_help(
    area: Rect,
    buf: &mut Buffer,
    module: Option<(&str, &[CommandDescriptor])>,
    scroll: &mut u16,
) {
    let popup_w = 120.min(area.width);
    let inner_width = popup_w.saturating_sub(4) as usize;
    let desc_budget = inner_width.saturating_sub(36);

    let desc_style = Style::default().fg(COLOR_SCHEME.text).bg(COLOR_SCHEME.bg);
    let make_lines = |(key, desc): (&str, &str)| -> Vec<Line<'static>> {
        let mut segments = crate::view::text::wrap(desc, desc_budget).into_iter();
        let first = segments.next().unwrap_or_default();
        let mut out = vec![Line::from(vec![
            Span::styled(
                format!("  {key:<30}"),
                Style::default()
                    .fg(COLOR_SCHEME.hi)
                    .bg(COLOR_SCHEME.bg)
                    .bold(),
            ),
            Span::styled(first, desc_style),
        ])];
        out.extend(
            segments.map(|seg| Line::from(Span::styled(format!("{:32}{seg}", ""), desc_style))),
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
    for section in help::GLOBAL_SECTIONS {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(section_title(section.title));
        lines.extend(section.keys.iter().flat_map(|&kd| make_lines(kd)));
    }
    if let Some((name, keys)) = module
        && !keys.is_empty()
    {
        lines.push(Line::default());
        lines.push(section_title(name));
        lines.extend(
            keys.iter()
                .flat_map(|k| make_lines((k.name, k.description))),
        );
    }

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
        .title(" Keybinds (Esc/q/? to close) ")
        .style(Style::default().fg(COLOR_SCHEME.hi).bg(COLOR_SCHEME.bg));
    let inner = block.inner(popup).inner(Margin::new(2, 1));
    ratatui::prelude::Widget::render(block, popup, buf);

    *scroll = (*scroll).min((lines.len() as u16).saturating_sub(inner.height));
    ratatui::prelude::Widget::render(
        Paragraph::new(lines)
            .scroll((*scroll, 0))
            .style(Style::default().bg(COLOR_SCHEME.bg)),
        inner,
        buf,
    );
}

fn render_command_help(cmd_area: Rect, buf: &mut Buffer, module_cmds: &[CommandDescriptor]) {
    const COLS: &[(&str, &str)] = &[
        (":q | :quit", "quit tab"),
        (":qa | :qall", "quit all tabs"),
        (":n | :new", "new module tab"),
        (":l | :load [path]", "load device config"),
        (":s | :save | :w | :write [path]", "save session"),
        (":swap <id> <id>", "swap tabs"),
        (":log clear", "clear log view"),
        (":script copy <tab>", "replace scripts with tab <tab>'s"),
        (":session", "session scripts + sim interval"),
    ];
    let popup_w: u16 = 62;
    let inner_width = (popup_w.min(cmd_area.width)).saturating_sub(2) as usize;
    let desc_budget = inner_width.saturating_sub(34);

    let desc_style = Style::default().fg(COLOR_SCHEME.text).bg(COLOR_SCHEME.bg);
    let make_lines = |(cmd, desc): (&str, &str)| -> Vec<Line<'static>> {
        let mut segments = crate::view::text::wrap(desc, desc_budget).into_iter();
        let first = segments.next().unwrap_or_default();
        let mut out = vec![Line::from(vec![
            Span::styled(
                format!("{cmd:<34}"),
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
    let lines: Vec<Line> = COLS
        .iter()
        .flat_map(|&(cmd, desc)| make_lines((cmd, desc)))
        .chain(
            module_cmds
                .iter()
                .flat_map(|c| make_lines((c.name, c.description))),
        )
        .collect();

    let popup_h: u16 = lines.len() as u16 + 2;
    let x = cmd_area.x;
    let y = cmd_area.y.saturating_sub(popup_h);
    let popup = Rect {
        x,
        y,
        width: popup_w.min(cmd_area.width),
        height: popup_h,
    };

    ratatui::prelude::Widget::render(Clear, popup, buf);
    let block = Block::bordered().style(Style::default().fg(COLOR_SCHEME.hi).bg(COLOR_SCHEME.bg));
    let inner = block.inner(popup);
    ratatui::prelude::Widget::render(block, popup, buf);

    ratatui::prelude::Widget::render(
        Paragraph::new(lines).style(Style::default().bg(COLOR_SCHEME.bg)),
        inner,
        buf,
    );
}

fn render_command(command: &mut CommandLine, focus: Focus, area: Rect, buf: &mut Buffer) {
    if focus == Focus::Command {
        buf.set_string(
            area.x,
            area.y,
            ":",
            Style::default().fg(COLOR_SCHEME.hi).bg(COLOR_SCHEME.bg),
        );
        let input_area = Rect {
            x: area.x.saturating_add(1),
            y: area.y,
            width: area.width.saturating_sub(1),
            height: area.height,
        };
        StatefulWidget::render(&command.widget, input_area, buf, &mut command.state);
    } else {
        buf.set_style(area, Style::default().bg(COLOR_SCHEME.bg));
        buf.set_string(
            area.x,
            area.y,
            "  :  command    |    C-w+j C-w+k  table/log    |    C-t+h C-t+l  tabs    |    ?  help",
            Style::default().fg(COLOR_SCHEME.text).bg(COLOR_SCHEME.bg),
        );
    }
}
