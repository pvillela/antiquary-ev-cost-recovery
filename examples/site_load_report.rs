//! Reports site real- and apparent-power model for Level 2 EV chargers fed from a
//! dedicated 600-208 V transformer.
//!
//! Lists total kW and kVA at the transformer primary for every vehicle
//! count from 0 up to the number of breakers in the panel.
//!
//! The rendering itself lives in the library, beside the one that renders an interval's estimates,
//! so this example is the invocation and nothing else. That also lets a golden file pin the table.

use ev_peak_contrib::sessions::site_load_report;

fn main() {
    print!("{}", site_load_report());
}
