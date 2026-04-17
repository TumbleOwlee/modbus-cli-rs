use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters, WithSetters};

use crate::style::TextStyle;
use crate::traits::Margins;
use crate::types::Border;
use crate::widgets::Title;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, HorizontalAlignment, Layout, Margin, Rect};
use ratatui::text::Text as UiText;
use ratatui::widgets::Widget;
use ratatui::widgets::{Block, Paragraph, StatefulWidget};

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters, WithSetters)]
#[getset(set = "pub")]
pub struct Text {
    #[getset(get = "pub")]
    #[builder(default = "Border::None")]
    border: Border,
    #[getset(get = "pub")]
    #[builder(default = "TextStyle::default()")]
    style: TextStyle,
    #[getset(get = "pub")]
    #[builder(default = "None")]
    title: Option<Title>,
    #[getset(get = "pub")]
    #[builder(default = "Margin::default()")]
    margin: Margin,
    #[getset(get = "pub")]
    #[builder(default = "false")]
    multiline: bool,
    #[getset(get = "pub")]
    #[builder(default = "HorizontalAlignment::Left")]
    horizontal_alignment: HorizontalAlignment,
}

impl StatefulWidget for Text {
    type State = String;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self, area, buf, state);
    }
}

impl StatefulWidget for &Text {
    type State = String;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        buf.set_style(area, self.style.general);

        let mut height = if let Border::Full(margin) = &self.border {
            2 + margin.vertical * 2
        } else {
            0
        };
        if self.multiline {
            height += std::cmp::max(1, area.height);
        } else {
            height += 1;
        }

        let area = Layout::vertical([
            Constraint::Length(self.margin.vertical),
            Constraint::Length(height),
            Constraint::Length(self.margin.vertical),
        ])
        .split(area)[1];

        let mut area = Layout::horizontal([
            Constraint::Length(self.margin.horizontal),
            Constraint::Min(1),
            Constraint::Length(self.margin.horizontal),
        ])
        .split(area)[1];

        // Create block if border is required
        if let Border::Full(margin) = &self.border {
            let style = self.style.general;
            let mut block = Block::bordered().style(style);
            if let Some(title) = self.title.as_ref() {
                block = block
                    .title(title.name.as_str())
                    .title_alignment(title.alignment);
            }
            let inner = block.inner(area);
            block.render(area, buf);
            area = inner.inner(margin.clone());
        }

        let area = match self.horizontal_alignment {
            HorizontalAlignment::Left => area,
            HorizontalAlignment::Center => Layout::horizontal([
                Constraint::Min(1),
                Constraint::Length(state.len() as u16),
                Constraint::Min(1),
            ])
            .split(area)[1],
            HorizontalAlignment::Right => {
                Layout::horizontal([Constraint::Min(1), Constraint::Length(state.len() as u16)])
                    .split(area)[1]
            }
        };

        let text_style = self.style.general;
        let input = Paragraph::new(UiText::from(state.as_str()).style(text_style));
        input.render(area, buf);
    }
}

impl Margins for Text {
    fn margins(&self) -> Margin {
        let horizontal = if let Border::Full(margin) = &self.border {
            4 + margin.horizontal * 2
        } else {
            0
        } + 2 * self.margin.horizontal
            + 1;
        let vertical = if let Border::Full(margin) = &self.border {
            2 + margin.vertical * 2
        } else if self.title.is_some() {
            1
        } else {
            0
        } + self.margin.vertical;
        Margin {
            horizontal,
            vertical,
        }
    }
}
