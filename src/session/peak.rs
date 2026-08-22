use super::excel::session_list;
use super::{Anomaly, Bracket, RSegment, RSession, SEGMENT_DURATION, Segment, Sessions};
use crate::time::Interval;
use std::{
    error::Error,
    path::{Path, PathBuf},
    rc::Rc,
};

/// Estimates for an interval of interest.
pub struct IntervalEstimates {
    /// Files the sessions were read from, in the order they were read. Held so the report is
    /// self-describing: it can be stored or rendered later without a caller having to remember what
    /// produced it.
    ///
    /// More than one when the estimate spans a billing period, which needs the two monthly session
    /// reports covering its ends. See [`api::pure::peak_power`](mod@crate::pure::peak_power).
    pub sources: Vec<PathBuf>,
    /// Interval of interest.
    pub interval: Interval,
    /// All segments and their estimates.
    pub seg_estimates: Vec<(RSegment, EstimateSet)>,
    /// Segment and estimate set that maximize the energy-based estimates.
    ///
    /// The segment is shared with the matching entry in [`Self::seg_estimates`] rather than copied
    /// from it, so [`std::rc::Rc::ptr_eq`] against
    /// [`Self::count_based_seg_estimate`] answers whether the two derivations peaked on the same
    /// segment — a question comparing clock times can only approximate.
    pub energy_based_seg_estimate: (RSegment, EstimateSet),
    /// Segment and estimate set that maximize the count-based estimates. Shared, as above.
    pub count_based_seg_estimate: (RSegment, EstimateSet),
    /// Every anomaly touching a session that intersects the interval of interest — the sessions'
    /// own faults, and the report-level ones that are relations between records rather than faults
    /// in any of them. Sessions excluded outright are *not* here; they are in
    /// [`Self::excluded_sessions`].
    pub session_anomalies: Vec<Anomaly>,
    /// Every session excluded from the estimates for
    /// [`crate::session::AnomalyKind::InconsistentDuration`] — the whole workbook's worth, not only those
    /// intersecting this interval.
    ///
    /// Unfiltered on purpose. Such a record's own fields contradict each other, so asking whether
    /// it intersects the interval is asking a question of the very timestamps that are in doubt.
    /// The report states which ones appear to touch the interval and lists the rest anyway,
    /// leaving the judgement to a reader who can go back to the source rows.
    pub excluded_sessions: Vec<RSession>,
}

/// The four estimates for one [`Segment`].
///
/// Two derivations times two units. The energy-based pair reads the sessions' own consumption; the
/// count-based pair reads how many of them were charging against the per-EV rating of the
/// infrastructure. Each is a [`Bracket`], because the reported session times are stated only to the
/// minute and the overlap they imply is therefore a range rather than a number.
pub struct EstimateSet {
    pub energy_based_kw: Bracket<f64>,
    pub energy_based_kva: Bracket<f64>,
    pub count_based_kw: Bracket<f64>,
    pub count_based_kva: Bracket<f64>,
}

impl EstimateSet {
    /// The four figures, in the order the report tabulates them.
    pub fn values(&self) -> [Bracket<f64>; 4] {
        [
            self.energy_based_kw,
            self.energy_based_kva,
            self.count_based_kw,
            self.count_based_kva,
        ]
    }
}

/// Produces EV maximum power estimates for the interval of interest `ioi` and the session
/// report at `path`.
pub fn interval_estimates(ioi: Interval, path: &Path) -> Result<IntervalEstimates, Box<dyn Error>> {
    let session_report = session_list(path)?;
    Ok(estimates_from_report(
        ioi,
        vec![path.to_path_buf()],
        &session_report,
    ))
}

/// The estimate proper, once the sessions have been read.
///
/// Separate from [`interval_estimates`] because there is more than one way to arrive at a
/// [`Sessions`]: that function reads one workbook, while [`crate::peak_power`] merges the two
/// monthly CSVs a billing period spans. Both must produce the same figures from the same sessions,
/// which they do by both coming through here.
///
/// Takes the report by reference so one set of sessions can feed several intervals of interest
/// without being read again — `peak_power` estimates over two.
pub(crate) fn estimates_from_report(
    ioi: Interval,
    sources: Vec<PathBuf>,
    report: &Sessions,
) -> IntervalEstimates {
    // Spikes take part in the estimates on the same footing as any other session. A spike's raw
    // energy over charge time is infinite or NaN, either of which would swamp or poison any
    // segment it entered, and [`Session::avg_kw`] substitutes a finite figure for exactly that
    // reason — so nothing has to be done to a spike here. See docs/session/README.md, "Other".
    let rsessions: Vec<RSession> = report
        .sessions
        .iter()
        .chain(&report.spikes)
        .cloned()
        .collect();
    let segments = segments_for_ioi(ioi, &rsessions);
    let seg_estimates: Vec<(RSegment, EstimateSet)> = segments
        .iter()
        .map(|seg| (seg.clone(), segment_estimate(seg)))
        .collect();
    let energy_based_seg_estimate = maximal_segment_estimate(&segments, |seg| seg.agg_kw().mid());
    let count_based_seg_estimate = maximal_segment_estimate(&segments, |seg| seg.agg_count().mid());
    let session_anomalies = collect_session_anomalies(&ioi, &rsessions, &report.anomalies);

    IntervalEstimates {
        sources,
        interval: ioi,
        seg_estimates,
        energy_based_seg_estimate,
        count_based_seg_estimate,
        session_anomalies,
        // `excluded` sessions contradict themselves and take no part in any estimate, but they are
        // still reported: a caller judging an estimate needs to know what was left out. See
        // docs/session/README.md, "Other".
        excluded_sessions: report.excluded.clone(),
    }
}

/// The [`SEGMENT_DURATION`]-wide segments tiling `ioi`, each holding the sessions that intersect
/// it.
///
/// # Panics
///
/// If `ioi`'s duration is not a whole number of [`SEGMENT_DURATION`]s, or is zero. That is the
/// precondition [`SEGMENT_DURATION`] states, and it is checked rather than accommodated.
///
/// Rounding the segment count up would make the segments overrun the interval — a 20-minute
/// interval would tile to 20:00–20:15 and 20:15–20:30 — so a session charging only in the overrun
/// would be counted into the estimates despite falling outside the interval of interest entirely,
/// and could be reported as its peak. Rounding down would silently leave part of the interval
/// unestimated. Neither is a defensible answer to a question that should not have been asked, and
/// both are wrong in a way no figure in the report would reveal.
///
/// The legal interval lengths are 15 minutes and an hour, so nothing coming through
/// [`crate::checked_interval`] can reach this. The core stays permissive about *when* an interval
/// starts, which is what exploratory callers and tests rely on; it was never permissive about how
/// long one may be.
fn segments_for_ioi(ioi: Interval, sessions: &[RSession]) -> Vec<RSegment> {
    let (ioi_secs, seg_secs) = (ioi.duration.as_secs(), SEGMENT_DURATION.as_secs());
    assert!(
        ioi_secs > 0 && ioi_secs % seg_secs == 0,
        "interval of interest is {ioi_secs}s, which is not a positive whole number of \
         {seg_secs}s segments; see SEGMENT_DURATION"
    );

    let nsegs = (ioi_secs / seg_secs) as usize;
    let mut segments = (0..nsegs)
        .map(|i| Segment::new(ioi.start + SEGMENT_DURATION * i as u32, SEGMENT_DURATION))
        .collect::<Vec<_>>();

    for s in sessions {
        for segment in segments.iter_mut() {
            if !s.intersects(&segment.interval) {
                continue;
            }
            segment.add_session(s.clone());
        }
    }

    segments.into_iter().map(Rc::new).collect()
}

pub(crate) fn segment_estimate(segment: &Segment) -> EstimateSet {
    let energy_based_load = segment.energy_based_load();
    let count_based_load = segment.count_based_load();
    EstimateSet {
        energy_based_kw: energy_based_load.map(|load| load.real_kw),
        energy_based_kva: energy_based_load.map(|load| load.apparent_kva()),
        count_based_kw: count_based_load.map(|load| load.real_kw),
        count_based_kva: count_based_load.map(|load| load.apparent_kva()),
    }
}

/// The segment maximizing `criterion`, with its estimates.
///
/// Seeded from the first segment's own criterion rather than from zero, so a segment is never
/// beaten by one that merely scores above zero — an empty interval's segments all score zero, and
/// the first of them is as much the maximum as any other.
///
/// Ties go to the earliest segment: the comparison is strict and the segments are visited in time
/// order, so a later segment has to *beat* the incumbent to displace it. That makes the choice
/// deterministic, which matters because a tie is not rare — every segment of an interval no session
/// reached is tied at the standing block.
pub(crate) fn maximal_segment_estimate(
    segments: &[RSegment],
    criterion: impl Fn(&Segment) -> f64,
) -> (RSegment, EstimateSet) {
    let mut seg_iter = segments.iter();
    let first = seg_iter
        .next()
        .expect("`segments` slice expected to be non-empty");
    let mut hi_crit = criterion(first);
    let mut hi_seg = first;
    let mut hi_est = segment_estimate(first);
    for segment in seg_iter {
        let crit = criterion(segment);
        if crit > hi_crit {
            hi_crit = crit;
            hi_seg = segment;
            hi_est = segment_estimate(segment);
        }
    }
    (hi_seg.clone(), hi_est)
}

/// Every anomaly on every session that intersects the interval of interest.
///
/// Deliberately blind to [`crate::AnomalyKind`]: it matches on nothing, so a kind added later
/// surfaces here without anyone having to remember to wire it up.
fn collect_session_anomalies(
    interval: &Interval,
    rsessions: &[RSession],
    report_anomalies: &[Anomaly],
) -> Vec<Anomaly> {
    let mut anomalies: Vec<Anomaly> = rsessions
        .iter()
        .filter(|s| s.intersects(interval))
        .flat_map(|s| {
            s.anomalies.iter().map(|kind| Anomaly {
                session: s.clone(),
                kind: *kind,
            })
        })
        // The report's own anomalies are relations between records rather than faults in one, so
        // they are not on any `Session::anomalies` and have to be picked up separately. Scoped to
        // the interval on the same test as the rest, so this table stays a statement about the
        // interval of interest and not about the whole file.
        .chain(
            report_anomalies
                .iter()
                .filter(|a| a.session.intersects(interval))
                .cloned(),
        )
        .collect();
    // By source file first, since a merged report interleaves two of them and a row number means
    // nothing without the file it is in. `sort_by` is stable, so a session's own anomalies keep the
    // order `Session::anomalies` states them in.
    anomalies.sort_by(|a, b| {
        a.session
            .path
            .cmp(&b.session.path)
            .then_with(|| a.session.row.cmp(&b.session.row))
            .then_with(|| a.session.id.cmp(&b.session.id))
    });
    anomalies
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    use crate::session::TIME_GRID_STEP;

    use super::super::{
        SEGMENT_DURATION,
        site_load::{ev_load, site_load},
    };
    use super::*;
    use jiff::Timestamp;
    use std::time::Duration;

    /// Tolerance for figures reached by two different routes through the same floating-point
    /// arithmetic. Loose enough to survive reassociation, far tighter than any constant here.
    const TOLERANCE: f64 = 1e-9;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    /// An hour of interest, 16:00–17:00 local on a date with no DST transition.
    fn hour() -> Interval {
        Interval::from_start_end(ts("2026-06-15T20:00:00Z"), ts("2026-06-15T21:00:00Z"))
    }

    /// A session spanning `[start, end)` exactly, drawing `kw` on average.
    ///
    /// `end` is the **adjusted** end, so a test states the geometry it means rather than the
    /// reported end that produces it. `conn_end` is worked back from it, and the result is checked
    /// against [`Session::adj_conn_end`], which is the only definition of that bound.
    ///
    /// The check is not ceremony. This helper used to subtract one [`TIME_GRID_STEP`] and stop
    /// there, inverting a formula `adj_conn_end` no longer has, and so quietly built sessions
    /// ending before the `end` named here for every `end` off the grid. An adjusted end is always
    /// on the grid -- that is what truncation means -- so an `end` that is not names a geometry the
    /// software cannot produce, and a test asking for one is asking about a session that cannot
    /// exist.
    ///
    /// `charge_time` and `energy_use` are chosen so that `avg_kw()` returns `kw`.
    fn session(id: &str, start: &str, end: &str, kw: f64) -> RSession {
        let conn_start = ts(start);
        let adj_conn_end = ts(end);
        let charge_time = Duration::from_secs(3600);
        let session = crate::session::Session {
            path: std::rc::Rc::new(PathBuf::from("Session_Report_Test.csv")),
            row: 2,
            id: id.to_owned(),
            conn_start,
            conn_end: adj_conn_end - TIME_GRID_STEP,
            conn_duration: adj_conn_end.duration_since(conn_start).unsigned_abs(),
            charge_time,
            energy_use: kw * charge_time.as_secs_f64() / 3600.0,
            anomalies: Vec::new(),
        };
        assert_eq!(
            session.adj_conn_end(),
            adj_conn_end,
            "{id}: no reported end yields an adjusted end of {adj_conn_end}; it is off the time \
             grid"
        );
        Rc::new(session)
    }

    // -----------------------------------------------------------------------
    // Structure. No electrical constant appears in any of these.
    // -----------------------------------------------------------------------

    /// An hour is four quarters, each starting one `SEGMENT_DURATION` after the last.
    ///
    /// This is the bug that made every segment of a 1-hour interval an hour apart: the stride was
    /// the interval's own length rather than a segment's.
    #[test]
    fn segment_starts_stride_by_one_segment_duration() {
        let segments = segments_for_ioi(hour(), &[]);
        assert_eq!(segments.len(), 4);
        for (i, segment) in segments.iter().enumerate() {
            assert_eq!(segment.start(), hour().start + SEGMENT_DURATION * i as u32);
            assert_eq!(segment.interval.duration, SEGMENT_DURATION);
        }
        // Half-open and gapless: each segment ends exactly where the next begins, and the last
        // ends where the interval does.
        for pair in segments.windows(2) {
            assert_eq!(pair[0].end(), pair[1].start());
        }
        assert_eq!(segments[3].end(), hour().end());
    }

    /// An interval that is not a whole number of segments is refused rather than tiled.
    ///
    /// Rounding up would put the last segment past the interval's end — 20 minutes would tile to
    /// 20:00–20:15 and 20:15–20:30 — and a session charging only in that overrun would be counted
    /// into an interval it falls outside of, possibly as the reported peak. Nothing in the output
    /// would show it.
    #[test]
    #[should_panic(expected = "not a positive whole number")]
    fn an_interval_that_is_not_whole_segments_is_refused() {
        let twenty_minutes = Interval::new(hour().start, Duration::from_secs(20 * 60));
        segments_for_ioi(twenty_minutes, &[]);
    }

    /// A zero-length interval has no segments to rank, so it is refused at the same gate rather
    /// than reaching the maximal-segment search with an empty slice.
    #[test]
    #[should_panic(expected = "not a positive whole number")]
    fn an_empty_interval_is_refused() {
        segments_for_ioi(Interval::new(hour().start, Duration::ZERO), &[]);
    }

    /// A 15-minute interval is one segment, not four.
    #[test]
    fn a_quarter_hour_interval_is_a_single_segment() {
        let quarter = Interval::new(hour().start, SEGMENT_DURATION);
        let segments = segments_for_ioi(quarter, &[]);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].interval, quarter);
    }

    /// Overlapping and abutting are different things, and the whole tiling rests on the
    /// difference.
    ///
    /// `intersects` returned `overlap.is_empty()` — the exact negation — so every segment used to
    /// collect precisely the sessions that missed it.
    #[test]
    fn intersects_distinguishes_overlap_from_abutment() {
        let first = Interval::from_start_end(hour().start, hour().start + SEGMENT_DURATION);

        // Ends one minute into the segment: overlapping.
        let overlapping = session("O", "2026-06-15T19:50:00Z", "2026-06-15T20:01:00Z", 1.0);
        assert!(overlapping.intersects(&first));

        // Ends exactly where the segment begins. The segment is half-open, so they abut and share
        // no instant.
        let abutting = session("A", "2026-06-15T19:50:00Z", "2026-06-15T20:00:00Z", 1.0);
        assert!(!abutting.intersects(&first));

        // Starts exactly where the segment ends: abutting on the other side.
        let after = session("B", "2026-06-15T20:15:00Z", "2026-06-15T20:30:00Z", 1.0);
        assert!(!after.intersects(&first));
    }

    /// An inverted span is a broken precondition for `intersects`, and answerable for
    /// `lenient_intersects`.
    ///
    /// A session whose reported end precedes its start is flagged `InconsistentDuration` and
    /// excluded, so it never reaches the estimating logic — which is why the strict test is free
    /// to refuse it rather than guess. The report lists such records on purpose and asks the
    /// lenient one, which reads the two endpoints in whichever order puts them the right way
    /// round.
    #[test]
    fn an_inverted_span_is_answered_only_by_the_lenient_test() {
        // Reported 16:30 to 16:21: the end precedes the start.
        let reversed = session("R", "2026-06-15T20:30:00Z", "2026-06-15T20:21:00Z", 1.0);
        let third = Interval::from_start_end(
            hour().start + SEGMENT_DURATION * 2,
            hour().start + SEGMENT_DURATION * 3,
        );

        // Read the right way round, 16:21-16:30 does meet the 16:30 quarter's predecessor and not
        // the quarter itself; what matters here is that an answer comes back at all.
        assert!(!reversed.lenient_intersects(&third));
        let second = Interval::from_start_end(
            hour().start + SEGMENT_DURATION,
            hour().start + SEGMENT_DURATION * 2,
        );
        assert!(reversed.lenient_intersects(&second));

        // The two agree on every span that is not inverted, which is every span reaching an
        // estimate.
        let ordinary = session("O", "2026-06-15T20:20:00Z", "2026-06-15T20:31:00Z", 1.0);
        for interval in [second, third] {
            assert_eq!(
                ordinary.intersects(&interval),
                ordinary.lenient_intersects(&interval)
            );
        }
    }

    /// The precondition itself: the strict test refuses an inverted span rather than answering.
    #[test]
    #[should_panic(expected = "before it starts")]
    fn the_strict_intersection_test_refuses_an_inverted_span() {
        let reversed = session("R", "2026-06-15T20:30:00Z", "2026-06-15T20:21:00Z", 1.0);
        reversed.intersects(&hour());
    }

    /// A session covering part of a segment counts as that fraction of a session in it.
    ///
    /// The fraction is a third rather than a half. Every adjusted end lands on the time grid, so a
    /// session can only ever cover a whole number of grid steps of a segment: with a 15-minute
    /// segment and a one-minute step, halves are not among the reachable fractions and five
    /// minutes of fifteen is.
    #[test]
    fn a_session_covering_part_of_a_segment_counts_that_fraction() {
        // A session running well past both edges of the segment covers all of it. Neither of its
        // reported boundaries falls inside, so no minute of doubt applies and the bracket is exact.
        let s = session("H", "2026-06-15T19:00:00Z", "2026-06-15T22:00:00Z", 1.0);
        let half = Interval::new(hour().start, SEGMENT_DURATION / 2);
        let mut segment = Segment::new(half.start, half.duration);
        segment.add_session(s);

        let count = segment.agg_count();
        assert!((count.min - 1.0).abs() < TOLERANCE, "{count:?}");
        assert!((count.max - 1.0).abs() < TOLERANCE, "{count:?}");

        // Against the whole segment, a session running 20:00–20:05 covers a third of it.
        let mut whole = Segment::new(hour().start, SEGMENT_DURATION);
        whole.add_session(session(
            "H",
            "2026-06-15T20:00:00Z",
            "2026-06-15T20:05:00Z",
            1.0,
        ));
        let count = whole.agg_count();
        assert!((count.max - 1.0 / 3.0).abs() < TOLERANCE, "{count:?}");
    }

    /// An empty segment is not a zero: the transformer is energised whether or not a car is
    /// plugged in, and both derivations agree on what that standing block is.
    #[test]
    fn an_empty_segment_is_the_standing_block() {
        let segment = Segment::new(hour().start, SEGMENT_DURATION);
        let standing = site_load(0);

        for load in [
            segment.count_based_load().min,
            segment.count_based_load().max,
            segment.energy_based_load().min,
            segment.energy_based_load().max,
        ] {
            assert!(
                (load.real_kw - standing.real_kw).abs() < TOLERANCE,
                "{load:?}"
            );
            assert!(
                (load.apparent_kva() - standing.apparent_kva()).abs() < TOLERANCE,
                "{load:?}"
            );
        }
    }

    /// Ties go to the earliest segment, and a maximum is never lost to a lower later one.
    ///
    /// `hi_crit` used to start at 0.0, so the first segment was displaced by any later segment
    /// scoring above zero — including when the first was the maximum.
    #[test]
    fn the_first_segment_wins_when_it_is_maximal() {
        // Fills the first quarter only.
        let sessions = vec![session(
            "F",
            "2026-06-15T20:00:00Z",
            "2026-06-15T20:15:00Z",
            1.0,
        )];
        let segments = segments_for_ioi(hour(), &sessions);
        let (seg, _) = maximal_segment_estimate(&segments, |s| s.agg_count().mid());
        assert_eq!(seg.start(), hour().start);

        // With nothing anywhere, every segment ties at the standing block and the earliest wins.
        let empty = segments_for_ioi(hour(), &[]);
        let (seg, _) = maximal_segment_estimate(&empty, |s| s.agg_kw().mid());
        assert_eq!(seg.start(), hour().start);

        // A later segment that genuinely beats the first still displaces it.
        let sessions = vec![
            session("F", "2026-06-15T20:00:00Z", "2026-06-15T20:15:00Z", 1.0),
            session("L", "2026-06-15T20:30:00Z", "2026-06-15T20:45:00Z", 1.0),
            session("M", "2026-06-15T20:30:00Z", "2026-06-15T20:45:00Z", 1.0),
        ];
        let segments = segments_for_ioi(hour(), &sessions);
        let (seg, _) = maximal_segment_estimate(&segments, |s| s.agg_count().mid());
        assert_eq!(seg.start(), hour().start + SEGMENT_DURATION * 2);
    }

    /// The maximum is the segment itself, shared, not a copy of it.
    ///
    /// This is what [`RSegment`] is for. Two derivations peaking on one segment must be *the same*
    /// segment, so a caller can settle the question with [`Rc::ptr_eq`] rather than by comparing
    /// clock times and trusting that equal times mean one segment.
    #[test]
    fn a_maximal_segment_is_shared_with_the_listing_not_copied() {
        // One session filling the third quarter, so both derivations peak there.
        let sessions = vec![session(
            "P",
            "2026-06-15T20:30:00Z",
            "2026-06-15T20:45:00Z",
            ev_load().real_kw,
        )];
        let segments = segments_for_ioi(hour(), &sessions);
        let (by_kw, _) = maximal_segment_estimate(&segments, |s| s.agg_kw().mid());
        let (by_count, _) = maximal_segment_estimate(&segments, |s| s.agg_count().mid());

        // Each maximum is one of the segments handed in, not a clone of one.
        assert!(segments.iter().any(|s| Rc::ptr_eq(s, &by_kw)));
        assert!(segments.iter().any(|s| Rc::ptr_eq(s, &by_count)));
        // And here the two derivations agree, which `ptr_eq` states exactly.
        assert!(Rc::ptr_eq(&by_kw, &by_count));
    }

    // -----------------------------------------------------------------------
    // Numbers, stated without naming one.
    // -----------------------------------------------------------------------

    /// The identity the numeric tests rest on: a segment fully covered by `n` sessions each drawing
    /// exactly one EV's real power is indistinguishable from `n` vehicles charging.
    ///
    /// Both derivations must land on `site_load(n)` — the count-based one because it counts them,
    /// the energy-based one because it divides their aggregate power by exactly the figure each was
    /// given. That the two agree is the point: they reach the same answer down two different
    /// routes, and neither route mentions a constant by value.
    #[test]
    fn full_segments_of_nominal_vehicles_agree_with_the_site_load_model() {
        for n in 0..=3u32 {
            let sessions: Vec<RSession> = (0..n)
                .map(|i| {
                    // Spanning the whole hour, so the quarter is covered end to end and the
                    // overlap bracket is exact.
                    session(
                        &format!("S{i}"),
                        "2026-06-15T19:00:00Z",
                        "2026-06-15T22:00:00Z",
                        ev_load().real_kw,
                    )
                })
                .collect();
            let segments = segments_for_ioi(hour(), &sessions);
            let expected = site_load(n);

            for segment in &segments {
                let count = segment.agg_count();
                assert!((count.min - f64::from(n)).abs() < TOLERANCE, "n={n}");
                assert!((count.max - f64::from(n)).abs() < TOLERANCE, "n={n}");

                let est = segment_estimate(segment);
                for figure in [est.energy_based_kw, est.count_based_kw] {
                    assert!((figure.min - expected.real_kw).abs() < TOLERANCE, "n={n}");
                    assert!((figure.max - expected.real_kw).abs() < TOLERANCE, "n={n}");
                }
                for figure in [est.energy_based_kva, est.count_based_kva] {
                    assert!(
                        (figure.min - expected.apparent_kva()).abs() < TOLERANCE,
                        "n={n}"
                    );
                    assert!(
                        (figure.max - expected.apparent_kva()).abs() < TOLERANCE,
                        "n={n}"
                    );
                }
            }
        }
    }

    /// Part of a segment's worth of vehicles is that part of a segment's worth of count, and the
    /// energy-based figure follows it exactly.
    ///
    /// The same identity as above read at a fractional count, which is what a segment ordinarily
    /// holds: `site_load` takes a whole number of vehicles, so the check is that the two
    /// derivations still agree with each other rather than against a third figure. A third rather
    /// than a half, for the reason given above: halves are off the time grid.
    #[test]
    fn the_two_derivations_agree_on_a_partially_covered_segment() {
        let sessions = vec![session(
            "P",
            "2026-06-15T20:00:00Z",
            "2026-06-15T20:05:00Z",
            ev_load().real_kw,
        )];
        let segments = segments_for_ioi(hour(), &sessions);
        let est = segment_estimate(&segments[0]);

        assert!((segments[0].agg_count().max - 1.0 / 3.0).abs() < TOLERANCE);
        assert!(
            (est.energy_based_kw.max - est.count_based_kw.max).abs() < TOLERANCE,
            "{:?} vs {:?}",
            est.energy_based_kw,
            est.count_based_kw
        );
        assert!(
            (est.energy_based_kva.max - est.count_based_kva.max).abs() < TOLERANCE,
            "{:?} vs {:?}",
            est.energy_based_kva,
            est.count_based_kva
        );
    }

    /// A session reaching no segment of the interval contributes nothing, and measuring its
    /// non-overlap does not panic.
    ///
    /// `SessionOverlap::empty()` used to build `(MAX,MAX)/(MIN,MIN)`, so `duration()` called
    /// `duration(MAX, MIN)` and panicked. The `Option` is the type-level backstop; the filter in
    /// `segments_for_ioi` is what ordinarily keeps such a session out.
    #[test]
    fn a_session_outside_the_interval_contributes_nothing() {
        let elsewhere = session("X", "2026-06-16T20:00:00Z", "2026-06-16T21:00:00Z", 5.0);
        let first = Interval::from_start_end(hour().start, hour().start + SEGMENT_DURATION);

        assert!(elsewhere.interval_overlap(&first).is_none());
        let ratio = elsewhere.interval_overlap_ratio(&first);
        assert_eq!(ratio.min, 0.0);
        assert_eq!(ratio.max, 0.0);

        // And it never reaches a segment in the first place.
        let segments = segments_for_ioi(hour(), &[elsewhere]);
        assert!(segments.iter().all(|s| s.sessions.is_empty()));
    }
}
