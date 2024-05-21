use crate::register::Handler;
use crate::util::str;
use crate::{Command, Status};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use itertools::Itertools;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::{prelude::*, widgets::*};
use std::io::stdout;
use std::time::Duration;
use style::palette::tailwind;
use tokio::sync::mpsc::{Receiver, Sender};
use unicode_width::UnicodeWidthStr;

const PALETTES: [tailwind::Palette; 4] = [
    tailwind::BLUE,
    tailwind::INDIGO,
    tailwind::EMERALD,
    tailwind::RED,
];

const INFO_TEXT: &str =
    "(q) quit | (k) up | (j) down | (h) left | (l) right | (t) color | (n) hex/dec | (d) disconnect | (c) connect";

const LOG_HEADER: &str = " Modbus Log";

const ITEM_HEIGHT: usize = 3;

struct TableColors {
    buffer_bg: Color,
    header_bg: Color,
    header_fg: Color,
    row_fg: Color,
    selected_style_fg: Color,
    normal_row_color: Color,
    alt_row_color: Color,
    error_color: Color,
    success_color: Color,
}

impl TableColors {
    const fn new(color: &tailwind::Palette) -> Self {
        Self {
            buffer_bg: tailwind::SLATE.c950,
            header_bg: color.c900,
            header_fg: tailwind::SLATE.c200,
            row_fg: tailwind::SLATE.c200,
            selected_style_fg: color.c400,
            normal_row_color: tailwind::SLATE.c950,
            alt_row_color: tailwind::SLATE.c900,
            error_color: tailwind::RED.c900,
            success_color: tailwind::GREEN.c900,
        }
    }
}

pub struct UiTable {
    table_state: TableState,
    vertical_scroll_state: ScrollbarState,
    horizontal_scroll_offset: u16,
    row_max_width: u16,
    visible_width: u16,
}

impl UiTable {
    pub fn new(len: usize, item_height: usize) -> Self {
        Self {
            table_state: TableState::default().with_selected(0),
            vertical_scroll_state: ScrollbarState::new((len - 1) * item_height),
            horizontal_scroll_offset: 0,
            row_max_width: 0,
            visible_width: 0,
        }
    }
}

pub struct App<'a, const SLICE_SIZE: usize> {
    register_handler: Handler<'a, SLICE_SIZE>,
    register_table: UiTable,
    log_entries: Vec<Result<String, String>>,
    log_table: UiTable,
    colors: TableColors,
    color_index: usize,
    show_as_hex: bool,
    history_len: usize,
}

impl<'a, const SLICE_SIZE: usize> App<'a, SLICE_SIZE> {
    pub fn new(register_handler: Handler<'a, SLICE_SIZE>, history_len: usize) -> Self {
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            disable_raw_mode().unwrap();
            crossterm::execute!(stdout(), LeaveAlternateScreen).unwrap();
            original_hook(panic);
        }));

        let len = register_handler.len();
        Self {
            register_handler,
            register_table: UiTable::new(len, ITEM_HEIGHT),
            log_entries: Vec::with_capacity(history_len + 10),
            log_table: UiTable::new(1, 1),
            colors: TableColors::new(&PALETTES[0]),
            color_index: 0,
            show_as_hex: true,
            history_len,
        }
    }

    pub fn log_move_down(&mut self) {
        if self.log_entries.is_empty() {
            return;
        }
        let i = match self.log_table.table_state.selected() {
            Some(i) => {
                if i >= self.log_entries.len() - 1 {
                    self.log_entries.len() - 1
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.log_table.table_state.select(Some(i));
        self.log_table.vertical_scroll_state = self.log_table.vertical_scroll_state.position(i);
    }

    pub fn log_move_up(&mut self) {
        if self.log_entries.is_empty() {
            return;
        }
        let i = match self.log_table.table_state.selected() {
            Some(i) => {
                if i == 0 {
                    0
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.log_table.table_state.select(Some(i));
        self.log_table.vertical_scroll_state = self.log_table.vertical_scroll_state.position(i);
    }

    pub fn move_down(&mut self) {
        let i = match self.register_table.table_state.selected() {
            Some(i) => {
                if i >= self.register_handler.values().len() - 1 {
                    self.register_handler.values().len() - 1
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.register_table.table_state.select(Some(i));
        self.register_table.vertical_scroll_state = self
            .register_table
            .vertical_scroll_state
            .position(i * ITEM_HEIGHT);
    }

    pub fn move_up(&mut self) {
        let i = match self.register_table.table_state.selected() {
            Some(i) => {
                if i == 0 {
                    0
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.register_table.table_state.select(Some(i));
        self.register_table.vertical_scroll_state = self
            .register_table
            .vertical_scroll_state
            .position(i * ITEM_HEIGHT);
    }

    pub fn move_left(&mut self) {
        self.register_table.horizontal_scroll_offset =
            std::cmp::max(3, self.register_table.horizontal_scroll_offset) - 3;
    }

    pub fn move_right(&mut self) {
        self.register_table.horizontal_scroll_offset = std::cmp::min(
            std::cmp::max(
                self.register_table.row_max_width,
                self.register_table.visible_width,
            ) - self.register_table.visible_width,
            self.register_table.horizontal_scroll_offset + 3,
        );
    }

    pub fn switch_color(&mut self) {
        self.color_index = (self.color_index + 1) % PALETTES.len();
    }

    pub fn set_colors(&mut self) {
        self.colors = TableColors::new(&PALETTES[self.color_index]);
    }

    pub fn switch(&mut self) {
        self.show_as_hex = !self.show_as_hex;
    }

    pub fn run(
        self,
        mut status_recv: Receiver<Status>,
        mut log_recv: Receiver<Result<String, String>>,
        command_send: Sender<Command>,
    ) -> anyhow::Result<()> {
        enable_raw_mode()?;
        let mut terminal = App::<SLICE_SIZE>::create_terminal()?;
        execute!(terminal.backend_mut(), DisableMouseCapture)?;

        let mut status = str!("");
        let mut app = self;
        loop {
            let _ = app.register_handler.update();
            if let Ok(v) = status_recv.try_recv() {
                match v {
                    Status::String(v) => {
                        status = v;
                    }
                }
            }
            for _ in 0..5 {
                if let Ok(v) = log_recv.try_recv() {
                    app.log_entries.push(v);
                    match app.log_table.table_state.selected() {
                        Some(i) if i + 2 == app.log_entries.len() => {
                            app.log_table
                                .table_state
                                .select(Some(std::cmp::min(19, app.log_entries.len() - 1)));
                        }
                        None => {
                            app.log_table.table_state.select(Some(0));
                        }
                        _ => {}
                    };
                } else {
                    break;
                }
            }
            //app.register_handler.update()?;
            terminal.draw(|f| ui(f, &mut app, status.clone()))?;

            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        use KeyCode::*;
                        match key.code {
                            Char('q') | Esc => break,
                            Char('j') | Down => app.move_down(),
                            Char('k') | Up => app.move_up(),
                            Char('h') | Left => app.move_left(),
                            Char('l') | Right => app.move_right(),
                            Char('n') | Tab => app.switch(),
                            Char('t') => app.switch_color(),
                            Char('d') => command_send.blocking_send(Command::Disconnect)?,
                            Char('c') => command_send.blocking_send(Command::Connect)?,
                            PageUp => app.log_move_up(),
                            PageDown => app.log_move_down(),
                            _ => {}
                        }
                    }
                }
            }
        }

        disable_raw_mode()?;

        // restore terminal
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        Ok(())
    }

    fn create_terminal() -> anyhow::Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
        let mut stdout = stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(terminal)
    }
}

fn ui<const SLICE_SIZE: usize>(f: &mut Frame, app: &mut App<SLICE_SIZE>, status: String) {
    let rects = Layout::vertical([
        Constraint::Min(5),
        Constraint::Length(2),
        Constraint::Max(7),
        Constraint::Length(1),
    ])
    .split(f.size());
    app.set_colors();
    // Draw register table
    render_register::<SLICE_SIZE>(f, app, rects[0]);
    render_scrollbar::<SLICE_SIZE>(f, &mut app.register_table.vertical_scroll_state, rects[0]);
    render_register_footer::<SLICE_SIZE>(f, app, rects[1], status);
    // Draw log table
    render_log::<SLICE_SIZE>(f, app, rects[2]);
    render_scrollbar::<SLICE_SIZE>(f, &mut app.log_table.vertical_scroll_state, rects[2]);
    render_log_footer::<SLICE_SIZE>(f, app, rects[3]);
}

fn vec_as_str(v: &[u16], hex: bool) -> String {
    if hex {
        format!("[ {:#06X} ]", v.iter().format(", "))
    } else {
        format!("{:?}", v)
    }
}

fn render_register<const SLICE_SIZE: usize>(f: &mut Frame, app: &mut App<SLICE_SIZE>, area: Rect) {
    let header_style = Style::default()
        .fg(app.colors.header_fg)
        .bg(app.colors.header_bg);
    let selected_style = Style::default()
        .add_modifier(Modifier::REVERSED)
        .fg(app.colors.selected_style_fg);

    let cols = ["Name", "Address", "Type", "Length", "Value", "Raw Data"];
    let header = cols
        .into_iter()
        .map(Cell::from)
        .collect::<Row>()
        .style(header_style)
        .height(1);
    let items = app
        .register_handler
        .values()
        .iter()
        .sorted_by(|a, b| Ord::cmp(&a.1.address(), &b.1.address()))
        .map(|(n, r)| {
            [
                str!(n),
                format!("{:#06X} ({})", r.address(), r.address()),
                format!("{:?}", r.r#type()),
                r.raw().len().to_string(),
                r.value()
                    .chars()
                    .map(|c| {
                        if c as u8 >= 0x20 && c as u8 <= 126 {
                            c
                        } else {
                            '.'
                        }
                    })
                    .collect(),
                vec_as_str(r.raw(), app.show_as_hex),
            ]
        })
        .collect::<Vec<_>>();
    let limits = items.iter().fold(
        (
            cols[0].width() as u16,
            cols[1].width() as u16,
            cols[2].width() as u16,
            cols[3].width() as u16,
            cols[4].width() as u16,
            cols[5].width() as u16,
        ),
        |acc, item| {
            (
                std::cmp::max(acc.0, item[0].width() as u16),
                std::cmp::max(acc.1, item[1].width() as u16),
                std::cmp::max(acc.2, item[2].width() as u16),
                std::cmp::max(acc.3, item[3].width() as u16),
                std::cmp::max(acc.4, item[4].width() as u16),
                std::cmp::max(acc.5, item[5].width() as u16),
            )
        },
    );

    app.register_table.row_max_width =
        limits.0 + limits.1 + limits.2 + limits.3 + limits.4 + limits.5 + 25;

    let rows = items.iter().enumerate().map(|(i, item)| {
        let color = match i % 2 {
            0 => app.colors.normal_row_color,
            _ => app.colors.alt_row_color,
        };
        item.iter()
            .map(|content| Cell::from(Text::from(format!("\n{content}\n"))))
            .collect::<Row>()
            .style(Style::new().fg(app.colors.row_fg).bg(color))
            .height(ITEM_HEIGHT as u16)
    });

    let bar = " █ ";
    let t = Table::new(
        rows,
        [
            // + 1 is for padding.
            Constraint::Min(limits.0 + 1),
            Constraint::Min(limits.1 + 1),
            Constraint::Min(limits.2 + 1),
            Constraint::Min(limits.3 + 1),
            Constraint::Min(limits.4 + 1),
            Constraint::Min(limits.5 + 3),
        ],
    )
    .header(header)
    .highlight_style(selected_style)
    .highlight_symbol(Text::from(vec!["".into(), bar.into(), "".into()]).style(header_style))
    .bg(app.colors.buffer_bg)
    .highlight_spacing(HighlightSpacing::Always);

    app.register_table.visible_width = f.size().width;
    if app.register_table.row_max_width <= f.size().width {
        f.render_stateful_widget(t, area, &mut app.register_table.table_state);
    } else {
        let f_rect = area;
        let rect = Rect {
            x: 0,
            y: 0,
            width: app.register_table.row_max_width,
            height: f_rect.height,
        };
        let mut buffer = Buffer::empty(rect);
        ratatui::widgets::StatefulWidget::render(
            t,
            rect,
            &mut buffer,
            &mut app.register_table.table_state,
        );
        let offset = std::cmp::min(
            app.register_table.row_max_width - f_rect.width,
            app.register_table.horizontal_scroll_offset,
        );
        let f_buffer = f.buffer_mut();
        for (x, y) in itertools::iproduct!(offset..(offset + f_rect.width), 0..(rect.height)) {
            f_buffer
                .get_mut(x - offset + f_rect.x, y + f_rect.y)
                .clone_from(buffer.get(x, y));
        }
    }
}

fn render_scrollbar<const SLICE_SIZE: usize>(
    f: &mut Frame,
    state: &mut ScrollbarState,
    area: Rect,
) {
    f.render_stateful_widget(
        Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        area.inner(&Margin {
            vertical: 1,
            horizontal: 0,
        }),
        state,
    );
}

fn render_register_footer<const SLICE_SIZE: usize>(
    f: &mut Frame,
    app: &App<SLICE_SIZE>,
    area: Rect,
    status: String,
) {
    let rects = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);
    let status_footer = Paragraph::new(Line::from(str!(" ") + &status))
        .style(
            Style::new()
                .fg(app.colors.header_fg)
                .bg(app.colors.header_bg),
        )
        .centered();
    let info_footer = Paragraph::new(Line::from(INFO_TEXT))
        .style(
            Style::new()
                .fg(app.colors.header_fg)
                .bg(app.colors.header_bg),
        )
        .centered();
    f.render_widget(status_footer, rects[0]);
    f.render_widget(info_footer, rects[1]);
}

fn render_log_footer<const SLICE_SIZE: usize>(f: &mut Frame, app: &App<SLICE_SIZE>, area: Rect) {
    let status_footer = Paragraph::new(Line::from(LOG_HEADER))
        .style(
            Style::new()
                .fg(app.colors.header_fg)
                .bg(app.colors.header_bg),
        )
        .left_aligned();
    f.render_widget(status_footer, area);
}

fn render_log<const SLICE_SIZE: usize>(f: &mut Frame, app: &mut App<SLICE_SIZE>, area: Rect) {
    let header_style = Style::default()
        .fg(app.colors.header_fg)
        .bg(app.colors.header_bg);
    let selected_style = Style::default()
        .add_modifier(Modifier::REVERSED)
        .fg(app.colors.selected_style_fg);

    if app.log_entries.len() > app.history_len {
        let len_to_remove = app.log_entries.len() - app.history_len;
        app.log_entries = app.log_entries[len_to_remove..].to_vec();
        match app.log_table.table_state.selected() {
            Some(i) if i < (app.log_entries.len() - 1) => {
                app.log_table
                    .table_state
                    .select(Some(std::cmp::max(i, len_to_remove) - len_to_remove));
            }
            _ => {}
        };
    } else {
    }
    app.log_table.vertical_scroll_state = ScrollbarState::new(app.log_entries.len());

    let items = app
        .log_entries
        .iter()
        .map(|item| match item {
            Ok(v) => (true, v),
            Err(v) => (false, v),
        })
        .collect::<Vec<_>>();
    let limits = items
        .iter()
        .fold(0, |acc, item| std::cmp::max(acc, item.1.width() as u16));

    app.log_table.row_max_width = limits;

    let rows = items.iter().enumerate().map(|(i, (is_ok, item))| {
        let color = match i % 2 {
            _ if !is_ok => app.colors.error_color,
            _ if !!is_ok => app.colors.success_color,
            0 => app.colors.normal_row_color,
            _ => app.colors.alt_row_color,
        };
        Row::new(Text::from((*item).clone()))
            .style(Style::new().fg(app.colors.row_fg).bg(color))
            .height(1)
    });

    let bar = " █ ";
    let t = Table::new(rows, [Constraint::Min(limits + 1)])
        .highlight_style(selected_style)
        .highlight_symbol(Text::from(bar).style(header_style))
        .bg(app.colors.buffer_bg)
        .highlight_spacing(HighlightSpacing::Always);

    app.log_table.visible_width = f.size().width;
    if app.log_table.row_max_width <= f.size().width {
        f.render_stateful_widget(t, area, &mut app.log_table.table_state);
    } else {
        let f_rect = area;
        let rect = Rect {
            x: 0,
            y: 0,
            width: app.log_table.row_max_width,
            height: f_rect.height,
        };
        let mut buffer = Buffer::empty(rect);
        ratatui::widgets::StatefulWidget::render(
            t,
            rect,
            &mut buffer,
            &mut app.log_table.table_state,
        );
        let offset = std::cmp::min(
            app.log_table.row_max_width - f_rect.width,
            app.log_table.horizontal_scroll_offset,
        );
        let f_buffer = f.buffer_mut();
        for (x, y) in itertools::iproduct!(offset..(offset + f_rect.width), 0..(rect.height)) {
            f_buffer
                .get_mut(x - offset + f_rect.x, y + f_rect.y)
                .clone_from(buffer.get(x, y));
        }
    }
}
