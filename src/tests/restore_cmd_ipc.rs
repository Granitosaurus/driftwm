//! `msg restore-cmd` through the IPC dispatcher: how the target is chosen
//! (selector, stand-in, caller's ancestor PIDs) and what a malformed request
//! does. The fixture's clients connect over a socketpair made in this process,
//! so their PID is the test's own.

use smithay::utils::{Point, SERIAL_COUNTER, Size};

use crate::ipc::dispatch;
use crate::ipc::protocol::{Reply, Request, Response, WindowSelector};
use crate::state::StageWindow;

use super::{Fixture, map_window, window_by_app_id};

fn restore_cmd(
    f: &mut Fixture,
    window: Option<WindowSelector>,
    pids: Vec<u32>,
    cmd: Option<Vec<&str>>,
    clear: bool,
) -> Reply {
    dispatch(
        Request::RestoreCmd {
            window,
            pids,
            cmd: cmd.map(|c| c.into_iter().map(String::from).collect()),
            clear,
        },
        f.state(),
    )
}

fn get(f: &mut Fixture, window: WindowSelector) -> Option<Vec<String>> {
    match restore_cmd(f, Some(window), vec![], None, false) {
        Ok(Response::RestoreCmd(cmd)) => cmd,
        other => panic!("expected a RestoreCmd reply, got {other:?}"),
    }
}

fn app(name: &str) -> WindowSelector {
    WindowSelector::AppId(name.into())
}

fn words(cmd: &[&str]) -> Option<Vec<String>> {
    Some(cmd.iter().map(|w| w.to_string()).collect())
}

/// The socket takes raw JSON, so an empty list must be an error, not an index
/// panic that takes the compositor down.
#[test]
fn empty_or_blank_command_is_refused() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "term", (400, 300));

    for bad in [vec![], vec![""], vec!["", "kitty"]] {
        let reply = restore_cmd(&mut f, Some(app("term")), vec![], Some(bad), false);
        assert!(reply.is_err());
    }
    assert_eq!(get(&mut f, app("term")), None, "nothing was stored");
}

#[test]
fn set_get_and_clear_on_a_live_window_by_selector() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "term", (400, 300));
    assert_eq!(get(&mut f, app("term")), None, "unset reads back as none");

    let set = restore_cmd(
        &mut f,
        Some(app("term")),
        vec![],
        Some(vec!["kitty", "nvim", "a b"]),
        false,
    );
    assert!(matches!(set, Ok(Response::Ok)));
    assert_eq!(get(&mut f, app("term")), words(&["kitty", "nvim", "a b"]));

    let cleared = restore_cmd(&mut f, Some(app("term")), vec![], None, true);
    assert!(matches!(cleared, Ok(Response::Ok)));
    assert_eq!(get(&mut f, app("term")), None);
}

/// A selector names a live window first and falls back to a stand-in, so the
/// command of a suspended window can be read and changed in place.
#[test]
fn selector_reaches_a_suspended_stand_in() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let sid = f.state().insert_suspended_for_test(
        1,
        Point::from((400, 300)),
        Size::from((400, 300)),
        "standin",
        "Stand-in",
    );
    let element = StageWindow::Suspended(f.state().find_suspended(sid).unwrap());
    let eid = f.state().stage.id_of(&element).unwrap().0;

    assert_eq!(get(&mut f, WindowSelector::Id(eid)), None);
    let set = restore_cmd(
        &mut f,
        Some(WindowSelector::Id(eid)),
        vec![],
        Some(vec!["standin", "--resume"]),
        false,
    );
    assert!(matches!(set, Ok(Response::Ok)));
    assert_eq!(
        *f.state().find_suspended(sid).unwrap().restore_cmd.borrow(),
        words(&["standin", "--resume"])
    );
    assert_eq!(
        get(&mut f, app("standin")),
        words(&["standin", "--resume"]),
        "the app_id substring finds the stand-in too"
    );

    restore_cmd(&mut f, Some(WindowSelector::Id(eid)), vec![], None, true).unwrap();
    assert_eq!(get(&mut f, WindowSelector::Id(eid)), None);
    f.state().dismiss_suspended(sid);
}

#[test]
fn unknown_selector_errors() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let reply = restore_cmd(&mut f, Some(app("nope")), vec![], Some(vec!["x"]), false);
    assert!(reply.is_err());
}

/// Without a selector the target is the window whose client PID is one of the
/// caller's ancestors; a PID that owns nothing is skipped, and with no match at
/// all the request errors rather than guessing.
#[test]
fn no_selector_targets_the_window_owned_by_an_ancestor() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "term", (400, 300));
    let own = std::process::id();
    let stranger = own.wrapping_add(1);

    let set = restore_cmd(
        &mut f,
        None,
        vec![stranger, own],
        Some(vec!["kitty", "htop"]),
        false,
    );
    assert!(matches!(set, Ok(Response::Ok)));
    assert_eq!(get(&mut f, app("term")), words(&["kitty", "htop"]));

    let got = restore_cmd(&mut f, None, vec![own], None, false);
    assert!(matches!(got, Ok(Response::RestoreCmd(Some(_)))));

    let none = restore_cmd(&mut f, None, vec![stranger], Some(vec!["x"]), false);
    assert!(none.is_err());
    let no_pids = restore_cmd(&mut f, None, vec![], Some(vec!["x"]), false);
    assert!(no_pids.is_err());
    assert_eq!(get(&mut f, app("term")), words(&["kitty", "htop"]));
}

/// One process owning several windows (a single-instance terminal, or every X11
/// window behind xwayland-satellite): the focused one wins, else the first.
#[test]
fn shared_process_resolves_to_the_focused_window() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "first", (400, 300));
    map_window(&mut f, id, "second", (400, 300));
    let own = std::process::id();

    let second = window_by_app_id(&mut f, "second").unwrap();
    f.state()
        .raise_and_focus(&second, SERIAL_COUNTER.next_serial());
    restore_cmd(&mut f, None, vec![own], Some(vec!["on-second"]), false).unwrap();
    assert_eq!(get(&mut f, app("second")), words(&["on-second"]));
    assert_eq!(get(&mut f, app("first")), None);

    let first = window_by_app_id(&mut f, "first").unwrap();
    f.state()
        .raise_and_focus(&first, SERIAL_COUNTER.next_serial());
    restore_cmd(&mut f, None, vec![own], Some(vec!["on-first"]), false).unwrap();
    assert_eq!(get(&mut f, app("first")), words(&["on-first"]));
    assert_eq!(get(&mut f, app("second")), words(&["on-second"]));
}
