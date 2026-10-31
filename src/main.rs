mod app;
mod captions;
mod db;
mod idle;
mod layout;
mod mascot;
mod period;
mod session;
mod summary;
mod theme;

use std::io::{self, IsTerminal};
use std::sync::mpsc;
use std::thread;

use crossterm::{
    cursor::Show,
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
};
use ratatui::DefaultTerminal;

const HELP: &str = "\
jiary: a work diary for the terminal

Usage: jiary [--help | --version]

Run without arguments to open it. Inside, press ? for every key, or / for
the command palette.";

fn main() {
    // Flags before anything touches the terminal or the database.
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("-h" | "--help") => {
            println!("{HELP}");
            return;
        }
        Some("-V" | "--version") => {
            println!("jiary {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some(other) => {
            eprintln!("jiary: unknown argument {other:?}\n\n{HELP}");
            std::process::exit(2);
        }
    }
    // A full-screen app needs a terminal to draw in; say so rather than
    // fail deep in the terminal setup (say, run from a script or a pipe).
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        eprintln!("jiary: needs to run in a terminal");
        std::process::exit(1);
    }

    let db = match db::Db::open() {
        Ok(db) => db,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    // try_init() enters raw mode and the alternate screen, and installs a panic
    // hook that restores the terminal; ours chains onto it to also show the
    // cursor (hidden outside input lines) and release the mouse. restore()
    // covers every other exit path, so an error never leaves the shell in
    // raw mode or with the mouse captured.
    let mut terminal = match ratatui::try_init() {
        Ok(terminal) => terminal,
        Err(e) => {
            eprintln!("jiary: could not set up the terminal: {e}");
            std::process::exit(1);
        }
    };
    let restore_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), Show, DisableMouseCapture);
        restore_hook(info);
    }));
    // Mouse capture drives resizing and scrolling; most terminals still
    // select text with Shift held while it is on.
    let _ = execute!(io::stdout(), EnableMouseCapture);

    // Db::open created the data dir, so the layout file can live beside it.
    let layout_path = db::data_dir().map(|dir| dir.join("layout.conf"));
    let result = run(&mut terminal, app::App::new(db, layout_path));
    let _ = terminal.show_cursor();
    let _ = execute!(io::stdout(), DisableMouseCapture);
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

    // Asking the system for its idle time runs a command, so it happens
    // off the draw loop; the loop picks up whatever has arrived.
    let (idle_tx, idle_rx) = mpsc::channel();
    thread::spawn(move || {
        while idle_tx.send(idle::system_idle()).is_ok() {
            thread::sleep(idle::POLL);
        }
    });

    loop {
        while let Ok(idle) = idle_rx.try_recv() {
            app.observe_idle(idle, chrono::Utc::now());
        }
        let size = terminal.size()?;
        app.set_size((size.width, size.height));
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

        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if app.handle_key(key) => return Ok(()),
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                _ => {}
            }
        }
    }
}
