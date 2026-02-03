use ratatui::style::Color;
use ratatui::style::palette::tailwind;

pub const PALETTES: [tailwind::Palette; 4] = [
    tailwind::BLUE,
    tailwind::INDIGO,
    tailwind::EMERALD,
    tailwind::RED,
];

#[derive(Clone, Debug)]
pub struct RowColorPair {
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

pub struct TableColors {
    pub buffer: ColorPair<Color, Color>,
    pub header: ColorPair<Color, Color>,
    pub selected_color: ColorPair<Color, Color>,
    pub selected_color_error: ColorPair<Color, Color>,
    pub selected_color_success: ColorPair<Color, Color>,
    pub row_color: ColorPair<Color, RowColorPair>,
    pub row_error_color: ColorPair<Color, RowColorPair>,
    pub row_success_color: ColorPair<Color, RowColorPair>,
}

impl TableColors {
    pub const fn new(color: &tailwind::Palette) -> Self {
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
