//! A terminal client for Tidal. The `tidally` and `tidally-remote` binaries are thin
//! wrappers around [`run`].

mod app;
mod art;
mod audio;
mod config;
mod eq;
mod event;
mod player;
mod search;
mod tidal;
mod ui;
mod visualizer;

use anyhow::Result;
use tokio::sync::mpsc;

use app::App;
use config::{Config, Paths};
use player::Player;
use std::io::IsTerminal;
use std::time::Duration;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};

use ratatui_image::picker::Picker;
use ratatui_image::picker::cap_parser::QueryStdioOptions;

/// Which app is running. The remote edition adds SSH audio forwarding (experimental).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edition {
    Local,
    #[cfg(feature = "remote")]
    Remote,
}

impl Edition {
    fn name(self) -> &'static str {
        match self {
            Edition::Local => "tidally",
            #[cfg(feature = "remote")]
            Edition::Remote => "tidally-remote",
        }
    }

    fn usage(self) -> String {
        let name = self.name();
        let blurb = match self {
            Edition::Local => "a terminal client for Tidal",
            #[cfg(feature = "remote")]
            Edition::Remote => {
                "a terminal client for Tidal, with sound forwarded to your SSH client (experimental)"
            }
        };
        format!(
            "{name} {version} - {blurb}

USAGE:
    {name} [--logout] [--help] [--version]

OPTIONS:
    --logout       Forget the saved Tidal session
    -h, --help     Show this message
    -V, --version  Show the version
",
            version = env!("CARGO_PKG_VERSION"),
        )
    }
}

/// If the graphics probe timed out, its reader thread is still blocked on stdin and would
/// swallow the user's first keypress. Ask the terminal for a status report so that read gets
/// a reply instead, then discard anything left over (including that reply, if nothing was
/// stuck).
fn unstick_probe() {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x1b[5n").and_then(|_| out.flush());
    std::thread::sleep(Duration::from_millis(100));
    while crossterm::event::poll(Duration::from_millis(30)).unwrap_or(false) {
        let _ = crossterm::event::read();
    }
}

/// Runs the app to completion.
pub fn run(edition: Edition) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run_async(edition))
}

async fn run_async(edition: Edition) -> Result<()> {
    let paths = Paths::new()?;

    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{}", edition.usage());
                return Ok(());
            }
            "-V" | "--version" => {
                println!("{} {}", edition.name(), env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--logout" => {
                match std::fs::remove_file(paths.session_file()) {
                    Ok(()) => println!("Signed out."),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        println!("Not signed in.")
                    }
                    Err(e) => return Err(e.into()),
                }
                return Ok(());
            }
            other => {
                eprint!("unknown argument: {other}\n\n{}", edition.usage());
                std::process::exit(2);
            }
        }
    }

    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        eprintln!(
            "{} needs a real terminal window: run it in a terminal emulator (or over SSH).",
            edition.name()
        );
        std::process::exit(1);
    }

    let config = Config::load(&paths)?;
    let (tx, rx) = mpsc::unbounded_channel();
    let route = match edition {
        Edition::Local => audio::Route::Local,
        #[cfg(feature = "remote")]
        Edition::Remote => audio::Route::detect(),
    };
    let player = Player::spawn(&paths.mpv_socket(), &route, tx.clone()).await?;

    let mut terminal = ratatui::init();
    // Must run after entering the alternate screen and before reading terminal events.
    // Terminals that support graphics answer within a few ms; don't stall startup on ones that don't.
    let options = QueryStdioOptions {
        timeout: Duration::from_millis(400),
        ..Default::default()
    };
    let picker = if std::env::var_os("TIDALLY_NO_GRAPHICS").is_some() {
        Picker::halfblocks()
    } else {
        let picker =
            Picker::from_query_stdio_with_options(options).unwrap_or_else(|_| Picker::halfblocks());
        // It reports a timeout as success, so we can't tell whether its reader got stuck.
        unstick_probe();
        picker
    };
    // Taps and swipes (Termux sends touches as mouse events). Undo it on panic too, or the
    // terminal is left spewing mouse escape codes.
    let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture);
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
        hook(info);
    }));
    let result = App::new(config, paths, player, route, picker, tx)
        .run(&mut terminal, rx)
        .await;
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}
