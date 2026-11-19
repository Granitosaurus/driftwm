//! Head-look: pan one output's camera from a head-tracking pose stream.
//!
//! For glasses that are only a display strapped to the head (VITURE and friends)
//! the picture follows the head, so a nod or a turn drags the text with it. This
//! turns that around for *small* adjustments: the camera moves opposite to the
//! head, so content looks pinned in space, but only within a bounded window
//! (default one display's size). Canvas navigation stays on the keyboard; the
//! head only nudges the view around wherever the keyboard put it.
//!
//! Input is OpenTrack UDP (what XRLinuxDriver emits): six little-endian f64
//! (x, y, z, yaw, pitch, roll; angles in degrees) and a u32 frame counter. Only
//! yaw and pitch are used.
//!
//! The pose is integrated as *deltas* into a clamped screen-px offset (so a
//! drifting absolute heading never walks the view off), then the existing
//! [`PanShaper`] One Euro filter smooths it. Pure maths, no compositor state.

use crate::camera_shaper::{PanShaper, ShaperConfig};

/// Bytes in an OpenTrack packet: six f64 and a u32 frame number.
pub const OPENTRACK_PACKET_LEN: usize = 52;

/// A pose gap longer than this (seconds) re-baselines tracking: the glasses were
/// unplugged, or the driver restarted, and the next angle is unrelated.
const GAP_S: f64 = 1.0;
/// A single-packet angle jump above this (degrees) is the driver settling or a
/// reconnect, not a head movement.
const JUMP_DEG: f64 = 20.0;
/// Seconds over which angular speed is averaged for the deadzone.
const SPEED_TAU: f64 = 0.1;

#[derive(Clone, Debug, PartialEq)]
pub struct HeadLookSettings {
    /// Listen for head-tracking packets at all.
    pub enabled: bool,
    /// Output whose camera the head pans (connector name, e.g. `DP-1`).
    pub output: String,
    /// UDP address to listen on, `host:port`.
    pub listen: String,
    /// Screen px the view moves per degree of head rotation.
    pub gain: f64,
    /// Furthest the head can pan the view, as a fraction of the output's size.
    pub range: f64,
    /// Head speed (deg/s) below which motion is ignored, to hide sensor drift.
    pub deadzone: f64,
    /// One Euro `mincutoff` in Hz (lower = smoother and laggier). 0 = off.
    pub smoothing: f64,
    /// One Euro `beta`: extra cutoff Hz per px/s of speed.
    pub beta: f64,
    /// Seconds of pose ignored when tracking starts, while the driver calibrates.
    pub settle: f64,
    /// Flip left/right.
    pub invert_x: bool,
    /// Flip up/down.
    pub invert_y: bool,
}

impl Default for HeadLookSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            output: String::new(),
            listen: "127.0.0.1:4242".to_string(),
            gain: 45.0,
            range: 1.0,
            deadzone: 0.5,
            smoothing: 1.5,
            beta: 0.01,
            settle: 1.5,
            invert_x: false,
            invert_y: false,
        }
    }
}

impl HeadLookSettings {
    /// Filter settings for [`PanShaper`]: no gain curve (the gain is applied on
    /// the way in), smoothing only.
    pub fn shaper(&self) -> ShaperConfig {
        ShaperConfig {
            speed: 1.0,
            speed_fast: 0.0,
            knee: 0.0,
            smoothing: self.smoothing,
            beta: self.beta,
        }
    }
}

/// Yaw and pitch in degrees from an OpenTrack packet, or `None` if it is short or
/// carries non-finite or absurd angles.
pub fn parse_opentrack(buf: &[u8]) -> Option<(f64, f64)> {
    if buf.len() < OPENTRACK_PACKET_LEN {
        return None;
    }
    let f = |i: usize| f64::from_le_bytes(buf[i * 8..i * 8 + 8].try_into().unwrap());
    let (yaw, pitch) = (f(3), f(4));
    (yaw.is_finite() && pitch.is_finite() && yaw.abs() <= 720.0 && pitch.abs() <= 720.0)
        .then_some((yaw, pitch))
}

/// Shortest signed angle from `from` to `to`, degrees, in (-180, 180].
fn angle_delta(from: f64, to: f64) -> f64 {
    let d = (to - from) % 360.0;
    if d > 180.0 {
        d - 360.0
    } else if d <= -180.0 {
        d + 360.0
    } else {
        d
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct HeadLook {
    shaper: PanShaper,
    /// Last accepted yaw, pitch (degrees) and the time it arrived (seconds).
    last: Option<([f64; 2], f64)>,
    /// When the current tracking run began, for the settle period.
    started_at: f64,
    /// Low-passed angular speed per axis, deg/s.
    speed: [f64; 2],
    /// Where the head is asking the view to be, screen px from neutral (Y-down).
    target: [f64; 2],
}

impl HeadLook {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Feed one pose. `t` is a monotonic time in seconds; `size` is the output's
    /// logical size in px, which bounds the offset.
    pub fn push_pose(
        &mut self,
        yaw: f64,
        pitch: f64,
        t: f64,
        cfg: &HeadLookSettings,
        size: [f64; 2],
    ) {
        let Some((prev, prev_t)) = self.last else {
            return self.rebaseline(yaw, pitch, t);
        };
        let dt = t - prev_t;
        if dt > GAP_S {
            return self.rebaseline(yaw, pitch, t);
        }
        if dt <= 0.0 {
            return;
        }
        let d = [angle_delta(prev[0], yaw), pitch - prev[1]];
        self.last = Some(([yaw, pitch], t));
        if d[0].abs() > JUMP_DEG || d[1].abs() > JUMP_DEG {
            return self.rebaseline(yaw, pitch, t);
        }
        if t - self.started_at < cfg.settle {
            return;
        }

        let a = dt / (SPEED_TAU + dt);
        let mut px = [0.0; 2];
        for i in 0..2 {
            self.speed[i] += (d[i] / dt - self.speed[i]) * a;
            if self.speed[i].abs() >= cfg.deadzone {
                px[i] = d[i] * cfg.gain;
            }
        }
        // Head right / up moves the camera right / up (content looks pinned);
        // canvas Y grows downward, so pitch is negated.
        let sx = if cfg.invert_x { -1.0 } else { 1.0 };
        let sy = if cfg.invert_y { 1.0 } else { -1.0 };
        let want = [self.target[0] + px[0] * sx, self.target[1] + px[1] * sy];
        let limit = [cfg.range * size[0], cfg.range * size[1]];
        let clamped = [
            want[0].clamp(-limit[0], limit[0]),
            want[1].clamp(-limit[1], limit[1]),
        ];
        self.shaper
            .push(clamped[0] - self.target[0], clamped[1] - self.target[1]);
        self.target = clamped;
    }

    fn rebaseline(&mut self, yaw: f64, pitch: f64, t: f64) {
        self.last = Some(([yaw, pitch], t));
        self.started_at = t;
        self.speed = [0.0, 0.0];
    }

    /// Advance one frame of `dt` seconds. Returns how far the camera moves, in
    /// screen px (Y-down); the caller divides by zoom.
    pub fn step(&mut self, cfg: &HeadLookSettings, dt: f64) -> [f64; 2] {
        self.shaper.step(&cfg.shaper(), dt)
    }

    /// True while there is offset still to travel, so frames keep coming.
    pub fn is_settling(&self) -> bool {
        self.shaper.is_settling()
    }

    /// Glide the view back to neutral and treat the head's current pose as the
    /// new neutral. Safe to call at any time.
    pub fn recenter(&mut self) {
        self.shaper.push(-self.target[0], -self.target[1]);
        self.target = [0.0, 0.0];
    }

    /// The offset the head is asking for, screen px from neutral.
    pub fn target(&self) -> [f64; 2] {
        self.target
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: [f64; 2] = [1920.0, 1080.0];

    fn cfg() -> HeadLookSettings {
        HeadLookSettings {
            smoothing: 0.0,
            settle: 0.0,
            deadzone: 0.0,
            ..Default::default()
        }
    }

    /// Feed a steady turn of `deg_per_s` for `secs` at 250 Hz, advancing the
    /// shared clock `t` (seconds) and heading `yaw`.
    fn turn(h: &mut HeadLook, c: &HeadLookSettings, yaw: &mut f64, deg_per_s: f64, secs: f64) {
        thread_local!(static CLOCK: std::cell::Cell<f64> = const { std::cell::Cell::new(10.0) });
        let dt = 0.004;
        let n = (secs / dt) as usize;
        for _ in 0..n {
            *yaw += deg_per_s * dt;
            let t = CLOCK.get() + dt;
            CLOCK.set(t);
            h.push_pose(*yaw, 0.0, t, c, SIZE);
        }
    }

    #[test]
    fn parse_reads_yaw_and_pitch() {
        let mut buf = vec![0u8; OPENTRACK_PACKET_LEN];
        buf[24..32].copy_from_slice(&12.5f64.to_le_bytes());
        buf[32..40].copy_from_slice(&(-3.0f64).to_le_bytes());
        assert_eq!(parse_opentrack(&buf), Some((12.5, -3.0)));
    }

    #[test]
    fn parse_rejects_short_and_nonsense() {
        assert_eq!(parse_opentrack(&[0u8; 20]), None);
        let mut buf = vec![0u8; OPENTRACK_PACKET_LEN];
        buf[24..32].copy_from_slice(&f64::NAN.to_le_bytes());
        assert_eq!(parse_opentrack(&buf), None);
        buf[24..32].copy_from_slice(&1.0e9f64.to_le_bytes());
        assert_eq!(parse_opentrack(&buf), None);
    }

    #[test]
    fn angle_delta_takes_the_short_way_round() {
        assert_eq!(angle_delta(170.0, -170.0), 20.0);
        assert_eq!(angle_delta(-170.0, 170.0), -20.0);
        assert_eq!(angle_delta(10.0, 30.0), 20.0);
    }

    #[test]
    fn turning_right_moves_the_camera_right_by_gain_per_degree() {
        let c = cfg();
        let mut h = HeadLook::default();
        h.push_pose(0.0, 0.0, 10.0, &c, SIZE); // baseline
        let mut yaw = 0.0;
        turn(&mut h, &c, &mut yaw, 20.0, 0.5); // 10 degrees
        let out = h.step(&c, 0.016);
        assert!((out[0] - 10.0 * c.gain).abs() < 1e-6, "{out:?}");
        assert!(out[1].abs() < 1e-9);
    }

    #[test]
    fn pitch_up_moves_the_camera_up_which_is_negative_y() {
        let c = cfg();
        let mut h = HeadLook::default();
        h.push_pose(0.0, 0.0, 10.0, &c, SIZE);
        let mut pitch = 0.0;
        for i in 0..100 {
            pitch += 0.1;
            h.push_pose(0.0, pitch, 10.0 + (i as f64 + 1.0) * 0.004, &c, SIZE);
        }
        assert!(h.step(&c, 0.016)[1] < 0.0);
    }

    #[test]
    fn invert_flips_both_axes() {
        let c = HeadLookSettings {
            invert_x: true,
            invert_y: true,
            ..cfg()
        };
        let mut h = HeadLook::default();
        h.push_pose(0.0, 0.0, 10.0, &c, SIZE);
        h.push_pose(1.0, 1.0, 10.004, &c, SIZE);
        let t = h.target();
        assert!(t[0] < 0.0 && t[1] > 0.0, "{t:?}");
    }

    #[test]
    fn offset_is_bounded_to_range_times_size() {
        let c = HeadLookSettings {
            range: 0.5,
            ..cfg()
        };
        let mut h = HeadLook::default();
        h.push_pose(0.0, 0.0, 10.0, &c, SIZE);
        let mut yaw = 0.0;
        turn(&mut h, &c, &mut yaw, 20.0, 5.0); // 100 degrees, far past the limit
        assert_eq!(h.target()[0], 0.5 * SIZE[0]);
        let mut total = 0.0;
        for _ in 0..10 {
            total += h.step(&c, 0.016)[0];
        }
        assert!((total - 0.5 * SIZE[0]).abs() < 1e-6);
    }

    #[test]
    fn slow_drift_under_the_deadzone_is_ignored() {
        let c = HeadLookSettings {
            deadzone: 0.5,
            ..cfg()
        };
        let mut h = HeadLook::default();
        h.push_pose(0.0, 0.0, 10.0, &c, SIZE);
        let mut yaw = 0.0;
        turn(&mut h, &c, &mut yaw, 0.2, 10.0); // 0.2 deg/s sensor creep
        assert_eq!(h.target(), [0.0, 0.0]);
        turn(&mut h, &c, &mut yaw, 10.0, 0.5); // a real turn still registers
        assert!(h.target()[0] > 0.0);
    }

    #[test]
    fn the_settle_period_ignores_calibration_motion() {
        let c = HeadLookSettings {
            settle: 1.0,
            ..cfg()
        };
        let mut h = HeadLook::default();
        h.push_pose(0.0, 0.0, 10.0, &c, SIZE);
        h.push_pose(5.0, 0.0, 10.5, &c, SIZE); // inside the settle window
        assert_eq!(h.target(), [0.0, 0.0]);
        h.push_pose(10.0, 0.0, 11.2, &c, SIZE); // after it: counts
        assert!(h.target()[0] > 0.0);
    }

    #[test]
    fn a_big_single_step_jump_rebaselines_instead_of_panning() {
        let c = cfg();
        let mut h = HeadLook::default();
        h.push_pose(0.0, 0.0, 10.0, &c, SIZE);
        h.push_pose(40.0, 0.0, 10.004, &c, SIZE);
        assert_eq!(h.target(), [0.0, 0.0]);
        h.push_pose(41.0, 0.0, 10.008, &c, SIZE);
        assert!(
            h.target()[0] > 0.0,
            "tracking resumes from the new baseline"
        );
    }

    #[test]
    fn a_long_gap_rebaselines() {
        let c = cfg();
        let mut h = HeadLook::default();
        h.push_pose(0.0, 0.0, 10.0, &c, SIZE);
        h.push_pose(90.0, 0.0, 20.0, &c, SIZE); // glasses came back, new heading
        assert_eq!(h.target(), [0.0, 0.0]);
    }

    #[test]
    fn recenter_glides_the_camera_back_to_where_it_started() {
        let c = cfg();
        let mut h = HeadLook::default();
        h.push_pose(0.0, 0.0, 10.0, &c, SIZE);
        let mut yaw = 0.0;
        turn(&mut h, &c, &mut yaw, 20.0, 0.5);
        let mut moved = h.step(&c, 0.016)[0];
        h.recenter();
        assert_eq!(h.target(), [0.0, 0.0]);
        moved += h.step(&c, 0.016)[0];
        assert!(moved.abs() < 1e-6, "net camera travel {moved}");
    }

    #[test]
    fn smoothing_spreads_the_move_over_frames_and_still_arrives() {
        let c = HeadLookSettings {
            smoothing: 1.5,
            ..cfg()
        };
        let mut h = HeadLook::default();
        h.push_pose(0.0, 0.0, 10.0, &c, SIZE);
        let mut yaw = 0.0;
        turn(&mut h, &c, &mut yaw, 20.0, 0.5);
        let first = h.step(&c, 0.016)[0];
        assert!(first > 0.0 && first < 10.0 * c.gain);
        let mut total = first;
        for _ in 0..2000 {
            total += h.step(&c, 0.016)[0];
        }
        assert!((total - 10.0 * c.gain).abs() < 1e-3, "{total}");
        assert!(!h.is_settling());
    }
}
