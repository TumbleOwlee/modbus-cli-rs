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

/// RAII wrapper around a ratatui terminal on the alternate screen.
///
/// Creation enables raw mode and enters the alternate screen on the output
/// `W` (stdout or stderr via [`Init`]); dropping restores the terminal.
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
    /// Enables raw mode, enters the alternate screen, and sets up the
    /// ratatui terminal.
    pub fn new() -> Result<Self, std::io::Error> {
        enable_raw_mode()?;

        let mut output = W::init();
        execute!(output, EnterAlternateScreen, EnableMouseCapture)?;

        let backend = CrosstermBackend::new(output);
        let mut terminal = Terminal::new(backend)?;
        execute!(terminal.backend_mut(), DisableMouseCapture)?;

        Ok(Self { terminal })
    }

    /// Draws a frame using the given render callback.
    pub fn draw<F>(
        &mut self,
        render_callback: F,
    ) -> Result<CompletedFrame<'_>, <CrosstermBackend<Stdout> as Backend>::Error>
    where
        F: FnOnce(&mut Frame),
    {
        self.terminal.draw(render_callback)
    }

    /// Restores the terminal (leave alternate screen, disable raw mode)
    /// without a value to drop — for use from panic/exit handlers.
    pub fn release() {
        disable_raw_mode().expect("Disable raw mode failed.");
        execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture)
            .expect("Failed to leave alternate screen.");
    }
}

/// A terminal-like surface `App` can render a frame onto. Abstracts the single
/// method `App` calls on its screen (`draw`) so a test double backed by
/// ratatui's `TestBackend` can be injected in place of a real terminal.
pub trait DrawSurface {
    /// Renders one frame via the given callback.
    fn draw<F: FnOnce(&mut Frame)>(&mut self, render: F) -> std::io::Result<()>;
}

impl<W> DrawSurface for AlternateScreen<W>
where
    W: Write + Init,
{
    fn draw<F: FnOnce(&mut Frame)>(&mut self, render: F) -> std::io::Result<()> {
        // Inherent method (returns a `CompletedFrame` we discard); named
        // explicitly to avoid resolving to this trait method.
        AlternateScreen::draw(self, render)?;
        Ok(())
    }
}

impl<W> Drop for AlternateScreen<W>
where
    W: Write + Init,
{
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
        let _ = self.terminal.show_cursor();
    }
}
