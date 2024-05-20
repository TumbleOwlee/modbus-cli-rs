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
    tailwind::EMERALD,
    tailwind::INDIGO,
    tailwind::RED,
];

const INFO_TEXT: &str =
    "(q) quit | (k) up | (j) down | (h) left | (l) right | (t) color | (n) hex/dec | (d) disconnect | (c) connect";

const ITEM_HEIGHT: usize = 3;

struct TableColors {
    buffer_bg: Color,
    header_bg: Color,
    header_fg: Color,
    row_fg: Color,
    selected_style_fg: Color,
    normal_row_color: Color,
    alt_row_color: Color,
    footer_border_color: Color,
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
            footer_border_color: color.c400,
        }
    }
}

pub struct App<'a, const SLICE_SIZE: usize> {
    register_handler: Handler<'a, SLICE_SIZE>,
    state: TableState,
    vertical_scroll_state: ScrollbarState,
    horizontal_scroll: u16,
    total_width: u16,
    visible_width: u16,
    colors: TableColors,
    color_index: usize,
    show_as_hex: bool,
}

impl<'a, const SLICE_SIZE: usize> App<'a, SLICE_SIZE> {
    pub fn new(register_handler: Handler<'a, SLICE_SIZE>) -> Self {
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            disable_raw_mode().unwrap();
            crossterm::execute!(stdout(), LeaveAlternateScreen).unwrap();
            original_hook(panic);
        }));

        let len = register_handler.len();
        Self {
            register_handler,
            state: TableState::default().with_selected(0),
            vertical_scroll_state: ScrollbarState::new((len - 1) * ITEM_HEIGHT),
            horizontal_scroll: 0,
            total_width: 0,
            visible_width: 0,
            colors: TableColors::new(&PALETTES[0]),
            color_index: 0,
            show_as_hex: true,
        }
    }

    pub fn move_down(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.register_handler.values().len() - 1 {
                    self.register_handler.values().len() - 1
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
        self.vertical_scroll_state = self.vertical_scroll_state.position(i * ITEM_HEIGHT);
    }

    pub fn move_up(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    0
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
        self.vertical_scroll_state = self.vertical_scroll_state.position(i * ITEM_HEIGHT);
    }

    pub fn move_left(&mut self) {
        self.horizontal_scroll = std::cmp::max(3, self.horizontal_scroll) - 3;
    }

    pub fn move_right(&mut self) {
        self.horizontal_scroll = std::cmp::min(
            std::cmp::max(self.total_width, self.visible_width) - self.visible_width,
            self.horizontal_scroll + 3,
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
    let rects = Layout::vertical([Constraint::Min(5), Constraint::Length(3)]).split(f.size());
    app.set_colors();
    render_table::<SLICE_SIZE>(f, app, rects[0]);
    render_scrollbar::<SLICE_SIZE>(f, app, rects[0]);
    render_footer::<SLICE_SIZE>(f, app, rects[1], status);
}

fn vec_as_str(v: &[u16], hex: bool) -> String {
    if hex {
        format!("[ {:#06X} ]", v.iter().format(", "))
    } else {
        format!("{:?}", v)
    }
}

fn render_table<const SLICE_SIZE: usize>(f: &mut Frame, app: &mut App<SLICE_SIZE>, area: Rect) {
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

    app.total_width = limits.0 + limits.1 + limits.2 + limits.3 + limits.4 + limits.5 + 25;

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
    .highlight_symbol(Text::from(vec!["".into(), bar.into(), "".into()]))
    .bg(app.colors.buffer_bg)
    .highlight_spacing(HighlightSpacing::Always);

    app.visible_width = f.size().width;
    if app.total_width <= f.size().width {
        f.render_stateful_widget(t, area, &mut app.state);
    } else {
        let f_rect = area;
        let rect = Rect {
            x: 0,
            y: 0,
            width: app.total_width,
            height: f_rect.height,
        };
        let mut buffer = Buffer::empty(rect);
        ratatui::widgets::StatefulWidget::render(t, rect, &mut buffer, &mut app.state);
        let offset = std::cmp::min(app.total_width - f_rect.width, app.horizontal_scroll);
        let f_buffer = f.buffer_mut();
        for (x, y) in itertools::iproduct!(offset..(offset + f_rect.width), 0..(rect.height)) {
            f_buffer
                .get_mut(x - offset + f_rect.x, y + f_rect.y)
                .clone_from(buffer.get(x, y));
        }
    }
}

fn render_scrollbar<const SLICE_SIZE: usize>(f: &mut Frame, app: &mut App<SLICE_SIZE>, area: Rect) {
    f.render_stateful_widget(
        Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        area.inner(&Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut app.vertical_scroll_state,
    );
}

fn render_footer<const SLICE_SIZE: usize>(
    f: &mut Frame,
    app: &App<SLICE_SIZE>,
    area: Rect,
    status: String,
) {
    let rects = Layout::horizontal([
        Constraint::Length(status.len() as u16 + 3),
        Constraint::Min(1),
        Constraint::Length(status.len() as u16 + 3),
    ])
    .split(area);
    let status_footer = Paragraph::new(Line::from(str!(" ") + &status))
        .style(Style::new().fg(app.colors.row_fg).bg(app.colors.buffer_bg))
        .left_aligned()
        .block(
            Block::default()
                .borders(Borders::LEFT | Borders::TOP | Borders::BOTTOM)
                .border_type(BorderType::Plain)
                .border_style(Style::new().fg(app.colors.footer_border_color)),
        );
    let info_footer = Paragraph::new(Line::from(INFO_TEXT))
        .style(Style::new().fg(app.colors.row_fg).bg(app.colors.buffer_bg))
        .centered()
        .block(
            Block::default()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_type(BorderType::Plain)
                .border_style(Style::new().fg(app.colors.footer_border_color)),
        );
    let add_footer = Paragraph::new(Line::from(""))
        .style(Style::new().fg(app.colors.row_fg).bg(app.colors.buffer_bg))
        .right_aligned()
        .block(
            Block::default()
                .borders(Borders::RIGHT | Borders::TOP | Borders::BOTTOM)
                .border_type(BorderType::Plain)
                .border_style(Style::new().fg(app.colors.footer_border_color)),
        );
    f.render_widget(status_footer, rects[0]);
    f.render_widget(info_footer, rects[1]);
    f.render_widget(add_footer, rects[2]);
}
