//! Reading the ESPI (Green Button) Atom feed.
//!
//! The feed is a flat list of `<entry>` elements. The hierarchy is not nested: it is reconstructed
//! from the `rel="self"` and `rel="related"` links, which is what this module does.
//!
//! ```text
//! IntervalBlock --rel=related--> MeterReading --rel=related--> ReadingType
//!                                                              (uom, powerOfTenMultiplier)
//! ```
//!
//! The Python this replaces took a shortcut: it read the last path segment of each ReadingType's
//! self-href as a key, and separately dug the same token out of each IntervalBlock's self-href.
//! That works only because Toronto Hydro happens to give a MeterReading and its ReadingType the
//! same identifier. Nothing in ESPI requires that, so this follows the links instead — a feed from
//! another utility either resolves or says which link is missing.
//!
//! Series are told apart by `uom`, the unit of measure. `kind` is not usable: kWh and kVA both
//! carry `kind=12` in this feed.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;

use jiff::Timestamp;
use roxmltree::{Document, Node};

use crate::{Anomaly, Reading};

const ATOM_NS: &str = "http://www.w3.org/2005/Atom";
const ESPI_NS: &str = "http://naesb.org/espi";

/// Unit-of-measure codes: watt-hours, watts, volt-amperes.
const UOM_KWH: &str = "72";
const UOM_KW: &str = "38";
const UOM_KVA: &str = "61";

/// Every reading in this feed covers one hour, and the whole design downstream depends on it —
/// the interval count that drives the red highlight, and the guarantee that an interval cannot
/// straddle a TOU boundary. So it is checked rather than assumed.
const INTERVAL_SECS: i64 = 3600;

/// One measurement series, in raw source integers.
///
/// Values stay integral all the way to the cell. See [`Series::divisor`].
#[derive(Debug, Clone)]
pub struct Series {
    pub values: BTreeMap<Timestamp, i64>,
    pub power_of_ten: i32,
    /// Interval starts that appeared more than once within this series.
    pub duplicates: BTreeSet<Timestamp>,
}

impl Series {
    /// What a raw value must be divided by to give kWh, kW or kVA.
    ///
    /// The feed reports, say, watt-hours scaled by `powerOfTenMultiplier`; the workbook wants
    /// kilowatt-hours. Both conversions fold into one integer divisor applied at cell-write time.
    pub fn divisor(&self) -> f64 {
        10f64.powi(3 - self.power_of_ten)
    }
}

/// The three series a Toronto Hydro export carries.
#[derive(Debug, Clone)]
pub struct Feed {
    pub kwh: Series,
    pub kw: Series,
    pub kva: Series,
}

/// Hourly rows assembled from a [`Feed`], with whatever is wrong with each one.
#[derive(Debug, Clone)]
pub struct Readings {
    /// Ascending by interval start, one row per hour, including placeholder rows for hours the
    /// feed skipped.
    pub rows: Vec<Reading>,
    pub anomalies: BTreeMap<Timestamp, BTreeSet<Anomaly>>,
}

/// Parses an ESPI feed.
///
/// # Errors
///
/// Returns an error if the XML is malformed, if any of the three series is absent, if a link
/// needed to attribute an IntervalBlock to a series is missing or dangling, or if any reading
/// covers something other than one hour.
pub fn parse(xml: &str) -> Result<Feed, Box<dyn Error>> {
    let doc = Document::parse(xml)?;
    let entries: Vec<Node> = doc
        .root_element()
        .children()
        .filter(|n| n.is_element() && named(*n, ATOM_NS, "entry"))
        .collect();

    // Pass 1: the ReadingTypes, which is where uom and the scaling live.
    let mut reading_types: HashMap<&str, (&str, i32)> = HashMap::new();
    for entry in &entries {
        let Some(rt) = content_of(*entry, "ReadingType") else {
            continue;
        };
        let href = link_href(*entry, "self")
            .ok_or("a ReadingType entry has no rel=\"self\" link to identify it by")?;
        let uom = espi_text(rt, "uom").ok_or("a ReadingType has no uom")?;
        let power_of_ten: i32 = espi_text(rt, "powerOfTenMultiplier")
            .ok_or("a ReadingType has no powerOfTenMultiplier")?
            .parse()?;
        let interval_length: i64 = espi_text(rt, "intervalLength")
            .ok_or("a ReadingType has no intervalLength")?
            .parse()?;
        if interval_length != INTERVAL_SECS {
            return Err(format!(
                "ReadingType {href} declares {interval_length}s intervals; this tool assumes \
                 hourly data throughout"
            )
            .into());
        }
        reading_types.insert(href, (uom, power_of_ten));
    }

    // Pass 2: each MeterReading names its ReadingType through a related link.
    let mut meter_readings: HashMap<&str, &str> = HashMap::new();
    for entry in &entries {
        if content_of(*entry, "MeterReading").is_none() {
            continue;
        }
        let href = link_href(*entry, "self")
            .ok_or("a MeterReading entry has no rel=\"self\" link to identify it by")?;
        let reading_type = related_hrefs(*entry)
            .find(|h| reading_types.contains_key(h))
            .ok_or_else(|| format!("MeterReading {href} links to no ReadingType in this feed"))?;
        meter_readings.insert(href, reading_type);
    }

    // Pass 3: the readings themselves.
    let mut series: HashMap<&str, Series> = HashMap::new();
    for entry in &entries {
        let Some(block) = content_of(*entry, "IntervalBlock") else {
            continue;
        };
        let meter_reading = related_hrefs(*entry)
            .find(|h| meter_readings.contains_key(h))
            .ok_or("an IntervalBlock links to no MeterReading in this feed")?;
        let reading_type = meter_readings[meter_reading];
        let (uom, power_of_ten) = reading_types[reading_type];

        let entry_series = series.entry(uom).or_insert_with(|| Series {
            values: BTreeMap::new(),
            power_of_ten,
            duplicates: BTreeSet::new(),
        });

        for reading in espi_children(block, "IntervalReading") {
            let period =
                espi_child(reading, "timePeriod").ok_or("an IntervalReading has no timePeriod")?;
            let duration: i64 = espi_text(period, "duration")
                .ok_or("an IntervalReading has no timePeriod/duration")?
                .parse()?;
            if duration != INTERVAL_SECS {
                return Err(format!(
                    "an IntervalReading covers {duration}s; this tool assumes hourly data"
                )
                .into());
            }
            let start: i64 = espi_text(period, "start")
                .ok_or("an IntervalReading has no timePeriod/start")?
                .parse()?;
            let value: i64 = espi_text(reading, "value")
                .ok_or("an IntervalReading has no value")?
                .parse()?;
            let at = Timestamp::from_second(start)?;
            if entry_series.values.insert(at, value).is_some() {
                entry_series.duplicates.insert(at);
            }
        }
    }

    let mut take = |uom: &str, name: &str| -> Result<Series, Box<dyn Error>> {
        series.remove(uom).ok_or_else(|| {
            format!("the feed carries no {name} series (uom {uom}); all three are required").into()
        })
    };
    Ok(Feed {
        kwh: take(UOM_KWH, "kWh")?,
        kw: take(UOM_KW, "kW")?,
        kva: take(UOM_KVA, "kVA")?,
    })
}

impl Feed {
    /// Assembles the three series into hourly rows.
    ///
    /// Rows come from the **union** of the three series' timestamps, not from the kWh series
    /// alone. The Python iterated kWh and filled a missing companion with zero, which cannot raise
    /// a maximum but does write a false `0.000` into the "kVA at interval" columns — and made a
    /// timestamp carrying kW but no kWh invisible entirely.
    ///
    /// Hours that no series carried, but that fall inside the span the feed covers, become
    /// placeholder rows carrying [`Anomaly::MissingInterval`], so a gap is something you can see
    /// in the sheet rather than a row you would have to notice is absent.
    pub fn readings(&self) -> Readings {
        let mut anomalies: BTreeMap<Timestamp, BTreeSet<Anomaly>> = BTreeMap::new();
        let mut note = |at: Timestamp, a: Anomaly| {
            anomalies.entry(at).or_default().insert(a);
        };

        let mut starts: BTreeSet<Timestamp> = BTreeSet::new();
        for s in [&self.kwh, &self.kw, &self.kva] {
            starts.extend(s.values.keys().copied());
            for at in &s.duplicates {
                note(*at, Anomaly::DuplicateInterval);
            }
        }

        let mut rows: Vec<Reading> = Vec::with_capacity(starts.len());
        let mut previous: Option<Timestamp> = None;
        for at in starts {
            // Fill the hours between the last row and this one, if the feed skipped any.
            if let Some(prev) = previous {
                let mut gap = prev.as_second() + INTERVAL_SECS;
                while gap < at.as_second() {
                    let missing = Timestamp::from_second(gap).expect("inside the feed's own span");
                    note(missing, Anomaly::MissingInterval);
                    rows.push(Reading {
                        start: missing,
                        kwh: None,
                        kw: None,
                        kva: None,
                    });
                    gap += INTERVAL_SECS;
                }
            }
            previous = Some(at);

            if at.as_second().rem_euclid(INTERVAL_SECS) != 0 {
                note(at, Anomaly::MisalignedInterval);
            }
            let reading = Reading {
                start: at,
                kwh: self.kwh.values.get(&at).copied(),
                kw: self.kw.values.get(&at).copied(),
                kva: self.kva.values.get(&at).copied(),
            };
            if reading.kwh.is_none() {
                note(at, Anomaly::MissingKwh);
            }
            if reading.kw.is_none() {
                note(at, Anomaly::MissingKw);
            }
            if reading.kva.is_none() {
                note(at, Anomaly::MissingKva);
            }
            rows.push(reading);
        }

        Readings { rows, anomalies }
    }
}

fn named(node: Node, namespace: &str, name: &str) -> bool {
    node.tag_name().namespace() == Some(namespace) && node.tag_name().name() == name
}

/// The ESPI payload of an entry, when it is of the named kind.
fn content_of<'a>(entry: Node<'a, 'a>, kind: &str) -> Option<Node<'a, 'a>> {
    entry
        .children()
        .find(|c| c.is_element() && named(*c, ATOM_NS, "content"))?
        .children()
        .find(|c| c.is_element() && named(*c, ESPI_NS, kind))
}

fn espi_children<'a>(node: Node<'a, 'a>, name: &'a str) -> impl Iterator<Item = Node<'a, 'a>> {
    node.children()
        .filter(move |c| c.is_element() && named(*c, ESPI_NS, name))
}

fn espi_child<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    node.children()
        .find(|c| c.is_element() && named(*c, ESPI_NS, name))
}

/// Text of a direct ESPI child. Direct rather than descendant on purpose: an `IntervalBlock`
/// carries its own `<espi:interval>` with `duration` and `start` children named exactly like the
/// ones inside each reading's `timePeriod`.
fn espi_text<'a>(node: Node<'a, 'a>, name: &str) -> Option<&'a str> {
    espi_child(node, name)?.text()
}

fn link_href<'a>(entry: Node<'a, 'a>, rel: &str) -> Option<&'a str> {
    entry
        .children()
        .find(|c| c.is_element() && named(*c, ATOM_NS, "link") && c.attribute("rel") == Some(rel))?
        .attribute("href")
}

fn related_hrefs<'a>(entry: Node<'a, 'a>) -> impl Iterator<Item = &'a str> {
    entry
        .children()
        .filter(|c| {
            c.is_element() && named(*c, ATOM_NS, "link") && c.attribute("rel") == Some("related")
        })
        .filter_map(|c| c.attribute("href"))
}

// cargo test --package green-button --lib -- espi::test --nocapture
#[cfg(test)]
mod test {
    use super::*;

    /// A minimal feed with the same link shape as the real export: two series, one block each.
    fn feed_xml(interval_length: &str, reading_duration: &str) -> String {
        format!(
            r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <content><espi:ReadingType xmlns:espi="http://naesb.org/espi">
      <espi:intervalLength>{interval_length}</espi:intervalLength>
      <espi:powerOfTenMultiplier>-3</espi:powerOfTenMultiplier>
      <espi:uom>72</espi:uom>
    </espi:ReadingType></content>
    <link rel="self" href="rt/kwh"/>
  </entry>
  <entry>
    <content><espi:ReadingType xmlns:espi="http://naesb.org/espi">
      <espi:intervalLength>3600</espi:intervalLength>
      <espi:powerOfTenMultiplier>-3</espi:powerOfTenMultiplier>
      <espi:uom>38</espi:uom>
    </espi:ReadingType></content>
    <link rel="self" href="rt/kw"/>
  </entry>
  <entry>
    <content><espi:ReadingType xmlns:espi="http://naesb.org/espi">
      <espi:intervalLength>3600</espi:intervalLength>
      <espi:powerOfTenMultiplier>-3</espi:powerOfTenMultiplier>
      <espi:uom>61</espi:uom>
    </espi:ReadingType></content>
    <link rel="self" href="rt/kva"/>
  </entry>
  {meter_readings}
  {blocks}
</feed>"#,
            meter_readings = ["kwh", "kw", "kva"]
                .map(|s| format!(
                    r#"<entry><content><espi:MeterReading xmlns:espi="http://naesb.org/espi"/></content>
                       <link rel="self" href="mr/{s}"/><link rel="related" href="rt/{s}"/></entry>"#
                ))
                .join("\n"),
            blocks = ["kwh", "kw", "kva"]
                .map(|s| format!(
                    r#"<entry><content><espi:IntervalBlock xmlns:espi="http://naesb.org/espi">
                         <espi:interval><espi:duration>86400</espi:duration><espi:start>0</espi:start></espi:interval>
                         <espi:IntervalReading>
                           <espi:timePeriod><espi:duration>{reading_duration}</espi:duration><espi:start>1732338000</espi:start></espi:timePeriod>
                           <espi:value>100</espi:value>
                         </espi:IntervalReading>
                         <espi:IntervalReading>
                           <espi:timePeriod><espi:duration>3600</espi:duration><espi:start>1732341600</espi:start></espi:timePeriod>
                           <espi:value>200</espi:value>
                         </espi:IntervalReading>
                       </espi:IntervalBlock></content>
                       <link rel="self" href="ib/{s}/1"/><link rel="related" href="mr/{s}"/></entry>"#
                ))
                .join("\n"),
        )
    }

    #[test]
    fn the_link_chain_attributes_each_block_to_its_series() {
        let feed = parse(&feed_xml("3600", "3600")).unwrap();
        assert_eq!(feed.kwh.values.len(), 2);
        assert_eq!(feed.kw.values.len(), 2);
        assert_eq!(feed.kva.values.len(), 2);
        assert_eq!(feed.kwh.power_of_ten, -3);
        assert_eq!(feed.kwh.divisor(), 1_000_000.0);
    }

    /// The block carries its own `<espi:interval>` with `duration` and `start` children named just
    /// like the ones inside a reading. Reading descendants rather than direct children would pick
    /// up the block's 86400 and reject the feed.
    #[test]
    fn the_blocks_own_interval_is_not_mistaken_for_a_readings_time_period() {
        let feed = parse(&feed_xml("3600", "3600")).unwrap();
        assert!(
            feed.kwh
                .values
                .contains_key(&Timestamp::from_second(1732338000).unwrap())
        );
    }

    #[test]
    fn a_non_hourly_reading_type_is_rejected() {
        let err = parse(&feed_xml("900", "3600")).unwrap_err().to_string();
        assert!(err.contains("900s intervals"), "{err}");
    }

    #[test]
    fn a_non_hourly_reading_is_rejected() {
        let err = parse(&feed_xml("3600", "1800")).unwrap_err().to_string();
        assert!(err.contains("covers 1800s"), "{err}");
    }

    #[test]
    fn a_missing_series_is_rejected() {
        let xml = feed_xml("3600", "3600")
            .replace(r#"<espi:uom>61</espi:uom>"#, "<espi:uom>9</espi:uom>");
        let err = parse(&xml).unwrap_err().to_string();
        assert!(err.contains("no kVA series"), "{err}");
    }

    /// A hole in the middle of the data becomes a visible placeholder row, not a silently absent
    /// one.
    #[test]
    fn a_gap_becomes_a_placeholder_row() {
        let feed = parse(&feed_xml("3600", "3600")).unwrap();
        let mut feed = feed;
        // Drop the second hour from every series, then add a fourth hour, leaving a two-hour hole.
        let hole = Timestamp::from_second(1732341600).unwrap();
        let later = Timestamp::from_second(1732349000 - 200).unwrap(); // 1732348800, three hours on
        for s in [&mut feed.kwh, &mut feed.kw, &mut feed.kva] {
            s.values.remove(&hole);
            s.values.insert(later, 300);
        }
        let readings = feed.readings();
        let starts: Vec<i64> = readings.rows.iter().map(|r| r.start.as_second()).collect();
        assert_eq!(starts, vec![1732338000, 1732341600, 1732345200, 1732348800]);
        assert!(readings.rows[1].is_empty());
        assert!(readings.rows[2].is_empty());
        assert!(readings.anomalies[&hole].contains(&Anomaly::MissingInterval));
    }

    /// A timestamp present in one series and not another is reported rather than zero-filled.
    #[test]
    fn a_missing_companion_is_reported_not_zeroed() {
        let mut feed = parse(&feed_xml("3600", "3600")).unwrap();
        let at = Timestamp::from_second(1732338000).unwrap();
        feed.kw.values.remove(&at);
        let readings = feed.readings();
        assert_eq!(readings.rows[0].kw, None);
        assert!(readings.anomalies[&at].contains(&Anomaly::MissingKw));
        assert!(!readings.anomalies[&at].contains(&Anomaly::MissingKwh));
    }
}
