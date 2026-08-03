//! Reports site real- and apparent-power model for Level 2 EV chargers fed from a
//! dedicated 600-208 V transformer.
//!
//! Lists total kW and kVA at the transformer primary for every vehicle
//! count from 0 up to the number of breakers in the panel.

use ev_peak_contrib::{
    BREAKER_COUNT, BREAKER_RATING_A, CONTINUOUS_DUTY_DERATE, PANEL_VOLTAGE_V, XFMR_RATING_KVA,
    ev_load, ev_pilot_current_a, loading_ratio, site_load,
};

pub const PERCENT: f64 = 100.0;

fn main() {
    let per_ev = ev_load();

    println!("Level 2 EV charging site - load at transformer primary");
    println!();
    println!(
        "  Panel            {:.0} V, {} x {:.0} A breakers",
        PANEL_VOLTAGE_V, BREAKER_COUNT, BREAKER_RATING_A
    );
    println!(
        "  Pilot current    {:.1} A per vehicle ({:.0}% continuous derate)",
        ev_pilot_current_a(),
        CONTINUOUS_DUTY_DERATE * PERCENT
    );
    println!(
        "  Per vehicle      {:.2} kVA = {:.2} kW + {:.2} kvar + {:.2} kvar distortion",
        per_ev.apparent_kva(),
        per_ev.real_kw,
        per_ev.reactive_kvar,
        per_ev.distortion_kvar
    );
    println!("  Transformer      {:.0} kVA", XFMR_RATING_KVA);
    println!();

    println!(
        "{:>4}  {:>9}  {:>9}  {:>11}  {:>9}  {:>7}  {:>8}",
        "EVs", "kW", "kvar", "kvar (dis)", "kVA", "PF", "% rated"
    );
    println!("{}", "-".repeat(69));

    for ev_count in 0..=BREAKER_COUNT {
        let load = site_load(ev_count);
        let percent = loading_ratio(load) * PERCENT;
        let flag = if percent > PERCENT {
            "  <- over nameplate"
        } else {
            ""
        };

        println!(
            "{:>4}  {:>9.2}  {:>9.2}  {:>11.2}  {:>9.2}  {:>7.3}  {:>7.1}%{}",
            ev_count,
            load.real_kw,
            load.reactive_kvar,
            load.distortion_kvar,
            load.apparent_kva(),
            load.true_power_factor(),
            percent,
            flag
        );
    }

    let full = site_load(BREAKER_COUNT);
    println!();
    println!(
        "At full occupancy: {:.2} kW, {:.2} kVA, {:.1}% of nameplate.",
        full.real_kw,
        full.apparent_kva(),
        loading_ratio(full) * PERCENT
    );
}
