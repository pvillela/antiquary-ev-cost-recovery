use crate::{
    sessions::Session,
    time::{Interval, Tou, tou_partition},
};

pub struct TouKkh {
    pub on_peak: f64,
    pub mid_peak: f64,
    pub off_peak: f64,
}

pub fn tou_kwh_in_time_span(time_span: Interval, sessions: &[Session]) -> TouKkh {
    let mut on_peak_kwh = 0.0;
    let mut mid_peak_kwh = 0.0;
    let mut off_peak_kwh = 0.0;

    for s in sessions {
        let session_interval = Interval::new(s.adj_conn_start(), s.adj_duration());
        let kwh_per_sec = s.energy_use / s.adj_duration().as_secs_f64();
        let overlap = session_interval.intersection(&time_span);
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

    TouKkh {
        on_peak: on_peak_kwh,
        mid_peak: mid_peak_kwh,
        off_peak: off_peak_kwh,
    }
}
