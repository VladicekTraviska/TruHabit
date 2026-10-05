use truhabit_rules::*;

const NOW: i64 = 1_800_000_000;
fn commitment() -> Commitment {
    Commitment::create("c1".into(), "owner".into(), 5000, 1000, NOW).unwrap()
}
fn evidence(c: &Commitment) -> Evidence {
    Evidence {
        activity_id: "activity-1".into(),
        starts_at: c.starts_at + 60,
        ends_at: c.starts_at + 60 + 1575,
        distance_m: Some(5000),
        heart_rate_bpm: Some(162),
        cadence_spm: Some(172),
        speed_milli_mps: Some(3175),
        is_run: true,
    }
}
fn short_run() -> Commitment {
    let mut c = commitment();
    let mut e = evidence(&c);
    e.distance_m = Some(2500);
    let now = e.ends_at;
    c.evaluate(e, now).unwrap();
    c
}
fn proposed() -> Commitment {
    let mut c = short_run();
    c.propose_failure(c.submission_deadline + 1).unwrap();
    c
}

#[test]
fn validates_limits_and_overflow_before_setting_deadlines() {
    for (distance, amount, now) in [
        (999, 1000, NOW),
        (5001, 1000, NOW),
        (1000, 99, NOW),
        (1000, 5001, NOW),
        (1000, 1000, i64::MAX),
        (1000, 1000, -1),
    ] {
        assert!(Commitment::create("c".into(), "o".into(), distance, amount, now).is_err());
    }
    let c = commitment();
    assert_eq!(c.hard_refund_after - c.goal_end, 12 * DAY);
}

#[test]
fn distance_goal_does_not_secretly_require_five_k_under_twenty_five_minutes() {
    let mut c = commitment();
    let e = evidence(&c);
    assert_eq!(e.ends_at - e.starts_at, 26 * 60 + 15);
    c.evaluate(e.clone(), e.ends_at).unwrap();
    assert_eq!(c.state, MoneyState::RefundedSuccess);
    assert_eq!(c.amount_cents, 1000);
    assert_eq!(c.assessment.unwrap().goal, GoalResult::Met);
}

#[test]
fn missing_heart_rate_is_neither_zero_nor_proof_of_failure() {
    let mut c = commitment();
    let mut e = evidence(&c);
    e.heart_rate_bpm = None;
    c.evaluate(e.clone(), e.ends_at).unwrap();
    assert_eq!(c.state, MoneyState::Funded);
    let a = c.assessment.as_ref().unwrap();
    assert_eq!(a.goal, GoalResult::Met);
    assert_eq!(a.evidence, EvidenceResult::Insufficient);
    assert_eq!(
        c.propose_failure(c.submission_deadline + 1),
        Err(RuleError::InvalidState)
    );
    c.refund_review(c.submission_deadline + 1).unwrap();
    assert_eq!(c.state, MoneyState::RefundedReview);
}

#[test]
fn zero_cadence_and_high_speed_require_review_never_automatic_forfeiture() {
    let mut c = commitment();
    let mut e = evidence(&c);
    e.cadence_spm = Some(0);
    e.heart_rate_bpm = Some(78);
    e.speed_milli_mps = Some(6111);
    c.evaluate(e.clone(), e.ends_at).unwrap();
    assert_eq!(
        c.assessment.as_ref().unwrap().evidence,
        EvidenceResult::NeedsReview
    );
    assert!(c.propose_failure(c.submission_deadline + 1).is_err());
    assert!(c.finalize_failure(c.hard_refund_after - 1).is_err());
    c.refund_timeout(c.hard_refund_after).unwrap();
    assert_eq!(c.state, MoneyState::RefundedTimeout);
}

#[test]
fn cancellation_requires_owner_and_is_strictly_before_start() {
    let mut c = commitment();
    assert_eq!(c.cancel("attacker", NOW), Err(RuleError::WrongOwner));
    assert_eq!(
        c.cancel("owner", c.starts_at),
        Err(RuleError::OutsideWindow)
    );
    c.cancel("owner", c.starts_at - 1).unwrap();
    assert_eq!(c.state, MoneyState::RefundedCancelled);
    assert!(c.cancel("owner", c.starts_at - 1).is_err());
}

#[test]
fn old_activity_cannot_satisfy_a_new_commitment() {
    let mut c = commitment();
    let mut e = evidence(&c);
    e.starts_at -= DAY * 2;
    e.ends_at -= DAY * 2;
    c.evaluate(e, NOW).unwrap();
    assert_eq!(c.state, MoneyState::Funded);
    assert_eq!(c.assessment.unwrap().goal, GoalResult::NotMet);
}

#[test]
fn future_activity_and_late_submission_are_rejected_without_events() {
    let mut c = commitment();
    let e = evidence(&c);
    assert_eq!(
        c.evaluate(e.clone(), e.ends_at - 1),
        Err(RuleError::OutsideWindow)
    );
    assert_eq!(
        c.evaluate(e, c.submission_deadline + 1),
        Err(RuleError::OutsideWindow)
    );
    assert_eq!(c.events.len(), 1);
    assert!(c.evidence.is_none());
}

#[test]
fn submission_at_exact_deadline_is_allowed() {
    let mut c = commitment();
    let e = evidence(&c);
    c.evaluate(e, c.submission_deadline).unwrap();
    assert_eq!(c.state, MoneyState::RefundedSuccess);
}

#[test]
fn invalid_activity_and_missing_distance_do_not_return_success() {
    let mut c = commitment();
    let mut e = evidence(&c);
    e.ends_at = e.starts_at;
    assert_eq!(
        c.evaluate(e.clone(), c.submission_deadline),
        Err(RuleError::InvalidInput)
    );
    e.ends_at += 100;
    e.distance_m = None;
    c.evaluate(e, c.submission_deadline).unwrap();
    let a = c.assessment.unwrap();
    assert_eq!(a.goal, GoalResult::Unknown);
    assert_eq!(a.evidence, EvidenceResult::Insufficient);
}

#[test]
fn proposal_only_in_notice_window_with_complete_not_met_evidence() {
    let mut c = short_run();
    assert_eq!(
        c.propose_failure(c.submission_deadline),
        Err(RuleError::OutsideWindow)
    );
    assert_eq!(
        c.propose_failure(c.decision_notice_deadline + 1),
        Err(RuleError::OutsideWindow)
    );
    c.propose_failure(c.decision_notice_deadline).unwrap();
    assert_eq!(c.appeal_end, Some(c.decision_notice_deadline + 3 * DAY));
    let mut empty = commitment();
    assert!(
        empty
            .propose_failure(empty.submission_deadline + 1)
            .is_err()
    );
}

#[test]
fn forfeiture_waits_for_full_appeal_window() {
    let mut c = proposed();
    let end = c.appeal_end.unwrap();
    assert_eq!(c.finalize_failure(end - 1), Err(RuleError::OutsideWindow));
    c.finalize_failure(end).unwrap();
    assert_eq!(c.state, MoneyState::Forfeited);
    assert!(c.finalize_failure(end).is_err());
}

#[test]
fn valid_dispute_blocks_forfeiture_until_review_refund() {
    let mut c = proposed();
    let end = c.appeal_end.unwrap();
    assert_eq!(
        c.dispute("attacker", "Důvod odvolání".into(), end - 1),
        Err(RuleError::WrongOwner)
    );
    c.dispute("owner", "Prosím o kontrolu záznamu.".into(), end - 1)
        .unwrap();
    assert_eq!(c.finalize_failure(end), Err(RuleError::InvalidState));
    c.refund_review(end).unwrap();
    assert_eq!(c.state, MoneyState::RefundedReview);
}

#[test]
fn appeal_end_boundary_and_blank_reason_cannot_create_dispute() {
    let mut c = proposed();
    let end = c.appeal_end.unwrap();
    assert_eq!(
        c.dispute("owner", "Dost dlouhý důvod".into(), end),
        Err(RuleError::OutsideWindow)
    );
    assert_eq!(
        c.dispute("owner", "          ".into(), end - 1),
        Err(RuleError::InvalidInput)
    );
    assert_eq!(c.state, MoneyState::FailureProposed);
}

#[test]
fn hard_timeout_wins_over_forfeiture_at_exact_boundary() {
    let mut c = proposed();
    let deadline = c.hard_refund_after;
    assert_eq!(c.finalize_failure(deadline), Err(RuleError::OutsideWindow));
    c.refund_timeout(deadline).unwrap();
    assert_eq!(c.state, MoneyState::RefundedTimeout);
}

#[test]
fn every_nonterminal_state_can_timeout_and_terminal_states_cannot_settle_again() {
    let mut disputed = proposed();
    disputed
        .dispute(
            "owner",
            "Záznam je potřeba znovu prověřit".into(),
            disputed.appeal_end.unwrap() - 1,
        )
        .unwrap();
    for mut c in [commitment(), proposed(), disputed] {
        let deadline = c.hard_refund_after;
        assert_eq!(
            c.refund_timeout(deadline - 1),
            Err(RuleError::OutsideWindow)
        );
        c.refund_timeout(deadline).unwrap();
        let events = c.events.len();
        assert!(c.refund_timeout(deadline).is_err());
        assert!(c.refund_review(deadline).is_err());
        assert!(c.finalize_failure(deadline).is_err());
        assert!(c.cancel("owner", deadline).is_err());
        assert!(c.evaluate(evidence(&c), deadline).is_err());
        assert_eq!(c.events.len(), events);
        assert_eq!(c.amount_cents, 1000);
    }
}

#[test]
fn second_evidence_cannot_overwrite_first_assessment() {
    let mut c = short_run();
    let e = evidence(&c);
    assert_eq!(
        c.evaluate(e.clone(), e.ends_at),
        Err(RuleError::InvalidState)
    );
    assert_eq!(c.evidence.unwrap().distance_m, Some(2500));
}

#[test]
fn clock_cannot_go_back_behind_last_event() {
    let mut c = proposed();
    assert_eq!(
        c.dispute("owner", "Dlouhý důvod odvolání".into(), NOW),
        Err(RuleError::OutsideWindow)
    );
}
