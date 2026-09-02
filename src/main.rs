use std::io::stdout;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use grspr::{app::App, cli::Cli};
use ratatui::{Terminal, backend::CrosstermBackend};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut app = App::load(&cli).await?;
    let mut output = stdout();
    enable_raw_mode()?;
    execute!(output, EnterAlternateScreen)?;
    if !cli.no_mouse {
        execute!(output, EnableMouseCapture)?;
    }
    let backend = CrosstermBackend::new(output);
    let mut terminal = Terminal::new(backend)?;
    let result = app.run(&mut terminal).await;
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    if let Err(error) = result {
        eprintln!("grspr: {error:#}");
        return Err(error);
    }
    Ok(())
}
