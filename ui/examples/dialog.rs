use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use derive_builder::Builder;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::palette::tailwind,
};
use std::{fmt::Debug, io::Stdout, time::Duration};

use ui::{
    AlternateScreen, EventResult,
    state::{InputFieldState, InputFieldStateBuilder, SelectionState, SelectionStateBuilder},
    style::{InputFieldStyle, SelectionStyle},
    traits::{AsConstraint, HandleEvents, SetFocus},
    widgets::Widget,
    widgets::{InputField, InputFieldBuilder, Selection, SelectionBuilder, Validate},
};

use derive_focus::{Focus, focusable};

#[derive(Debug, Clone)]
struct Day {}

impl Validate for Day {
    fn validate(input: &str) -> Result<(), String> {
        let day = input.parse::<usize>();
        if let Ok(day) = day {
            if day < 32 {
                Ok(())
            } else {
                Err("Invalid day input".into())
            }
        } else {
            Err("Input is not numerical".into())
        }
    }
}

#[derive(Debug, Clone)]
struct Year {}

impl Validate for Year {
    fn validate(input: &str) -> Result<(), String> {
        let year = input.parse::<usize>();
        if year.is_ok() {
            Ok(())
        } else {
            Err("Input is not numerical".into())
        }
    }
}

#[derive(Debug, Clone)]
struct Code {}

impl Validate for Code {
    fn validate(input: &str) -> Result<(), String> {
        let code = input.parse::<usize>();
        if let Ok(code) = code {
            if code >= 10000 && code <= 99999 {
                Ok(())
            } else {
                Err("Invalid postal code".into())
            }
        } else {
            Err("Input is not numerical".into())
        }
    }
}

#[focusable]
#[derive(Builder, Debug, Focus)]
struct App {
    #[focus]
    pub name: Widget<InputFieldState, InputField<String>>,
    #[focus]
    pub lastname: Widget<InputFieldState, InputField<String>>,
    #[focus]
    pub day: Widget<InputFieldState, InputField<Day>>,
    #[focus]
    pub month: Widget<SelectionState<String>, Selection<String>>,
    #[focus]
    pub year: Widget<InputFieldState, InputField<Year>>,
    #[focus]
    pub street: Widget<InputFieldState, InputField<String>>,
    #[focus]
    pub code: Widget<InputFieldState, InputField<Code>>,
    #[focus]
    pub city: Widget<InputFieldState, InputField<String>>,
    pub error: Widget<InputFieldState, InputField<String>>,
}

#[derive(Debug)]
#[allow(dead_code)]
struct Person {
    name: String,
    birthday: String,
    address: String,
}

impl App {
    fn result(&self) -> Result<Person, String> {
        if let Err(_) = Day::validate(self.day.state.input()) {
            Err("Invalid input for day".into())
        } else if let Err(_) = Year::validate(self.year.state.input()) {
            Err("Invalid input for year".into())
        } else if let Err(_) = Code::validate(self.code.state.input()) {
            Err("Invalid input for postal code".into())
        } else {
            let name = format!(
                "{} {}",
                self.name.state.input(),
                self.lastname.state.input()
            );
            let birthday = format!(
                "{}.{}.{}",
                self.day.state.input(),
                self.month.state.values()[self.month.state.selection()],
                self.year.state.input()
            );
            let address = format!(
                "{}, {} {}",
                self.street.state.input(),
                self.code.state.input(),
                self.city.state.input()
            );
            Ok(Person {
                name,
                birthday,
                address,
            })
        }
    }
}

// Render simple input field
fn ui(f: &mut Frame, app: &mut App) {
    let horizontal_layout: [Rect; 3] =
        Layout::horizontal([Constraint::Min(1), Constraint::Max(70), Constraint::Min(1)])
            .areas(f.area());
    let vertical_layout: [Rect; 3] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(15),
        Constraint::Min(1),
    ])
    .areas(horizontal_layout[1]);
    let vertical_layout: [Rect; 5] = Layout::vertical([
        app.name.widget.vertical(),
        app.day.widget.vertical(),
        app.street.widget.vertical(),
        app.code.widget.vertical(),
        app.error.widget.vertical(),
    ])
    .areas(vertical_layout[1]);

    let horizontal_layout: [Rect; 2] = Layout::horizontal([
        app.name.widget.horizontal(),
        app.lastname.widget.horizontal(),
    ])
    .areas(vertical_layout[0]);

    f.render_stateful_widget(&app.name.widget, horizontal_layout[0], &mut app.name.state);
    f.render_stateful_widget(
        &app.lastname.widget,
        horizontal_layout[1],
        &mut app.lastname.state,
    );

    let horizontal_layout: [Rect; 3] = Layout::horizontal([
        app.day.widget.horizontal(),
        app.month.widget.horizontal(),
        app.year.widget.horizontal(),
    ])
    .areas(vertical_layout[1]);

    f.render_stateful_widget(&app.day.widget, horizontal_layout[0], &mut app.day.state);
    f.render_stateful_widget(
        &app.month.widget,
        horizontal_layout[1],
        &mut app.month.state,
    );
    f.render_stateful_widget(&app.year.widget, horizontal_layout[2], &mut app.year.state);

    f.render_stateful_widget(
        &app.street.widget,
        vertical_layout[2],
        &mut app.street.state,
    );

    let horizontal_layout: [Rect; 2] =
        Layout::horizontal([app.code.widget.horizontal(), app.city.widget.horizontal()])
            .areas(vertical_layout[3]);

    f.render_stateful_widget(&app.code.widget, horizontal_layout[0], &mut app.code.state);
    f.render_stateful_widget(&app.city.widget, horizontal_layout[1], &mut app.city.state);

    if !app.error.state.input().is_empty() {
        f.render_stateful_widget(&app.error.widget, vertical_layout[4], &mut app.error.state);
    }
}

fn main() {
    let selection_style = SelectionStyle {
        focused: ratatui::prelude::Style::default()
            .bg(tailwind::INDIGO.c400)
            .fg(tailwind::BLACK),
        border: ratatui::prelude::Style::default().fg(tailwind::INDIGO.c400),
        ..SelectionStyle::default()
    };
    let input_style = InputFieldStyle {
        focused: ratatui::prelude::Style::default().fg(tailwind::INDIGO.c400),
        cursor: ratatui::prelude::Style::default()
            .bg(tailwind::INDIGO.c400)
            .fg(tailwind::WHITE),
        ..InputFieldStyle::default()
    };
    let error_style = InputFieldStyle {
        focused: ratatui::prelude::Style::default().fg(tailwind::RED.c500),
        cursor: ratatui::prelude::Style::default(),
        default: ratatui::prelude::Style::default().fg(tailwind::RED.c500),
        ..InputFieldStyle::default()
    };
    // Create app state
    let mut app = AppBuilder::default()
        .name(Widget {
            state: InputFieldStateBuilder::default()
                .focused(true)
                .disabled(false)
                .build()
                .unwrap(),
            widget: InputFieldBuilder::default()
                .border(true)
                .title(Some("Name".to_string()))
                .margins(Margin {
                    vertical: 0,
                    horizontal: 1,
                })
                .style(input_style.clone())
                .min_width(4)
                .build()
                .unwrap(),
        })
        .lastname(Widget {
            state: InputFieldStateBuilder::default()
                .focused(false)
                .disabled(false)
                .build()
                .unwrap(),
            widget: InputFieldBuilder::default()
                .border(true)
                .title(Some("Lastname".to_string()))
                .margins(Margin {
                    vertical: 0,
                    horizontal: 1,
                })
                .style(input_style.clone())
                .min_width(4)
                .build()
                .unwrap(),
        })
        .day(Widget {
            state: InputFieldStateBuilder::default()
                .focused(false)
                .disabled(false)
                .build()
                .unwrap(),
            widget: InputFieldBuilder::default()
                .border(true)
                .title(Some("Day".to_string()))
                .margins(Margin {
                    vertical: 0,
                    horizontal: 1,
                })
                .style(input_style.clone())
                .min_width(2)
                .build()
                .unwrap(),
        })
        .month(Widget {
            state: SelectionStateBuilder::default()
                .focused(false)
                .values(vec![
                    "January".into(),
                    "February".into(),
                    "March".into(),
                    "April".into(),
                    "May".into(),
                    "June".into(),
                    "July".into(),
                    "August".into(),
                    "September".into(),
                    "October".into(),
                    "November".into(),
                    "December".into(),
                ])
                .build()
                .unwrap(),
            widget: SelectionBuilder::default()
                .border(true)
                .title(Some("Month".to_string()))
                .margins(Margin {
                    vertical: 0,
                    horizontal: 1,
                })
                .style(selection_style.clone())
                .min_width(10)
                .build()
                .unwrap(),
        })
        .year(Widget {
            state: InputFieldStateBuilder::default()
                .focused(false)
                .disabled(false)
                .build()
                .unwrap(),
            widget: InputFieldBuilder::default()
                .border(true)
                .title(Some("Year".to_string()))
                .margins(Margin {
                    vertical: 0,
                    horizontal: 1,
                })
                .style(input_style.clone())
                .min_width(4)
                .build()
                .unwrap(),
        })
        .street(Widget {
            state: InputFieldStateBuilder::default()
                .focused(false)
                .disabled(false)
                .build()
                .unwrap(),
            widget: InputFieldBuilder::default()
                .border(true)
                .title(Some("Street".to_string()))
                .margins(Margin {
                    vertical: 0,
                    horizontal: 1,
                })
                .style(input_style.clone())
                .min_width(5)
                .build()
                .unwrap(),
        })
        .code(Widget {
            state: InputFieldStateBuilder::default()
                .focused(false)
                .disabled(false)
                .build()
                .unwrap(),
            widget: InputFieldBuilder::default()
                .border(true)
                .title(Some("Postalcode".to_string()))
                .margins(Margin {
                    vertical: 0,
                    horizontal: 1,
                })
                .style(input_style.clone())
                .min_width(5)
                .build()
                .unwrap(),
        })
        .city(Widget {
            state: InputFieldStateBuilder::default()
                .focused(false)
                .disabled(false)
                .build()
                .unwrap(),
            widget: InputFieldBuilder::default()
                .border(true)
                .title(Some("City".to_string()))
                .margins(Margin {
                    vertical: 0,
                    horizontal: 1,
                })
                .style(input_style.clone())
                .min_width(5)
                .build()
                .unwrap(),
        })
        .error(Widget {
            state: InputFieldStateBuilder::default()
                .focused(true)
                .disabled(false)
                .build()
                .unwrap(),
            widget: InputFieldBuilder::default()
                .border(false)
                .title(None)
                .margins(Margin {
                    vertical: 0,
                    horizontal: 1,
                })
                .style(error_style.clone())
                .min_width(5)
                .build()
                .unwrap(),
        })
        .focus(0)
        .build()
        .unwrap();

    let mut screen: AlternateScreen<Stdout> =
        AlternateScreen::new().expect("Failed to create alternate screen.");

    loop {
        // Draw app
        screen.draw(|f| ui(f, &mut app)).unwrap();

        // Show error
        match app.result() {
            Ok(_) => {
                app.error.state.set_input("".into());
            }
            Err(e) => {
                app.error.state.set_input(format!("ERROR: {}", e));
            }
        }

        // Check for events
        if event::poll(Duration::from_millis(50)).unwrap() {
            if let Event::Key(key) = event::read().unwrap() {
                if key.kind == KeyEventKind::Press {
                    if let KeyCode::Esc = key.code {
                        break;
                    } else {
                        let event_result: EventResult = app.handle_events(key.modifiers, key.code);
                        match event_result {
                            EventResult::Unhandled(_, KeyCode::Enter) => {
                                break;
                            }
                            EventResult::Unhandled(KeyModifiers::SHIFT, KeyCode::BackTab)
                            | EventResult::Unhandled(KeyModifiers::SHIFT, KeyCode::Tab) => {
                                app.focus_previous();
                            }
                            EventResult::Unhandled(_, KeyCode::Tab) => {
                                app.focus_next();
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    drop(screen);

    println!("{:?}", app.result());
}
