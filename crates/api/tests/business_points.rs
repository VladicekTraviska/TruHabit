//! Company point flows against a disposable PostgreSQL schema, never a live workspace.
use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use chrono::{DateTime, Duration, Utc};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use truhabit_api::{AppState, MIGRATOR, config::Config, router};
use uuid::Uuid;

const PASSWORD: &str = "a long unique company points test passphrase 123";
const VALID: &[u8] = include_bytes!("../../../web/public/prototype/valid-run.gpx");
const REVIEW: &[u8] = include_bytes!("../../../web/public/prototype/review-run.gpx");
const SHORT: &[u8] = include_bytes!("../../../web/public/prototype/short-run.gpx");

#[tokio::test]
async fn company_pool_is_owner_funded_idempotent_and_separate_from_personal_credits() {
    let t = TestApp::new().await;
    let owner = t.account("pool-owner@example.test").await;
    let admin = t.account("pool-admin@example.test").await;
    let member = t.account("pool-member@example.test").await;
    let stranger = t.account("pool-stranger@example.test").await;
    let org = t.org(&owner).await;
    t.member(org, &admin, "ADMIN").await;
    t.member(org, &member, "MEMBER").await;
    let owner_local = t.local_balance(&owner).await;
    let member_local = t.local_balance(&member).await;
    let path = format!("/api/organizations/{org}/points/top-up");
    let id = Uuid::new_v4();
    let request = json!({"id":id,"points":2000});
    for actor in [&admin, &member] {
        let denied = t.req("POST", &path, request.clone(), actor).await;
        assert_eq!(denied.0, StatusCode::FORBIDDEN, "{}", denied.1);
    }
    let first = t.req("POST", &path, request.clone(), &owner).await;
    assert_eq!(first.0, StatusCode::OK, "{}", first.1);
    let retry = t.req("POST", &path, request, &owner).await;
    assert_eq!(retry.0, StatusCode::OK, "{}", retry.1);
    assert_eq!(retry.1["pool_available_points"], 2000);
    let changed = t
        .req("POST", &path, json!({"id":id,"points":2001}), &owner)
        .await;
    assert_eq!(changed.0, StatusCode::CONFLICT, "{}", changed.1);
    for points in [0_i64, -1, 1_000_000_001] {
        let invalid = t
            .req(
                "POST",
                &path,
                json!({"id":Uuid::new_v4(),"points":points}),
                &owner,
            )
            .await;
        assert_eq!(invalid.0, StatusCode::BAD_REQUEST, "{}", invalid.1);
    }
    let manager = t.points(org, &owner).await;
    assert_eq!(manager["unit"], "POINTS");
    assert_eq!(manager["simulation"], true);
    assert_eq!(manager["pool_available_points"], 2000);
    assert_eq!(manager["pool_reserved_points"], 0);
    assert_eq!(manager["total_awarded_points"], 0);
    let personal = t.points(org, &member).await;
    assert_eq!(personal["own_available_points"], 0);
    assert_eq!(personal["own_staked_points"], 0);
    assert!(personal.get("pool_available_points").is_none());
    assert!(personal.get("pool_reserved_points").is_none());
    assert!(personal.get("total_awarded_points").is_none());
    assert!(personal["movements"].as_array().unwrap().is_empty());
    let forbidden = t
        .req(
            "GET",
            &format!("/api/organizations/{org}/points"),
            Value::Null,
            &stranger,
        )
        .await;
    assert_eq!(forbidden.0, StatusCode::NOT_FOUND);
    assert_eq!(t.local_balance(&owner).await, owner_local);
    assert_eq!(t.local_balance(&member).await, member_local);
    assert_eq!(t.movement_count(org, "TOP_UP").await, 1);
    t.assert_conserved(org).await;
    t.finish().await;
}

#[tokio::test]
async fn funded_publication_retry_keeps_its_terms_after_the_start_without_reserving_again() {
    let t = TestApp::new().await;
    let owner = t.account("publish-retry-owner@example.test").await;
    let org = t.org(&owner).await;
    t.top_up(org, &owner, 1000).await;
    let program = t.draft(org, &owner, "ACTIVITY_POINTS", 100, 0, 2).await;
    let start = Utc::now() + Duration::minutes(10);
    let end = start + Duration::hours(1);
    let path = format!("{}/publish", program_path(org, program));
    let published = t.req("POST", &path, json!({"version":1,"profile":"LIVE","starts_at":start,"ends_at":end,"upload_deadline":end+Duration::hours(1),"review_deadline":end+Duration::hours(2)}), &owner).await;
    assert_eq!(published.0, StatusCode::OK, "{}", published.1);
    // Advance the stored agreement's clock without sleeping or touching a live
    // database, then retry exactly those stored, previously funded terms.
    let elapsed = Utc::now() - Duration::minutes(1);
    sqlx::query("UPDATE company_programs SET starts_at=$1 WHERE id=$2")
        .bind(elapsed)
        .bind(program)
        .execute(&t.pool)
        .await
        .unwrap();
    let retry = t.req("POST", &path, json!({"version":1,"profile":"LIVE","starts_at":elapsed,"ends_at":end,"upload_deadline":end+Duration::hours(1),"review_deadline":end+Duration::hours(2)}), &owner).await;
    assert_eq!(retry.0, StatusCode::OK, "{}", retry.1);
    assert_eq!(retry.1["state"], "PUBLISHED");
    assert_eq!(retry.1["version"], 2);
    assert_eq!(t.movement_count(org, "FUND").await, 1);
    assert_eq!(t.points(org, &owner).await["pool_available_points"], 800);
    assert_eq!(t.points(org, &owner).await["pool_reserved_points"], 200);
    let changed = t.req("POST", &path, json!({"version":2,"profile":"LIVE","starts_at":start,"ends_at":end+Duration::minutes(1),"upload_deadline":end+Duration::hours(1),"review_deadline":end+Duration::hours(2)}), &owner).await;
    assert_eq!(changed.0, StatusCode::CONFLICT, "{}", changed.1);
    t.assert_conserved(org).await;
    t.finish().await;
}

#[tokio::test]
async fn accepted_run_awards_points_once_even_with_parallel_upload_and_retries() {
    let t = TestApp::new().await;
    let owner = t.account("automatic-owner@example.test").await;
    let runner = t.account("automatic-runner@example.test").await;
    let org = t.org(&owner).await;
    t.member(org, &runner, "MEMBER").await;
    let owner_local = t.local_balance(&owner).await;
    let runner_local = t.local_balance(&runner).await;
    t.top_up(org, &owner, 1000).await;
    let p = t.draft(org, &owner, "ACTIVITY_POINTS", 100, 0, 1).await;
    t.publish(org, p, &owner).await;
    let enrollment = t.join(org, p, &runner, false).await;
    let path = enrollment_path(org, p, enrollment);
    let upload_path = format!("{path}/upload");
    let (a, b) = tokio::join!(
        t.raw(&upload_path, VALID, &runner),
        t.raw(&upload_path, VALID, &runner)
    );
    for response in [&a, &b] {
        assert_eq!(response.0, StatusCode::OK, "{}", response.1);
        assert_eq!(response.1["decision"], "ACCEPTED");
    }
    assert_eq!(a.1["id"], b.1["id"]);
    assert_ne!(a.1["duplicate"], b.1["duplicate"]);
    let own = t.req("GET", &path, Value::Null, &runner).await;
    assert_eq!(own.0, StatusCode::OK);
    assert_eq!(own.1["enrollment"]["state"], "REWARDED");
    assert_eq!(own.1["enrollment"]["awarded_points"], 100);
    assert_eq!(own.1["uploads"].as_array().unwrap().len(), 1);
    for _ in 0..2 {
        let claim = t
            .req("POST", &format!("{path}/claim"), json!({}), &runner)
            .await;
        assert_eq!(claim.0, StatusCode::OK, "{}", claim.1);
    }
    t.close(org, p, &owner).await;
    let late_retry = t.raw(&upload_path, VALID, &runner).await;
    assert_eq!(late_retry.0, StatusCode::OK, "{}", late_retry.1);
    assert_eq!(late_retry.1["duplicate"], true);
    let wallet = t.points(org, &runner).await;
    assert_eq!(wallet["own_available_points"], 100);
    assert_eq!(wallet["own_staked_points"], 0);
    let pool = t.points(org, &owner).await;
    assert_eq!(pool["pool_available_points"], 900);
    assert_eq!(pool["pool_reserved_points"], 0);
    assert_eq!(pool["total_awarded_points"], 100);
    assert_eq!(t.movement_count(org, "REWARD").await, 1);
    assert_eq!(t.local_balance(&owner).await, owner_local);
    assert_eq!(t.local_balance(&runner).await, runner_local);
    let exported = t
        .req("GET", "/api/account/export", Value::Null, &runner)
        .await;
    assert_eq!(exported.0, StatusCode::OK, "{}", exported.1);
    let movements = exported.1["business_point_movements"].as_array().unwrap();
    assert_eq!(movements.len(), 1);
    assert_eq!(movements[0]["kind"], "REWARD");
    assert_eq!(movements[0]["employee_delta"], 100);
    assert!(movements[0].get("request_hash").is_none());
    let deletion = t
        .req(
            "DELETE",
            "/api/account",
            json!({"password":PASSWORD,"confirmation":"DELETE"}),
            &runner,
        )
        .await;
    assert_eq!(deletion.0, StatusCode::CONFLICT, "{}", deletion.1);
    assert!(deletion.1.to_string().contains("firemní body"));
    let version: i32 = sqlx::query_scalar("SELECT version FROM organizations WHERE id=$1")
        .bind(org)
        .fetch_one(&t.pool)
        .await
        .unwrap();
    let archived = t
        .req(
            "POST",
            &format!("/api/organizations/{org}/archive"),
            json!({"version":version}),
            &owner,
        )
        .await;
    assert_eq!(archived.0, StatusCode::OK, "{}", archived.1);
    let settled_retry = t
        .req("POST", &format!("{path}/claim"), json!({}), &runner)
        .await;
    assert_eq!(settled_retry.0, StatusCode::OK, "{}", settled_retry.1);
    assert_eq!(settled_retry.1["awarded_points"], 100);
    assert_eq!(t.movement_count(org, "REWARD").await, 1);
    t.assert_conserved(org).await;
    t.finish().await;
}

#[tokio::test]
async fn employer_match_needs_consent_and_own_points_then_returns_pledge_and_bonus() {
    let t = TestApp::new().await;
    let owner = t.account("match-owner@example.test").await;
    let runner = t.account("match-runner@example.test").await;
    let empty = t.account("match-empty@example.test").await;
    let org = t.org(&owner).await;
    for actor in [&runner, &empty] {
        t.member(org, actor, "MEMBER").await;
    }
    t.top_up(org, &owner, 2000).await;
    t.earn(org, &owner, &runner, 200, VALID).await;
    let p = t.draft(org, &owner, "EMPLOYER_MATCH", 800, 200, 1).await;
    t.publish(org, p, &owner).await;
    let join_path = format!("{}/join", program_path(org, p));
    let id = Uuid::new_v4();
    let no_consent = t.req("POST", &join_path, json!({"id":id}), &runner).await;
    assert_eq!(no_consent.0, StatusCode::BAD_REQUEST, "{}", no_consent.1);
    assert_eq!(t.points(org, &runner).await["own_available_points"], 200);
    let insufficient = t
        .req(
            "POST",
            &join_path,
            json!({"id":Uuid::new_v4(),"accept_terms":true}),
            &empty,
        )
        .await;
    assert_eq!(
        insufficient.0,
        StatusCode::BAD_REQUEST,
        "{}",
        insufficient.1
    );
    let request = json!({"id":id,"accept_terms":true});
    let joined = t.req("POST", &join_path, request.clone(), &runner).await;
    assert_eq!(joined.0, StatusCode::OK, "{}", joined.1);
    let retried = t.req("POST", &join_path, request, &runner).await;
    assert_eq!(retried.0, StatusCode::OK, "{}", retried.1);
    assert_eq!(joined.1["id"], retried.1["id"]);
    assert_eq!(joined.1["staked_points"], 200);
    assert!(joined.1["consented_at"].is_string());
    assert!(joined.1["terms_version"].as_i64().unwrap() > 0);
    let wallet = t.points(org, &runner).await;
    assert_eq!(wallet["own_available_points"], 0);
    assert_eq!(wallet["own_staked_points"], 200);
    assert_eq!(t.movement_count(org, "STAKE_LOCK").await, 1);
    let enrollment = id_from(&joined.1);
    let run = distinct_run(2);
    let accepted = t
        .raw(
            &format!("{}/upload", enrollment_path(org, p, enrollment)),
            &run,
            &runner,
        )
        .await;
    assert_eq!(accepted.0, StatusCode::OK, "{}", accepted.1);
    assert_eq!(accepted.1["decision"], "ACCEPTED");
    let wallet = t.points(org, &runner).await;
    assert_eq!(wallet["own_available_points"], 1000);
    assert_eq!(wallet["own_staked_points"], 0);
    assert_eq!(t.movement_count(org, "STAKE_REFUND").await, 1);
    assert_eq!(t.movement_count(org, "STAKE_FORFEIT").await, 0);
    t.close(org, p, &owner).await;
    let pool = t.points(org, &owner).await;
    assert_eq!(pool["pool_available_points"], 1000);
    assert_eq!(pool["pool_reserved_points"], 0);
    t.assert_conserved(org).await;
    t.finish().await;
}

#[tokio::test]
async fn unmet_match_settles_only_after_deadline_and_forfeits_pledge_once() {
    let t = TestApp::new().await;
    let owner = t.account("unmet-owner@example.test").await;
    let runner = t.account("unmet-runner@example.test").await;
    let org = t.org(&owner).await;
    t.member(org, &runner, "MEMBER").await;
    t.top_up(org, &owner, 2000).await;
    t.earn(org, &owner, &runner, 200, VALID).await;
    let p = t.draft(org, &owner, "EMPLOYER_MATCH", 800, 200, 1).await;
    t.publish(org, p, &owner).await;
    let e = t.join(org, p, &runner, true).await;
    let rejected = t
        .raw(
            &format!("{}/upload", enrollment_path(org, p, e)),
            SHORT,
            &runner,
        )
        .await;
    assert_eq!(rejected.0, StatusCode::OK, "{}", rejected.1);
    assert_eq!(rejected.1["decision"], "REJECTED");
    let premature = t
        .req(
            "POST",
            &format!("{}/close", program_path(org, p)),
            json!({"version":t.version(org,p,&owner).await}),
            &owner,
        )
        .await;
    assert_eq!(premature.0, StatusCode::CONFLICT, "{}", premature.1);
    assert_eq!(t.points(org, &runner).await["own_staked_points"], 200);
    assert_eq!(t.movement_count(org, "STAKE_FORFEIT").await, 0);
    t.expire_upload(p).await;
    // The closer's ledger FK must remain compatible with an employee's account
    // mutation lock. Otherwise close and an upload waiting on the company row
    // can deadlock even though they touch different users.
    let mut runner_guard = t.pool.begin().await.unwrap();
    truhabit_api::auth::lock_user(&mut runner_guard, runner.user)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), t.close(org, p, &owner))
        .await
        .expect("Company closure must not wait on an employee FK check");
    runner_guard.rollback().await.unwrap();
    t.close(org, p, &owner).await;
    let wallet = t.points(org, &runner).await;
    assert_eq!(wallet["own_available_points"], 0);
    assert_eq!(wallet["own_staked_points"], 0);
    let pool = t.points(org, &owner).await;
    assert_eq!(pool["pool_available_points"], 2000);
    assert_eq!(pool["pool_reserved_points"], 0);
    assert_eq!(t.movement_count(org, "STAKE_FORFEIT").await, 1);
    assert_eq!(t.movement_count(org, "STAKE_REFUND").await, 0);
    let own = t
        .req("GET", &enrollment_path(org, p, e), Value::Null, &runner)
        .await;
    assert_eq!(own.1["enrollment"]["state"], "CLOSED");
    assert_eq!(own.1["enrollment"]["awarded_points"], 0);
    t.assert_conserved(org).await;
    t.finish().await;
}

#[tokio::test]
async fn monthly_allocation_declines_without_erasing_earned_points_and_next_cycle_is_a_draft() {
    let t = TestApp::new().await;
    let owner = t.account("monthly-owner@example.test").await;
    let runner = t.account("monthly-runner@example.test").await;
    let org = t.org(&owner).await;
    t.member(org, &runner, "MEMBER").await;
    t.top_up(org, &owner, 3000).await;
    t.earn(org, &owner, &runner, 100, &distinct_run(2)).await;
    let p = t.draft(org, &owner, "MONTHLY_BUDGET", 1000, 0, 1).await;
    t.publish(org, p, &owner).await;
    let consent = t
        .req(
            "POST",
            &format!("{}/join", program_path(org, p)),
            json!({"id":Uuid::new_v4()}),
            &runner,
        )
        .await;
    assert_eq!(consent.0, StatusCode::BAD_REQUEST, "{}", consent.1);
    let e = t.join(org, p, &runner, true).await;
    assert_eq!(t.points(org, &runner).await["own_available_points"], 100);
    assert_eq!(t.points(org, &runner).await["own_staked_points"], 0);
    // The fixture ends at 10:18, exactly halfway through this activity window.
    // Its upload remains admissible now; only the test clock/window is adjusted.
    let start: DateTime<Utc> = "2026-09-01T09:18:00Z".parse().unwrap();
    let end: DateTime<Utc> = "2026-09-01T11:18:00Z".parse().unwrap();
    let upload = Utc::now() + Duration::minutes(10);
    sqlx::query("UPDATE company_programs SET profile='LIVE',starts_at=$1,ends_at=$2,upload_deadline=$3,review_deadline=$4 WHERE id=$5")
        .bind(start).bind(end).bind(upload).bind(upload+Duration::minutes(5)).bind(p).execute(&t.pool).await.unwrap();
    let accepted = t
        .raw(
            &format!("{}/upload", enrollment_path(org, p, e)),
            VALID,
            &runner,
        )
        .await;
    assert_eq!(accepted.0, StatusCode::OK, "{}", accepted.1);
    assert_eq!(accepted.1["decision"], "ACCEPTED");
    let own = t
        .req("GET", &enrollment_path(org, p, e), Value::Null, &runner)
        .await;
    assert_eq!(own.1["enrollment"]["awarded_points"], 500);
    assert_eq!(t.points(org, &runner).await["own_available_points"], 600);
    let pool = t.points(org, &owner).await;
    assert_eq!(pool["pool_available_points"], 2400);
    assert_eq!(pool["pool_reserved_points"], 0);
    t.close(org, p, &owner).await;
    let next_id = Uuid::new_v4();
    let request = json!({"id":next_id,"version":t.version(org,p,&owner).await});
    let next_path = format!("{}/next-cycle", program_path(org, p));
    let next = t.req("POST", &next_path, request.clone(), &owner).await;
    assert_eq!(next.0, StatusCode::OK, "{}", next.1);
    assert_eq!(next.1["id"], next_id.to_string());
    assert_eq!(next.1["state"], "DRAFT");
    assert_eq!(next.1["template"], "MONTHLY_BUDGET");
    assert_eq!(next.1["point_reward"], 1000);
    assert_eq!(next.1["previous_cycle_id"], p.to_string());
    assert_eq!(next.1["budget_units"], 0);
    assert!(next.1["published_at"].is_null());
    assert_eq!(next.1["cycle_starts_at"], "2026-10-01T09:18:00Z");
    assert_eq!(next.1["cycle_ends_at"], "2026-10-01T11:18:00Z");
    let repeat = t.req("POST", &next_path, request, &owner).await;
    assert_eq!(repeat.0, StatusCode::OK, "{}", repeat.1);
    assert_eq!(repeat.1, next.1);
    let duplicate_month = t
        .req(
            "POST",
            &next_path,
            json!({"id":Uuid::new_v4(),"version":t.version(org,p,&owner).await}),
            &owner,
        )
        .await;
    assert_eq!(
        duplicate_month.0,
        StatusCode::CONFLICT,
        "{}",
        duplicate_month.1
    );
    assert!(
        duplicate_month
            .1
            .to_string()
            .contains("BUSINESS_NEXT_CYCLE_EXISTS")
    );
    assert_eq!(t.points(org, &runner).await["own_available_points"], 600);
    assert_eq!(t.points(org, &owner).await["pool_available_points"], 2400);
    assert_eq!(t.movement_count(org, "REWARD").await, 2);
    t.assert_conserved(org).await;
    t.finish().await;
}

#[tokio::test]
async fn review_requires_an_operator_and_employer_cannot_read_private_evidence_or_other_wallets() {
    let t = TestApp::new().await;
    let owner = t.account("review-points-owner@example.test").await;
    let runner = t.account("review-points-runner@example.test").await;
    let operator = t.account("review-points-operator@example.test").await;
    let org = t.org(&owner).await;
    t.member(org, &runner, "MEMBER").await;
    sqlx::query("INSERT INTO prototype_operators(user_id) VALUES($1)")
        .bind(operator.user)
        .execute(&t.pool)
        .await
        .unwrap();
    t.top_up(org, &owner, 1000).await;
    let p = t.draft(org, &owner, "EVENT", 100, 0, 1).await;
    t.publish(org, p, &owner).await;
    let e = t.join(org, p, &runner, false).await;
    let private = enrollment_path(org, p, e);
    let upload = t.raw(&format!("{private}/upload"), REVIEW, &runner).await;
    assert_eq!(upload.0, StatusCode::OK, "{}", upload.1);
    assert_eq!(upload.1["decision"], "REVIEW_REQUIRED");
    assert_eq!(t.points(org, &runner).await["own_available_points"], 0);
    let managed = t
        .req("GET", &program_path(org, p), Value::Null, &owner)
        .await;
    for forbidden in [
        "heart_rate",
        "cadence",
        "fingerprint",
        "<gpx",
        "uploads",
        "telemetry",
    ] {
        assert!(
            !managed.1.to_string().contains(forbidden),
            "Employer response exposed {forbidden}: {}",
            managed.1
        );
    }
    assert_eq!(
        t.req("GET", &private, Value::Null, &owner).await.0,
        StatusCode::NOT_FOUND
    );
    let source = format!(
        "{private}/uploads/{}/source",
        upload.1["id"].as_str().unwrap()
    );
    assert_eq!(
        t.req("GET", &source, Value::Null, &owner).await.0,
        StatusCode::NOT_FOUND
    );
    let review = json!({"upload_id":upload.1["id"],"accept":true,"reason":"Reviewed this synthetic recording and accepted its plausibility."});
    assert_eq!(
        t.req("POST", &format!("{private}/review"), review.clone(), &owner)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    t.expire_upload(p).await;
    let premature = t
        .req(
            "POST",
            &format!("{}/close", program_path(org, p)),
            json!({"version":t.version(org,p,&owner).await}),
            &owner,
        )
        .await;
    assert_eq!(premature.0, StatusCode::CONFLICT, "{}", premature.1);
    assert_eq!(premature.1["error"], "BUSINESS_REVIEW_PENDING");
    let accepted = t
        .req("POST", &format!("{private}/review"), review, &operator)
        .await;
    assert_eq!(accepted.0, StatusCode::OK, "{}", accepted.1);
    assert_eq!(t.points(org, &runner).await["own_available_points"], 100);
    assert_eq!(t.movement_count(org, "REWARD").await, 1);
    let owner_points = t.points(org, &owner).await;
    assert_eq!(owner_points["own_available_points"], 0);
    assert!(
        owner_points["movements"]
            .as_array()
            .unwrap()
            .iter()
            .all(|movement| {
                movement["user_id"].is_null() || movement["user_id"] == owner.user.to_string()
            })
    );
    let owned = t.points(org, &runner).await;
    assert_eq!(owned["movements"].as_array().unwrap().len(), 1);
    assert_eq!(owned["movements"][0]["kind"], "REWARD");
    let claim = t
        .req("POST", &format!("{private}/claim"), json!({}), &runner)
        .await;
    assert_eq!(claim.0, StatusCode::OK, "{}", claim.1);
    t.close(org, p, &owner).await;
    t.assert_conserved(org).await;
    t.finish().await;
}

#[tokio::test]
async fn insufficient_pool_and_invalid_templates_cannot_publish_or_mutate_points() {
    let t = TestApp::new().await;
    let owner = t.account("invalid-points-owner@example.test").await;
    let org = t.org(&owner).await;
    let p = t.draft(org, &owner, "ACTIVITY_POINTS", 100, 0, 2).await;
    t.top_up(org, &owner, 199).await;
    let path = format!("{}/publish", program_path(org, p));
    let missing = t.req("POST", &path, TestApp::publish_input(), &owner).await;
    assert_eq!(missing.0, StatusCode::BAD_REQUEST, "{}", missing.1);
    assert_eq!(t.points(org, &owner).await["pool_available_points"], 199);
    assert_eq!(t.points(org, &owner).await["pool_reserved_points"], 0);
    for (template, reward, stake) in [
        ("UNKNOWN", 100, 0),
        ("ACTIVITY_POINTS", 0, 0),
        ("ACTIVITY_POINTS", 100, 10),
        ("EMPLOYER_MATCH", 100, 0),
        ("EVENT", 100_001, 0),
    ] {
        let invalid = t
            .req(
                "POST",
                &format!("/api/organizations/{org}/programs"),
                draft_input(Uuid::new_v4(), template, reward, stake, 1),
                &owner,
            )
            .await;
        assert_eq!(invalid.0, StatusCode::BAD_REQUEST, "{}", invalid.1);
    }
    t.top_up(org, &owner, 1).await;
    t.publish(org, p, &owner).await;
    let repeated = t.req("POST", &path, TestApp::publish_input(), &owner).await;
    assert_eq!(repeated.0, StatusCode::OK, "{}", repeated.1);
    assert_eq!(t.points(org, &owner).await["pool_available_points"], 0);
    assert_eq!(t.points(org, &owner).await["pool_reserved_points"], 200);
    assert_eq!(t.movement_count(org, "FUND").await, 1);
    t.assert_conserved(org).await;
    t.finish().await;
}

#[tokio::test]
async fn point_wallets_are_scoped_to_company_and_parallel_join_respects_capacity() {
    let t = TestApp::new().await;
    let owner = t.account("scoped-owner@example.test").await;
    let runner = t.account("scoped-runner@example.test").await;
    let other = t.account("scoped-other@example.test").await;
    let org = t.org(&owner).await;
    let second_org = t.org(&owner).await;
    for organization in [org, second_org] {
        for actor in [&runner, &other] {
            t.member(organization, actor, "MEMBER").await;
        }
        t.top_up(organization, &owner, 1000).await;
    }
    t.earn(org, &owner, &runner, 200, VALID).await;
    let matched = t
        .draft(second_org, &owner, "EMPLOYER_MATCH", 800, 200, 1)
        .await;
    t.publish(second_org, matched, &owner).await;
    let denied = t
        .req(
            "POST",
            &format!("{}/join", program_path(second_org, matched)),
            json!({"id":Uuid::new_v4(),"accept_terms":true}),
            &runner,
        )
        .await;
    assert_eq!(denied.0, StatusCode::BAD_REQUEST, "{}", denied.1);
    assert_eq!(t.points(org, &runner).await["own_available_points"], 200);
    assert_eq!(
        t.points(second_org, &runner).await["own_available_points"],
        0
    );
    assert_eq!(t.points(second_org, &runner).await["own_staked_points"], 0);
    let event = t.draft(org, &owner, "EVENT", 100, 0, 1).await;
    t.publish(org, event, &owner).await;
    let join = format!("{}/join", program_path(org, event));
    let (a, b) = tokio::join!(
        t.req("POST", &join, json!({"id":Uuid::new_v4()}), &runner),
        t.req("POST", &join, json!({"id":Uuid::new_v4()}), &other)
    );
    let statuses = [a.0, b.0];
    assert_eq!(statuses.iter().filter(|&&s| s == StatusCode::OK).count(), 1);
    assert_eq!(
        statuses
            .iter()
            .filter(|&&s| s == StatusCode::CONFLICT)
            .count(),
        1
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM company_enrollments WHERE program_id=$1")
            .bind(event)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    let reserved: i64 =
        sqlx::query_scalar("SELECT reserved_units FROM company_programs WHERE id=$1")
            .bind(event)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(reserved, 100_000_000);
    t.assert_conserved(org).await;
    t.assert_conserved(second_org).await;
    t.finish().await;
}

#[derive(Clone)]
struct Session {
    user: Uuid,
    cookie: String,
    csrf: String,
}

struct TestApp {
    app: Router,
    pool: PgPool,
    admin: PgPool,
    schema: String,
}

impl TestApp {
    async fn new() -> Self {
        let url = std::env::var("TEST_DATABASE_URL")
            .expect("TEST_DATABASE_URL required; company points tests never silently skipped");
        assert!(url::Url::parse(&url).unwrap().path().ends_with("_test"));
        let admin = PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .unwrap();
        let schema = format!("test_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .unwrap();
        let search = schema.clone();
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .after_connect(move |c, _| {
                let search = search.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('search_path',$1,false)")
                        .bind(search)
                        .execute(c)
                        .await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let config = Config {
            production: false,
            origin: "http://127.0.0.1:8787".into(),
            bind: "127.0.0.1:8787".parse().unwrap(),
            trusted_proxy_ip: None,
            database_url: url,
            mail_key: None,
            smtp_host: None,
            smtp_user: None,
            smtp_password: None,
            mail_from: None,
        };
        let state = AppState::new(pool.clone(), config).await.unwrap();
        Self {
            app: router(state, "missing-dist"),
            pool,
            admin,
            schema,
        }
    }

    async fn bytes(
        &self,
        method: &str,
        path: &str,
        bytes: Vec<u8>,
        session: Option<&Session>,
        raw: bool,
    ) -> (StatusCode, Value, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:8787")
            .header("origin", "http://127.0.0.1:8787")
            .header("x-truhabit-request", "web")
            .header(
                "content-type",
                if raw {
                    "application/octet-stream"
                } else {
                    "application/json"
                },
            );
        if let Some(session) = session {
            request = request
                .header("cookie", &session.cookie)
                .header("x-csrf-token", &session.csrf);
        }
        let mut request = request.body(Body::from(bytes)).unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:6000".parse::<std::net::SocketAddr>().unwrap(),
        ));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let cookie = response
            .headers()
            .get("set-cookie")
            .and_then(|s| s.to_str().ok())
            .unwrap_or("")
            .to_string();
        let data = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&data).unwrap_or_else(|_| json!({"bytes":data.len()})),
            cookie,
        )
    }

    async fn req(
        &self,
        method: &str,
        path: &str,
        body: Value,
        actor: &Session,
    ) -> (StatusCode, Value) {
        let (status, response, _) = self
            .bytes(
                method,
                path,
                body.to_string().into_bytes(),
                Some(actor),
                false,
            )
            .await;
        (status, response)
    }

    async fn raw(&self, path: &str, bytes: &[u8], actor: &Session) -> (StatusCode, Value) {
        let (status, response, _) = self
            .bytes("POST", path, bytes.to_vec(), Some(actor), true)
            .await;
        (status, response)
    }

    async fn account(&self, email: &str) -> Session {
        let registered = self
            .bytes(
                "POST",
                "/api/auth/register",
                json!({"email":email,"password":PASSWORD,"display_name":"Company runner"})
                    .to_string()
                    .into_bytes(),
                None,
                false,
            )
            .await;
        assert_eq!(registered.0, StatusCode::OK, "{}", registered.1);
        let (status, body, cookie) = self
            .bytes(
                "POST",
                "/api/auth/login",
                json!({"email":email,"password":PASSWORD})
                    .to_string()
                    .into_bytes(),
                None,
                false,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let mut session = Session {
            user: Uuid::nil(),
            cookie: cookie.split(';').next().unwrap().into(),
            csrf: body["csrf_token"].as_str().unwrap().into(),
        };
        let response = self
            .req("GET", "/api/auth/session", Value::Null, &session)
            .await;
        session.user = id_from(&response.1["user"]);
        session
    }

    async fn org(&self, owner: &Session) -> Uuid {
        let id = Uuid::new_v4();
        let response = self
            .req(
                "POST",
                "/api/organizations",
                json!({"id":id,"name":"Company points test"}),
                owner,
            )
            .await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.1);
        id
    }

    async fn member(&self, org: Uuid, actor: &Session, role: &str) {
        sqlx::query(
            "INSERT INTO organization_members(organization_id,user_id,role) VALUES($1,$2,$3)",
        )
        .bind(org)
        .bind(actor.user)
        .bind(role)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn top_up(&self, org: Uuid, owner: &Session, points: i64) {
        let response = self
            .req(
                "POST",
                &format!("/api/organizations/{org}/points/top-up"),
                json!({"id":Uuid::new_v4(),"points":points}),
                owner,
            )
            .await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.1);
    }

    async fn points(&self, org: Uuid, actor: &Session) -> Value {
        let response = self
            .req(
                "GET",
                &format!("/api/organizations/{org}/points"),
                Value::Null,
                actor,
            )
            .await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.1);
        response.1
    }

    async fn draft(
        &self,
        org: Uuid,
        owner: &Session,
        template: &str,
        reward: i64,
        stake: i64,
        capacity: i32,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let response = self
            .req(
                "POST",
                &format!("/api/organizations/{org}/programs"),
                draft_input(id, template, reward, stake, capacity),
                owner,
            )
            .await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.1);
        assert_eq!(response.1["template"], template);
        assert_eq!(response.1["point_reward"], reward);
        id
    }

    fn publish_input() -> Value {
        let now = Utc::now();
        json!({"version":1,"profile":"REPLAY","starts_at":now,"ends_at":now+Duration::minutes(10),"upload_deadline":now+Duration::minutes(10),"review_deadline":now+Duration::minutes(15)})
    }

    async fn publish(&self, org: Uuid, program: Uuid, owner: &Session) {
        let response = self
            .req(
                "POST",
                &format!("{}/publish", program_path(org, program)),
                Self::publish_input(),
                owner,
            )
            .await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.1);
    }

    async fn join(&self, org: Uuid, program: Uuid, actor: &Session, consent: bool) -> Uuid {
        let response = self
            .req(
                "POST",
                &format!("{}/join", program_path(org, program)),
                json!({"id":Uuid::new_v4(),"accept_terms":consent}),
                actor,
            )
            .await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.1);
        id_from(&response.1)
    }

    async fn earn(&self, org: Uuid, owner: &Session, runner: &Session, points: i64, bytes: &[u8]) {
        let p = self
            .draft(org, owner, "ACTIVITY_POINTS", points, 0, 1)
            .await;
        self.publish(org, p, owner).await;
        let e = self.join(org, p, runner, false).await;
        let response = self
            .raw(
                &format!("{}/upload", enrollment_path(org, p, e)),
                bytes,
                runner,
            )
            .await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.1);
        assert_eq!(response.1["decision"], "ACCEPTED");
        self.close(org, p, owner).await;
    }

    async fn version(&self, org: Uuid, program: Uuid, owner: &Session) -> i64 {
        let response = self
            .req("GET", &program_path(org, program), Value::Null, owner)
            .await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.1);
        response.1["program"]["version"].as_i64().unwrap()
    }

    async fn close(&self, org: Uuid, program: Uuid, owner: &Session) {
        let version = self.version(org, program, owner).await;
        let response = self
            .req(
                "POST",
                &format!("{}/close", program_path(org, program)),
                json!({"version":version}),
                owner,
            )
            .await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.1);
    }

    async fn local_balance(&self, actor: &Session) -> Value {
        let response = self
            .req("GET", "/api/prototype/local/balance", Value::Null, actor)
            .await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.1);
        response.1
    }

    async fn expire_upload(&self, program: Uuid) {
        sqlx::query("UPDATE company_programs SET starts_at=now()-interval '30 minutes',ends_at=now()-interval '20 minutes',upload_deadline=now()-interval '10 minutes',review_deadline=now()+interval '10 minutes' WHERE id=$1")
            .bind(program).execute(&self.pool).await.unwrap();
    }

    async fn movement_count(&self, org: Uuid, kind: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM business_point_movements WHERE organization_id=$1 AND kind=$2",
        )
        .bind(org)
        .bind(kind)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn assert_conserved(&self, org: Uuid) {
        let totals: (i64, i64, i64, i64, i64) = sqlx::query_as("SELECT COALESCE(sum(pool_delta),0)::bigint,COALESCE(sum(reserved_delta),0)::bigint,COALESCE(sum(employee_delta),0)::bigint,COALESCE(sum(stake_delta),0)::bigint,COALESCE(sum(issued_delta),0)::bigint FROM business_point_movements WHERE organization_id=$1")
            .bind(org).fetch_one(&self.pool).await.unwrap();
        assert!(totals.0 >= 0 && totals.1 >= 0 && totals.2 >= 0 && totals.3 >= 0);
        assert_eq!(totals.0 + totals.1 + totals.2 + totals.3 + totals.4, 0);
        let negative: bool = sqlx::query_scalar("SELECT EXISTS(SELECT user_id FROM business_point_movements WHERE organization_id=$1 AND user_id IS NOT NULL GROUP BY user_id HAVING sum(employee_delta)<0 OR sum(stake_delta)<0)")
            .bind(org).fetch_one(&self.pool).await.unwrap();
        assert!(!negative, "A company's employee points cannot be overdrawn");
        let mixed: i64 = sqlx::query_scalar("SELECT count(*) FROM prototype_local_movements m JOIN company_programs p ON p.id=m.business_program_id WHERE p.organization_id=$1 AND p.template<>'LEGACY'")
            .bind(org).fetch_one(&self.pool).await.unwrap();
        assert_eq!(mixed, 0, "Company points must not use the B2C LOCAL ledger");
    }

    async fn finish(self) {
        self.pool.close().await;
        assert!(
            self.schema.starts_with("test_")
                && self.schema.len() == 37
                && self.schema[5..].bytes().all(|b| b.is_ascii_hexdigit())
        );
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.admin)
        .await
        .unwrap();
        self.admin.close().await;
    }
}

fn id_from(value: &Value) -> Uuid {
    Uuid::parse_str(value["id"].as_str().unwrap()).unwrap()
}

fn program_path(org: Uuid, program: Uuid) -> String {
    format!("/api/organizations/{org}/programs/{program}")
}

fn enrollment_path(org: Uuid, program: Uuid, enrollment: Uuid) -> String {
    format!("{}/enrollments/{enrollment}", program_path(org, program))
}

fn draft_input(id: Uuid, template: &str, reward: i64, stake: i64, capacity: i32) -> Value {
    json!({"id":id,"title":"Voluntary company activity","target_m":3000,"max_participants":capacity,"template":template,"point_reward":reward,"point_stake":stake})
}

fn distinct_run(day: u8) -> Vec<u8> {
    std::str::from_utf8(VALID)
        .unwrap()
        .replace("2026-09-01", &format!("2026-09-{day:02}"))
        .into_bytes()
}
