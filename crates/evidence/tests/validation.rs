use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use truhabit_evidence::{EvidenceError, MAX_BYTES, MAX_SAMPLES, parse};

fn now() -> DateTime<Utc> {
    "2026-09-26T12:00:00Z".parse().unwrap()
}
fn sample(offset: u32) -> Value {
    json!({"offset_ms":offset,"distance_m":null,"speed_mps":null,"heart_rate_bpm":null,"cadence":null})
}
fn record() -> Value {
    json!({"schema_version":"1","evidence_id":"record-1","commitment_id":"goal-1","provider":"direct_file","source_class":"user_supplied_untrusted","rights_policy_id":null,"activity_type":"run","started_at":"2026-09-26T10:00:00Z","ended_at":"2026-09-26T10:25:00Z","received_at":"2026-09-26T11:00:00Z","distance_m":5000.0,"elapsed_seconds":1500,"moving_seconds":1400,"cadence_unit":"steps_per_minute","samples":[sample(0),sample(1000)]})
}
fn check(v: &Value) -> Result<truhabit_evidence::ParsedEvidence, EvidenceError> {
    parse(&serde_json::to_vec(v).unwrap(), now())
}

#[test]
fn missing_metrics_are_not_zeros_and_never_prove_authenticity() {
    let mut v = record();
    v["samples"][1]["heart_rate_bpm"] = json!(0);
    v["samples"][1]["cadence"] = json!(0);
    let report = check(&v).unwrap().report().unwrap();
    assert_eq!(report.coverage.missing_heart_rate, 1);
    assert_eq!(report.coverage.zero_heart_rate, 1);
    assert_eq!(report.coverage.missing_cadence, 1);
    assert_eq!(report.coverage.zero_cadence, 1);
    assert_eq!(report.source_authenticity, "unverified");
    assert_eq!(report.payment_eligibility, "not_assessed");
}
#[test]
fn nullable_fields_must_still_be_present_and_unknown_fields_are_rejected() {
    for field in ["distance_m", "moving_seconds", "rights_policy_id"] {
        let mut v = record();
        v.as_object_mut().unwrap().remove(field);
        assert_eq!(check(&v).unwrap_err(), EvidenceError::InvalidSchema);
    }
    for field in ["distance_m", "speed_mps", "heart_rate_bpm", "cadence"] {
        let mut v = record();
        v["samples"][0].as_object_mut().unwrap().remove(field);
        assert_eq!(check(&v).unwrap_err(), EvidenceError::InvalidSchema);
    }
    let mut v = record();
    v["verified"] = json!(true);
    assert_eq!(check(&v).unwrap_err(), EvidenceError::InvalidSchema);
    let mut v = record();
    v["samples"][0]["extra"] = json!(true);
    assert_eq!(check(&v).unwrap_err(), EvidenceError::InvalidSchema);
}
#[test]
fn duplicate_keys_trailing_json_and_nonfinite_numbers_are_rejected() {
    let text = record().to_string();
    let duplicate = text.replacen("{", "{\"schema_version\":\"1\",", 1);
    assert_eq!(
        parse(duplicate.as_bytes(), now()).unwrap_err(),
        EvidenceError::InvalidSchema
    );
    assert_eq!(
        parse(format!("{text}{{}}").as_bytes(), now()).unwrap_err(),
        EvidenceError::InvalidSchema
    );
    for number in ["NaN", "Infinity", "1e309"] {
        let invalid = text.replace("5000.0", number);
        assert_ne!(invalid, text);
        assert_eq!(
            parse(invalid.as_bytes(), now()).unwrap_err(),
            EvidenceError::InvalidSchema
        );
    }
}
#[test]
fn version_ids_and_forged_source_claims_are_not_trusted() {
    let mut v = record();
    v["schema_version"] = json!("2");
    assert_eq!(check(&v).unwrap_err(), EvidenceError::UnsupportedVersion);
    for id in [String::new(), "x".repeat(129), "with space".into()] {
        let mut v = record();
        v["evidence_id"] = json!(id);
        assert_eq!(check(&v).unwrap_err(), EvidenceError::InvalidIdentifier);
    }
    let mut v = record();
    v["source_class"] = json!("provider_asserted");
    assert_eq!(check(&v).unwrap_err(), EvidenceError::SourceClaim);
    v["provider"] = json!("approved_provider");
    assert_eq!(check(&v).unwrap_err(), EvidenceError::SourceClaim);
    v["rights_policy_id"] = json!("some-self-reported-policy");
    assert_eq!(
        check(&v).unwrap().report().unwrap().source_authenticity,
        "unverified"
    );
}
#[test]
fn timeline_checks_duration_moving_time_future_and_offsets() {
    for (key, value) in [
        ("ended_at", json!("2026-09-26T09:59:59Z")),
        ("received_at", json!("2026-09-26T12:01:00Z")),
        ("received_at", json!("2026-09-26T10:10:00Z")),
        ("elapsed_seconds", json!(1498)),
        ("moving_seconds", json!(1501)),
    ] {
        let mut v = record();
        v[key] = value;
        assert_eq!(check(&v).unwrap_err(), EvidenceError::Timeline);
    }
    let mut v = record();
    v["samples"][1]["offset_ms"] = json!(1500001);
    assert_eq!(check(&v).unwrap_err(), EvidenceError::SampleOrder);
    v["samples"][1]["offset_ms"] = json!(0);
    assert_eq!(check(&v).unwrap_err(), EvidenceError::SampleOrder);
}
#[test]
fn timestamp_normalization_requires_utc_and_rejects_leap_seconds() {
    for timestamp in [
        "2026-09-26T10:00:00+00:00",
        "2026-09-26T10:00:60Z",
        "invalidZ",
    ] {
        let mut v = record();
        v["started_at"] = json!(timestamp);
        assert_eq!(check(&v).unwrap_err(), EvidenceError::InvalidSchema);
    }
}
#[test]
fn gps_bounds_and_pairs_are_checked_without_assuming_origin_is_missing() {
    let mut v = record();
    v["samples"][0]["lat_e7"] = json!(0);
    assert_eq!(check(&v).unwrap_err(), EvidenceError::Coordinates);
    v["samples"][0]["lon_e7"] = json!(0);
    assert_eq!(check(&v).unwrap().report().unwrap().coverage.missing_gps, 1);
    v["samples"][0]["lat_e7"] = json!(900000001);
    assert_eq!(check(&v).unwrap_err(), EvidenceError::Coordinates);
}
#[test]
fn negative_metrics_and_reversing_cumulative_distance_are_rejected() {
    let mut v = record();
    v["distance_m"] = json!(-0.1);
    assert_eq!(check(&v).unwrap_err(), EvidenceError::Metric);
    let mut v = record();
    v["samples"][0]["distance_m"] = json!(10);
    v["samples"][1]["distance_m"] = json!(9);
    assert_eq!(check(&v).unwrap_err(), EvidenceError::SampleOrder);
}
#[test]
fn cadence_units_are_explicit_and_unknown_is_not_guessed() {
    let mut v = record();
    v["samples"][0]["cadence"] = json!(86);
    assert_eq!(check(&v).unwrap().cadence_steps_per_minute(0), Some(86.0));
    v["cadence_unit"] = json!("cycles_per_minute");
    assert_eq!(check(&v).unwrap().cadence_steps_per_minute(0), Some(172.0));
    v["cadence_unit"] = json!("unknown");
    assert_eq!(check(&v).unwrap().cadence_steps_per_minute(0), None);
}
#[test]
fn byte_and_sample_limits_are_enforced() {
    assert_eq!(
        parse(&vec![b' '; MAX_BYTES + 1], now()).unwrap_err(),
        EvidenceError::PayloadTooLarge
    );
    let mut v = record();
    v["samples"] = Value::Array((0..=MAX_SAMPLES).map(|i| sample(i as u32)).collect());
    assert_eq!(check(&v).unwrap_err(), EvidenceError::TooManySamples);
}
#[test]
fn integrity_digest_is_stable_across_json_layout_but_binds_record_changes() {
    let v = record();
    let parsed = check(&v).unwrap();
    let pretty = parse(&serde_json::to_vec_pretty(&v).unwrap(), now()).unwrap();
    assert_eq!(
        parsed.report().unwrap().record_sha256,
        pretty.report().unwrap().record_sha256
    );
    let mut changed = v;
    changed["commitment_id"] = json!("goal-2");
    assert_ne!(
        parsed.report().unwrap().record_sha256,
        check(&changed).unwrap().report().unwrap().record_sha256
    );
}
