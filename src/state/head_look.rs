//! Head-look glue: the OpenTrack UDP listener and the per-frame camera nudge.
//! The maths lives in [`driftwm::head_look`].

use std::io::ErrorKind;
use std::net::UdpSocket;
use std::time::Duration;

use smithay::output::Output;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};
use smithay::utils::{Logical, Point};

use driftwm::head_look::parse_opentrack;

use super::{DriftWm, monotonic_now, output_logical_size, output_state};

/// Bind `listen` (`host:port`) and feed every pose packet into
/// [`DriftWm::head_look`]. Bound once at startup; `[head_look]` numbers reload
/// live, but turning it on or moving the port needs a restart.
pub fn start_head_look(
    event_loop: &LoopHandle<'static, DriftWm>,
    listen: &str,
) -> std::io::Result<()> {
    let socket = UdpSocket::bind(listen)?;
    socket.set_nonblocking(true)?;
    event_loop
        .insert_source(
            Generic::new(socket, Interest::READ, Mode::Level),
            |_, socket, state| {
                state.drain_head_look(socket);
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(())
}

impl DriftWm {
    /// Read every queued packet; only the newest pose matters, since the offset
    /// is built from the change between poses.
    fn drain_head_look(&mut self, socket: &UdpSocket) {
        let mut buf = [0u8; 128];
        let mut latest = None;
        loop {
            match socket.recv_from(&mut buf) {
                Ok((n, _)) => latest = parse_opentrack(&buf[..n]).or(latest),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => {
                    tracing::debug!("head-look: recv error: {e}");
                    break;
                }
            }
        }
        let Some((yaw, pitch)) = latest else {
            return;
        };
        let Some(output) = self.output_by_name(&self.config.head_look.output) else {
            return;
        };
        let size = output_logical_size(&output);
        self.head_look.push_pose(
            yaw,
            pitch,
            monotonic_now().as_secs_f64(),
            &self.config.head_look,
            [size.w as f64, size.h as f64],
        );
        if self.head_look.is_settling() {
            self.redraws_needed.insert(output);
        }
    }

    /// The configured head-look output, if head-look is on and it is connected.
    fn head_look_output(&self) -> Option<Output> {
        let cfg = &self.config.head_look;
        if !cfg.enabled {
            return None;
        }
        self.output_by_name(&cfg.output)
    }

    /// Per-frame: let the smoothed head offset move the camera.
    pub fn tick_head_look(&mut self, dt: Duration) {
        let Some(output) = self.head_look_output() else {
            return;
        };
        if self.is_output_fullscreen(&output) {
            // A fullscreen window parks the viewport; don't queue travel for it.
            self.head_look.reset();
            return;
        }
        let step = self
            .head_look
            .step(&self.config.head_look, dt.as_secs_f64());
        if step == [0.0, 0.0] {
            return;
        }
        let zoom = output_state(&output).zoom;
        self.head_look_pan(&output, Point::from((step[0] / zoom, step[1] / zoom)));
    }

    /// Pan `output`'s camera by `delta` canvas px. Unlike a navigation move this
    /// rides along with whatever the camera is already doing: a keyboard
    /// navigation flight keeps its course (its target shifts by the same amount)
    /// and no momentum is banked, so the view stops when the head does.
    fn head_look_pan(&mut self, output: &Output, delta: Point<f64, Logical>) {
        self.user_moved_camera();
        {
            let mut os = output_state(output);
            if let Some(target) = &mut os.camera_target {
                *target += delta;
            }
            if let Some(anchor) = &mut os.zoom_animation_anchor {
                anchor.canvas += delta;
            }
            os.camera.x += delta.x;
            os.camera.y += delta.y;
        }
        self.update_output_from_camera();
        // Keep the cursor on the same spot of the screen, as camera mode does,
        // when it is on the output that moved.
        if self.active_output().is_some_and(|o| o == *output) {
            let at = self.seat.get_pointer().unwrap().current_location();
            self.warp_pointer(at + delta);
        }
    }

    /// Glide the head-look offset back to neutral and take the head's current
    /// pose as the new neutral.
    pub fn head_look_recenter(&mut self) {
        self.head_look.recenter();
        self.mark_all_dirty();
    }
}
