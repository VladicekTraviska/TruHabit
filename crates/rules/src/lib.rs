//! Goal validation and the historical simulation state machine. Neither attests to payment or data provenance.
pub mod plan;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DAY: i64 = 86_400;
pub const POLICY_VERSION: &str = "demo-distance-v1";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RuleError {
    #[error("Neplatné parametry závazku nebo aktivity.")]
    InvalidInput,
    #[error("Tuto operaci už aktuální stav závazku neumožňuje.")]
    InvalidState,
    #[error("Operace není v povoleném časovém okně.")]
    OutsideWindow,
    #[error("Závazek patří jinému uživateli.")]
    WrongOwner,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MoneyState {
    Funded,
    FailureProposed,
    Disputed,
    RefundedSuccess,
    RefundedCancelled,
    RefundedTimeout,
    RefundedReview,
    Forfeited,
}
impl MoneyState {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Funded | Self::FailureProposed | Self::Disputed)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GoalResult {
    Met,
    NotMet,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidenceResult {
    Sufficient,
    Insufficient,
    NeedsReview,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Assessment {
    pub goal: GoalResult,
    pub evidence: EvidenceResult,
    pub reasons: Vec<String>,
}

/// Deliberately limited synthetic summary; not the production normalized evidence schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub activity_id: String,
    pub starts_at: i64,
    pub ends_at: i64,
    pub distance_m: Option<u32>,
    pub heart_rate_bpm: Option<u16>,
    pub cadence_spm: Option<u16>,
    pub speed_milli_mps: Option<u32>,
    pub is_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub at: i64,
    pub kind: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commitment {
    pub id: String,
    pub owner_id: String,
    pub target_m: u32,
    pub amount_cents: u32,
    pub policy_version: String,
    pub recipient: String,
    pub created_at: i64,
    pub starts_at: i64,
    pub goal_end: i64,
    pub submission_deadline: i64,
    pub decision_notice_deadline: i64,
    pub hard_refund_after: i64,
    pub appeal_end: Option<i64>,
    pub state: MoneyState,
    pub assessment: Option<Assessment>,
    pub evidence: Option<Evidence>,
    pub dispute_reason: Option<String>,
    pub events: Vec<Event>,
}

impl Commitment {
    pub fn create(
        id: String,
        owner_id: String,
        target_m: u32,
        amount_cents: u32,
        now: i64,
    ) -> Result<Self, RuleError> {
        if id.is_empty()
            || owner_id.is_empty()
            || !(1000..=5000).contains(&target_m)
            || !(100..=5000).contains(&amount_cents)
            || !(0..=4_102_444_800).contains(&now)
        {
            return Err(RuleError::InvalidInput);
        }
        let starts_at = now + 300;
        let goal_end = starts_at + DAY;
        let mut c = Self {
            id,
            owner_id,
            target_m,
            amount_cents,
            policy_version: POLICY_VERSION.into(),
            recipient: "Testovací příjemce · bez převodu peněz".into(),
            created_at: now,
            starts_at,
            goal_end,
            submission_deadline: goal_end + DAY,
            decision_notice_deadline: goal_end + 2 * DAY,
            hard_refund_after: goal_end + 12 * DAY,
            appeal_end: None,
            state: MoneyState::Funded,
            assessment: None,
            evidence: None,
            dispute_reason: None,
            events: vec![],
        };
        c.record(
            now,
            "created",
            "Testovací jistina zapsána. Běžecké okno začne za 5 minut.",
        );
        Ok(c)
    }

    fn live(&self, now: i64) -> Result<(), RuleError> {
        if self.state.is_terminal() {
            return Err(RuleError::InvalidState);
        }
        if now < self.created_at
            || self.events.last().is_some_and(|e| now < e.at)
            || now >= self.hard_refund_after
        {
            return Err(RuleError::OutsideWindow);
        }
        Ok(())
    }

    fn owner(&self, owner: &str) -> Result<(), RuleError> {
        if self.owner_id != owner {
            Err(RuleError::WrongOwner)
        } else {
            Ok(())
        }
    }

    fn record(&mut self, now: i64, kind: &str, description: &str) {
        self.events.push(Event {
            at: now,
            kind: kind.into(),
            description: description.into(),
        });
    }

    pub fn cancel(&mut self, owner: &str, now: i64) -> Result<(), RuleError> {
        self.owner(owner)?;
        self.live(now)?;
        if self.state != MoneyState::Funded {
            return Err(RuleError::InvalidState);
        }
        if now >= self.starts_at {
            return Err(RuleError::OutsideWindow);
        }
        self.state = MoneyState::RefundedCancelled;
        self.record(
            now,
            "cancelled",
            "Zrušeno před začátkem. Testovací jistina vrácena.",
        );
        Ok(())
    }

    pub fn evaluate(&mut self, evidence: Evidence, now: i64) -> Result<(), RuleError> {
        self.live(now)?;
        if self.state != MoneyState::Funded || self.evidence.is_some() {
            return Err(RuleError::InvalidState);
        }
        if now > self.submission_deadline || now < evidence.ends_at {
            return Err(RuleError::OutsideWindow);
        }
        if evidence.activity_id.is_empty()
            || evidence.activity_id.len() > 128
            || evidence.ends_at <= evidence.starts_at
            || evidence.starts_at < 0
        {
            return Err(RuleError::InvalidInput);
        }
        let assessment = assess(self, &evidence);
        if assessment.goal == GoalResult::Met && assessment.evidence == EvidenceResult::Sufficient {
            self.state = MoneyState::RefundedSuccess;
        }
        self.record(now, "evaluated", &assessment.reasons.join(" "));
        if self.state == MoneyState::RefundedSuccess {
            self.record(
                now,
                "refunded",
                "Cíl splněn podle testovacích pravidel. Celá testovací jistina vrácena.",
            );
        }
        self.assessment = Some(assessment);
        self.evidence = Some(evidence);
        Ok(())
    }

    /// In production this requires a separately authorized reviewer and confirmed notice.
    pub fn propose_failure(&mut self, now: i64) -> Result<(), RuleError> {
        self.live(now)?;
        if self.state != MoneyState::Funded {
            return Err(RuleError::InvalidState);
        }
        let a = self.assessment.as_ref().ok_or(RuleError::InvalidState)?;
        if a.goal != GoalResult::NotMet || a.evidence != EvidenceResult::Sufficient {
            return Err(RuleError::InvalidState);
        }
        if now <= self.submission_deadline || now > self.decision_notice_deadline {
            return Err(RuleError::OutsideWindow);
        }
        self.state = MoneyState::FailureProposed;
        self.appeal_end = Some(now + 3 * DAY);
        self.record(
            now,
            "failure_proposed",
            "Simulované oznámení nesplnění. Od této chvíle běží 72 hodin na odvolání.",
        );
        Ok(())
    }

    pub fn dispute(&mut self, owner: &str, reason: String, now: i64) -> Result<(), RuleError> {
        self.owner(owner)?;
        self.live(now)?;
        if self.state != MoneyState::FailureProposed {
            return Err(RuleError::InvalidState);
        }
        if now >= self.appeal_end.ok_or(RuleError::InvalidState)? {
            return Err(RuleError::OutsideWindow);
        }
        let reason = reason.trim().to_string();
        if !(10..=1000).contains(&reason.chars().count()) {
            return Err(RuleError::InvalidInput);
        }
        self.state = MoneyState::Disputed;
        self.dispute_reason = Some(reason);
        self.record(
            now,
            "disputed",
            "Odvolání přijato. Propadnutí jistiny je zablokované do přezkumu.",
        );
        Ok(())
    }

    pub fn finalize_failure(&mut self, now: i64) -> Result<(), RuleError> {
        self.live(now)?;
        if self.state != MoneyState::FailureProposed {
            return Err(RuleError::InvalidState);
        }
        if now < self.appeal_end.ok_or(RuleError::InvalidState)? {
            return Err(RuleError::OutsideWindow);
        }
        self.state = MoneyState::Forfeited;
        self.record(
            now,
            "forfeited",
            "Odvolací lhůta skončila bez odvolání. Simulované propadnutí testovacímu příjemci.",
        );
        Ok(())
    }

    pub fn refund_review(&mut self, now: i64) -> Result<(), RuleError> {
        self.live(now)?;
        let uncertain = self
            .assessment
            .as_ref()
            .is_some_and(|a| a.evidence != EvidenceResult::Sufficient);
        if self.state != MoneyState::Disputed && !(self.state == MoneyState::Funded && uncertain) {
            return Err(RuleError::InvalidState);
        }
        self.state = MoneyState::RefundedReview;
        self.record(
            now,
            "review_refund",
            "Simulovaný přezkum ukončen vrácením celé testovací jistiny.",
        );
        Ok(())
    }

    pub fn refund_timeout(&mut self, now: i64) -> Result<(), RuleError> {
        if self.state.is_terminal() {
            return Err(RuleError::InvalidState);
        }
        if now < self.hard_refund_after {
            return Err(RuleError::OutsideWindow);
        }
        self.state = MoneyState::RefundedTimeout;
        self.record(
            now,
            "timeout_refund",
            "Nejzazší termín vypršel. Nevyřešená testovací jistina vrácena.",
        );
        Ok(())
    }
}

fn assess(c: &Commitment, e: &Evidence) -> Assessment {
    let mut reasons = vec![];
    let within_window = e.starts_at >= c.starts_at && e.ends_at <= c.goal_end;
    let goal = match e.distance_m {
        Some(d) if within_window && e.is_run && d >= c.target_m => GoalResult::Met,
        Some(_) => GoalResult::NotMet,
        None => GoalResult::Unknown,
    };
    if !within_window {
        reasons.push("Aktivita neleží celá v běžeckém okně závazku.".into());
    }
    if !e.is_run {
        reasons.push("Záznam není označen jako běh.".into());
    }
    // Missing values are not zeros. No biometric identity or fraud claim is made.
    let completeness = e.distance_m.is_some()
        && e.heart_rate_bpm.is_some()
        && e.cadence_spm.is_some()
        && e.speed_milli_mps.is_some();
    let evidence = if !completeness {
        reasons.push(
            "Chybí část testovacích údajů. Neúplnost není důkaz podvodu; je nutný přezkum.".into(),
        );
        EvidenceResult::Insufficient
    } else if e.cadence_spm == Some(0)
        || e.heart_rate_bpm == Some(0)
        || e.speed_milli_mps.is_some_and(|v| v > 6000)
    {
        reasons.push("Záznam obsahuje hodnoty mimo jednoduchá demo pravidla. Anomálie sama neprokazuje podvod.".into());
        EvidenceResult::NeedsReview
    } else {
        reasons.push("Syntetický záznam je úplný podle demo pravidel; toto neověřuje identitu ani pravost dat.".into());
        EvidenceResult::Sufficient
    };
    if let Some(distance) = e.distance_m {
        reasons.push(format!(
            "Vzdálenost {distance} m / cíl {} m. Tempo není podmínkou.",
            c.target_m
        ));
    }
    Assessment {
        goal,
        evidence,
        reasons,
    }
}
