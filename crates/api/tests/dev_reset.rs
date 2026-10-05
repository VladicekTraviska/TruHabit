//! Disposable-schema tests: never reset a real account or contact a blockchain worker.
use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use truhabit_api::{AppState, MIGRATOR, config::Config, crypto, router};
use uuid::Uuid;

const PASSWORD: &str = "private disposable reset test passphrase 123";
const PATH: &str = "/api/account/dev-reset";

#[tokio::test]
async fn reset_erases_own_funded_history_preserves_credentials_wallet_and_other_tenants() {
    let t = TestApp::new().await;
    let a = t.actor().await;
    let b = t.actor().await;
    let before_user = t.user(a.id).await;
    sqlx::query("INSERT INTO wallets(id,user_id,public_key) VALUES($1,$2,$3)")
        .bind(Uuid::new_v4())
        .bind(a.id)
        .bind("synthetic-reset-wallet-public-key")
        .execute(&t.pool)
        .await
        .unwrap();
    t.goal(a.id).await;
    let challenge = t.challenge(a.id, "LOCAL", "ACTIVE").await;
    t.personal_source(a.id, challenge).await;
    t.movement(a.id, None, None, "GRANT", 100_000_000, 0, -100_000_000, 0)
        .await;
    t.personal_deposit(a.id, challenge).await;
    let owned = t.org(a.id).await;
    t.member(owned, b.id).await;
    let p = t.program(owned, a.id, "PUBLISHED", 2).await;
    t.movement(
        a.id,
        Some(p),
        None,
        "BUSINESS_FUND",
        -10_000_000,
        10_000_000,
        0,
        0,
    )
    .await;
    let own_e = t.enrol(p, a.id, "MET", "ENROLLED").await;
    let affected_e = t.enrol(p, b.id, "UNKNOWN", "ENROLLED").await;
    t.team_source(own_e, a.id).await;
    t.team_source(affected_e, b.id).await;
    t.movement(
        a.id,
        Some(p),
        Some(own_e),
        "BUSINESS_PAY",
        0,
        -5_000_000,
        0,
        5_000_000,
    )
    .await;
    t.movement(
        a.id,
        Some(p),
        Some(own_e),
        "BUSINESS_REWARD",
        5_000_000,
        0,
        0,
        -5_000_000,
    )
    .await;
    sqlx::query("UPDATE company_enrollments SET state='REWARDED',paid_at=now() WHERE id=$1")
        .bind(own_e)
        .execute(&t.pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE company_programs SET reserved_units=5000000,paid_units=5000000 WHERE id=$1",
    )
    .bind(p)
    .execute(&t.pool)
    .await
    .unwrap();
    let foreign = t.org(b.id).await;
    t.member(foreign, a.id).await;
    let foreign_p = t.program(foreign, b.id, "CLOSED", 0).await;
    let own_foreign_e = t.enrol(foreign_p, a.id, "NOT_MET", "CLOSED").await;
    let other_e = t.enrol(foreign_p, b.id, "NOT_MET", "CLOSED").await;
    t.team_source(own_foreign_e, a.id).await;
    let untouched_upload = t.team_source(other_e, b.id).await;
    t.movement(b.id, None, None, "GRANT", 100_000_000, 0, -100_000_000, 0)
        .await;
    let other_challenge = t.challenge(b.id, "LOCAL", "ACTIVE").await;
    t.personal_source(b.id, other_challenge).await;
    let other_before = t.financial_snapshot(b.id).await;
    let preview = t.preview(&a).await;
    assert_eq!(preview["allowed"], true, "{preview}");
    assert_eq!(preview["counts"]["owned_workspace_memberships"], 1);
    assert_eq!(preview["counts"]["owned_team_activity_files"], 2);
    assert_eq!(preview["counts"]["foreign_activity_files"], 1);
    assert_eq!(preview["owned_workspace_impacts"][0]["member_count"], 1);
    assert!(!preview.to_string().contains("synthetic-private-source"));
    let response = t.reset(&a, &preview).await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.1);
    assert_eq!(
        response.1["remaining_local_balance"],
        json!({"available":0,"locked":0,"forfeited":0})
    );
    assert_eq!(t.user(a.id).await, before_user);
    assert_eq!(t.financial_snapshot(b.id).await, other_before);
    assert_eq!(
        t.req("GET", "/api/auth/session", Value::Null, &a).await.0,
        StatusCode::OK
    );
    assert_eq!(
        t.req("GET", "/api/auth/session", Value::Null, &b).await.0,
        StatusCode::OK
    );
    let linked: i64 = sqlx::query_scalar("SELECT count(*) FROM wallets WHERE user_id=$1")
        .bind(a.id)
        .fetch_one(&t.pool)
        .await
        .unwrap();
    assert_eq!(linked, 1);
    let retained: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM company_uploads WHERE id=$1 AND content IS NOT NULL)",
    )
    .bind(untouched_upload)
    .fetch_one(&t.pool)
    .await
    .unwrap();
    assert!(retained);
    let anonymized: Option<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM company_enrollments WHERE id=$1")
            .bind(own_foreign_e)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert!(anonymized.is_none());
    let owned_gone: bool =
        sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM organizations WHERE id=$1)")
            .bind(owned)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert!(owned_gone);
    for table in [
        "goal_events",
        "idempotency_keys",
        "organization_invitations",
        "organization_events",
    ] {
        let count: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&t.pool)
                .await
                .unwrap();
        assert_eq!(count, 0, "Owned dependent rows in {table} must cascade");
    }
    let after = t.preview(&a).await;
    assert!(
        after["counts"]
            .as_object()
            .unwrap()
            .values()
            .all(|v| v == 0)
    );
    let again = t.reset(&a, &after).await;
    assert_eq!(again.0, StatusCode::OK, "Empty repeat keeps the account");
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM security_events WHERE user_id=$1 AND kind='development_data_reset'",
    )
    .bind(a.id)
    .fetch_one(&t.pool)
    .await
    .unwrap();
    assert_eq!(audited, 2);
    let security:i64=sqlx::query_scalar("SELECT count(*) FROM security_events WHERE user_id=$1 AND kind='preserved_test_security_event'")
        .bind(a.id).fetch_one(&t.pool).await.unwrap();
    assert_eq!(security, 1);
    t.finish().await;
}

#[tokio::test]
async fn devnet_active_pending_and_unverified_records_block_without_rpc_or_partial_erasure() {
    let t = TestApp::new().await;
    let a = t.actor().await;
    t.goal(a.id).await;
    let active = t.challenge(a.id, "DEVNET", "ACTIVE").await;
    let pending = t.challenge(a.id, "DEVNET", "DRAFT").await;
    t.command(pending, "DEPOSIT", "PREPARED").await;
    let signed = t.challenge(a.id, "DEVNET", "DRAFT").await;
    t.command(signed, "DEPOSIT", "SIGNED").await;
    let ambiguous = t.challenge(a.id, "DEVNET", "DRAFT").await;
    t.command(ambiguous, "DEPOSIT", "SIGNED").await;
    sqlx::query("UPDATE prototype_commands SET status='FAILED' WHERE challenge_id=$1")
        .bind(ambiguous)
        .execute(&t.pool)
        .await
        .unwrap();
    t.challenge(a.id, "DEVNET", "REFUNDED").await;
    let preview = t.preview(&a).await;
    assert_eq!(preview["allowed"], false);
    assert!(has_blocker(&preview, "DEV_RESET_DEVNET_ESCROW_ACTIVE"));
    assert!(has_blocker(&preview, "DEV_RESET_DEVNET_COMMAND_PENDING"));
    assert!(has_blocker(&preview, "DEV_RESET_DEVNET_STATE_UNVERIFIED"));
    let response = t.reset(&a, &preview).await;
    assert_eq!(response.0, StatusCode::CONFLICT);
    assert_eq!(response.1["code"], "DEV_RESET_BLOCKED");
    assert_eq!(t.preview(&a).await["fingerprint"], preview["fingerprint"]);
    assert_eq!(t.preview(&a).await["counts"]["goals"], 1);
    let state: String = sqlx::query_scalar("SELECT state FROM prototype_challenges WHERE id=$1")
        .bind(active)
        .fetch_one(&t.pool)
        .await
        .unwrap();
    assert_eq!(state, "ACTIVE");
    t.finish().await;
}

#[tokio::test]
async fn unfunded_draft_and_confirmed_terminal_devnet_cache_can_be_erased_locally() {
    let t = TestApp::new().await;
    let a = t.actor().await;
    t.challenge(a.id, "DEVNET", "DRAFT").await;
    for (state, action) in [
        ("REFUNDED", "SUCCESS"),
        ("FORFEITED", "FAILURE"),
        ("CANCELLED", "CANCEL"),
        ("EXPIRED", "TIMEOUT"),
    ] {
        let id = t.challenge(a.id, "DEVNET", state).await;
        t.command(id, action, "CONFIRMED").await;
        sqlx::query("UPDATE prototype_challenges SET closed_at=now() WHERE id=$1")
            .bind(id)
            .execute(&t.pool)
            .await
            .unwrap();
    }
    let preview = t.preview(&a).await;
    assert_eq!(preview["allowed"], true, "{preview}");
    assert_eq!(t.reset(&a, &preview).await.0, StatusCode::OK);
    assert_eq!(t.preview(&a).await["counts"]["personal_challenges"], 0);
    t.finish().await;
}

#[tokio::test]
async fn foreign_active_reservations_and_accepted_unpaid_rewards_are_never_released_by_reset() {
    let t = TestApp::new().await;
    let a = t.actor().await;
    let b = t.actor().await;
    let org = t.org(b.id).await;
    t.member(org, a.id).await;
    let p = t.program(org, b.id, "PUBLISHED", 1).await;
    t.enrol(p, a.id, "UNKNOWN", "ENROLLED").await;
    sqlx::query("UPDATE company_programs SET reserved_units=5000000 WHERE id=$1")
        .bind(p)
        .execute(&t.pool)
        .await
        .unwrap();
    let preview = t.preview(&a).await;
    assert!(has_blocker(
        &preview,
        "DEV_RESET_FOREIGN_PARTICIPATION_ACTIVE"
    ));
    assert_eq!(t.reset(&a, &preview).await.0, StatusCode::CONFLICT);
    let reserved: i64 =
        sqlx::query_scalar("SELECT reserved_units FROM company_programs WHERE id=$1")
            .bind(p)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(reserved, 5_000_000);
    sqlx::query("UPDATE company_programs SET state='CLOSED' WHERE id=$1")
        .bind(p)
        .execute(&t.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE company_enrollments SET assessment='MET' WHERE program_id=$1")
        .bind(p)
        .execute(&t.pool)
        .await
        .unwrap();
    assert!(has_blocker(
        &t.preview(&a).await,
        "DEV_RESET_FOREIGN_PARTICIPATION_ACTIVE"
    ));
    t.finish().await;
}

#[tokio::test]
async fn paired_foreign_payout_and_transferred_owner_funding_are_preserved_by_blockers() {
    let t = TestApp::new().await;
    let a = t.actor().await;
    let b = t.actor().await;
    let org = t.org(b.id).await;
    let p = t.program(org, b.id, "CLOSED", 1).await;
    let e = t.enrol(p, a.id, "MET", "REWARDED").await;
    t.movement(
        b.id,
        Some(p),
        Some(e),
        "BUSINESS_PAY",
        0,
        -5_000_000,
        0,
        5_000_000,
    )
    .await;
    t.movement(
        a.id,
        Some(p),
        Some(e),
        "BUSINESS_REWARD",
        5_000_000,
        0,
        0,
        -5_000_000,
    )
    .await;
    let before_a = t.financial_snapshot(a.id).await;
    let before_b = t.financial_snapshot(b.id).await;
    let preview = t.preview(&a).await;
    assert!(has_blocker(&preview, "DEV_RESET_FOREIGN_FINANCIAL_HISTORY"));
    assert_eq!(t.reset(&a, &preview).await.0, StatusCode::CONFLICT);
    assert_eq!(t.financial_snapshot(a.id).await, before_a);
    assert_eq!(t.financial_snapshot(b.id).await, before_b);
    // Ownership transfer does not make another person's historic funding disposable.
    sqlx::query("UPDATE organizations SET owner_id=$1 WHERE id=$2")
        .bind(a.id)
        .bind(org)
        .execute(&t.pool)
        .await
        .unwrap();
    let transferred = t.preview(&a).await;
    assert!(has_blocker(
        &transferred,
        "DEV_RESET_SHARED_FINANCIAL_HISTORY"
    ));
    assert_eq!(t.reset(&a, &transferred).await.0, StatusCode::CONFLICT);
    assert_eq!(t.financial_snapshot(a.id).await, before_a);
    assert_eq!(t.financial_snapshot(b.id).await, before_b);
    t.finish().await;
}

#[tokio::test]
async fn fingerprint_rejects_same_count_role_source_and_assessment_changes() {
    let t = TestApp::new().await;
    let a = t.actor().await;
    let b = t.actor().await;
    let org = t.org(a.id).await;
    t.member(org, b.id).await;
    let p = t.program(org, a.id, "CLOSED", 0).await;
    let e = t.enrol(p, a.id, "NOT_MET", "CLOSED").await;
    let upload = t.team_source(e, a.id).await;
    let initial = t.preview(&a).await;
    sqlx::query(
        "UPDATE organization_members SET role='ADMIN' WHERE organization_id=$1 AND user_id=$2",
    )
    .bind(org)
    .bind(b.id)
    .execute(&t.pool)
    .await
    .unwrap();
    let stale = t.reset(&a, &initial).await;
    assert_eq!(stale.1["code"], "DEV_RESET_PREVIEW_CHANGED");
    let next = t.preview(&a).await;
    sqlx::query("UPDATE company_uploads SET content=NULL,content_deleted_at=now() WHERE id=$1")
        .bind(upload)
        .execute(&t.pool)
        .await
        .unwrap();
    assert_eq!(
        t.reset(&a, &next).await.1["code"],
        "DEV_RESET_PREVIEW_CHANGED"
    );
    let next = t.preview(&a).await;
    sqlx::query("UPDATE company_enrollments SET assessment='UNKNOWN' WHERE id=$1")
        .bind(e)
        .execute(&t.pool)
        .await
        .unwrap();
    assert_eq!(
        t.reset(&a, &next).await.1["code"],
        "DEV_RESET_PREVIEW_CHANGED"
    );
    assert_eq!(t.preview(&a).await["counts"]["owned_workspaces"], 1);
    t.finish().await;
}

#[tokio::test]
async fn other_employees_earned_unpaid_reward_blocks_owned_workspace_reset() {
    let t = TestApp::new().await;
    let owner = t.actor().await;
    let employee = t.actor().await;
    let org = t.org(owner.id).await;
    t.member(org, employee.id).await;
    let p = t.program(org, owner.id, "PUBLISHED", 1).await;
    t.movement(
        owner.id,
        Some(p),
        None,
        "BUSINESS_FUND",
        -5_000_000,
        5_000_000,
        0,
        0,
    )
    .await;
    let e = t.enrol(p, employee.id, "MET", "ENROLLED").await;
    let source = t.team_source(e, employee.id).await;
    sqlx::query("UPDATE company_programs SET reserved_units=5000000 WHERE id=$1")
        .bind(p)
        .execute(&t.pool)
        .await
        .unwrap();
    let before = t.financial_snapshot(owner.id).await;
    let preview = t.preview(&owner).await;
    assert!(has_blocker(&preview, "DEV_RESET_SHARED_FINANCIAL_HISTORY"));
    let result = t.reset(&owner, &preview).await;
    assert_eq!(result.0, StatusCode::CONFLICT);
    assert_eq!(result.1["code"], "DEV_RESET_BLOCKED");
    assert_eq!(t.financial_snapshot(owner.id).await, before);
    let retained: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM company_uploads WHERE id=$1 AND content IS NOT NULL)",
    )
    .bind(source)
    .fetch_one(&t.pool)
    .await
    .unwrap();
    assert!(retained);
    let assessment: String =
        sqlx::query_scalar("SELECT assessment FROM company_enrollments WHERE id=$1")
            .bind(e)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(assessment, "MET");
    t.finish().await;
}

#[tokio::test]
async fn lock_serialization_prevents_reset_from_erasing_newly_admitted_upload() {
    let t = TestApp::new().await;
    let a = t.actor().await;
    let challenge = t.challenge(a.id, "LOCAL", "ACTIVE").await;
    let initial = t.preview(&a).await;
    let mut admission = t.pool.begin().await.unwrap();
    truhabit_api::auth::lock_user(&mut admission, a.id)
        .await
        .unwrap();
    let upload = Uuid::new_v4();
    sqlx::query("INSERT INTO prototype_uploads(id,challenge_id,user_id,file_hash,fingerprint,content,activity,goal_result,decision,reason) VALUES($1,$2,$3,$1::text,$1::text,$4,'{}','NOT_MET','ACCEPTED','new timely upload')")
        .bind(upload).bind(challenge).bind(a.id).bind(b"new upload".to_vec()).execute(&mut *admission).await.unwrap();
    let response = {
        let reset = t.reset(&a, &initial);
        tokio::pin!(reset);
        // Start the real handler while admission owns the account lock. It must wait.
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut reset)
                .await
                .is_err()
        );
        admission.commit().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), reset)
            .await
            .unwrap()
    };
    assert_eq!(response.0, StatusCode::CONFLICT);
    assert_eq!(response.1["code"], "DEV_RESET_PREVIEW_CHANGED");
    let retained: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM prototype_uploads WHERE id=$1)")
            .bind(upload)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert!(retained);
    t.finish().await;
}

#[tokio::test]
async fn concurrent_confirmations_commit_once_and_reject_the_stale_second_snapshot() {
    let t = TestApp::new().await;
    let a = t.actor().await;
    t.goal(a.id).await;
    let preview = t.preview(&a).await;
    let (one, two) = tokio::join!(t.reset(&a, &preview), t.reset(&a, &preview));
    let statuses = [one.0, two.0];
    assert_eq!(statuses.iter().filter(|s| **s == StatusCode::OK).count(), 1);
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::CONFLICT)
            .count(),
        1
    );
    let failed = if one.0 == StatusCode::CONFLICT {
        one.1
    } else {
        two.1
    };
    assert_eq!(failed["code"], "DEV_RESET_PREVIEW_CHANGED");
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM security_events WHERE user_id=$1 AND kind='development_data_reset'",
    )
    .bind(a.id)
    .fetch_one(&t.pool)
    .await
    .unwrap();
    assert_eq!(events, 1);
    t.finish().await;
}

#[tokio::test]
async fn password_csrf_confirmation_and_rate_limits_do_not_destroy_login_or_data() {
    let t = TestApp::new().await;
    let a = t.actor().await;
    t.goal(a.id).await;
    let preview = t.preview(&a).await;
    let mut bad_csrf = a.clone();
    bad_csrf.csrf = "incorrect".into();
    assert_eq!(t.reset(&bad_csrf, &preview).await.0, StatusCode::FORBIDDEN);
    let mut payload = input(&preview);
    payload["password"] = json!("an incorrect password");
    let wrong = t.req("POST", PATH, payload, &a).await;
    assert_eq!(wrong.0, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong.1["code"], "INVALID_CREDENTIALS");
    assert_eq!(
        t.req("GET", "/api/auth/session", Value::Null, &a).await.0,
        StatusCode::OK
    );
    let mut payload = input(&preview);
    payload["confirmation"] = json!("reset");
    assert_eq!(
        t.req("POST", PATH, payload, &a).await.1["code"],
        "DEV_RESET_CONFIRMATION_REQUIRED"
    );
    let mut payload = input(&preview);
    payload["fingerprint"] = json!("invalid");
    assert_eq!(
        t.req("POST", PATH, payload, &a).await.1["code"],
        "DEV_RESET_INVALID_FINGERPRINT"
    );
    assert_eq!(t.preview(&a).await["fingerprint"], preview["fingerprint"]);
    let changed = t.goal(a.id).await;
    assert_eq!(
        t.reset(&a, &preview).await.1["code"],
        "DEV_RESET_PREVIEW_CHANGED"
    );
    let now = t.preview(&a).await;
    assert_eq!(t.reset(&a, &now).await.0, StatusCode::OK);
    // The reset itself must not reset its security/rate-limit budget.
    assert_eq!(
        t.reset(&a, &t.preview(&a).await).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    let gone: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM goals WHERE id=$1)")
        .bind(changed)
        .fetch_one(&t.pool)
        .await
        .unwrap();
    assert!(gone);
    t.finish().await;
}

#[tokio::test]
async fn endpoint_is_unavailable_in_production_proxy_public_bind_and_remote_peer() {
    let t = TestApp::new().await;
    let a = t.actor().await;
    for variant in 0..4 {
        let mut config = (*t.state.config).clone();
        match variant {
            0 => config.production = true,
            1 => config.trusted_proxy_ip = Some("127.0.0.1".parse().unwrap()),
            2 => config.bind = "0.0.0.0:8787".parse().unwrap(),
            _ => config.origin = "https://public.example".into(),
        }
        assert!(!truhabit_api::dev_reset::enabled(&config));
        let mut state = t.state.clone();
        state.config = std::sync::Arc::new(config);
        let app = router(state.clone(), "missing-dist");
        let response = t
            .request(
                app,
                "GET",
                PATH,
                Value::Null,
                None,
                "127.0.0.1:6000",
                &state.config.origin,
            )
            .await;
        assert_eq!(response.0, StatusCode::NOT_FOUND, "Gate must precede auth");
        let readiness = t
            .request(
                router(state.clone(), "missing-dist"),
                "GET",
                "/api/readiness",
                Value::Null,
                None,
                "127.0.0.1:6000",
                &state.config.origin,
            )
            .await;
        assert_eq!(readiness.1["dev_reset"], false);
    }
    let remote = t
        .request(
            t.app.clone(),
            "GET",
            PATH,
            Value::Null,
            Some(&a),
            "192.0.2.1:6000",
            &t.state.config.origin,
        )
        .await;
    assert_eq!(remote.0, StatusCode::NOT_FOUND);
    t.finish().await;
}

fn has_blocker(preview: &Value, code: &str) -> bool {
    preview["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["code"] == code)
}
fn input(preview: &Value) -> Value {
    json!({"password":PASSWORD,"confirmation":"RESET","fingerprint":preview["fingerprint"]})
}

#[derive(Clone)]
struct Actor {
    id: Uuid,
    cookie: String,
    csrf: String,
}
struct TestApp {
    app: Router,
    state: AppState,
    pool: PgPool,
    admin: PgPool,
    schema: String,
    hash: String,
}
impl TestApp {
    async fn new() -> Self {
        let url = std::env::var("TEST_DATABASE_URL")
            .expect("TEST_DATABASE_URL required; reset tests never silently skipped");
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
            .max_connections(8)
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
        let hash = crypto::hash_password(PASSWORD.into()).await.unwrap();
        Self {
            app: router(state.clone(), "missing-dist"),
            state,
            pool,
            admin,
            schema,
            hash,
        }
    }
    async fn actor(&self) -> Actor {
        let id = Uuid::new_v4();
        let token = crypto::token().unwrap();
        let csrf = crypto::token().unwrap();
        sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'Reset test actor',$3)")
            .bind(id).bind(format!("{}@example.test",id.simple())).bind(&self.hash).execute(&self.pool).await.unwrap();
        sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf_token,expires_at) VALUES($1,$2,$3,now()+interval '24 hours')")
            .bind(crypto::digest(&token)).bind(id).bind(&csrf).execute(&self.pool).await.unwrap();
        sqlx::query(
            "INSERT INTO security_events(user_id,kind) VALUES($1,'preserved_test_security_event')",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .unwrap();
        Actor {
            id,
            cookie: format!("truhabit_local={token}"),
            csrf,
        }
    }
    #[allow(clippy::too_many_arguments)]
    async fn request(
        &self,
        app: Router,
        method: &str,
        path: &str,
        value: Value,
        actor: Option<&Actor>,
        peer: &str,
        origin: &str,
    ) -> (StatusCode, Value) {
        let parsed = url::Url::parse(origin).unwrap();
        let host = &parsed[url::Position::BeforeHost..url::Position::AfterPort];
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", host)
            .header("origin", origin)
            .header("x-truhabit-request", "web")
            .header("content-type", "application/json");
        if let Some(a) = actor {
            request = request
                .header("cookie", &a.cookie)
                .header("x-csrf-token", &a.csrf);
        }
        let mut request = request.body(Body::from(value.to_string())).unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(peer.parse::<std::net::SocketAddr>().unwrap()));
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap())
    }
    async fn req(&self, method: &str, path: &str, value: Value, a: &Actor) -> (StatusCode, Value) {
        self.request(
            self.app.clone(),
            method,
            path,
            value,
            Some(a),
            "127.0.0.1:6000",
            &self.state.config.origin,
        )
        .await
    }
    async fn preview(&self, a: &Actor) -> Value {
        let r = self.req("GET", PATH, Value::Null, a).await;
        assert_eq!(r.0, StatusCode::OK, "{}", r.1);
        r.1
    }
    async fn reset(&self, a: &Actor, preview: &Value) -> (StatusCode, Value) {
        self.req("POST", PATH, input(preview), a).await
    }
    async fn user(&self, id: Uuid) -> Value {
        sqlx::query_scalar("SELECT to_jsonb(u) FROM users u WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
    async fn financial_snapshot(&self, id: Uuid) -> Value {
        sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY id),'[]') FROM prototype_local_movements m WHERE user_id=$1").bind(id).fetch_one(&self.pool).await.unwrap()
    }
    async fn goal(&self, user: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO goals(id,user_id,target_m,pledge_cents,starts_at,ends_at,policy_version) VALUES($1,$2,3000,500,now(),now()+interval '24 hours','test')").bind(id).bind(user).execute(&self.pool).await.unwrap();
        sqlx::query("INSERT INTO goal_events(goal_id,actor_id,kind) VALUES($1,$2,'created')")
            .bind(id)
            .bind(user)
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO idempotency_keys(user_id,key,request_hash,resource_id) VALUES($1,$2,'test',$3)")
            .bind(user).bind(Uuid::new_v4()).bind(id).execute(&self.pool).await.unwrap();
        id
    }
    async fn challenge(&self, user: Uuid, network: &str, state: &str) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO prototype_challenges(id,user_id,title,target_m,amount_units,profile,network,starts_at,ends_at,upload_deadline,refund_after,state,creation_hash) VALUES($1,$2,'Test run',3000,5000000,'REPLAY',$3,now(),now()+interval '1 hour',now()+interval '2 hours',now()+interval '3 hours',$4,'test')").bind(id).bind(user).bind(network).bind(state).execute(&self.pool).await.unwrap();
        id
    }
    async fn command(&self, challenge: Uuid, action: &str, status: &str) {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO prototype_commands(id,challenge_id,action,payload,status,signed_transaction,signature) VALUES($1,$2,$3,'{}',$4,$5,$6)").bind(id).bind(challenge).bind(action).bind(status).bind((status=="SIGNED").then_some("synthetic signed transaction")).bind((status=="CONFIRMED").then(||format!("synthetic-confirmed-{id}"))).execute(&self.pool).await.unwrap();
    }
    async fn personal_source(&self, user: Uuid, challenge: Uuid) {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO prototype_uploads(id,challenge_id,user_id,file_hash,fingerprint,content,activity,goal_result,decision,reason) VALUES($1,$2,$3,$1::text,$1::text,$4,'{}','NOT_MET','ACCEPTED','test')").bind(id).bind(challenge).bind(user).bind(b"synthetic-private-source".to_vec()).execute(&self.pool).await.unwrap();
    }
    async fn org(&self, user: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO organizations(id,owner_id,name,creation_hash) VALUES($1,$2,'Reset workspace','test')").bind(id).bind(user).execute(&self.pool).await.unwrap();
        id
    }
    async fn member(&self, org: Uuid, user: Uuid) {
        sqlx::query(
            "INSERT INTO organization_members(organization_id,user_id,role) VALUES($1,$2,'MEMBER')",
        )
        .bind(org)
        .bind(user)
        .execute(&self.pool)
        .await
        .unwrap();
    }
    async fn program(&self, org: Uuid, funder: Uuid, state: &str, funded_cap: i64) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO company_programs(id,organization_id,title,target_m,currency,reward_minor,max_participants,state,creation_hash,reward_units,budget_units,funder_id) VALUES($1,$2,'Reset program',3000,'CZK',25000,2,$3,'test',5000000,$4,$5)").bind(id).bind(org).bind(state).bind(5_000_000*funded_cap).bind(funder).execute(&self.pool).await.unwrap();
        id
    }
    async fn enrol(&self, p: Uuid, user: Uuid, assessment: &str, state: &str) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO company_enrollments(id,program_id,user_id,assessment,state,reward_units) VALUES($1,$2,$3,$4,$5,5000000)").bind(id).bind(p).bind(user).bind(assessment).bind(state).execute(&self.pool).await.unwrap();
        sqlx::query("INSERT INTO company_enrollment_events(enrollment_id,actor_id,kind) VALUES($1,$2,'joined')").bind(id).bind(user).execute(&self.pool).await.unwrap();
        id
    }
    async fn team_source(&self, e: Uuid, user: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO company_uploads(id,enrollment_id,user_id,file_hash,fingerprint,content,activity,goal_result,decision,reason,received_at) VALUES($1,$2,$3,$1::text,$1::text,$4,'{}','NOT_MET','ACCEPTED','test',now())").bind(id).bind(e).bind(user).bind(b"synthetic-private-source".to_vec()).execute(&self.pool).await.unwrap();
        id
    }
    #[allow(clippy::too_many_arguments)]
    async fn movement(
        &self,
        user: Uuid,
        p: Option<Uuid>,
        e: Option<Uuid>,
        action: &str,
        wallet: i64,
        locked: i64,
        issued: i64,
        business: i64,
    ) {
        sqlx::query("INSERT INTO prototype_local_movements(id,user_id,business_program_id,business_enrollment_id,action,wallet_delta,locked_delta,recipient_delta,issued_delta,business_delta) VALUES($1,$2,$3,$4,$5,$6,$7,0,$8,$9)").bind(Uuid::new_v4()).bind(user).bind(p).bind(e).bind(action).bind(wallet).bind(locked).bind(issued).bind(business).execute(&self.pool).await.unwrap();
    }
    async fn personal_deposit(&self, user: Uuid, c: Uuid) {
        sqlx::query("INSERT INTO prototype_local_movements(id,user_id,challenge_id,action,wallet_delta,locked_delta,recipient_delta,issued_delta) VALUES($1,$2,$3,'DEPOSIT',-5000000,5000000,0,0)").bind(Uuid::new_v4()).bind(user).bind(c).execute(&self.pool).await.unwrap();
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
