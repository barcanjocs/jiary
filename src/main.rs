mod app;
mod db;
mod session;
mod theme;

use std::io;

use crossterm::{
    event::{self, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

fn main() {
    let db = match db::Db::open() {
        Ok(db) => db,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = run(db) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run(db: db::Db) -> io::Result<()> {
    enable_raw_mode()?;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    // Hidden at startup; cursor_position() says where the cursor belongs each
    // frame (only the start form's input steps show it).
    terminal.hide_cursor()?;

    let mut app = app::App::new(db);

    loop {
        terminal.draw(|frame| app.draw(frame))?;
        // The cursor is only visible on input lines. set_cursor_position
        // moves the cursor but does not show it, so show it explicitly.
        match app.cursor_position(terminal.size()?.into()) {
            Some(pos) => {
                terminal.show_cursor()?;
                terminal.set_cursor_position(pos)?;
            }
            None => terminal.hide_cursor()?,
        }

        if event::poll(std::time::Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && app.handle_key(key.code)
        {
            break;
        }
    }
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}
