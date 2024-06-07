use crate::mem::data::DataType;
use crate::mem::register::{AccessType, Definition};
use crate::mem::register::{Handler, Register};
use crate::util::str;
use crate::widgets::{EditDialog, EditFieldType};
use crate::{Command, Config, LogMsg, Status};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use itertools::Itertools;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::{prelude::*, widgets::*};
use std::io::stdout;
use std::sync::{Arc, Mutex};
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

const REGISTER_INFO_TEXT: &str =
    "(q)uit | (k) up | (j) down | (h) left | (l) right | (g) top | (G) bottom | (t)heme | (f)ormat | (d)isconnect | (c)onnect | (e)dit | (s)ave";
const LOGGER_INFO_TEXT: &str = "(m) up | (n) down | (b) left | (,) right | (v) top | (V) bottom";

const LOG_HEADER: &str = " Modbus Log";

const ITEM_HEIGHT: usize = 3;

enum LoopAction {
    Break,
    Continue,
}

#[derive(Clone, Debug)]
struct RowColorPair {
    pub normal: Color,
    pub alt: Color,
}

impl RowColorPair {
    pub const fn new(normal: Color, alt: Color) -> Self {
        Self { normal, alt }
    }

    pub fn get(&self, i: usize) -> Color {
        match i {
            0 => self.normal,
            1 => self.alt,
            _ => panic!("Invalid index."),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ColorPair<Forground: Clone, Background: Clone> {
    pub fg: Forground,
    pub bg: Background,
}

impl<Forground: Clone, Background: Clone> ColorPair<Forground, Background> {
    pub const fn new(fg: Forground, bg: Background) -> Self {
        Self { fg, bg }
    }
}

struct TableColors {
    buffer: ColorPair<Color, Color>,
    header: ColorPair<Color, Color>,
    selected_color: ColorPair<Color, Color>,
    selected_color_error: ColorPair<Color, Color>,
    selected_color_success: ColorPair<Color, Color>,
    row_color: ColorPair<Color, RowColorPair>,
    row_error_color: ColorPair<Color, RowColorPair>,
    row_success_color: ColorPair<Color, RowColorPair>,
}

impl TableColors {
    const fn new(color: &tailwind::Palette) -> Self {
        Self {
            buffer: ColorPair::new(tailwind::SLATE.c200, tailwind::SLATE.c950),
            header: ColorPair::new(tailwind::SLATE.c200, color.c900),
            selected_color: ColorPair::new(tailwind::SLATE.c950, color.c400),
            selected_color_error: ColorPair::new(tailwind::WHITE, tailwind::RED.c900),
            selected_color_success: ColorPair::new(tailwind::WHITE, tailwind::GREEN.c950),
            row_color: ColorPair::new(
                tailwind::SLATE.c200,
                RowColorPair::new(tailwind::SLATE.c950, tailwind::SLATE.c900),
            ),
            row_error_color: ColorPair::new(
                tailwind::SLATE.c200,
                RowColorPair::new(tailwind::RED.c950, tailwind::RED.c800),
            ),
            row_success_color: ColorPair::new(
                tailwind::SLATE.c200,
                RowColorPair::new(tailwind::GREEN.c900, tailwind::GREEN.c800),
            ),
        }
    }
}

pub struct UiTable {
    table_state: TableState,
    vertical_scroll: ScrollbarState,
    horizontal_scroll: u16,
    table_max_width: u16,
    table_visible_width: u16,
    reached_end_of_table: bool,
}

impl UiTable {
    pub fn new(len: usize, item_height: usize) -> Self {
        Self {
            table_state: TableState::default().with_selected(0),
            vertical_scroll: ScrollbarState::new((len - 1) * item_height),
            horizontal_scroll: 0,
            table_max_width: 0,
            table_visible_width: 0,
            reached_end_of_table: true,
        }
    }
}

pub enum Popup {
    None,
    Edit(Register),
}

pub struct App<const SLICE_SIZE: usize> {
    register_handler: Handler<SLICE_SIZE>,
    register_table: UiTable,
    log_entries: Vec<LogMsg>,
    log_table: UiTable,
    colors: TableColors,
    color_index: usize,
    show_as_hex: bool,
    config: Arc<Mutex<Config>>,
    popup: Popup,
    edit_dialog: EditDialog,
}

impl<const SLICE_SIZE: usize> App<SLICE_SIZE> {
    pub fn new(register_handler: Handler<SLICE_SIZE>, config: Arc<Mutex<Config>>) -> Self {
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            disable_raw_mode().unwrap();
            crossterm::execute!(stdout(), LeaveAlternateScreen).unwrap();
            original_hook(panic);
        }));

        let history_len = config.lock().unwrap().history_length;

        let len = register_handler.len();
        let colors = TableColors::new(&PALETTES[0]);
        let bg_color = colors.buffer.bg;
        Self {
            register_handler,
            register_table: UiTable::new(len, ITEM_HEIGHT),
            log_entries: Vec::new(),
            log_table: UiTable::new(history_len, 1),
            colors,
            color_index: 0,
            show_as_hex: true,
            config,
            popup: Popup::None,
            edit_dialog: EditDialog::new(PALETTES[0].c400, bg_color),
        }
    }

    pub fn log_move_bottom(&mut self) {
        let i = self.log_entries.len() - 1;
        self.log_table.reached_end_of_table = true;
        self.log_table.table_state.select(Some(i));
        self.log_table.vertical_scroll = self.log_table.vertical_scroll.position(i);
    }

    pub fn log_move_top(&mut self) {
        self.log_table.reached_end_of_table = false;
        self.log_table.table_state.select(Some(0));
        self.log_table.vertical_scroll = self.log_table.vertical_scroll.position(0);
    }

    pub fn log_move_down(&mut self) {
        if !self.log_entries.is_empty() {
            let i = self
                .log_table
                .table_state
                .selected()
                .map(|i| std::cmp::min(i + 1, std::cmp::max(self.log_entries.len(), 1) - 1))
                .unwrap_or(0);

            self.log_table.reached_end_of_table = i == (self.log_entries.len() - 1);
            self.log_table.table_state.select(Some(i));
            self.log_table.vertical_scroll = self.log_table.vertical_scroll.position(i);
        }
    }

    pub fn log_move_up(&mut self) {
        if !self.log_entries.is_empty() {
            let i = self
                .log_table
                .table_state
                .selected()
                .map(|i| std::cmp::max(i, 1) - 1)
                .unwrap_or(0);
            self.log_table.reached_end_of_table = i == (self.log_entries.len() - 1);
            self.log_table.table_state.select(Some(i));
            self.log_table.vertical_scroll = self.log_table.vertical_scroll.position(i);
        }
    }

    pub fn log_move_left(&mut self) {
        self.log_table.horizontal_scroll = std::cmp::max(3, self.log_table.horizontal_scroll) - 3;
    }

    pub fn log_move_right(&mut self) {
        self.log_table.horizontal_scroll = std::cmp::min(
            std::cmp::max(
                self.log_table.table_max_width,
                self.log_table.table_visible_width,
            ) - self.log_table.table_visible_width,
            self.log_table.horizontal_scroll + 3,
        );
    }

    pub fn move_bottom(&mut self) {
        let i = self
            .register_handler
            .values()
            .iter()
            .filter(|(n, _)| !n.starts_with("hide_"))
            .count()
            - 1;

        self.register_table.table_state.select(Some(i));
        self.register_table.vertical_scroll = self
            .register_table
            .vertical_scroll
            .position(i * ITEM_HEIGHT);
    }

    pub fn move_top(&mut self) {
        self.register_table.table_state.select(Some(0));
        self.register_table.vertical_scroll = self.register_table.vertical_scroll.position(0);
    }

    pub fn move_down(&mut self) {
        if !self.register_handler.values().is_empty() {
            let len = self
                .register_handler
                .values()
                .iter()
                .filter(|(n, _)| !n.starts_with("hide_"))
                .count();
            let i = self
                .register_table
                .table_state
                .selected()
                .map(|i| std::cmp::min(i + 1, std::cmp::max(len, 1) - 1))
                .unwrap_or(0);
            self.register_table.table_state.select(Some(i));
            self.register_table.vertical_scroll = self
                .register_table
                .vertical_scroll
                .position(i * ITEM_HEIGHT);
        }
    }

    pub fn move_up(&mut self) {
        if !self.register_handler.values().is_empty() {
            let i = self
                .register_table
                .table_state
                .selected()
                .map(|i| std::cmp::max(i, 1) - 1)
                .unwrap_or(0);
            self.register_table.table_state.select(Some(i));
            self.register_table.vertical_scroll = self
                .register_table
                .vertical_scroll
                .position(i * ITEM_HEIGHT);
        }
    }

    pub fn move_left(&mut self) {
        self.register_table.horizontal_scroll =
            std::cmp::max(3, self.register_table.horizontal_scroll) - 3;
    }

    pub fn move_right(&mut self) {
        self.register_table.horizontal_scroll = std::cmp::min(
            std::cmp::max(
                self.register_table.table_max_width,
                self.register_table.table_visible_width,
            ) - self.register_table.table_visible_width,
            self.register_table.horizontal_scroll + 3,
        );
    }

    pub fn switch_color(&mut self) {
        self.color_index = (self.color_index + 1) % PALETTES.len();
    }

    pub fn set_colors(&mut self) {
        self.colors = TableColors::new(&PALETTES[self.color_index]);
        self.edit_dialog
            .set_highlight_color(PALETTES[self.color_index].c400);
    }

    pub fn switch(&mut self) {
        self.show_as_hex = !self.show_as_hex;
    }

    fn handle_event(
        &mut self,
        key: KeyEvent,
        cmd_sender: &Option<Sender<Command>>,
    ) -> anyhow::Result<LoopAction> {
        match key.code {
            KeyCode::Char('w') => {
                // TODO: Dialog to add new definition
                self.config.lock().unwrap().definitions.insert(
                    format!("Def{}", self.log_entries.len()),
                    Definition::new(0x4000u16, 2, DataType::U8, 4, AccessType::ReadWrite),
                );
            }
            KeyCode::Char('s') => {
                // TODO: Dialog to specify output path
                let config = self.config.lock().unwrap();
                let f = std::fs::File::create_new("./test.json")?;
                let writer = std::io::BufWriter::new(f);
                let _ = serde_json::to_writer_pretty::<_, Config>(writer, &config);
            }
            KeyCode::Char('q') | KeyCode::Esc => return Ok(LoopAction::Break),
            KeyCode::Char('j') | KeyCode::Down => self.move_down(),
            KeyCode::Char('k') | KeyCode::Up => self.move_up(),
            KeyCode::Char('h') | KeyCode::Left => self.move_left(),
            KeyCode::Char('l') | KeyCode::Right => self.move_right(),
            KeyCode::Char('f') | KeyCode::Tab => self.switch(),
            KeyCode::Char('t') => self.switch_color(),
            KeyCode::Char('d') => {
                if let Some(ref sender) = cmd_sender {
                    sender.blocking_send(Command::Disconnect)?
                }
            }
            KeyCode::Char('g') => self.move_top(),
            KeyCode::Char('G') => self.move_bottom(),
            KeyCode::Char('c') => {
                if let Some(ref sender) = cmd_sender {
                    sender.blocking_send(Command::Connect)?
                }
            }
            KeyCode::PageUp | KeyCode::Char('m') => self.log_move_up(),
            KeyCode::PageDown | KeyCode::Char('n') => self.log_move_down(),
            KeyCode::Home | KeyCode::Char('b') => self.log_move_left(),
            KeyCode::End | KeyCode::Char(',') => self.log_move_right(),
            KeyCode::Char('v') => self.log_move_top(),
            KeyCode::Char('V') => self.log_move_bottom(),
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(i) = self.register_table.table_state.selected() {
                    let entry = self
                        .register_handler
                        .values()
                        .iter()
                        .filter(|(n, _)| !n.starts_with("hide_"))
                        .sorted_by(|a, b| {
                            Ord::cmp(&a.1.address(), &b.1.address()).then(a.0.cmp(b.0))
                        })
                        .enumerate()
                        .filter_map(|(j, (name, r))| {
                            if j == i {
                                Some((name.clone(), (*r).clone()))
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<(String, Register)>>();
                    if !entry.is_empty() {
                        let entry = entry.first().unwrap();
                        self.log_entries.push(LogMsg::info(&format!(
                            "Start edit of register {entry:#06X} ({entry})",
                            entry = entry.1.address()
                        )));
                        self.edit_dialog
                            .set(EditFieldType::Name, Some(entry.0.clone()), None);
                        self.edit_dialog.set(
                            EditFieldType::Register,
                            Some(format!("{a:#06X} ({a})", a = entry.1.address())),
                            None,
                        );
                        self.edit_dialog.set(
                            EditFieldType::DataType,
                            Some(format!("{:?}", entry.1.r#type())),
                            None,
                        );
                        self.edit_dialog.set(
                            EditFieldType::Value,
                            None,
                            Some(entry.1.value().clone()),
                        );
                        self.edit_dialog.focus();
                        self.popup = Popup::Edit(entry.1.clone());
                    }
                }
            }
            _ => {}
        };
        Ok(LoopAction::Continue)
    }

    fn handle_event_edit_dialog(&mut self, key: KeyEvent, cmd_sender: &Option<Sender<Command>>) {
        match key.code {
            KeyCode::Enter => {
                match self.popup {
                    Popup::Edit(ref register) => {
                        if let Some(input) = self.edit_dialog.get_input(EditFieldType::Value) {
                            match register.r#type().encode(&input) {
                                Ok(v) => {
                                    if v.len() > register.raw().len() {
                                        self.log_entries.push(LogMsg::err(
                                                                "Provided input requires a longer register as available.",
                                                            ));
                                    } else if let Some(ref sender) = cmd_sender {
                                        if let Err(e) =
                                            sender.blocking_send(Command::WriteMultipleRegisters((
                                                register.address(),
                                                v,
                                            )))
                                        {
                                            self.log_entries.push(LogMsg::err(&format!("{}", e)));
                                        } else {
                                            self.popup = Popup::None;
                                        }
                                    } else if let Err(e) =
                                        self.register_handler.set_values(register.address(), &v)
                                    {
                                        self.log_entries.push(LogMsg::err(&format!("{}", e)));
                                    } else {
                                        self.popup = Popup::None;
                                    }
                                }
                                Err(e) => {
                                    self.log_entries.push(LogMsg::err(&format!("{}", e)));
                                }
                            }
                        }
                    }
                    Popup::None => panic!("No popup value."),
                };
            }
            KeyCode::Esc => {
                self.popup = Popup::None;
            }
            _ => {
                self.edit_dialog.handle_events(key.modifiers, key.code);
                self.edit_dialog.focus();
            }
        }
    }

    pub fn run(
        mut self,
        mut status_recv: Receiver<Status>,
        mut log_recv: Receiver<LogMsg>,
        cmd_sender: Option<Sender<Command>>,
    ) -> anyhow::Result<()> {
        enable_raw_mode()?;
        let mut terminal = App::<SLICE_SIZE>::create_terminal()?;
        execute!(terminal.backend_mut(), DisableMouseCapture)?;

        let mut status = str!("");
        loop {
            // Update status
            if let Ok(v) = status_recv.try_recv() {
                match v {
                    Status::String(v) => {
                        status = v;
                    }
                }
            }

            // Update log
            for _ in 0..5 {
                if let Ok(v) = log_recv.try_recv() {
                    self.log_entries.push(v);
                } else {
                    break;
                }
            }

            //self.register_handler.update()?;
            terminal.draw(|f| ui(f, &mut self, status.clone()))?;

            // Handle inputs
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match self.popup {
                            Popup::None => match self.handle_event(key, &cmd_sender) {
                                Ok(LoopAction::Continue) => {}
                                Ok(LoopAction::Break) => break,
                                Err(e) => return Err(e),
                            },
                            Popup::Edit(_) => self.handle_event_edit_dialog(key, &cmd_sender),
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
        Constraint::Max(10),
        Constraint::Length(2),
    ])
    .split(f.size());
    app.set_colors();
    // Draw register table
    render_register::<SLICE_SIZE>(f, app, rects[0]);
    render_scrollbar::<SLICE_SIZE>(f, &mut app.register_table.vertical_scroll, rects[0]);
    render_register_footer::<SLICE_SIZE>(f, app, rects[1], status);
    // Draw log table
    render_log::<SLICE_SIZE>(f, app, rects[2]);
    render_scrollbar::<SLICE_SIZE>(f, &mut app.log_table.vertical_scroll, rects[2]);
    render_log_footer::<SLICE_SIZE>(f, app, rects[3]);

    // Render popup
    if let Popup::Edit(_) = app.popup {
        app.edit_dialog.render_ref(f.size(), f.buffer_mut())
    }
}

fn render_register<const SLICE_SIZE: usize>(f: &mut Frame, app: &mut App<SLICE_SIZE>, area: Rect) {
    let header_style = Style::default()
        .fg(app.colors.header.fg)
        .bg(app.colors.header.bg);
    let selected_style = Style::default()
        .fg(app.colors.selected_color.fg)
        .bg(app.colors.selected_color.bg);
    let bar_style = Style::default()
        .fg(app.colors.header.bg)
        .bg(app.colors.selected_color.bg);

    let cols = [
        "Access", "Name", "Address", "Type", "Length", "Value", "Raw Data",
    ];
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
        .filter(|(n, _)| !n.starts_with("hide_"))
        .sorted_by(|a, b| Ord::cmp(&a.1.address(), &b.1.address()).then(a.0.cmp(b.0)))
        .map(|(n, r)| {
            [
                format!("{}", r.access_type()),
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
                if app.show_as_hex {
                    format!("[ {:#06X} ]", r.raw().iter().format(", "))
                } else {
                    format!("{:?}", r.raw())
                },
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
            cols[6].width() as u16,
        ),
        |acc, item| {
            (
                std::cmp::max(acc.0, item[0].width() as u16),
                std::cmp::max(acc.1, item[1].width() as u16),
                std::cmp::max(acc.2, item[2].width() as u16),
                std::cmp::max(acc.3, item[3].width() as u16),
                std::cmp::max(acc.4, item[4].width() as u16),
                std::cmp::max(acc.5, item[5].width() as u16),
                std::cmp::max(acc.6, item[6].width() as u16),
            )
        },
    );

    app.register_table.table_max_width =
        limits.0 + limits.1 + limits.2 + limits.3 + limits.4 + limits.5 + limits.6 + 25;

    let rows = items.iter().enumerate().map(|(i, item)| {
        let color = app.colors.row_color.bg.get(i % 2);
        item.iter()
            .map(|content| Cell::from(Text::from(format!("\n{content}\n"))))
            .collect::<Row>()
            .style(Style::new().fg(app.colors.row_color.fg).bg(color))
            .height(ITEM_HEIGHT as u16)
    });

    let bar = " █ ";
    let t = Table::new(
        rows,
        [
            Constraint::Min(limits.0 + 1),
            Constraint::Min(limits.1 + 1),
            Constraint::Min(limits.2 + 1),
            Constraint::Min(limits.3 + 1),
            Constraint::Min(limits.4 + 1),
            Constraint::Min(limits.5 + 1),
            Constraint::Min(limits.6 + 3),
        ],
    )
    .header(header)
    .highlight_style(selected_style)
    .highlight_symbol(Text::from(vec!["".into(), bar.into(), "".into()]).style(bar_style))
    .bg(app.colors.buffer.bg)
    .highlight_spacing(HighlightSpacing::Always);

    app.register_table.table_visible_width = f.size().width;
    if app.register_table.table_max_width <= f.size().width {
        f.render_stateful_widget(t, area, &mut app.register_table.table_state);
    } else {
        let f_rect = area;
        let rect = Rect {
            x: 0,
            y: 0,
            width: app.register_table.table_max_width,
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
            app.register_table.table_max_width - f_rect.width,
            app.register_table.horizontal_scroll,
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
    let rects = Layout::vertical([Constraint::Length(1), Constraint::Length(2)]).split(area);
    let status_footer = Paragraph::new(Line::from(str!(" ") + &status))
        .style(
            Style::new()
                .fg(app.colors.header.fg)
                .bg(app.colors.header.bg),
        )
        .centered();
    let info_footer = Paragraph::new(Line::from(REGISTER_INFO_TEXT))
        .style(Style::new().fg(tailwind::WHITE).bg(tailwind::SLATE.c900))
        .centered();
    f.render_widget(status_footer, rects[0]);
    f.render_widget(info_footer, rects[1]);
}

fn render_log_footer<const SLICE_SIZE: usize>(f: &mut Frame, app: &App<SLICE_SIZE>, area: Rect) {
    let rects = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);
    let status_footer = Paragraph::new(Line::from(LOG_HEADER))
        .style(
            Style::new()
                .fg(app.colors.header.fg)
                .bg(app.colors.header.bg),
        )
        .left_aligned();
    f.render_widget(status_footer, rects[0]);

    let info_footer = Paragraph::new(Line::from(LOGGER_INFO_TEXT))
        .style(Style::new().fg(tailwind::WHITE).bg(tailwind::SLATE.c900))
        .centered();
    f.render_widget(info_footer, rects[1]);
}

fn render_log<const SLICE_SIZE: usize>(f: &mut Frame, app: &mut App<SLICE_SIZE>, area: Rect) {
    let header_style = Style::default()
        .fg(app.colors.header.fg)
        .bg(app.colors.header.bg);
    let selected_style = Style::default()
        .fg(app.colors.selected_color.fg)
        .bg(app.colors.selected_color.bg);
    let selected_style_error = Style::default()
        .fg(app.colors.selected_color_error.fg)
        .bg(app.colors.selected_color_error.bg);
    let selected_style_success = Style::default()
        .fg(app.colors.selected_color_success.fg)
        .bg(app.colors.selected_color_success.bg);

    let cols = ["Timestamp", "Message"];
    let header = cols
        .into_iter()
        .map(Cell::from)
        .collect::<Row>()
        .style(header_style)
        .height(1);

    let history_len = app.config.lock().unwrap().history_length;
    if app.log_entries.len() > history_len {
        let len_to_remove = app.log_entries.len() - history_len;
        app.log_entries = app.log_entries[len_to_remove..].to_vec();
        if !app.log_table.reached_end_of_table {
            if let Some(i) = app.log_table.table_state.selected() {
                app.log_table
                    .table_state
                    .select(Some(std::cmp::max(i, len_to_remove) - len_to_remove));
                app.log_table.vertical_scroll = app
                    .log_table
                    .vertical_scroll
                    .position(std::cmp::max(i, len_to_remove) - len_to_remove);
            }
        }
    } else if app.log_table.reached_end_of_table && !app.log_entries.is_empty() {
        app.log_table
            .table_state
            .select(Some(app.log_entries.len() - 1));
        app.log_table.vertical_scroll = app
            .log_table
            .vertical_scroll
            .position(app.log_entries.len() - 1);
    }

    let limits = (
        LogMsg::info("").timestamp().width() as u16,
        app.log_entries.iter().fold(0, |acc, item| match item {
            LogMsg::Err(v) => std::cmp::max(acc, v.message.width() as u16),
            LogMsg::Info(v) => std::cmp::max(acc, v.message.width() as u16),
            LogMsg::Ok(v) => std::cmp::max(acc, v.message.width() as u16),
        }),
    );

    let selected_style = match app
        .log_entries
        .get(app.log_table.table_state.selected().unwrap_or(0))
        .unwrap_or(&LogMsg::info(""))
    {
        LogMsg::Info(_) => selected_style,
        LogMsg::Err(_) => selected_style_error,
        LogMsg::Ok(_) => selected_style_success,
    };

    app.log_table.table_max_width = limits.0 + limits.1 + 10;

    let rows = app.log_entries.iter().enumerate().map(|(i, item)| {
        let (item, fg, bg) = match item {
            LogMsg::Info(v) => (
                [&v.timestamp, &v.message],
                app.colors.row_error_color.fg,
                app.colors.row_color.bg.get(i % 2),
            ),
            LogMsg::Err(v) => (
                [&v.timestamp, &v.message],
                app.colors.row_error_color.fg,
                app.colors.row_error_color.bg.get(i % 2),
            ),
            LogMsg::Ok(v) => (
                [&v.timestamp, &v.message],
                app.colors.row_error_color.fg,
                app.colors.row_success_color.bg.get(i % 2),
            ),
        };
        item.iter()
            .map(|content| Cell::from(Text::from(str!(*content))))
            .collect::<Row>()
            .style(Style::new().fg(fg).bg(bg))
            .height(1)
    });

    let bar = " █ ";
    let t = Table::new(
        rows,
        [Constraint::Max(limits.0 + 1), Constraint::Min(limits.1 + 1)],
    )
    .header(header)
    .highlight_style(selected_style)
    .highlight_symbol(Text::from(bar).style(header_style))
    .fg(app.colors.buffer.fg)
    .bg(app.colors.buffer.bg)
    .highlight_spacing(HighlightSpacing::Always);

    app.log_table.table_visible_width = f.size().width;
    if app.log_table.table_max_width <= f.size().width {
        f.render_stateful_widget(t, area, &mut app.log_table.table_state);
    } else {
        let f_rect = area;
        let rect = Rect {
            x: 0,
            y: 0,
            width: app.log_table.table_max_width,
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
            app.log_table.table_max_width - f_rect.width,
            app.log_table.horizontal_scroll,
        );
        let f_buffer = f.buffer_mut();
        for (x, y) in itertools::iproduct!(offset..(offset + f_rect.width), 0..(rect.height)) {
            f_buffer
                .get_mut(x - offset + f_rect.x, y + f_rect.y)
                .clone_from(buffer.get(x, y));
        }
    }
}
