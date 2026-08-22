use std::ops::Deref;

use crate::{
    session::Session,
    time::{Interval, Tou, tou_partition},
};

#[derive(Debug, Clone, Copy, PartialEq)]
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

pub fn tou_kwh(time_range: Interval, sessions: &[impl Deref<Target = Session>]) -> TouKwh {
    let mut on_peak_kwh = 0.0;
    let mut mid_peak_kwh = 0.0;
    let mut off_peak_kwh = 0.0;

    for s in sessions {
        // A zero adjusted duration would make the rate infinite or NaN and poison every bucket it
        // touched.
        //
        // Unreachable from anything this crate calls with. `adj_conn_end` is
        // `truncate(conn_end + 1s) + TIME_GRID_STEP` while `adj_conn_start` is
        // `truncate(conn_start)`, so the two coincide only when `conn_end` precedes `conn_start` by
        // a full step -- an inverted record, which is flagged `InconsistentDuration` and sorted into
        // `Sessions::excluded` before any caller here sees it. See
        // `a_sound_record_cannot_have_a_zero_adjusted_duration` below.
        //
        // Kept because this function is public and takes whatever list it is handed. A caller that
        // skipped the bucketing gets the other sessions' figures rather than a poisoned total.
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

// cargo test --lib -- session::energy::test
#[cfg(test)]
mod test {
    use super::*;
    use crate::session::TIME_GRID_STEP;
    use crate::session::test_support::session;

    /// The guard in [`tou_kwh`] is unreachable for any record that survives bucketing, and this is
    /// what establishes it: the adjusted span of a record whose end does not precede its start is
    /// always at least one [`TIME_GRID_STEP`] wide.
    ///
    /// Swept across a step's worth of offsets, because the two ends are truncated to the grid and
    /// the question is whether they can ever land on the same multiple. A single fixture would sit
    /// at one offset and say nothing about the rest.
    #[test]
    fn a_sound_record_cannot_have_a_zero_adjusted_duration() {
        let step = TIME_GRID_STEP.as_secs();
        for offset in 0..step {
            // The shortest sound record there is: start and end the same instant, at every offset
            // within one grid step. Anything longer only widens the span.
            let start = format!("2026-06-10T06:{:02}:{:02}Z", offset / 60, offset % 60);
            let s = session("June.csv", 2, "ZERO", &start, 0, 1.0);
            assert!(
                s.adj_duration() >= TIME_GRID_STEP,
                "offset {offset}s gave {:?}",
                s.adj_duration()
            );
        }
    }

    /// And the energy of such a record is counted rather than dropped, which is the behaviour the
    /// guard would have taken away had it fired.
    #[test]
    fn a_zero_length_session_still_contributes_its_energy() {
        // 02:00 EDT on 10 June, off-peak, reported as a single instant.
        let s = session("June.csv", 2, "INSTANT", "2026-06-10T06:00:00Z", 0, 3.0);
        let day = Interval::from_start_end(
            "2026-06-10T04:00:00Z"
                .parse()
                .expect("an RFC 3339 timestamp"),
            "2026-06-11T04:00:00Z"
                .parse()
                .expect("an RFC 3339 timestamp"),
        );
        let kwh = tou_kwh(day, &[s]);
        assert!((kwh.total_kwh() - 3.0).abs() < 1e-9, "{kwh:?}");
    }
}
