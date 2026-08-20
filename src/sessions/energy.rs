use crate::{
    sessions::Session,
    time::{Interval, Tou, tou_partition},
};

#[derive(Debug)]
pub struct TouKwh {
    pub on_peak: f64,
    pub mid_peak: f64,
    pub off_peak: f64,
}

impl TouKwh {
    pub fn total_kwh(&self) -> f64 {
        self.on_peak + self.mid_peak + self.off_peak
    }
}

pub fn tou_kwh(time_range: Interval, sessions: &[Session]) -> TouKwh {
    let mut on_peak_kwh = 0.0;
    let mut mid_peak_kwh = 0.0;
    let mut off_peak_kwh = 0.0;

    for s in sessions {
        // A zero adjusted duration would make the rate infinite or NaN and poison every bucket it
        // touched. It cannot arise from a sound record -- `adj_conn_end` is at least one
        // `TIME_GRID_STEP` past a truncated `conn_end` -- but this function takes whatever list it
        // is handed, and silently dropping the session is the only outcome that leaves the other
        // sessions' figures readable.
        let adj_duration = s.adj_duration();
        if adj_duration.is_zero() {
            continue;
        }
        let session_interval = Interval::new(s.adj_conn_start(), adj_duration);
        let kwh_per_sec = s.energy_use / adj_duration.as_secs_f64();
        let overlap = session_interval.intersection(&time_range);
        let partition = tou_partition(overlap);
        for (tou, itvl) in partition {
            let tou_kwh = kwh_per_sec * itvl.duration.as_secs_f64();
            match tou {
                Tou::OnPeak => on_peak_kwh += tou_kwh,
                Tou::MidPeak => mid_peak_kwh += tou_kwh,
                Tou::OffPeak => off_peak_kwh += tou_kwh,
            }
        }
    }

    TouKwh {
        on_peak: on_peak_kwh,
        mid_peak: mid_peak_kwh,
        off_peak: off_peak_kwh,
    }
}
