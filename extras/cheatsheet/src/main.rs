//! Cheatsheet of the keybindings the running driftwm has loaded, read from
//! `driftwm msg --json bindings`. A plain terminal program: bind it to a key
//! with `spawn <terminal> driftwm-cheatsheet`, or give it a scratchpad.

mod model;
mod ui;

use std::io::Read;
use std::process::{Command, ExitCode};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use model::{App, parse_reply};

const USAGE: &str = "\
Usage: driftwm-cheatsheet [--from <file|->]

Shows the keybindings of the running driftwm, read from
`driftwm msg --json bindings`. --from reads that JSON from a file (or stdin
with `-`) instead, to try the cheatsheet outside a driftwm session.

Type to filter, ↑/↓ PgUp/PgDn Home/End to scroll, Esc to clear the filter and
then quit.";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("driftwm-cheatsheet: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => fetch_from_driftwm()?,
        ["--from", source] => read_source(source)?,
        ["-h" | "--help"] => {
            println!("{USAGE}");
            return Ok(());
        }
        _ => return Err(format!("unrecognised arguments\n\n{USAGE}")),
    };
    let mut app = App::new(&parse_reply(&json)?);

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    result.map_err(|e| e.to_string())
}

fn fetch_from_driftwm() -> Result<String, String> {
    let output = Command::new("driftwm")
        .args(["msg", "--json", "bindings"])
        .output()
        .map_err(|e| format!("cannot run `driftwm`: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "`driftwm msg bindings` failed: {} (needs a running driftwm that has the `bindings` request)",
            stderr.trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

fn read_source(source: &str) -> Result<String, String> {
    if source == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("cannot read stdin: {e}"))?;
        Ok(buf)
    } else {
        std::fs::read_to_string(source).map_err(|e| format!("cannot read {source}: {e}"))
    }
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> std::io::Result<()> {
    loop {
        terminal.draw(|frame| ui::render(frame, app))?;
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Release && handle_key(app, key).is_break() {
            return Ok(());
        }
    }
}

fn handle_key(app: &mut App, key: KeyEvent) -> std::ops::ControlFlow<()> {
    let page = app.viewport.saturating_sub(1).max(1) as isize;
    match (key.code, key.modifiers.contains(KeyModifiers::CONTROL)) {
        (KeyCode::Char('c'), true) => return std::ops::ControlFlow::Break(()),
        (KeyCode::Esc, _) if app.filter.is_empty() => return std::ops::ControlFlow::Break(()),
        (KeyCode::Esc, _) | (KeyCode::Char('u'), true) => app.edit_filter(String::clear),
        (KeyCode::Backspace, _) => app.edit_filter(|f| {
            f.pop();
        }),
        (KeyCode::Up, _) => app.scroll_by(-1),
        (KeyCode::Down, _) => app.scroll_by(1),
        (KeyCode::PageUp, _) => app.scroll_by(-page),
        (KeyCode::PageDown, _) => app.scroll_by(page),
        (KeyCode::Home, _) => app.scroll = 0,
        (KeyCode::End, _) => app.scroll = usize::MAX,
        (KeyCode::Char(c), false) => app.edit_filter(|f| f.push(c)),
        _ => {}
    }
    std::ops::ControlFlow::Continue(())
}

#[cfg(test)]
mod tests {
    use std::ops::ControlFlow;

    use super::*;
    use crate::model::tests::SAMPLE;

    fn app() -> App {
        App::new(&parse_reply(SAMPLE).unwrap())
    }

    fn press(app: &mut App, code: KeyCode) -> ControlFlow<()> {
        handle_key(app, KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn typing_filters_and_escape_clears_before_quitting() {
        let mut app = app();
        for c in "zoom".chars() {
            assert!(press(&mut app, KeyCode::Char(c)).is_continue());
        }
        assert_eq!(app.filter, "zoom");
        assert!(press(&mut app, KeyCode::Esc).is_continue());
        assert_eq!(app.filter, "");
        assert!(press(&mut app, KeyCode::Esc).is_break());
    }

    #[test]
    fn filtering_resets_the_scroll() {
        let mut app = app();
        app.scroll = 7;
        let _ = press(&mut app, KeyCode::Char('z'));
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn ctrl_c_quits_even_with_a_filter() {
        let mut app = app();
        let _ = press(&mut app, KeyCode::Char('z'));
        let quit = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(quit.is_break());
    }
}
