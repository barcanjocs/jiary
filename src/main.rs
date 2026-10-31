mod app;
mod db;
mod session;
mod theme;

use std::io;

use crossterm::{
    cursor::Show,
    event::{self, Event},
    execute,
};
use ratatui::DefaultTerminal;

fn main() {
    let db = match db::Db::open() {
        Ok(db) => db,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    // init() enters raw mode and the alternate screen, and installs a panic
    // hook that restores the terminal; ours chains onto it to also show the
    // cursor (hidden outside input lines). restore() covers every other exit
    // path, so an error never leaves the shell in raw mode.
    let mut terminal = ratatui::init();
    let restore_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), Show);
        restore_hook(info);
    }));

    let result = run(&mut terminal, app::App::new(db));
    let _ = terminal.show_cursor();
    ratatui::restore();

    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run(terminal: &mut DefaultTerminal, mut app: app::App) -> io::Result<()> {
    // Hidden at startup; cursor_position() says where the cursor belongs each
    // frame (only the start form's input steps show it).
    terminal.hide_cursor()?;

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
            && app.handle_key(key)
        {
            return Ok(());
        }
    }
}
