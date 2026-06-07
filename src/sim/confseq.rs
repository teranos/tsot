//! Anytime-valid confidence sequences for a bounded mean.
//!
//! A *confidence sequence* (CS) is a sequence of intervals `C_1, C_2, …`
//! such that the true mean `μ` is contained in **every** interval
//! simultaneously with probability `≥ 1 − α`:
//!
//! ```text
//!   P( ∀t : μ ∈ C_t )  ≥  1 − α
//! ```
//!
//! Unlike a fixed-N confidence interval, this guarantee holds *no matter
//! how often or when you look* — so a balance designer can watch a
//! win-rate accumulate on a dashboard and stop the moment the interval
//! clears (or enters) an indifference band, with no optional-stopping
//! penalty. SPRT is the ancestor of this idea; a CS gives an **estimate
//! with an interval** rather than a bare accept/reject verdict, which is
//! what a human reading a tool actually wants.
//!
//! ## Construction (betting / e-process)
//!
//! For each candidate mean `m` we run two nonnegative "capital" processes
//! (Waudby-Smith & Ramdas, *Estimating means of bounded random variables
//! by betting*):
//!
//! ```text
//!   K⁺_t(m) = ∏_{i≤t} (1 + λ_i (X_i − m))
//!   K⁻_t(m) = ∏_{i≤t} (1 − λ_i (X_i − m))
//! ```
//!
//! with a *predictable* bet `λ_i` (depends only on `X_1..X_{i-1}`). When
//! `m = μ`, each `K_t` is a nonnegative martingale with mean 1, so by
//! **Ville's inequality** `P(∃t : K_t ≥ 1/α') ≤ α'`. We test each side at
//! `α' = α/2` (threshold `2/α`, union bound), and the confidence set at
//! time `t` is every `m` whose capital has not crossed the threshold:
//!
//! ```text
//!   C_t = { m : max(K⁺_t(m), K⁻_t(m)) < 2/α }
//! ```
//!
//! `λ_i` only affects how *tight* the interval is, never its validity —
//! validity comes from the martingale property, which holds for any
//! predictable bet that keeps the factors nonnegative. The predictable
//! plug-in below scales `λ` by the running variance, so a lopsided
//! matchup (`p` near 0 or 1) clears the band fast and a near-coin-flip
//! tightens slowly. The coverage guarantee is checked empirically in the
//! tests (Monte-Carlo miss-rate under optional stopping `≤ α`), in the
//! spirit of the journal's "inverse-correctness is a test invariant."
//!
//! The candidate means are evaluated on a fixed grid; the reported
//! interval is `[min, max]` of the surviving grid points (the convex hull
//! of the confidence set — never anti-conservative).

#![allow(dead_code)]

/// Stopping verdict against a two-sided indifference band
/// `[center − delta, center + delta]` (for a win-rate, `center = 0.5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandDecision {
    /// Interval is disjoint from the band — one side is favored.
    Decided,
    /// Interval lies entirely inside the band — indistinguishable from
    /// the center to within `delta`.
    Balanced,
    /// Interval still straddles a band edge — keep sampling.
    Continue,
}

/// Betting confidence sequence for a mean of `[0, 1]`-bounded observations.
///
/// Update with each observation as it arrives; read [`interval`],
/// [`point_estimate`], or [`band_decision`] at any time.
///
/// [`interval`]: BettingCs::interval
/// [`point_estimate`]: BettingCs::point_estimate
/// [`band_decision`]: BettingCs::band_decision
#[derive(Debug, Clone)]
pub struct BettingCs {
    /// Mis-coverage budget (overall, two-sided).
    alpha: f64,
    /// `ln(2/alpha)` — the per-side log-capital rejection threshold.
    log_threshold: f64,
    /// Candidate means in `[0, 1]`, ascending.
    grid: Vec<f64>,
    /// Running log of `K⁺` per grid point.
    log_kp: Vec<f64>,
    /// Running log of `K⁻` per grid point.
    log_km: Vec<f64>,
    n: u64,
    sum: f64,
    sumsq: f64,
}

impl BettingCs {
    /// Default grid resolution (step between candidate means).
    pub const DEFAULT_GRID_STEP: f64 = 0.005;

    /// New CS at mis-coverage level `alpha` (e.g. `0.05` for 95%).
    pub fn new(alpha: f64) -> Self {
        Self::with_grid_step(alpha, Self::DEFAULT_GRID_STEP)
    }

    /// New CS with an explicit grid step. Finer steps give a smoother
    /// interval at linear cost.
    pub fn with_grid_step(alpha: f64, step: f64) -> Self {
        assert!(alpha > 0.0 && alpha < 1.0, "alpha must be in (0, 1)");
        assert!(step > 0.0 && step < 1.0, "grid step must be in (0, 1)");
        // Inclusive grid 0.0 ..= 1.0. The `min(1/m, 1/(1-m))` bet cap is
        // finite at both ends (= 1 there), so the endpoints need no
        // special-casing.
        let points = (1.0 / step).round() as usize;
        let grid: Vec<f64> = (0..=points)
            .map(|i| (i as f64 * step).min(1.0))
            .collect();
        let len = grid.len();
        Self {
            alpha,
            log_threshold: (2.0 / alpha).ln(),
            grid,
            log_kp: vec![0.0; len],
            log_km: vec![0.0; len],
            n: 0,
            sum: 0.0,
            sumsq: 0.0,
        }
    }

    /// Incorporate one observation `x ∈ [0, 1]`.
    pub fn update(&mut self, x: f64) {
        debug_assert!(x.is_finite() && (0.0..=1.0).contains(&x), "x must be in [0,1], got {x}");
        let t = (self.n + 1) as f64; // 1-based step index

        // Predictable plug-in: variance of the data seen *so far* (before
        // this observation), shrunk toward the [0,1]-max prior 0.25. Only
        // past data is used, keeping `lambda` predictable so the capital
        // stays a martingale under the null.
        let var_hat = if self.n == 0 {
            0.25
        } else {
            let count = self.n as f64;
            let mean = self.sum / count;
            // (0.25 + Σ(xᵢ − mean)²) / (1 + count), with the sum of squared
            // deviations = Σxᵢ² − count·mean².
            ((0.25 + self.sumsq - count * mean * mean) / (1.0 + count)).max(1e-4)
        };

        // Base bet magnitude (the m-independent part). Scaling by 1/√var
        // makes the interval variance-adaptive — the selling point over a
        // Hoeffding CS. Validity does not depend on this choice.
        let lam_base = (2.0 * self.log_threshold / (var_hat * t * (t + 1.0).ln())).sqrt();

        for i in 0..self.grid.len() {
            let m = self.grid[i];
            // Keep both (1 ± λ(x−m)) factors ≥ 0.5 > 0 for any x ∈ [0,1].
            let cap = 0.5 * (1.0 / m).min(1.0 / (1.0 - m));
            let lam = lam_base.min(cap).max(0.0);
            let d = x - m;
            self.log_kp[i] += (1.0 + lam * d).ln();
            self.log_km[i] += (1.0 - lam * d).ln();
        }

        self.n += 1;
        self.sum += x;
        self.sumsq += x * x;
    }

    /// Incorporate a batch of observations.
    pub fn observe_many(&mut self, xs: &[f64]) {
        for &x in xs {
            self.update(x);
        }
    }

    /// Number of observations seen.
    pub fn n(&self) -> u64 {
        self.n
    }

    /// Running sample mean (`0.5` before any data).
    pub fn point_estimate(&self) -> f64 {
        if self.n == 0 {
            0.5
        } else {
            self.sum / self.n as f64
        }
    }

    /// Current anytime-valid confidence interval `(lo, hi)`.
    ///
    /// The reported interval is the convex hull of the surviving grid
    /// points `{ m : max(K⁺, K⁻) < 2/α }` — quantized to the grid and
    /// never anti-conservative. Before any data it is the full `[0, 1]`.
    pub fn interval(&self) -> (f64, f64) {
        let thr = self.log_threshold;
        let mut lo: Option<f64> = None;
        let mut hi: Option<f64> = None;
        for i in 0..self.grid.len() {
            if self.log_kp[i] < thr && self.log_km[i] < thr {
                let m = self.grid[i];
                if lo.is_none() {
                    lo = Some(m);
                }
                hi = Some(m);
            }
        }
        match (lo, hi) {
            (Some(l), Some(h)) => (l, h),
            // Degenerate: even the MLE was rejected (numerical only).
            // Fall back to the point estimate rather than a bogus span.
            _ => {
                let p = self.point_estimate();
                (p, p)
            }
        }
    }

    /// Width of the current interval.
    pub fn width(&self) -> f64 {
        let (lo, hi) = self.interval();
        hi - lo
    }

    /// Verdict against the band `[center − delta, center + delta]`.
    pub fn band_decision(&self, center: f64, delta: f64) -> BandDecision {
        let (lo, hi) = self.interval();
        let band_lo = center - delta;
        let band_hi = center + delta;
        if hi < band_lo || lo > band_hi {
            BandDecision::Decided
        } else if lo >= band_lo && hi <= band_hi {
            BandDecision::Balanced
        } else {
            BandDecision::Continue
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    fn bernoulli_stream(rng: &mut StdRng, p: f64, n: usize) -> Vec<f64> {
        (0..n).map(|_| if rng.gen::<f64>() < p { 1.0 } else { 0.0 }).collect()
    }

    #[test]
    fn fresh_cs_is_the_full_interval() {
        let cs = BettingCs::new(0.05);
        let (lo, hi) = cs.interval();
        assert!(lo <= 1e-9, "lo should start at 0, got {lo}");
        assert!(hi >= 1.0 - 1e-9, "hi should start at 1, got {hi}");
        assert_eq!(cs.n(), 0);
    }

    #[test]
    fn point_estimate_tracks_running_mean() {
        let mut cs = BettingCs::new(0.05);
        assert!((cs.point_estimate() - 0.5).abs() < 1e-12, "prior should be 0.5");
        cs.observe_many(&[1.0, 1.0, 0.0, 0.0]);
        assert!((cs.point_estimate() - 0.5).abs() < 1e-12);
        cs.observe_many(&[1.0, 1.0]);
        assert!((cs.point_estimate() - 4.0 / 6.0).abs() < 1e-12);
    }

    #[test]
    fn interval_shrinks_with_data() {
        let mut rng = StdRng::seed_from_u64(0xA11CE);
        let mut cs = BettingCs::new(0.05);
        cs.observe_many(&bernoulli_stream(&mut rng, 0.3, 20));
        let early = cs.width();
        cs.observe_many(&bernoulli_stream(&mut rng, 0.3, 480));
        let late = cs.width();
        assert!(late < early, "interval should tighten: early={early} late={late}");
    }

    #[test]
    fn interval_brackets_the_point_estimate() {
        let mut cs = BettingCs::new(0.05);
        for _ in 0..200 {
            cs.update(1.0);
            cs.update(0.0);
        }
        let (lo, hi) = cs.interval();
        let est = cs.point_estimate();
        assert!(lo <= est + 1e-9 && est <= hi + 1e-9, "[{lo},{hi}] must contain {est}");
    }

    #[test]
    fn same_input_gives_byte_identical_interval() {
        let data = {
            let mut rng = StdRng::seed_from_u64(7);
            bernoulli_stream(&mut rng, 0.42, 300)
        };
        let mut a = BettingCs::new(0.05);
        let mut b = BettingCs::new(0.05);
        a.observe_many(&data);
        b.observe_many(&data);
        assert_eq!(a.interval(), b.interval());
    }

    /// The headline guarantee: under optional stopping (we inspect the
    /// interval after *every* observation and count a miss if the true
    /// mean is *ever* excluded), the miss-rate stays at or below `alpha`.
    /// Deterministic via a fixed seed.
    #[test]
    fn coverage_holds_under_optional_stopping() {
        let alpha = 0.1;
        let p = 0.3; // on-grid at the default step
        let streams = 400;
        let n_max = 400;
        let mut rng = StdRng::seed_from_u64(0xC0FFEE);
        let mut ever_miss = 0;
        for _ in 0..streams {
            let mut cs = BettingCs::new(alpha);
            let mut missed = false;
            for _ in 0..n_max {
                let x = if rng.gen::<f64>() < p { 1.0 } else { 0.0 };
                cs.update(x);
                let (lo, hi) = cs.interval();
                if p < lo - 1e-9 || p > hi + 1e-9 {
                    missed = true;
                }
            }
            if missed {
                ever_miss += 1;
            }
        }
        let miss_rate = ever_miss as f64 / streams as f64;
        assert!(
            miss_rate <= alpha,
            "anytime miss-rate {miss_rate} exceeded alpha {alpha} ({ever_miss}/{streams})"
        );
    }

    #[test]
    fn band_decision_balanced_for_a_fair_coin() {
        let mut rng = StdRng::seed_from_u64(0xFA12);
        let mut cs = BettingCs::new(0.1);
        cs.observe_many(&bernoulli_stream(&mut rng, 0.5, 4000));
        assert_eq!(cs.band_decision(0.5, 0.05), BandDecision::Balanced);
    }

    #[test]
    fn band_decision_decided_for_a_biased_coin() {
        let mut rng = StdRng::seed_from_u64(0xB1A5);
        let mut cs = BettingCs::new(0.1);
        cs.observe_many(&bernoulli_stream(&mut rng, 0.85, 1000));
        assert_eq!(cs.band_decision(0.5, 0.05), BandDecision::Decided);
    }

    #[test]
    fn band_decision_continue_when_undecided() {
        let cs = BettingCs::new(0.05);
        // No data: interval is [0,1], straddles both band edges.
        assert_eq!(cs.band_decision(0.5, 0.05), BandDecision::Continue);
    }
}
