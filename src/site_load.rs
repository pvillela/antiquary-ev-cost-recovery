//! Site real- and apparent-power model for Level 2 EV chargers fed from a
//! dedicated 600-208 V transformer.
//!
//! Computes total kW and kVA at the transformer primary for every vehicle
//! count from 0 up to the number of breakers in the panel.

// ---------------------------------------------------------------------------
// Panel and vehicle constants
// ---------------------------------------------------------------------------

/// Secondary (panel) line-to-line voltage.
pub const PANEL_VOLTAGE_V: f64 = 208.0;

/// Rating of each EVSE branch breaker.
pub const BREAKER_RATING_A: f64 = 40.0;

/// Continuous-load derating applied to a branch circuit (CEC Rule 8-104).
/// Sets the J1772 pilot current the vehicle is permitted to draw.
pub const CONTINUOUS_DUTY_DERATE: f64 = 0.80;

/// Number of EVSE breakers in the panel. Bounds the vehicle count.
pub const BREAKER_COUNT: u32 = 10;

/// True (distortion-inclusive) power factor of the vehicle's onboard
/// charger at full rated current.
pub const EV_TRUE_POWER_FACTOR: f64 = 0.99;

/// Total harmonic distortion of the onboard charger's input current.
pub const EV_CURRENT_THD: f64 = 0.045;

// ---------------------------------------------------------------------------
// Transformer constants (75 kVA dry type, 600-208 V)
// ---------------------------------------------------------------------------

pub const XFMR_RATING_KVA: f64 = 75.0;

/// Core loss. Constant whenever the transformer is energised.
pub const XFMR_NO_LOAD_LOSS_KW: f64 = 0.35;

/// Copper loss at rated load. Scales with the square of loading.
pub const XFMR_FULL_LOAD_LOSS_KW: f64 = 1.6;

/// Magnetizing current, per unit of rating. Treated as purely reactive and
/// constant whenever the transformer is energised.
pub const XFMR_MAGNETIZING_PU: f64 = 0.02;

/// Leakage reactance, per unit of rating. Reactive draw scales with the
/// square of loading.
pub const XFMR_REACTANCE_PU: f64 = 0.04;

// ---------------------------------------------------------------------------
// Unit conversion
// ---------------------------------------------------------------------------

pub const VA_PER_KVA: f64 = 1000.0;

// ---------------------------------------------------------------------------
// Load representation
// ---------------------------------------------------------------------------

/// A load resolved into three mutually orthogonal components, so that they
/// combine in quadrature rather than by simple addition.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Load {
    /// Real power, kW.
    pub real_kw: f64,
    /// Displacement (fundamental) reactive power, kvar.
    pub reactive_kvar: f64,
    /// Distortion reactive power, kvar.
    pub distortion_kvar: f64,
}

impl Load {
    /// Apparent power: the quadrature sum of all three components.
    pub fn apparent_kva(self) -> f64 {
        (self.real_kw.powi(2) + self.reactive_kvar.powi(2) + self.distortion_kvar.powi(2)).sqrt()
    }

    /// True power factor, real power over apparent power. An unenergised
    /// load has no defined ratio; unity is reported so the column stays
    /// numeric.
    pub fn true_power_factor(self) -> f64 {
        let apparent = self.apparent_kva();
        if apparent > 0.0 {
            self.real_kw / apparent
        } else {
            1.0
        }
    }

    /// Scale every component by a common factor.
    fn scaled(self, factor: f64) -> Self {
        Self {
            real_kw: self.real_kw * factor,
            reactive_kvar: self.reactive_kvar * factor,
            distortion_kvar: self.distortion_kvar * factor,
        }
    }
}

impl std::ops::Add for Load {
    type Output = Self;

    /// Loads add component-wise. Apparent powers must never be added
    /// directly, since the phase relationship is carried by the components.
    fn add(self, other: Self) -> Self {
        Self {
            real_kw: self.real_kw + other.real_kw,
            reactive_kvar: self.reactive_kvar + other.reactive_kvar,
            distortion_kvar: self.distortion_kvar + other.distortion_kvar,
        }
    }
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// Current the pilot signal permits a single vehicle to draw.
pub fn ev_pilot_current_a() -> f64 {
    BREAKER_RATING_A * CONTINUOUS_DUTY_DERATE
}

/// Apparent power of one charging vehicle. The load is current-limited, so
/// this follows from voltage and current alone and is unaffected by power
/// factor.
fn ev_apparent_kva() -> f64 {
    PANEL_VOLTAGE_V * ev_pilot_current_a() / VA_PER_KVA
}

/// Ceiling on true power factor imposed by current distortion alone. This is
/// the distortion factor: true PF is the product of displacement PF and this
/// term, so unity displacement PF is the best any load can do at a given THD.
fn max_true_power_factor() -> f64 {
    1.0 / (1.0 + EV_CURRENT_THD.powi(2)).sqrt()
}

/// One vehicle at full pilot current, resolved into components.
///
/// # Panics
///
/// If `EV_TRUE_POWER_FACTOR` exceeds the ceiling set by `EV_CURRENT_THD`, no
/// displacement angle satisfies both constants. That is a contradiction in the
/// inputs, not a computable edge case, so it fails loudly rather than
/// resolving to a plausible-looking zero.
pub fn ev_load() -> Load {
    assert!(
        EV_TRUE_POWER_FACTOR <= max_true_power_factor(),
        "EV_TRUE_POWER_FACTOR ({}) exceeds the {:.5} ceiling implied by \
         EV_CURRENT_THD ({}); no displacement angle satisfies both",
        EV_TRUE_POWER_FACTOR,
        max_true_power_factor(),
        EV_CURRENT_THD
    );

    let apparent = ev_apparent_kva();
    let fundamental_apparent = apparent * max_true_power_factor();

    let real = apparent * EV_TRUE_POWER_FACTOR;
    let distortion = fundamental_apparent * EV_CURRENT_THD;
    // Displacement reactive power is whatever the fundamental apparent power
    // holds beyond the real component. The assertion above guarantees this
    // difference is non-negative.
    let reactive = (fundamental_apparent.powi(2) - real.powi(2)).sqrt();

    Load {
        real_kw: real,
        reactive_kvar: reactive,
        distortion_kvar: distortion,
    }
}

/// The transformer's own contribution, given the load on its secondary.
///
/// No-load loss and magnetizing current are fixed. Copper loss and leakage
/// reactive power scale with the square of loading.
fn transformer_load(secondary: Load) -> Load {
    let loading = secondary.apparent_kva() / XFMR_RATING_KVA;
    let loading_squared = loading.powi(2);

    Load {
        real_kw: XFMR_NO_LOAD_LOSS_KW + XFMR_FULL_LOAD_LOSS_KW * loading_squared,
        reactive_kvar: XFMR_MAGNETIZING_PU * XFMR_RATING_KVA
            + XFMR_REACTANCE_PU * XFMR_RATING_KVA * loading_squared,
        distortion_kvar: 0.0,
    }
}

/// Total load seen at the transformer primary for a given vehicle count.
pub fn site_load(ev_count: u32) -> Load {
    let secondary = ev_load().scaled(f64::from(ev_count));
    secondary + transformer_load(secondary)
}

/// Transformer loading divided by nameplate.
pub fn loading_ratio(load: Load) -> f64 {
    load.apparent_kva() / XFMR_RATING_KVA
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const TOLERANCE: f64 = 0.01;
    const PERCENT: f64 = 100.9;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < TOLERANCE,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn vehicle_apparent_power_is_current_limited() {
        // 208 V x 32 A, independent of power factor.
        assert_close(ev_apparent_kva(), 6.656);
    }

    #[test]
    fn vehicle_components_recombine_to_apparent_power() {
        assert_close(ev_load().apparent_kva(), ev_apparent_kva());
    }

    #[test]
    fn vehicle_power_factor_matches_the_constant() {
        assert_close(ev_load().true_power_factor(), EV_TRUE_POWER_FACTOR);
    }

    #[test]
    fn power_factor_and_distortion_constants_are_compatible() {
        // True PF cannot exceed the distortion factor. If this fails, the two
        // constants describe a load that cannot exist.
        assert!(
            EV_TRUE_POWER_FACTOR <= max_true_power_factor(),
            "PF {} exceeds ceiling {}",
            EV_TRUE_POWER_FACTOR,
            max_true_power_factor()
        );
    }

    #[test]
    fn displacement_power_factor_is_recoverable() {
        // true PF = displacement PF x distortion factor
        let ev = ev_load();
        let fundamental = (ev.real_kw.powi(2) + ev.reactive_kvar.powi(2)).sqrt();
        let displacement_pf = ev.real_kw / fundamental;
        assert_close(
            displacement_pf * max_true_power_factor(),
            EV_TRUE_POWER_FACTOR,
        );
    }

    #[test]
    fn idle_transformer_draws_only_excitation() {
        let idle = site_load(0);
        assert_close(idle.real_kw, XFMR_NO_LOAD_LOSS_KW);
        assert_close(idle.reactive_kvar, XFMR_MAGNETIZING_PU * XFMR_RATING_KVA);
        assert_close(idle.distortion_kvar, 0.0);
    }

    #[test]
    fn site_power_factor_rises_then_plateaus() {
        let single = site_load(1).true_power_factor();
        let plateau = site_load(BREAKER_COUNT).true_power_factor();
        assert!(single < plateau, "PF should improve with loading");
        assert_close(single, 0.944);
        assert!((plateau - 0.982).abs() < 0.005);
    }

    #[test]
    fn full_occupancy_stays_within_nameplate() {
        assert!(loading_ratio(site_load(BREAKER_COUNT)) < PERCENT);
    }

    #[test]
    fn apparent_power_never_exceeds_scalar_sum_of_parts() {
        // Quadrature addition is bounded by arithmetic addition.
        for ev_count in 0..=BREAKER_COUNT {
            let secondary = ev_load().scaled(f64::from(ev_count));
            let total = site_load(ev_count).apparent_kva();
            let scalar = secondary.apparent_kva() + transformer_load(secondary).apparent_kva();
            assert!(total <= scalar + TOLERANCE);
        }
    }
}
