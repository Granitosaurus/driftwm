//! Head-look pans only the configured output's camera, rides along with a
//! navigation flight already under way, and does nothing while disabled.

use std::time::Duration;

use smithay::utils::Point;

use crate::state::output_state;

use super::{Fixture, config};

const TICK: Duration = Duration::from_millis(16);

fn head_look_config(enabled: bool) -> driftwm::config::Config {
    config(&format!(
        "[head_look]\nenabled = {enabled}\noutput = \"HEADLESS-2\"\n\
         smoothing = 0.0\nsettle = 0.0\ndeadzone = 0.0\ngain = 40.0\n"
    ))
}

/// Feed a 10° right turn over half a second to the head-look state.
fn turn_right_10_degrees(f: &mut Fixture) {
    let cfg = f.state().config.head_look.clone();
    let mut yaw = 0.0;
    f.state()
        .head_look
        .push_pose(yaw, 0.0, 100.0, &cfg, [1280.0, 720.0]);
    for i in 1..=125 {
        yaw += 0.08;
        f.state()
            .head_look
            .push_pose(yaw, 0.0, 100.0 + i as f64 * 0.004, &cfg, [1280.0, 720.0]);
    }
}

#[test]
fn head_turn_pans_only_the_head_look_output() {
    let mut f = Fixture::with_config(head_look_config(true));
    let main = f.add_output(1, (1920, 1080));
    let glasses = f.add_output(2, (1280, 720));
    let main_cam = output_state(&main).camera;
    let glasses_cam = output_state(&glasses).camera;

    turn_right_10_degrees(&mut f);
    f.state().tick_head_look(TICK);

    assert_eq!(
        output_state(&main).camera,
        main_cam,
        "other outputs stay put"
    );
    let moved = output_state(&glasses).camera - glasses_cam;
    assert!(
        (moved.x - 400.0).abs() < 1e-6,
        "10 deg * 40 px/deg, got {moved:?}"
    );
    assert!(moved.y.abs() < 1e-9);
}

#[test]
fn head_turn_does_nothing_while_disabled() {
    let mut f = Fixture::with_config(head_look_config(false));
    let _main = f.add_output(1, (1920, 1080));
    let glasses = f.add_output(2, (1280, 720));
    let before = output_state(&glasses).camera;

    turn_right_10_degrees(&mut f);
    f.state().tick_head_look(TICK);

    assert_eq!(output_state(&glasses).camera, before);
}

#[test]
fn head_pan_carries_a_navigation_flight_along_instead_of_cancelling_it() {
    let mut f = Fixture::with_config(head_look_config(true));
    let _main = f.add_output(1, (1920, 1080));
    let glasses = f.add_output(2, (1280, 720));
    let flight_to = Point::from((5000.0, 0.0));
    output_state(&glasses).camera_target = Some(flight_to);

    turn_right_10_degrees(&mut f);
    f.state().tick_head_look(TICK);

    let target = output_state(&glasses).camera_target.expect("flight kept");
    assert!(
        (target.x - (flight_to.x + 400.0)).abs() < 1e-6,
        "the flight's target shifts by the head offset, got {target:?}"
    );
}

#[test]
fn recenter_returns_the_camera_to_where_the_keyboard_left_it() {
    let mut f = Fixture::with_config(head_look_config(true));
    let _main = f.add_output(1, (1920, 1080));
    let glasses = f.add_output(2, (1280, 720));
    let start = output_state(&glasses).camera;

    turn_right_10_degrees(&mut f);
    f.state().tick_head_look(TICK);
    f.state().head_look_recenter();
    f.state().tick_head_look(TICK);

    let end = output_state(&glasses).camera;
    assert!(
        (end.x - start.x).abs() < 1e-6 && (end.y - start.y).abs() < 1e-6,
        "{end:?}"
    );
}
