use truhabit_evidence::upload::parse;
const FIT: &[u8] = include_bytes!("../../../web/public/prototype/valid-run.fit");
const MULTI: &[u8] = include_bytes!("../../../web/public/prototype/multi-session.fit");
#[test]
fn fit_running_units_timestamps_and_missing_sensors() {
    let a = parse(FIT, None).unwrap();
    assert_eq!(a.format, "FIT");
    assert_eq!(a.distance_m, 3100.0);
    assert_eq!(a.elapsed_seconds, 1080.0);
    assert_eq!(a.starts_at.to_rfc3339(), "2026-09-04T10:00:00+00:00");
    assert_eq!(a.sample_count, 181);
    assert_eq!(a.heart_rate_samples, 0);
    assert_eq!(a.cadence_samples, 0);
    assert_eq!(a.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
    assert_eq!(a.source_authenticity, "unverified");
    let exact = parse(
        include_bytes!("../../../web/public/prototype/three-km-run.fit"),
        None,
    )
    .unwrap();
    assert_eq!(exact.distance_m, 3000.0);
    assert_eq!(exact.elapsed_seconds, 1080.0);
    assert_eq!(exact.distance_m / exact.elapsed_seconds * 3.6, 10.0);
    assert_eq!(exact.elapsed_seconds / (exact.distance_m / 1000.0), 360.0);
    assert_eq!(exact.heart_rate_samples, 0);
    assert_eq!(exact.cadence_samples, 0);
}
#[test]
fn fit_sessions_are_selected_never_silently_added() {
    assert_eq!(parse(MULTI, None).unwrap_err().0, "SELECT_FIT_SESSION");
    assert_eq!(parse(MULTI, Some(0)).unwrap().distance_m, 3100.0);
    assert_eq!(parse(MULTI, Some(1)).unwrap().distance_m, 1000.0);
    assert_eq!(parse(MULTI, Some(2)).unwrap_err().0, "INVALID_FIT_SESSION");
    assert_eq!(
        parse(
            include_bytes!("../../../web/public/prototype/cycling.fit"),
            None
        )
        .unwrap_err()
        .0,
        "RUNNING_SESSION_REQUIRED"
    );
}
#[test]
fn fit_integrity_and_trailing_records_are_rejected() {
    let mut corrupt = FIT.to_vec();
    corrupt[30] ^= 1;
    assert_eq!(
        parse(&corrupt, None).unwrap_err().0,
        "INVALID_FIT_CRC_OR_FORMAT"
    );
    let mut trailing = FIT.to_vec();
    trailing.extend_from_slice(FIT);
    assert_eq!(parse(&trailing, None).unwrap_err().0, "INVALID_FIT_LENGTH");
    for length in [12, 14, 30, FIT.len() - 1] {
        assert!(parse(&FIT[..length], None).is_err());
    }
}
#[test]
fn replay_fixtures_have_expected_goals_and_review_evidence() {
    let a = parse(
        include_bytes!("../../../web/public/prototype/valid-run.gpx"),
        None,
    )
    .unwrap();
    assert!((a.distance_m - 3100.0).abs() < 0.1);
    assert_eq!(a.elapsed_seconds, 1080.0);
    assert_eq!(a.reasons, vec!["MANUAL_UPLOAD_UNVERIFIED"]);
    let b = parse(
        include_bytes!("../../../web/public/prototype/short-run.gpx"),
        None,
    )
    .unwrap();
    assert!(b.distance_m < 3000.0);
    let c = parse(
        include_bytes!("../../../web/public/prototype/review-run.gpx"),
        None,
    )
    .unwrap();
    assert!(c.reasons.contains(&"UNUSUAL_SPEED_REVIEW".into()));
}

#[test]
fn parsed_gpx_movement_and_recorded_low_hr_zero_cadence_require_consistency_review() {
    let start = chrono::DateTime::parse_from_rfc3339("2026-09-01T10:00:00Z").unwrap();
    let mut gpx = String::from(
        r#"<gpx xmlns="http://www.topografix.com/GPX/1/1" version="1.1" xmlns:g="http://www.garmin.com/xmlschemas/TrackPointExtension/v1"><trk><trkseg>"#,
    );
    for seconds in 0..=180 {
        let lat = 50.0 + f64::from(seconds) * (22.0 / 3.6) / 111_194.926_644_558_73;
        let at = start + chrono::Duration::seconds(i64::from(seconds));
        gpx.push_str(&format!(r#"<trkpt lat="{lat:.10}" lon="14"><time>{}</time><extensions><g:TrackPointExtension><g:hr>78</g:hr><g:cad>0</g:cad></g:TrackPointExtension></extensions></trkpt>"#, at.to_rfc3339()));
    }
    gpx.push_str("</trkseg></trk></gpx>");
    let activity = parse(gpx.as_bytes(), None).unwrap();
    assert_eq!(activity.telemetry.speed_samples, 0);
    assert_eq!(
        activity.telemetry.longest_running_signal_mismatch_seconds,
        180.0
    );
    assert_eq!(activity.telemetry.heart_rate.mean, Some(78.0));
    assert_eq!(activity.telemetry.cadence.zero_samples, 181);
    assert_eq!(
        activity.reasons,
        vec!["MANUAL_UPLOAD_UNVERIFIED", "RUNNING_SIGNAL_MISMATCH"]
    );
    assert_eq!(activity.source_authenticity, "unverified");

    let fixture = parse(
        include_bytes!("../../../web/public/prototype/sensor-mismatch.gpx"),
        None,
    )
    .unwrap();
    assert!((fixture.distance_m - 3_104.444).abs() < 0.1);
    assert_eq!(fixture.elapsed_seconds, 508.0);
    assert_eq!(fixture.sample_count, 509);
    assert_eq!(
        fixture.telemetry.longest_running_signal_mismatch_seconds,
        508.0
    );
    assert_eq!(fixture.telemetry.heart_rate.mean, Some(78.0));
    assert_eq!(fixture.telemetry.cadence.zero_samples, 509);
    assert_eq!(
        fixture.reasons,
        vec!["MANUAL_UPLOAD_UNVERIFIED", "RUNNING_SIGNAL_MISMATCH"]
    );
    assert_eq!(fixture.source_authenticity, "unverified");
}
