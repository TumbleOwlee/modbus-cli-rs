use crate::util::str;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::palette::tailwind;
use ratatui::style::Style;
use ratatui::text::Text;
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

pub enum Action {
    InputTaken,
    FocusNext,
    FocusPrevious,
    InputConfirm,
    InputIgnored,
}

pub struct InputField {
    input: Option<String>,
    placeholder: Option<String>,
    bordered: bool,
    style: InputStyle,
    title: Option<String>,
    margins: Margin,
    cursor_pos: usize,
    selected: bool,
    disabled: bool,
}

pub struct InputStyle {
    default: Style,
    selected: Style,
    cursor: Style,
}

impl Default for InputStyle {
    fn default() -> Self {
        InputStyle {
            default: Style::default()
                .fg(tailwind::WHITE)
                .bg(tailwind::SLATE.c950),
            selected: Style::default()
                .fg(tailwind::INDIGO.c400)
                .bg(tailwind::SLATE.c950),
            cursor: Style::default()
                .fg(tailwind::WHITE)
                .bg(tailwind::INDIGO.c600),
        }
    }
}

impl InputField {
    pub fn new() -> Self {
        Self {
            input: None,
            placeholder: None,
            bordered: false,
            style: InputStyle::default(),
            title: None,
            margins: Margin {
                vertical: 0,
                horizontal: 0,
            },
            cursor_pos: 0,
            selected: false,
            disabled: false,
        }
    }

    pub fn select(&mut self) {
        if self.disabled {
            panic!("Tried to select disabled input field.");
        }
        self.selected = true;
    }

    pub fn disabled(self) -> Self {
        Self {
            disabled: true,
            ..self
        }
    }

    pub fn disable(&mut self) {
        self.disabled = true;
    }

    pub fn enable(&mut self) {
        self.disabled = false;
    }

    pub fn input(self, input: String) -> Self {
        Self {
            input: Some(input),
            ..self
        }
    }

    pub fn title(self, title: String) -> Self {
        Self {
            title: Some(title),
            ..self
        }
    }

    pub fn placeholder(self, placeholder: String) -> Self {
        Self {
            placeholder: Some(placeholder),
            ..self
        }
    }

    pub fn bordered(self, bordered: bool) -> Self {
        Self { bordered, ..self }
    }

    pub fn style(self, style: InputStyle) -> Self {
        Self { style, ..self }
    }

    pub fn margins(self, margins: Margin) -> Self {
        Self { margins, ..self }
    }

    pub fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> Option<Action> {
        if self.disabled {
            return Some(Action::InputIgnored);
        }

        match (modifiers, code) {
            (_, KeyCode::Home) => {
                self.cursor_pos = 0;
                Some(Action::InputTaken)
            }
            (_, KeyCode::End) => {
                if let Some(input) = self.input.as_ref() {
                    self.cursor_pos = input.len();
                }
                Some(Action::InputTaken)
            }
            (_, KeyCode::Char(c)) => {
                let mut input = self.input.clone().unwrap_or(str!(""));
                input.insert(self.cursor_pos, c);
                self.input = Some(input);
                self.cursor_pos += 1;
                Some(Action::InputTaken)
            }
            (_, KeyCode::Backspace) => {
                if self.input.is_some() && self.cursor_pos > 0 {
                    if let Some(input) = self.input.as_mut() {
                        input.remove(self.cursor_pos - 1);
                        if input.is_empty() {
                            self.input = None;
                        }
                    }
                    self.cursor_pos -= 1;
                }
                Some(Action::InputTaken)
            }
            (_, KeyCode::Delete) => {
                if self.input.is_some() && self.cursor_pos < self.input.as_ref().unwrap().len() {
                    if let Some(input) = self.input.as_mut() {
                        input.remove(self.cursor_pos);
                        if input.is_empty() {
                            self.input = None;
                        }
                    }
                }
                Some(Action::InputTaken)
            }
            (_, KeyCode::Left) => {
                if self.input.is_some() && self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                }
                Some(Action::InputTaken)
            }
            (_, KeyCode::Right) => {
                let len = self.input.as_ref().map(|i| i.len()).unwrap_or(0);
                if self.input.is_some() && len > self.cursor_pos {
                    self.cursor_pos += 1;
                }
                Some(Action::InputTaken)
            }
            (KeyModifiers::SHIFT, KeyCode::Tab) => {
                self.selected = false;
                Some(Action::FocusPrevious)
            }
            (_, KeyCode::Tab) => {
                self.selected = false;
                Some(Action::FocusNext)
            }
            (_, KeyCode::Enter) => Some(Action::InputConfirm),
            _ => None,
        }
    }

    pub fn get_input(&self) -> Option<String> {
        self.input.clone()
    }

    pub fn set_input(&mut self, input: String) {
        self.cursor_pos = input.len();
        self.input = Some(input);
    }

    pub fn clear_input(&mut self) {
        self.input = None;
        self.cursor_pos = 0;
    }

    pub fn set_placeholder(&mut self, input: String) {
        self.placeholder = Some(input);
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let height = if self.bordered { 3 } else { 1 };

        let area = Layout::vertical([
            Constraint::Length(self.margins.vertical),
            Constraint::Length(height),
            Constraint::Length(self.margins.vertical),
        ])
        .split(area)[1];

        let mut area = Layout::horizontal([
            Constraint::Length(self.margins.horizontal),
            Constraint::Min(1),
            Constraint::Length(self.margins.horizontal),
        ])
        .split(area)[1];

        // Create block if border is required
        if self.bordered {
            let style = if self.selected && !self.disabled {
                self.style.selected
            } else {
                self.style.default
            };
            let mut block = Block::bordered().style(style);
            if let Some(title) = self.title.as_ref() {
                block = block.title(title.clone());
            }
            let inner = block.inner(area);
            f.render_widget(block, area);
            area = inner.inner(&Margin {
                vertical: 0,
                horizontal: 1,
            });
        }

        let mut text = self
            .input
            .as_ref()
            .map(|i| format!("{} ", i))
            .unwrap_or(self.placeholder.clone().unwrap_or(str!(" ")).clone());

        let mut x_start = 0;

        // Calculate range of text to display
        if (area.width as usize) < text.len() {
            let width = (area.width / 2) as usize;
            // Display width characters left of cursor
            x_start = std::cmp::max(self.cursor_pos, width) - width;
            // Display width characters right of cursor
            let mut x_end = std::cmp::min(self.cursor_pos + width, text.len());
            // Add more characters to the left, if right of cursor are not enough
            if (x_end - self.cursor_pos) < (area.width as usize - width) {
                let remaining = (area.width as usize - width) - (x_end - self.cursor_pos);
                x_start = std::cmp::max(x_start, remaining) - remaining;
            }
            // Add more characters to the right, if left of cursor are not enough
            if (self.cursor_pos - x_start) < width {
                let remaining = width - (self.cursor_pos - x_start);
                x_end = std::cmp::min(text.len(), x_end + remaining);
            }
            // Get displayable text area
            text = text[x_start..x_end].to_owned();
        }

        // Display text
        let input = Paragraph::new(Text::from(text).style(self.style.default));
        f.render_widget(input, area);
        if !self.disabled {
            // Display cursor
            if self.selected {
                f.buffer_mut()
                    .get_mut(area.x + (self.cursor_pos - x_start) as u16, area.y)
                    .set_style(self.style.cursor);
            }
        }
    }
}
