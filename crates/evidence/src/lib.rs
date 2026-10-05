//! Bounded parsing of normalized private activity records.
//! Schema validity and self-reported provider metadata NEVER establish provenance or payment eligibility.
use chrono::{DateTime, Timelike, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MAX_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_SAMPLES: usize = 100_000;
const MAX_ELAPSED: u32 = 86_400;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EvidenceError {
    #[error("PAYLOAD_TOO_LARGE")]
    PayloadTooLarge,
    #[error("INVALID_JSON_OR_SCHEMA")]
    InvalidSchema,
    #[error("UNSUPPORTED_SCHEMA_VERSION")]
    UnsupportedVersion,
    #[error("INVALID_IDENTIFIER")]
    InvalidIdentifier,
    #[error("INCONSISTENT_SOURCE_CLAIM")]
    SourceClaim,
    #[error("INVALID_TIMELINE")]
    Timeline,
    #[error("INVALID_METRIC")]
    Metric,
    #[error("INVALID_SAMPLE_ORDER")]
    SampleOrder,
    #[error("INVALID_COORDINATES")]
    Coordinates,
    #[error("TOO_MANY_SAMPLES")]
    TooManySamples,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderClaim {
    Synthetic,
    DirectFile,
    ApprovedProvider,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceClass {
    Synthetic,
    UserSuppliedUntrusted,
    ProviderAsserted,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityType {
    Run,
    Walk,
    Ride,
    Other,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CadenceUnit {
    StepsPerMinute,
    CyclesPerMinute,
    Unknown,
}

// A required nullable field differs from an omitted field. No serde(default) here.
fn nullable<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(d)
}

fn utc<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
    let value = String::deserialize(d)?;
    if value.len() > 40 || !value.ends_with('Z') {
        return Err(serde::de::Error::custom("UTC Z timestamp required"));
    }
    let instant = DateTime::parse_from_rfc3339(&value)
        .map_err(|_| serde::de::Error::custom("Invalid timestamp"))?;
    if instant.nanosecond() >= 1_000_000_000 {
        return Err(serde::de::Error::custom("Leap second unsupported"));
    }
    Ok(instant.with_timezone(&Utc))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub offset_ms: u32,
    #[serde(deserialize_with = "nullable")]
    pub distance_m: Option<f64>,
    #[serde(deserialize_with = "nullable")]
    pub speed_mps: Option<f64>,
    #[serde(deserialize_with = "nullable")]
    pub heart_rate_bpm: Option<f64>,
    #[serde(deserialize_with = "nullable")]
    pub cadence: Option<f64>,
    pub lat_e7: Option<i32>,
    pub lon_e7: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedEvidence {
    pub schema_version: String,
    pub evidence_id: String,
    pub commitment_id: String,
    pub provider: ProviderClaim,
    pub source_class: SourceClass,
    #[serde(deserialize_with = "nullable")]
    pub rights_policy_id: Option<String>,
    pub activity_type: ActivityType,
    #[serde(deserialize_with = "utc")]
    pub started_at: DateTime<Utc>,
    #[serde(deserialize_with = "utc")]
    pub ended_at: DateTime<Utc>,
    #[serde(deserialize_with = "utc")]
    pub received_at: DateTime<Utc>,
    #[serde(deserialize_with = "nullable")]
    pub distance_m: Option<f64>,
    pub elapsed_seconds: u32,
    #[serde(deserialize_with = "nullable")]
    pub moving_seconds: Option<u32>,
    pub cadence_unit: CadenceUnit,
    pub samples: Vec<Sample>,
}

/// Immutable after validation. Construction is private; callers cannot skip validation.
#[derive(Debug)]
pub struct ParsedEvidence(NormalizedEvidence);

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Coverage {
    pub samples: usize,
    pub missing_distance: usize,
    pub missing_speed: usize,
    pub missing_heart_rate: usize,
    pub missing_cadence: usize,
    pub unknown_cadence_unit: bool,
    pub zero_heart_rate: usize,
    pub zero_cadence: usize,
    pub missing_gps: usize,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ValidationReport {
    pub schema_version: &'static str,
    pub validation_policy: &'static str,
    pub source_authenticity: &'static str,
    pub payment_eligibility: &'static str,
    /// An integrity digest of this normalized record, NOT a proof of activity identity or deduplication.
    pub record_sha256: String,
    pub coverage: Coverage,
}

fn identifier(v: &str) -> bool {
    !v.is_empty() && v.len() <= 128 && v.bytes().all(|b| b.is_ascii_graphic())
}
fn metric(v: Option<f64>, maximum: f64) -> bool {
    v.is_none_or(|v| v.is_finite() && (0.0..=maximum).contains(&v))
}

pub fn parse(bytes: &[u8], server_now: DateTime<Utc>) -> Result<ParsedEvidence, EvidenceError> {
    if bytes.len() > MAX_BYTES {
        return Err(EvidenceError::PayloadTooLarge);
    }
    // Keep serde_json's recursion limit enabled. Typed structs reject duplicates and unknown fields.
    let e: NormalizedEvidence =
        serde_json::from_slice(bytes).map_err(|_| EvidenceError::InvalidSchema)?;
    if e.schema_version != "1" {
        return Err(EvidenceError::UnsupportedVersion);
    }
    if !identifier(&e.evidence_id)
        || !identifier(&e.commitment_id)
        || e.rights_policy_id.as_ref().is_some_and(|v| !identifier(v))
    {
        return Err(EvidenceError::InvalidIdentifier);
    }
    let consistent = matches!(
        (e.provider, e.source_class),
        (ProviderClaim::Synthetic, SourceClass::Synthetic)
            | (
                ProviderClaim::DirectFile,
                SourceClass::UserSuppliedUntrusted
            )
            | (
                ProviderClaim::ApprovedProvider,
                SourceClass::ProviderAsserted
            )
    );
    if !consistent
        || (e.provider == ProviderClaim::ApprovedProvider) != e.rights_policy_id.is_some()
    {
        return Err(EvidenceError::SourceClaim);
    }
    if !(1..=MAX_ELAPSED).contains(&e.elapsed_seconds)
        || e.ended_at <= e.started_at
        || e.received_at < e.ended_at
        || e.received_at > server_now
        || e.moving_seconds.is_some_and(|v| v > e.elapsed_seconds)
    {
        return Err(EvidenceError::Timeline);
    }
    let duration_ms = (e.ended_at - e.started_at).num_milliseconds();
    let declared_ms = i64::from(e.elapsed_seconds) * 1000;
    // Allow sub-second source precision to round to the declared whole second.
    if (duration_ms - declared_ms).abs() > 999 {
        return Err(EvidenceError::Timeline);
    }
    if !metric(e.distance_m, 1_000_000.0) {
        return Err(EvidenceError::Metric);
    }
    if e.samples.len() > MAX_SAMPLES {
        return Err(EvidenceError::TooManySamples);
    }
    let mut previous_offset = None;
    let mut previous_distance = None;
    for s in &e.samples {
        if i64::from(s.offset_ms) > duration_ms || previous_offset.is_some_and(|v| s.offset_ms <= v)
        {
            return Err(EvidenceError::SampleOrder);
        }
        previous_offset = Some(s.offset_ms);
        // Generous transport sanity limits, not physiological fraud thresholds.
        if !metric(s.distance_m, 1_000_000.0)
            || !metric(s.speed_mps, 1_000.0)
            || !metric(s.heart_rate_bpm, 1_000.0)
            || !metric(s.cadence, 10_000.0)
        {
            return Err(EvidenceError::Metric);
        }
        if let Some(d) = s.distance_m {
            if previous_distance.is_some_and(|v| d < v) {
                return Err(EvidenceError::SampleOrder);
            }
            previous_distance = Some(d);
        }
        if s.lat_e7.is_some() != s.lon_e7.is_some()
            || s.lat_e7
                .is_some_and(|v| !(-900_000_000..=900_000_000).contains(&v))
            || s.lon_e7
                .is_some_and(|v| !(-1_800_000_000..=1_800_000_000).contains(&v))
        {
            return Err(EvidenceError::Coordinates);
        }
    }
    Ok(ParsedEvidence(e))
}

impl ParsedEvidence {
    pub fn record(&self) -> &NormalizedEvidence {
        &self.0
    }

    pub fn cadence_steps_per_minute(&self, index: usize) -> Option<f64> {
        let cadence = self.0.samples.get(index)?.cadence?;
        match self.0.cadence_unit {
            CadenceUnit::StepsPerMinute => Some(cadence),
            CadenceUnit::CyclesPerMinute => Some(cadence * 2.0),
            CadenceUnit::Unknown => None,
        }
    }

    pub fn report(&self) -> Result<ValidationReport, EvidenceError> {
        let bytes = serde_json::to_vec(&self.0).map_err(|_| EvidenceError::InvalidSchema)?;
        let samples = &self.0.samples;
        Ok(ValidationReport {
            schema_version: "1",
            validation_policy: "normalized-envelope-v1",
            source_authenticity: "unverified",
            payment_eligibility: "not_assessed",
            record_sha256: hex::encode(Sha256::digest(bytes)),
            coverage: Coverage {
                samples: samples.len(),
                missing_distance: samples.iter().filter(|s| s.distance_m.is_none()).count(),
                missing_speed: samples.iter().filter(|s| s.speed_mps.is_none()).count(),
                missing_heart_rate: samples
                    .iter()
                    .filter(|s| s.heart_rate_bpm.is_none())
                    .count(),
                missing_cadence: samples.iter().filter(|s| s.cadence.is_none()).count(),
                unknown_cadence_unit: self.0.cadence_unit == CadenceUnit::Unknown,
                zero_heart_rate: samples
                    .iter()
                    .filter(|s| s.heart_rate_bpm == Some(0.0))
                    .count(),
                zero_cadence: samples.iter().filter(|s| s.cadence == Some(0.0)).count(),
                missing_gps: samples.iter().filter(|s| s.lat_e7.is_none()).count(),
            },
        })
    }
}
pub mod upload;
