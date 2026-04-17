use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    CompletedFrame, Frame, Terminal,
    prelude::{Backend, CrosstermBackend},
};
use std::io::{Stdout, Write, stdout};

use crate::traits::Init;

pub struct AlternateScreen<W>
where
    W: Write + Init,
{
    terminal: Terminal<CrosstermBackend<W>>,
}

impl<W> AlternateScreen<W>
where
    W: Write + Init,
{
    pub fn new() -> Result<Self, std::io::Error> {
        enable_raw_mode().unwrap();

        // Setup output
        let mut output = W::init();
        execute!(output, EnterAlternateScreen, EnableMouseCapture)?;

        // Setup terminal
        let backend = CrosstermBackend::new(output);
        let mut terminal = Terminal::new(backend)?;
        execute!(terminal.backend_mut(), DisableMouseCapture)?;

        Ok(Self { terminal })
    }

    pub fn draw<F>(
        &mut self,
        render_callback: F,
    ) -> Result<CompletedFrame<'_>, <CrosstermBackend<Stdout> as Backend>::Error>
    where
        F: FnOnce(&mut Frame),
    {
        self.terminal.draw(render_callback)
    }

    pub fn release() {
        // restore terminal
        disable_raw_mode().expect("Disable raw mode failed.");
        execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture)
            .expect("Failed to leave alternate screen.");
    }
}

impl<W> Drop for AlternateScreen<W>
where
    W: Write + Init,
{
    fn drop(&mut self) {
        // restore terminal
        disable_raw_mode().expect("Disable raw mode failed.");
        execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .expect("Failed to leave alternate screen.");
        self.terminal.show_cursor().expect("Failed to show cursor.");
    }
}
