//! Camera-mode pan shaping: a speed-dependent gain curve in front of a One Euro
//! filter (Casiez, Roussel & Vogel, CHI 2012).
//!
//! The raw pointer deltas are multiplied by a gain that ramps from the slow gain
//! to the fast gain as the hand speeds up (the same idea as a pointer-acceleration
//! transfer function), then added to a `lag` — how far the camera still has to
//! travel. Each frame the camera takes a fraction of that lag: a low-pass whose
//! cutoff rises with speed, so a slow drift is smoothed hard (jitter vanishes)
//! while a fast flick passes nearly unfiltered (no sluggishness). Tracking the
//! remaining distance instead of a position keeps the total travel exact and
//! the numbers bounded however far the camera goes.
//!
//! Pure maths, no compositor state, so it can be unit tested.

use std::f64::consts::TAU;

/// Pan shaping settings. All-zero optional features (`speed_fast`, `smoothing`)
/// make [`Self::active`] false, and the compositor then keeps its direct,
/// per-event pan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShaperConfig {
    /// Gain at low hand speed (the `camera_mode_speed` multiplier).
    pub speed: f64,
    /// Gain at and above `knee`. 0 = no curve, `speed` everywhere.
    pub speed_fast: f64,
    /// Raw pointer speed (px/s) at which the gain has fully reached `speed_fast`.
    pub knee: f64,
    /// One Euro `mincutoff`, Hz: the smoothing at rest (lower = smoother, laggier).
    /// 0 = no smoothing.
    pub smoothing: f64,
    /// One Euro `beta`: Hz of extra cutoff per px/s of speed (higher = less lag
    /// when moving fast).
    pub beta: f64,
}

impl ShaperConfig {
    pub fn curve_on(&self) -> bool {
        self.speed_fast > 0.0 && self.knee > 0.0
    }

    pub fn smoothing_on(&self) -> bool {
        self.smoothing > 0.0
    }

    pub fn active(&self) -> bool {
        self.curve_on() || self.smoothing_on()
    }
}

/// Seconds over which the raw hand speed (for the gain curve) is averaged.
const RAW_SPEED_TAU: f64 = 0.04;
/// One Euro's derivative cutoff, Hz.
const DERIVATIVE_CUTOFF: f64 = 1.0;
/// Lag (screen px) under which the camera snaps the rest of the way.
const SETTLED_PX: f64 = 0.05;

#[derive(Clone, Copy, Debug, Default)]
pub struct PanShaper {
    /// Raw pointer delta received since the last [`Self::step`].
    pending: [f64; 2],
    /// Screen px the camera still has to travel.
    lag: [f64; 2],
    /// Low-passed raw pointer speed, px/s.
    raw_speed: f64,
    /// Low-passed speed of the shaped target, px/s per axis (One Euro `dx_hat`).
    dx_hat: [f64; 2],
}

fn alpha(cutoff: f64, dt: f64) -> f64 {
    let tau = 1.0 / (TAU * cutoff);
    1.0 / (1.0 + tau / dt)
}

impl PanShaper {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Feed raw pointer motion (device px).
    pub fn push(&mut self, dx: f64, dy: f64) {
        self.pending[0] += dx;
        self.pending[1] += dy;
    }

    /// True while there is input unprocessed or camera travel left, so the
    /// compositor keeps rendering frames (and so ticking) until it settles.
    pub fn is_settling(&self) -> bool {
        self.pending != [0.0, 0.0] || self.lag[0].hypot(self.lag[1]) > SETTLED_PX
    }

    /// Gain for a raw pointer speed in px/s.
    pub fn gain(cfg: &ShaperConfig, raw_speed: f64) -> f64 {
        if !cfg.curve_on() {
            return cfg.speed;
        }
        let t = (raw_speed / cfg.knee).clamp(0.0, 1.0);
        let smooth = t * t * (3.0 - 2.0 * t);
        cfg.speed + (cfg.speed_fast - cfg.speed) * smooth
    }

    /// Advance one frame of `dt` seconds. Returns how far the camera moves, in
    /// screen px, in the direction of the raw motion (the caller applies the
    /// invert sign and divides by zoom).
    pub fn step(&mut self, cfg: &ShaperConfig, dt: f64) -> [f64; 2] {
        let dt = dt.max(1e-4);
        let raw = std::mem::take(&mut self.pending);

        let inst = raw[0].hypot(raw[1]) / dt;
        self.raw_speed += (inst - self.raw_speed) * alpha(1.0 / (TAU * RAW_SPEED_TAU), dt);
        let gain = Self::gain(cfg, self.raw_speed);
        let added = [raw[0] * gain, raw[1] * gain];
        self.lag[0] += added[0];
        self.lag[1] += added[1];

        if !cfg.smoothing_on() {
            return std::mem::take(&mut self.lag);
        }

        let a_d = alpha(DERIVATIVE_CUTOFF, dt);
        for i in 0..2 {
            self.dx_hat[i] += (added[i] / dt - self.dx_hat[i]) * a_d;
        }
        let speed = self.dx_hat[0].hypot(self.dx_hat[1]);
        let a = alpha(cfg.smoothing + cfg.beta * speed, dt);
        let mut out = [self.lag[0] * a, self.lag[1] * a];
        self.lag[0] -= out[0];
        self.lag[1] -= out[1];
        if self.lag[0].hypot(self.lag[1]) <= SETTLED_PX {
            out[0] += self.lag[0];
            out[1] += self.lag[1];
            self.lag = [0.0, 0.0];
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 60.0;

    fn cfg() -> ShaperConfig {
        ShaperConfig {
            speed: 3.0,
            speed_fast: 0.0,
            knee: 600.0,
            smoothing: 0.0,
            beta: 0.0,
        }
    }

    #[test]
    fn default_off_is_a_straight_multiplier() {
        let c = cfg();
        assert!(!c.active());
        let mut s = PanShaper::default();
        s.push(2.0, -1.0);
        assert_eq!(s.step(&c, DT), [6.0, -3.0]);
        assert!(!s.is_settling());
    }

    #[test]
    fn gain_ramps_from_slow_to_fast() {
        let c = ShaperConfig {
            speed_fast: 8.0,
            ..cfg()
        };
        assert_eq!(PanShaper::gain(&c, 0.0), 3.0);
        assert_eq!(PanShaper::gain(&c, 600.0), 8.0);
        assert_eq!(PanShaper::gain(&c, 9000.0), 8.0);
        let mid = PanShaper::gain(&c, 300.0);
        assert!(
            (mid - 5.5).abs() < 1e-9,
            "smoothstep midpoint is halfway: {mid}"
        );
    }

    #[test]
    fn smoothing_conserves_total_travel() {
        let c = ShaperConfig {
            smoothing: 2.0,
            beta: 0.01,
            ..cfg()
        };
        let mut s = PanShaper::default();
        let (mut sum, mut want) = (0.0, 0.0);
        for i in 0..600 {
            if i < 30 {
                s.push(4.0, 0.0);
                want += 4.0 * 3.0;
            }
            sum += s.step(&c, DT)[0];
        }
        assert!(!s.is_settling(), "settles after input stops");
        assert!((sum - want).abs() < 1e-6, "{sum} vs {want}");
    }

    #[test]
    fn smoothing_damps_jitter_more_than_a_fast_flick() {
        let c = ShaperConfig {
            smoothing: 1.0,
            beta: 0.02,
            ..cfg()
        };
        // Slow jitter: alternating +-1 px events. Output should be far smaller
        // in swing than the raw x3 input.
        let mut s = PanShaper::default();
        let mut peak: f64 = 0.0;
        for i in 0..120 {
            s.push(if i % 2 == 0 { 1.0 } else { -1.0 }, 0.0);
            peak = peak.max(s.step(&c, DT)[0].abs());
        }
        assert!(
            peak < 1.5,
            "jitter swing {peak} should be well under raw 3.0"
        );

        // Fast flick: steady 60 px/frame. After warm-up the camera keeps up
        // with the target (little lag left).
        let mut s = PanShaper::default();
        for _ in 0..30 {
            s.push(60.0, 0.0);
            s.step(&c, DT);
        }
        let lag = s.lag[0];
        assert!(lag < 180.0 * 0.35, "fast motion lags {lag} of 180 px/frame");
    }
}
