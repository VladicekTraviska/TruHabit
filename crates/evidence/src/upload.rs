//! Untrusted, manually supplied activities. Parsing never authenticates a runner.
use chrono::{DateTime, Timelike, Utc};
use fitparser::{FitDataRecord, Value, profile::MesgNum};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;

pub const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_POINTS: usize = 100_000;
const GPX: &str = "http://www.topografix.com/GPX/1/1";
const SENSOR_POLICY: &str = "manual-running-plausibility-v3";
// Bounds are input/resource limits, not an accusation about an ultrarunner.
const MAX_ELAPSED_SECONDS: f64 = 7.0 * 86_400.0;
const CLOSE_SAMPLE_SECONDS: f64 = 30.0;
const FAST_WINDOW_SECONDS: f64 = 30.0;
const FAST_SPEED_MPS: f64 = 8.0;
const SUSTAINED_FAST_SECONDS: f64 = 120.0;
// Conservative file-consistency heuristics, not physiology or runner identity evidence.
const RUNNING_MISMATCH_SPEED_MPS: f64 = 4.0;
const RUNNING_MISMATCH_HR_BPM: f64 = 100.0;
const SUSTAINED_SIGNAL_MISMATCH_SECONDS: f64 = 120.0;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct UploadError(pub &'static str);
type Result<T> = std::result::Result<T, UploadError>;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    pub format: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub distance_m: f64,
    pub elapsed_seconds: f64,
    pub sample_count: usize,
    pub heart_rate_samples: usize,
    pub cadence_samples: usize,
    pub max_speed_mps: f64,
    pub reasons: Vec<String>,
    pub fingerprint: String,
    pub source_authenticity: String,
    /// Aggregate sensor observations only. Existing stored activities can omit this field.
    #[serde(default)]
    pub telemetry: TelemetrySummary,
    #[serde(default)]
    pub checks: Vec<SensorCheck>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SensorMetric {
    pub samples: usize,
    pub missing_samples: usize,
    pub zero_samples: usize,
    pub out_of_range_samples: usize,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Arithmetic sample mean, including recorded zeros; not a time-weighted medical metric.
    pub mean: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelemetrySummary {
    pub policy_version: String,
    pub distance_source: String,
    pub gps_samples: usize,
    pub distance_samples: usize,
    pub speed_samples: usize,
    pub heart_rate: SensorMetric,
    /// Raw source cadence. Never guess whether a source has already counted both feet.
    pub cadence: SensorMetric,
    pub cadence_unit: String,
    pub gps_distance_m: Option<f64>,
    pub recorded_distance_m: Option<f64>,
    pub reported_distance_m: Option<f64>,
    pub reported_elapsed_seconds: Option<f64>,
    pub reported_timer_seconds: Option<f64>,
    pub timed_coverage_seconds: f64,
    pub timed_coverage_ratio: f64,
    pub max_sample_gap_seconds: f64,
    pub median_sample_interval_seconds: f64,
    pub sample_gaps: usize,
    pub segment_breaks: usize,
    pub implausible_speed_segments: usize,
    pub gps_jump_segments: usize,
    pub longest_fast_window_seconds: f64,
    /// Contiguous measured movement with recorded zero cadence and positive HR below 100 bpm.
    /// Older policy summaries omit this aggregate and retain their stored policy/checks.
    #[serde(default)]
    pub longest_running_signal_mismatch_seconds: f64,
    pub integrated_speed_distance_m: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorCheck {
    pub code: String,
    /// pass | limited | review | unverified. A pass concerns plausibility, never identity.
    pub outcome: String,
    pub detail: String,
}

#[derive(Clone)]
struct Point {
    at: DateTime<Utc>,
    lat: Option<f64>,
    lon: Option<f64>,
    distance: Option<f64>,
    speed: Option<f64>,
    hr: Option<f64>,
    cadence: Option<f64>,
    segment: usize,
}

struct SessionSummary {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    distance: Option<f64>,
    elapsed: Option<f64>,
    timer: Option<f64>,
}
pub fn parse(bytes: &[u8], session: Option<usize>) -> Result<Activity> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(UploadError("UPLOAD_SIZE"));
    }
    if bytes.get(8..12) == Some(b".FIT") {
        fit(bytes, session)
    } else {
        if session.is_some() {
            return Err(UploadError("SESSION_NOT_APPLICABLE"));
        }
        gpx(bytes)
    }
}
fn time(text: &str) -> Result<DateTime<Utc>> {
    let value = DateTime::parse_from_rfc3339(text.trim())
        .map_err(|_| UploadError("INVALID_ACTIVITY_TIME"))?;
    if value.nanosecond() >= 1_000_000_000 {
        return Err(UploadError("INVALID_ACTIVITY_TIME"));
    }
    Ok(value.with_timezone(&Utc))
}
fn number(text: &str) -> Result<f64> {
    let value = text
        .trim()
        .parse::<f64>()
        .map_err(|_| UploadError("INVALID_ACTIVITY_NUMBER"))?;
    if !value.is_finite() {
        return Err(UploadError("INVALID_ACTIVITY_NUMBER"));
    }
    Ok(value)
}

fn gpx_sensor(point: roxmltree::Node<'_, '_>, name: &str) -> Result<Option<f64>> {
    let mut values = point
        .children()
        .filter(|node| node.has_tag_name((GPX, "extensions")))
        .flat_map(|node| node.descendants())
        .filter(|node| {
            node.tag_name().name() == name
                && matches!(
                    node.tag_name().namespace(),
                    Some("http://www.garmin.com/xmlschemas/TrackPointExtension/v1")
                        | Some("http://www.garmin.com/xmlschemas/TrackPointExtension/v2")
                )
        });
    let value = values
        .next()
        .map(|node| number(node.text().unwrap_or("")))
        .transpose()?;
    if values.next().is_some() {
        return Err(UploadError("DUPLICATE_ACTIVITY_SENSOR"));
    }
    Ok(value)
}
fn gpx(bytes: &[u8]) -> Result<Activity> {
    let text = std::str::from_utf8(bytes).map_err(|_| UploadError("EXPECTED_GPX_OR_FIT"))?;
    if text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
        return Err(UploadError("XML_DTD_FORBIDDEN"));
    }
    let document = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 1_000_000,
            entity_resolver: None,
        },
    )
    .map_err(|_| UploadError("INVALID_GPX"))?;
    let root = document.root_element();
    if !root.has_tag_name((GPX, "gpx")) || root.attribute("version") != Some("1.1") {
        return Err(UploadError("EXPECTED_GPX_1_1"));
    }
    let tracks: Vec<_> = root
        .children()
        .filter(|n| n.has_tag_name((GPX, "trk")))
        .collect();
    if tracks.len() != 1 {
        return Err(UploadError("EXPORT_ONE_TRACK"));
    }
    let mut points = Vec::new();
    for (segment, node) in tracks[0]
        .children()
        .filter(|n| n.has_tag_name((GPX, "trkseg")))
        .enumerate()
    {
        for pt in node.children().filter(|n| n.has_tag_name((GPX, "trkpt"))) {
            if points.len() >= MAX_POINTS {
                return Err(UploadError("TOO_MANY_SAMPLES"));
            }
            let lat = number(
                pt.attribute("lat")
                    .ok_or(UploadError("MISSING_COORDINATES"))?,
            )?;
            let lon = number(
                pt.attribute("lon")
                    .ok_or(UploadError("MISSING_COORDINATES"))?,
            )?;
            if lat.abs() > 90.0 || lon.abs() > 180.0 {
                return Err(UploadError("INVALID_COORDINATES"));
            }
            let times: Vec<_> = pt
                .children()
                .filter(|n| n.has_tag_name((GPX, "time")))
                .collect();
            if times.len() != 1 {
                return Err(UploadError("TIMED_TRACK_REQUIRED"));
            }
            points.push(Point {
                at: time(times[0].text().unwrap_or(""))?,
                lat: Some(lat),
                lon: Some(lon),
                distance: None,
                speed: gpx_sensor(pt, "speed")?,
                hr: gpx_sensor(pt, "hr")?,
                cadence: gpx_sensor(pt, "cad")?,
                segment,
            });
        }
    }
    summarize("GPX", points, None)
}
fn field<'a>(r: &'a FitDataRecord, name: &str) -> Option<&'a Value> {
    r.fields()
        .iter()
        .find(|f| f.name() == name)
        .map(|f| f.value())
}
fn numeric(r: &FitDataRecord, name: &str) -> Option<f64> {
    let v = field(r, name)?;
    if matches!(
        v,
        Value::String(_) | Value::Array(_) | Value::Timestamp(_) | Value::Invalid
    ) {
        return None;
    }
    v.to_string().parse::<f64>().ok().filter(|v| v.is_finite())
}
fn timestamp(r: &FitDataRecord, name: &str) -> Option<DateTime<Utc>> {
    match field(r, name)? {
        Value::Timestamp(v) => Some(v.with_timezone(&Utc)),
        _ => None,
    }
}
fn fit(bytes: &[u8], selected: Option<usize>) -> Result<Activity> {
    // Reject concatenated/trailing files instead of silently accepting their first activity.
    let header = usize::from(bytes[0]);
    if !matches!(header, 12 | 14) || bytes.len() < header + 2 {
        return Err(UploadError("INVALID_FIT"));
    }
    let size = u32::from_le_bytes(
        bytes[4..8]
            .try_into()
            .map_err(|_| UploadError("INVALID_FIT"))?,
    ) as usize;
    if size.checked_add(header + 2) != Some(bytes.len()) {
        return Err(UploadError("INVALID_FIT_LENGTH"));
    }
    let records =
        fitparser::from_bytes(bytes).map_err(|_| UploadError("INVALID_FIT_CRC_OR_FORMAT"))?;
    if records.len() > 200_000 {
        return Err(UploadError("TOO_MANY_SAMPLES"));
    }
    let sessions: Vec<_> = records
        .iter()
        .filter(|r| r.kind() == MesgNum::Session)
        .collect();
    if sessions.is_empty() {
        return Err(UploadError("FIT_SESSION_REQUIRED"));
    }
    if sessions.len() > 1 && selected.is_none() {
        return Err(UploadError("SELECT_FIT_SESSION"));
    }
    let s = *sessions
        .get(selected.unwrap_or(0))
        .ok_or(UploadError("INVALID_FIT_SESSION"))?;
    let start = timestamp(s, "start_time").ok_or(UploadError("INVALID_ACTIVITY_TIME"))?;
    let end = timestamp(s, "timestamp").ok_or(UploadError("INVALID_ACTIVITY_TIME"))?;
    if end <= start {
        return Err(UploadError("INVALID_ACTIVITY_TIME"));
    }
    if !matches!(field(s,"sport"),Some(Value::String(v)) if v=="running") {
        return Err(UploadError("RUNNING_SESSION_REQUIRED"));
    }
    let mut points = Vec::new();
    for r in records.iter().filter(|r| r.kind() == MesgNum::Record) {
        let at = timestamp(r, "timestamp").ok_or(UploadError("INVALID_ACTIVITY_TIME"))?;
        if at < start || at > end {
            continue;
        }
        if points.len() >= MAX_POINTS {
            return Err(UploadError("TOO_MANY_SAMPLES"));
        }
        let coordinate = |name| numeric(r, name).map(|v| v * 180.0 / 2_147_483_648.0);
        let lat = coordinate("position_lat");
        let lon = coordinate("position_long");
        if lat.is_some_and(|v| !(-90.0..=90.0).contains(&v))
            || lon.is_some_and(|v| !(-180.0..=180.0).contains(&v))
            || lat.is_some() != lon.is_some()
        {
            return Err(UploadError("INVALID_COORDINATE"));
        }
        points.push(Point {
            at,
            lat,
            lon,
            distance: numeric(r, "distance"),
            speed: numeric(r, "enhanced_speed").or_else(|| numeric(r, "speed")),
            hr: numeric(r, "heart_rate"),
            cadence: numeric(r, "cadence"),
            segment: 0,
        });
    }
    summarize(
        "FIT",
        points,
        Some(SessionSummary {
            start,
            end,
            distance: numeric(s, "total_distance"),
            elapsed: numeric(s, "total_elapsed_time"),
            timer: numeric(s, "total_timer_time"),
        }),
    )
}
fn metres(a: &Point, b: &Point) -> Option<f64> {
    let (lat1, lon1, lat2, lon2) = (a.lat?, a.lon?, b.lat?, b.lon?);
    let h = ((lat2 - lat1).to_radians() / 2.0).sin().powi(2)
        + lat1.to_radians().cos()
            * lat2.to_radians().cos()
            * ((lon2 - lon1).to_radians() / 2.0).sin().powi(2);
    Some(6_371_000.0 * 2.0 * h.clamp(0.0, 1.0).sqrt().asin())
}
fn summarize(
    format: &str,
    points: Vec<Point>,
    session: Option<SessionSummary>,
) -> Result<Activity> {
    if points.len() < 2 {
        return Err(UploadError("TIMED_SAMPLES_REQUIRED"));
    }
    let start = points[0].at;
    let end = points
        .last()
        .ok_or(UploadError("TIMED_SAMPLES_REQUIRED"))?
        .at;
    let elapsed = (end - start).num_milliseconds() as f64 / 1000.0;
    if elapsed <= 0.0 || elapsed > MAX_ELAPSED_SECONDS {
        return Err(UploadError("INVALID_ACTIVITY_DURATION"));
    }
    for point in &points {
        for (value, maximum) in [
            (point.distance, 1_000_000.0),
            (point.speed, 1_000.0),
            (point.hr, 1_000.0),
            (point.cadence, 10_000.0),
        ] {
            if value.is_some_and(|v| !v.is_finite() || !(0.0..=maximum).contains(&v)) {
                return Err(UploadError("INVALID_ACTIVITY_METRIC"));
            }
        }
    }
    let mut previous_distance = None;
    for point in &points {
        if let Some(distance) = point.distance {
            if previous_distance.is_some_and(|previous| distance < previous) {
                return Err(UploadError("DECREASING_DISTANCE"));
            }
            previous_distance = Some(distance);
        }
    }
    let mut distance = 0.0;
    let mut max_speed = points
        .iter()
        .filter_map(|point| point.speed)
        .fold(0.0, f64::max);
    let mut gps_distance = 0.0;
    let mut gps_segments = 0usize;
    let mut distance_segments = 0usize;
    let mut recorded_distance = 0.0;
    let mut sample_intervals = Vec::with_capacity(points.len() - 1);
    let mut timed_coverage = 0.0;
    let mut covered_distance = 0.0;
    let mut sample_gaps = 0usize;
    let mut segment_breaks = 0usize;
    let mut implausible_speed_segments = 0usize;
    let mut gps_jump_segments = 0usize;
    let mut integrated_speed_distance = 0.0;
    let mut speed_covered_distance = 0.0;
    let mut integrated_speed_seconds = 0.0;
    let mut fast_window: VecDeque<(f64, f64)> = VecDeque::new();
    let mut window_seconds = 0.0;
    let mut window_distance = 0.0;
    let mut consecutive_fast_seconds: f64 = 0.0;
    let mut longest_fast_seconds: f64 = 0.0;
    let mut consecutive_signal_mismatch_seconds: f64 = 0.0;
    let mut longest_signal_mismatch_seconds: f64 = 0.0;
    for pair in points.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let seconds = (b.at - a.at).num_milliseconds() as f64 / 1000.0;
        if seconds <= 0.0 {
            return Err(UploadError("UNORDERED_ACTIVITY_TIME"));
        }
        sample_intervals.push(seconds);
        sample_gaps += usize::from(seconds > 60.0);
        segment_breaks += usize::from(a.segment != b.segment);
        let gps = if a.segment == b.segment {
            metres(a, b)
        } else {
            None
        };
        if let Some(v) = gps {
            gps_distance += v;
            gps_segments += 1;
            if v > 100.0 && v / seconds > 20.0 {
                gps_jump_segments += 1;
            }
        }
        let d = match (a.distance, b.distance) {
            (Some(x), Some(y)) if x >= 0.0 && y >= x => {
                distance_segments += 1;
                recorded_distance += y - x;
                y - x
            }
            (Some(_), Some(_)) => return Err(UploadError("DECREASING_DISTANCE")),
            _ => gps.unwrap_or(0.0),
        };
        distance += d;
        let speed = d / seconds;
        max_speed = max_speed.max(speed).max(gps.unwrap_or(0.0) / seconds);
        if speed > 12.0
            || gps.is_some_and(|v| v / seconds > 12.0)
            || a.speed.is_some_and(|v| v > 12.0)
            || b.speed.is_some_and(|v| v > 12.0)
        {
            implausible_speed_segments += 1;
        }
        let well_sampled = seconds <= CLOSE_SAMPLE_SECONDS
            && a.segment == b.segment
            && (gps.is_some() || (a.distance.is_some() && b.distance.is_some()));
        // Require every interval and both sensor endpoints. Missing/zero HR is not a
        // resting pulse, and gaps or segment breaks never bridge a mismatch period.
        if well_sampled
            && speed > RUNNING_MISMATCH_SPEED_MPS
            && a.cadence == Some(0.0)
            && b.cadence == Some(0.0)
            && a.hr
                .is_some_and(|hr| hr > 0.0 && hr < RUNNING_MISMATCH_HR_BPM)
            && b.hr
                .is_some_and(|hr| hr > 0.0 && hr < RUNNING_MISMATCH_HR_BPM)
        {
            consecutive_signal_mismatch_seconds += seconds;
            longest_signal_mismatch_seconds =
                longest_signal_mismatch_seconds.max(consecutive_signal_mismatch_seconds);
        } else {
            consecutive_signal_mismatch_seconds = 0.0;
        }
        if well_sampled {
            timed_coverage += seconds;
            covered_distance += d;
            if let (Some(left), Some(right)) = (a.speed, b.speed) {
                integrated_speed_distance += (left + right) / 2.0 * seconds;
                integrated_speed_seconds += seconds;
                speed_covered_distance += d;
            }
            // Thirty-second distance windows suppress ordinary one-second GPS jitter.
            fast_window.push_back((seconds, d));
            window_seconds += seconds;
            window_distance += d;
            while window_seconds > FAST_WINDOW_SECONDS {
                let excess = window_seconds - FAST_WINDOW_SECONDS;
                let Some((duration, travel)) = fast_window.pop_front() else {
                    break;
                };
                let removed = duration.min(excess);
                window_seconds -= removed;
                window_distance -= travel * removed / duration;
                if removed < duration {
                    fast_window
                        .push_front((duration - removed, travel * (duration - removed) / duration));
                }
            }
            if window_seconds >= FAST_WINDOW_SECONDS - 0.001
                && window_distance / window_seconds > FAST_SPEED_MPS
            {
                consecutive_fast_seconds = if consecutive_fast_seconds == 0.0 {
                    window_seconds
                } else {
                    consecutive_fast_seconds + seconds
                };
                longest_fast_seconds = longest_fast_seconds.max(consecutive_fast_seconds);
            } else {
                consecutive_fast_seconds = 0.0;
            }
        } else {
            fast_window.clear();
            window_seconds = 0.0;
            window_distance = 0.0;
            consecutive_fast_seconds = 0.0;
        }
    }
    if distance <= 0.0 || distance > 1_000_000.0 {
        return Err(UploadError("INVALID_ACTIVITY_DISTANCE"));
    }
    sample_intervals.sort_by(f64::total_cmp);
    let middle = sample_intervals.len() / 2;
    let median_interval = if sample_intervals.len() % 2 == 0 {
        (sample_intervals[middle - 1] + sample_intervals[middle]) / 2.0
    } else {
        sample_intervals[middle]
    };
    let heart_rate = sensor_metric(
        &points,
        |point| point.hr,
        |v| v > 0.0 && !(25.0..=250.0).contains(&v),
    );
    let cadence = sensor_metric(&points, |point| point.cadence, |v| v > 300.0);
    let telemetry = TelemetrySummary {
        policy_version: SENSOR_POLICY.into(),
        distance_source: if distance_segments == points.len() - 1 {
            "recorded_distance"
        } else if distance_segments == 0 {
            "gps_segments"
        } else {
            "mixed_distance_and_gps"
        }
        .into(),
        gps_samples: points.iter().filter(|point| point.lat.is_some()).count(),
        distance_samples: points
            .iter()
            .filter(|point| point.distance.is_some())
            .count(),
        speed_samples: points.iter().filter(|point| point.speed.is_some()).count(),
        heart_rate,
        cadence,
        // Garmin GPX declares revolutions/minute; keep the actual value instead of guessing steps.
        cadence_unit: if format == "FIT" {
            "cycles_per_minute"
        } else {
            "revolutions_per_minute"
        }
        .into(),
        gps_distance_m: (gps_segments > 0).then_some(gps_distance),
        recorded_distance_m: (distance_segments > 0).then_some(recorded_distance),
        reported_distance_m: session.as_ref().and_then(|s| s.distance),
        reported_elapsed_seconds: session.as_ref().and_then(|s| s.elapsed),
        reported_timer_seconds: session.as_ref().and_then(|s| s.timer),
        timed_coverage_seconds: timed_coverage,
        timed_coverage_ratio: (timed_coverage / elapsed).clamp(0.0, 1.0),
        max_sample_gap_seconds: *sample_intervals.last().unwrap_or(&0.0),
        median_sample_interval_seconds: median_interval,
        sample_gaps,
        segment_breaks,
        implausible_speed_segments,
        gps_jump_segments,
        longest_fast_window_seconds: longest_fast_seconds,
        longest_running_signal_mismatch_seconds: longest_signal_mismatch_seconds,
        integrated_speed_distance_m: (integrated_speed_seconds > 0.0)
            .then_some(integrated_speed_distance),
    };
    let mut checks = vec![
        check("TIMELINE", "pass", "Strictly increasing sample times; positive bounded duration.".into()),
        check("MANUAL_UPLOAD_UNVERIFIED", "unverified", "GPS, heart rate and cadence are editable file contents; they do not authenticate the runner.".into()),
        check("UNUSUAL_SPEED_REVIEW", if max_speed > 12.0 { "review" } else { "pass" }, format!("Maximum observed speed {:.2} m/s; short-segment review threshold 12 m/s.", max_speed)),
        check("GPS_JUMP_REVIEW", if gps_jump_segments > 0 { "review" } else if gps_segments == 0 { "limited" } else { "pass" }, format!("{gps_jump_segments} GPS jumps exceeding both 100 m and 20 m/s; {gps_segments} coordinate intervals evaluated.")),
        check("SUSTAINED_SPEED_REVIEW", if longest_fast_seconds >= SUSTAINED_FAST_SECONDS { "review" } else { "pass" }, format!("Longest continuous period with 30-second distance windows above 8 m/s: {:.1} s; review threshold 120 s.", longest_fast_seconds)),
        // A stationary pause can consume hours without adding unobserved movement.
        check("SPARSE_SAMPLING_REVIEW", if covered_distance / distance < 0.5 { "review" } else if timed_coverage < elapsed - 0.001 || segment_breaks > 0 { "limited" } else { "pass" }, format!("{:.1}% of measured distance and {:.1}% of elapsed time have intervals at most 30 s; {sample_gaps} gaps above 60 s, {segment_breaks} segment breaks. Review only when less than half the distance is closely sampled.", covered_distance / distance * 100.0, telemetry.timed_coverage_ratio * 100.0)),
        check("HEART_RATE_RANGE_REVIEW", if telemetry.heart_rate.out_of_range_samples > 0 { "review" } else if telemetry.heart_rate.missing_samples > 0 || telemetry.heart_rate.zero_samples > 0 { "limited" } else { "pass" }, format!("{} recorded, {} missing, {} zero, {} positive values outside the broad 25–250 bpm review band. Missing and zero are distinct; mean heart rate is not a fraud rule.", telemetry.heart_rate.samples, telemetry.heart_rate.missing_samples, telemetry.heart_rate.zero_samples, telemetry.heart_rate.out_of_range_samples)),
        check("CADENCE_RANGE_REVIEW", if telemetry.cadence.out_of_range_samples > 0 { "review" } else if telemetry.cadence.missing_samples > 0 || telemetry.cadence.zero_samples > 0 { "limited" } else { "pass" }, format!("{} recorded, {} missing, {} zero; {} raw values above 300. Source cadence is not automatically doubled or treated as proof of running.", telemetry.cadence.samples, telemetry.cadence.missing_samples, telemetry.cadence.zero_samples, telemetry.cadence.out_of_range_samples)),
        check("RUNNING_SIGNAL_MISMATCH", if longest_signal_mismatch_seconds >= SUSTAINED_SIGNAL_MISMATCH_SECONDS { "review" } else if telemetry.heart_rate.missing_samples > 0 || telemetry.heart_rate.zero_samples > 0 || telemetry.cadence.missing_samples > 0 || telemetry.cadence.zero_samples > 0 || timed_coverage < elapsed - 0.001 || segment_breaks > 0 { "limited" } else { "pass" }, format!("Longest continuous measured movement above {RUNNING_MISMATCH_SPEED_MPS} m/s with recorded zero cadence and positive heart rate below {RUNNING_MISMATCH_HR_BPM} bpm at both interval endpoints: {longest_signal_mismatch_seconds:.1} s; review threshold {SUSTAINED_SIGNAL_MISMATCH_SECONDS} s. Every interval must be at most {CLOSE_SAMPLE_SECONDS} s within one segment and have distance plus both sensors. Missing sensors, zero heart rate, gaps or a nonmatching interval reset the period. This prototype consistency check does not identify the cause or authenticate a runner.")),
    ];
    if let Some(ref session) = session {
        let session_elapsed = (session.end - session.start).num_milliseconds() as f64 / 1000.0;
        if session_elapsed <= 0.0 || session_elapsed > MAX_ELAPSED_SECONDS {
            return Err(UploadError("INVALID_ACTIVITY_DURATION"));
        }
        if session
            .elapsed
            .is_some_and(|v| !v.is_finite() || v <= 0.0 || v > MAX_ELAPSED_SECONDS)
            || session
                .timer
                .is_some_and(|v| !v.is_finite() || v < 0.0 || v > session_elapsed + 1.0)
            || session
                .distance
                .is_some_and(|v| !v.is_finite() || !(0.0..=1_000_000.0).contains(&v))
        {
            return Err(UploadError("INVALID_ACTIVITY_SESSION_METRIC"));
        }
        checks.push(check(
            "SESSION_SAMPLE_TIME_MISMATCH",
            if (start - session.start).num_milliseconds().abs() > 5_000
                || (end - session.end).num_milliseconds().abs() > 5_000
            {
                "review"
            } else {
                "pass"
            },
            "Session start/end compared with first/last sample using a 5-second tolerance.".into(),
        ));
        checks.push(check("SESSION_ELAPSED_TIME_MISMATCH", if session.elapsed.is_some_and(|v| (v - session_elapsed).abs() > 2.0_f64.max(session_elapsed * 0.02)) { "review" } else if session.elapsed.is_none() { "limited" } else { "pass" }, "Reported elapsed time compared with session timestamps; timer time remains separate from elapsed time.".into()));
        checks.push(check("DISTANCE_MISMATCH", if session.distance.is_some_and(|total| (total - distance).abs() > 100.0_f64.max(total * 0.05)) { "review" } else if session.distance.is_none() { "limited" } else { "pass" }, format!("Sample distance {:.1} m compared with reported session distance; tolerance max(100 m, 5%).", distance)));
    } else {
        checks.push(check("DISTANCE_MISMATCH", "limited", "GPX distance is calculated from timed coordinates; no independent cumulative distance or session total is available.".into()));
    }
    let gps_comparable = format == "FIT"
        && gps_segments == points.len() - 1
        && distance_segments == points.len() - 1;
    checks.push(check("GPS_DISTANCE_MISMATCH", if gps_comparable && (gps_distance - distance).abs() > 100.0_f64.max(distance * 0.10) { "review" } else if gps_comparable { "pass" } else { "limited" }, "Independent GPS versus recorded distance comparison requires both signals throughout the activity; tolerance max(100 m, 10%). Treadmill or missing GPS is a coverage limit.".into()));
    let speed_comparable =
        integrated_speed_seconds >= timed_coverage * 0.9 && integrated_speed_seconds > 0.0;
    checks.push(check("SPEED_DISTANCE_MISMATCH", if speed_comparable && (integrated_speed_distance - speed_covered_distance).abs() > 200.0_f64.max(speed_covered_distance * 0.25) { "review" } else if speed_comparable { "pass" } else { "limited" }, "Recorded speed integrated over closely sampled intervals compared with distance over the same intervals; tolerance max(200 m, 25%). Missing recorded speed is not zero.".into()));
    let mut reasons = vec!["MANUAL_UPLOAD_UNVERIFIED".into()];
    reasons.extend(
        checks
            .iter()
            .filter(|check| check.outcome == "review")
            .map(|check| check.code.clone()),
    );
    reasons.sort();
    reasons.dedup();
    // Independent of challenge ID, file formatting, GPX extensions and upload time.
    let fingerprint = hex::encode(Sha256::digest(format!(
        "activity-v1:{}:{}",
        start.timestamp_millis(),
        end.timestamp_millis()
    )));
    Ok(Activity {
        format: format.into(),
        starts_at: start,
        ends_at: end,
        distance_m: distance,
        elapsed_seconds: elapsed,
        sample_count: points.len(),
        heart_rate_samples: telemetry.heart_rate.samples,
        cadence_samples: telemetry.cadence.samples,
        max_speed_mps: max_speed,
        reasons,
        fingerprint,
        source_authenticity: "unverified".into(),
        telemetry,
        checks,
    })
}

fn check(code: &str, outcome: &str, detail: String) -> SensorCheck {
    SensorCheck {
        code: code.into(),
        outcome: outcome.into(),
        detail,
    }
}

fn sensor_metric(
    points: &[Point],
    value: impl Fn(&Point) -> Option<f64>,
    out_of_range: impl Fn(f64) -> bool,
) -> SensorMetric {
    let mut result = SensorMetric::default();
    let mut total = 0.0;
    for point in points {
        if let Some(value) = value(point) {
            result.samples += 1;
            result.zero_samples += usize::from(value == 0.0);
            result.out_of_range_samples += usize::from(out_of_range(value));
            result.min = Some(result.min.map_or(value, |current| current.min(value)));
            result.max = Some(result.max.map_or(value, |current| current.max(value)));
            total += value;
        } else {
            result.missing_samples += 1;
        }
    }
    result.mean = (result.samples > 0).then(|| total / result.samples as f64);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn track(extra: &str) -> String {
        format!(
            r#"<gpx xmlns="{GPX}" version="1.1"><trk><trkseg><trkpt lat="50" lon="14"><time>2026-09-01T10:00:00Z</time>{extra}</trkpt><trkpt lat="50.001" lon="14"><time>2026-09-01T10:00:30Z</time></trkpt></trkseg></trk></gpx>"#
        )
    }
    #[test]
    fn gpx_units_and_repackaged_identity() {
        let a = parse(track("").as_bytes(), None).unwrap();
        let b = parse(track("<name>changed</name>").as_bytes(), None).unwrap();
        assert!((a.distance_m - 111.195).abs() < 0.1);
        assert_eq!(a.elapsed_seconds, 30.0);
        assert_eq!(a.heart_rate_samples, 0);
        assert_eq!(a.fingerprint, b.fingerprint);
    }
    #[test]
    fn rejects_dtd_namespace_missing_time_and_reversal() {
        for s in [
            format!("<!DOCTYPE gpx>{}", track("")),
            track("").replace(GPX, "https://fake.test"),
            track("").replace("<time>2026-09-01T10:00:00Z</time>", ""),
            track("").replace("10:00:30", "09:00:30"),
        ] {
            assert!(parse(s.as_bytes(), None).is_err());
        }
    }
    #[test]
    fn suspicious_speed_is_review_not_cheating() {
        let a = parse(track("").replace("50.001", "50.1").as_bytes(), None).unwrap();
        assert!(a.reasons.contains(&"UNUSUAL_SPEED_REVIEW".into()));
        assert_eq!(a.source_authenticity, "unverified");
    }

    fn recorded_run(seconds: usize, speed: f64) -> Vec<Point> {
        let start = time("2026-09-01T10:00:00Z").unwrap();
        (0..=seconds)
            .map(|offset| Point {
                at: start + chrono::Duration::seconds(offset as i64),
                lat: None,
                lon: None,
                distance: Some(offset as f64 * speed),
                speed: Some(speed),
                hr: Some(120.0),
                cadence: Some(80.0),
                segment: 0,
            })
            .collect()
    }

    #[test]
    fn sensor_values_are_parsed_not_merely_counted_and_missing_is_not_zero() {
        let extensions = r#"<extensions><gpxtpx:TrackPointExtension xmlns:gpxtpx="http://www.garmin.com/xmlschemas/TrackPointExtension/v1"><gpxtpx:hr>0</gpxtpx:hr><gpxtpx:cad>80</gpxtpx:cad></gpxtpx:TrackPointExtension></extensions>"#;
        let a = parse(track(extensions).as_bytes(), None).unwrap();
        assert_eq!(a.telemetry.heart_rate.samples, 1);
        assert_eq!(a.telemetry.heart_rate.zero_samples, 1);
        assert_eq!(a.telemetry.heart_rate.missing_samples, 1);
        assert_eq!(a.telemetry.heart_rate.min, Some(0.0));
        assert_eq!(a.telemetry.cadence.mean, Some(80.0));
        assert_eq!(a.telemetry.cadence_unit, "revolutions_per_minute");
        assert_eq!(a.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
        for invalid in ["NaN", "Infinity", "", "-1"] {
            let invalid =
                track(&extensions.replace("<gpxtpx:hr>0", &format!("<gpxtpx:hr>{invalid}")));
            assert!(parse(invalid.as_bytes(), None).is_err());
        }
        let duplicated = track(&extensions.replace(
            "<gpxtpx:hr>0</gpxtpx:hr>",
            "<gpxtpx:hr>0</gpxtpx:hr><gpxtpx:hr>100</gpxtpx:hr>",
        ));
        assert_eq!(
            parse(duplicated.as_bytes(), None).unwrap_err().0,
            "DUPLICATE_ACTIVITY_SENSOR"
        );
    }

    #[test]
    fn extreme_hr_is_a_sensor_review_while_low_hr_and_cadence_stops_are_plausible() {
        let mut points = recorded_run(300, 2.0);
        for point in &mut points {
            point.hr = Some(48.0);
            point.cadence = Some(0.0);
        }
        let a = summarize("FIT", points.clone(), None).unwrap();
        assert_eq!(a.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
        assert_eq!(a.telemetry.heart_rate.mean, Some(48.0));
        points[40].hr = Some(600.0);
        let b = summarize("FIT", points, None).unwrap();
        assert_eq!(b.telemetry.heart_rate.out_of_range_samples, 1);
        assert!(b.reasons.contains(&"HEART_RATE_RANGE_REVIEW".into()));
        assert_eq!(b.source_authenticity, "unverified");
    }

    fn mismatch_pattern(seconds: usize, speed: f64) -> Vec<Point> {
        let mut points = recorded_run(seconds, speed);
        for point in &mut points {
            point.hr = Some(78.0);
            point.cadence = Some(0.0);
        }
        points
    }

    fn mismatch_outcome(activity: &Activity) -> &str {
        &activity
            .checks
            .iter()
            .find(|check| check.code == "RUNNING_SIGNAL_MISMATCH")
            .unwrap()
            .outcome
    }

    #[test]
    fn continuous_fast_movement_low_hr_and_zero_cadence_requires_review() {
        let activity = summarize("FIT", mismatch_pattern(180, 22.0 / 3.6), None).unwrap();
        assert_eq!(
            activity.telemetry.policy_version,
            "manual-running-plausibility-v3"
        );
        assert_eq!(
            activity.telemetry.longest_running_signal_mismatch_seconds,
            180.0
        );
        assert_eq!(mismatch_outcome(&activity), "review");
        assert_eq!(
            activity.reasons,
            vec!["MANUAL_UPLOAD_UNVERIFIED", "RUNNING_SIGNAL_MISMATCH"]
        );
        assert_eq!(activity.source_authenticity, "unverified");
        let boundary = summarize("FIT", mismatch_pattern(120, 22.0 / 3.6), None).unwrap();
        assert_eq!(mismatch_outcome(&boundary), "review");
        let short = summarize("FIT", mismatch_pattern(119, 22.0 / 3.6), None).unwrap();
        assert_eq!(mismatch_outcome(&short), "limited");
        assert_eq!(short.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
    }

    #[test]
    fn brief_zero_cadence_glitch_does_not_review_an_otherwise_recorded_run() {
        let mut points = recorded_run(300, 22.0 / 3.6);
        for point in &mut points[100..=115] {
            point.hr = Some(78.0);
            point.cadence = Some(0.0);
        }
        let activity = summarize("FIT", points, None).unwrap();
        assert_eq!(
            activity.telemetry.longest_running_signal_mismatch_seconds,
            15.0
        );
        assert_eq!(mismatch_outcome(&activity), "limited");
        assert_eq!(activity.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
    }

    #[test]
    fn missing_sensors_and_recorded_zero_hr_limit_and_reset_consistency_check() {
        for missing_hr in [true, false] {
            let mut points = mismatch_pattern(240, 22.0 / 3.6);
            if missing_hr {
                points[120].hr = None;
            } else {
                points[120].cadence = None;
            }
            let activity = summarize("FIT", points, None).unwrap();
            assert_eq!(
                activity.telemetry.longest_running_signal_mismatch_seconds,
                119.0
            );
            assert_eq!(mismatch_outcome(&activity), "limited");
            assert_eq!(activity.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
        }
        let mut points = mismatch_pattern(240, 22.0 / 3.6);
        for point in &mut points {
            point.hr = Some(0.0);
        }
        let zero_hr = summarize("FIT", points, None).unwrap();
        assert_eq!(
            zero_hr.telemetry.longest_running_signal_mismatch_seconds,
            0.0
        );
        assert_eq!(mismatch_outcome(&zero_hr), "limited");
        assert_eq!(zero_hr.telemetry.heart_rate.zero_samples, 241);
        assert_eq!(zero_hr.telemetry.heart_rate.missing_samples, 0);
        assert_eq!(zero_hr.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
        let mut points = mismatch_pattern(240, 22.0 / 3.6);
        for point in &mut points {
            point.hr = None;
            point.cadence = None;
        }
        let missing = summarize("FIT", points, None).unwrap();
        assert_eq!(
            missing.telemetry.longest_running_signal_mismatch_seconds,
            0.0
        );
        assert_eq!(mismatch_outcome(&missing), "limited");
        assert_eq!(missing.telemetry.heart_rate.zero_samples, 0);
        assert_eq!(missing.telemetry.cadence.zero_samples, 0);
        assert_eq!(missing.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
    }

    #[test]
    fn sample_gaps_and_segment_breaks_do_not_join_short_mismatch_periods() {
        for segment_break in [false, true] {
            let mut points = mismatch_pattern(180, 22.0 / 3.6);
            for point in &mut points[90..] {
                if segment_break {
                    point.segment = 1;
                } else {
                    // A 31-second interval is beyond the close-sampling threshold.
                    point.at += chrono::Duration::seconds(30);
                }
            }
            let activity = summarize("FIT", points, None).unwrap();
            assert_eq!(
                activity.telemetry.longest_running_signal_mismatch_seconds,
                90.0
            );
            assert_eq!(mismatch_outcome(&activity), "limited");
            assert_eq!(activity.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
        }
    }

    #[test]
    fn fast_movement_with_recorded_running_signals_passes_consistency_check() {
        let activity = summarize("FIT", recorded_run(300, 22.0 / 3.6), None).unwrap();
        assert_eq!(
            activity.telemetry.longest_running_signal_mismatch_seconds,
            0.0
        );
        assert_eq!(mismatch_outcome(&activity), "pass");
        assert_eq!(activity.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
        let slow = summarize("FIT", mismatch_pattern(300, 4.0), None).unwrap();
        assert_eq!(slow.telemetry.longest_running_signal_mismatch_seconds, 0.0);
        assert_eq!(slow.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
        let mut points = mismatch_pattern(300, 22.0 / 3.6);
        for point in &mut points {
            point.hr = Some(100.0);
        }
        let hr_boundary = summarize("FIT", points, None).unwrap();
        assert_eq!(
            hr_boundary
                .telemetry
                .longest_running_signal_mismatch_seconds,
            0.0
        );
        assert_eq!(hr_boundary.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
    }

    #[test]
    fn v2_stored_sensor_summary_preserves_its_policy_and_checks() {
        let activity = summarize("FIT", mismatch_pattern(180, 22.0 / 3.6), None).unwrap();
        let mut stored = serde_json::to_value(activity).unwrap();
        let telemetry = stored
            .get_mut("telemetry")
            .unwrap()
            .as_object_mut()
            .unwrap();
        telemetry.insert(
            "policy_version".into(),
            "manual-running-plausibility-v2".into(),
        );
        telemetry.remove("longest_running_signal_mismatch_seconds");
        stored
            .get_mut("checks")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .retain(|check| check.get("code").unwrap() != "RUNNING_SIGNAL_MISMATCH");
        stored["reasons"] = serde_json::json!(["MANUAL_UPLOAD_UNVERIFIED"]);
        let old: Activity = serde_json::from_value(stored).unwrap();
        assert_eq!(
            old.telemetry.policy_version,
            "manual-running-plausibility-v2"
        );
        assert_eq!(old.telemetry.longest_running_signal_mismatch_seconds, 0.0);
        assert!(
            !old.checks
                .iter()
                .any(|check| check.code == "RUNNING_SIGNAL_MISMATCH")
        );
        assert_eq!(old.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
    }

    #[test]
    fn long_stationary_pauses_do_not_look_like_unobserved_travel() {
        let mut points = recorded_run(300, 2.0);
        for point in &mut points[150..] {
            point.at += chrono::Duration::hours(25);
        }
        let a = summarize("FIT", points, None).unwrap();
        assert!(a.elapsed_seconds > 86_400.0);
        assert_eq!(a.telemetry.sample_gaps, 1);
        assert!(a.telemetry.timed_coverage_ratio < 0.01);
        assert_eq!(a.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
        assert_eq!(
            a.checks
                .iter()
                .find(|check| check.code == "SPARSE_SAMPLING_REVIEW")
                .unwrap()
                .outcome,
            "limited"
        );
    }

    #[test]
    fn medium_gaps_are_reported_as_limited_even_below_the_large_gap_counter() {
        let mut points = recorded_run(300, 2.0);
        for point in &mut points[150..] {
            point.at += chrono::Duration::seconds(40);
        }
        let a = summarize("FIT", points, None).unwrap();
        assert_eq!(a.telemetry.sample_gaps, 0);
        assert_eq!(a.telemetry.max_sample_gap_seconds, 41.0);
        assert_eq!(a.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
        assert_eq!(
            a.checks
                .iter()
                .find(|check| check.code == "SPARSE_SAMPLING_REVIEW")
                .unwrap()
                .outcome,
            "limited"
        );
    }

    #[test]
    fn isolated_recorded_speed_artifact_is_counted_and_not_a_sustained_fast_period() {
        let mut points = recorded_run(180, 3.0);
        points[60].speed = Some(30.0);
        let a = summarize("FIT", points, None).unwrap();
        assert!(a.reasons.contains(&"UNUSUAL_SPEED_REVIEW".into()));
        assert_eq!(a.telemetry.implausible_speed_segments, 2);
        assert!(!a.reasons.contains(&"SUSTAINED_SPEED_REVIEW".into()));
        assert_eq!(a.telemetry.longest_fast_window_seconds, 0.0);
    }

    #[test]
    fn poorly_sampled_distance_and_gps_teleports_require_review() {
        let a = parse(track("").replace("10:00:30", "10:10:30").as_bytes(), None).unwrap();
        assert!(a.reasons.contains(&"SPARSE_SAMPLING_REVIEW".into()));
        let b = parse(track("").replace("50.001", "50.1").as_bytes(), None).unwrap();
        assert!(b.reasons.contains(&"GPS_JUMP_REVIEW".into()));
        assert!(b.telemetry.gps_jump_segments > 0);
    }

    #[test]
    fn sustained_fast_travel_is_detected_below_the_single_segment_speed_limit() {
        let a = summarize("FIT", recorded_run(180, 9.0), None).unwrap();
        assert!(!a.reasons.contains(&"UNUSUAL_SPEED_REVIEW".into()));
        assert!(a.reasons.contains(&"SUSTAINED_SPEED_REVIEW".into()));
        assert!(a.telemetry.longest_fast_window_seconds >= 120.0);
        let b = summarize("FIT", recorded_run(75, 9.0), None).unwrap();
        assert!(!b.reasons.contains(&"SUSTAINED_SPEED_REVIEW".into()));
    }

    #[test]
    fn contradictory_distance_and_speed_are_not_silently_replaced_by_summary_totals() {
        let mut points = recorded_run(600, 3.0);
        for point in &mut points {
            point.speed = Some(6.0);
        }
        let session = SessionSummary {
            start: points[0].at,
            end: points.last().unwrap().at,
            distance: Some(5_000.0),
            elapsed: Some(700.0),
            timer: Some(590.0),
        };
        let a = summarize("FIT", points, Some(session)).unwrap();
        assert_eq!(a.distance_m, 1_800.0);
        assert!(a.reasons.contains(&"DISTANCE_MISMATCH".into()));
        assert!(a.reasons.contains(&"SPEED_DISTANCE_MISMATCH".into()));
        assert!(a.reasons.contains(&"SESSION_ELAPSED_TIME_MISMATCH".into()));
    }

    #[test]
    fn cumulative_distance_cannot_reverse_across_missing_distance_samples() {
        let mut points = recorded_run(3, 3.0);
        points[1].distance = Some(10.0);
        points[2].distance = None;
        assert_eq!(
            summarize("FIT", points, None).unwrap_err().0,
            "DECREASING_DISTANCE"
        );
    }

    #[test]
    fn old_stored_activity_without_new_telemetry_still_deserializes() {
        let activity = parse(track("").as_bytes(), None).unwrap();
        let mut stored = serde_json::to_value(activity).unwrap();
        stored.as_object_mut().unwrap().remove("telemetry");
        stored.as_object_mut().unwrap().remove("checks");
        let old: Activity = serde_json::from_value(stored).unwrap();
        assert!(old.checks.is_empty());
        assert!(old.telemetry.policy_version.is_empty());
    }

    #[test]
    fn leap_second_and_duplicate_times_are_rejected_without_sorting() {
        for invalid in [
            track("").replace("10:00:00Z", "09:59:60Z"),
            track("").replace("10:00:30Z", "10:00:00Z"),
        ] {
            assert!(parse(invalid.as_bytes(), None).is_err());
        }
    }
}
