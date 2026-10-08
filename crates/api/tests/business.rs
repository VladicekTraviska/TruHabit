//! Real PostgreSQL integration tests in disposable schemas; never use a live user/database.
use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use truhabit_api::{AppState, MIGRATOR, config::Config, router};
use uuid::Uuid;
const PASSWORD: &str = "a long unique business test passphrase 123";
const VALID: &[u8] = include_bytes!("../../../web/public/prototype/valid-run.gpx");
const REVIEW: &[u8] = include_bytes!("../../../web/public/prototype/review-run.gpx");
const SHORT: &[u8] = include_bytes!("../../../web/public/prototype/short-run.gpx");

#[tokio::test]
async fn deleting_a_settled_legacy_participant_cannot_erase_the_shared_reward_ledger() {
    let t = TestApp::new().await;
    let owner = t.account("legacy-history-owner@example.test").await;
    let runner = t.account("legacy-history-runner@example.test").await;
    let org = t.org(&owner).await;
    t.member(org, &runner, "MEMBER").await;
    t.grant(&owner).await;
    let p = t.draft(org, &owner, 1).await;
    t.publish(org, p, &owner).await;
    let e = t.join(org, p, &runner).await;
    let path = ep(org, p, e);
    assert_eq!(
        t.raw(&format!("{path}/upload"), VALID, &runner).await.0,
        StatusCode::OK
    );
    assert_eq!(
        t.req("POST", &format!("{path}/claim"), json!({}), &runner)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.req(
            "POST",
            &format!("{}/close", base(org, p)),
            json!({"version":2}),
            &owner
        )
        .await
        .0,
        StatusCode::OK
    );
    // Membership is not a financial-history guard: a settled legacy member can
    // legitimately leave the workspace, while both sides of the reward must stay.
    assert_eq!(
        t.req(
            "DELETE",
            &format!("/api/organizations/{org}/members/{}", runner.user),
            json!({}),
            &owner
        )
        .await
        .0,
        StatusCode::OK
    );
    let before: (i64, i64) = sqlx::query_as(
        "SELECT count(*),COALESCE(sum(business_delta),0)::bigint FROM prototype_local_movements WHERE business_program_id=$1",
    )
    .bind(p)
    .fetch_one(&t.pool)
    .await
    .unwrap();
    assert_eq!(before, (4, 0));
    let deletion = t
        .req(
            "DELETE",
            "/api/account",
            json!({"password":PASSWORD,"confirmation":"DELETE"}),
            &runner,
        )
        .await;
    let after: (i64, i64) = sqlx::query_as(
        "SELECT count(*),COALESCE(sum(business_delta),0)::bigint FROM prototype_local_movements WHERE business_program_id=$1",
    )
    .bind(p)
    .fetch_one(&t.pool)
    .await
    .unwrap();
    let still_signed_in = t
        .req("GET", "/api/auth/session", Value::Null, &runner)
        .await;
    assert_eq!(
        (deletion.0, after),
        (StatusCode::CONFLICT, before),
        "Deleting the employee must preserve both reward-transfer entries; response: {}",
        deletion.1
    );
    assert_eq!(deletion.1["error"], "ACCOUNT_HAS_SHARED_CREDIT_HISTORY");
    assert_eq!(still_signed_in.0, StatusCode::OK);
    // The funding side is equally important. After a legitimate ownership
    // transfer and departure, the former owner must not erase funding/payment
    // rows that are paired with the employee's retained reward.
    t.member(org, &runner, "MEMBER").await;
    assert_eq!(
        t.req(
            "POST",
            &format!("/api/organizations/{org}/transfer-owner"),
            json!({"user_id":runner.user,"version":1}),
            &owner
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        t.req(
            "DELETE",
            &format!("/api/organizations/{org}/members/{}", owner.user),
            json!({}),
            &runner
        )
        .await
        .0,
        StatusCode::OK
    );
    let funder_deletion = t
        .req(
            "DELETE",
            "/api/account",
            json!({"password":PASSWORD,"confirmation":"DELETE"}),
            &owner,
        )
        .await;
    let final_ledger: (i64, i64) = sqlx::query_as(
        "SELECT count(*),COALESCE(sum(business_delta),0)::bigint FROM prototype_local_movements WHERE business_program_id=$1",
    )
    .bind(p)
    .fetch_one(&t.pool)
    .await
    .unwrap();
    t.finish().await;
    assert_eq!(
        (funder_deletion.0, final_ledger),
        (StatusCode::CONFLICT, before),
        "Former funder deletion must retain the complete shared ledger: {}",
        funder_deletion.1
    );
    assert_eq!(
        funder_deletion.1["error"],
        "ACCOUNT_HAS_SHARED_CREDIT_HISTORY"
    );
}

#[tokio::test]
async fn settled_workspace_archive_and_restore_preserve_private_evidence_and_ledger() {
    let t = TestApp::new().await;
    let owner = t.account("archive-owner@example.test").await;
    let member = t.account("archive-member@example.test").await;
    let invited = t.account("archive-invited@example.test").await;
    let org = t.org(&owner).await;
    let workspace = format!("/api/organizations/{org}");
    t.member(org, &member, "MEMBER").await;
    t.grant(&owner).await;
    let p = t.draft(org, &owner, 1).await;
    let path = base(org, p);
    let invite = t
        .req(
            "POST",
            &format!("{workspace}/invitations"),
            json!({"id":Uuid::new_v4(),"email":"archive-invited@example.test","role":"MEMBER"}),
            &owner,
        )
        .await;
    assert_eq!(invite.0, StatusCode::OK);
    t.publish(org, p, &owner).await;
    let e = t.join(org, p, &member).await;
    let private = ep(org, p, e);
    let upload = t.raw(&format!("{private}/upload"), VALID, &member).await;
    assert_eq!(upload.0, StatusCode::OK);
    let managed = t.req("GET", &workspace, Value::Null, &owner).await;
    assert_eq!(managed.1["management"]["can_archive"], false);
    assert_eq!(
        managed.1["management"]["archive_reason"],
        "WORKSPACE_HAS_PUBLISHED_PROGRAMS"
    );
    let blocked = t
        .req(
            "POST",
            &format!("{workspace}/archive"),
            json!({"version":1}),
            &owner,
        )
        .await;
    assert_eq!(blocked.0, StatusCode::CONFLICT);
    assert_eq!(
        blocked.1["error"],
        managed.1["management"]["archive_reason"]
    );
    assert!(
        t.req("GET", &workspace, Value::Null, &member)
            .await
            .1
            .get("management")
            .is_none()
    );
    assert!(
        t.req("GET", &path, Value::Null, &member)
            .await
            .1
            .get("closure")
            .is_none()
    );
    assert_eq!(
        t.req("POST", &format!("{private}/claim"), json!({}), &member)
            .await
            .0,
        StatusCode::OK
    );
    let ready = t.req("GET", &path, Value::Null, &owner).await;
    assert_eq!(
        ready.1["closure"]["allowed"], true,
        "Full paid capacity may close early"
    );
    assert_eq!(ready.1["closure"]["rewarded_count"], 1);
    assert!(ready.1["closure"]["reason"].is_null());
    assert_eq!(
        t.req(
            "POST",
            &format!("{path}/close"),
            json!({"version":2}),
            &owner
        )
        .await
        .0,
        StatusCode::OK
    );
    let before = t.req("GET", &private, Value::Null, &member).await.1;
    let owner_balance = t.balance(&owner).await;
    let member_balance = t.balance(&member).await;
    let ledger_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM prototype_local_movements WHERE business_program_id=$1",
    )
    .bind(p)
    .fetch_one(&t.pool)
    .await
    .unwrap();
    let archived = t
        .req(
            "POST",
            &format!("{workspace}/archive"),
            json!({"version":1}),
            &owner,
        )
        .await;
    assert_eq!(archived.0, StatusCode::OK, "{}", archived.1);
    assert!(archived.1["archived_at"].is_string());
    assert_eq!(archived.1["version"], 2);
    let repeated = t
        .req(
            "POST",
            &format!("{workspace}/archive"),
            json!({"version":1}),
            &owner,
        )
        .await;
    assert_eq!(repeated.1, archived.1);
    let listed = t
        .req("GET", "/api/organizations", Value::Null, &member)
        .await
        .1;
    assert!(listed["organizations"].as_array().unwrap().is_empty());
    assert_eq!(listed["archived_organizations"][0]["id"], org.to_string());
    let history = t.req("GET", &workspace, Value::Null, &owner).await.1;
    assert_eq!(history["programs"][0]["state"], "CLOSED");
    assert_eq!(
        history["management"]["archive_reason"],
        "WORKSPACE_ARCHIVED"
    );
    let private_after = t.req("GET", &private, Value::Null, &member).await;
    assert_eq!(private_after.0, StatusCode::OK);
    assert_eq!(private_after.1, before);
    let source = format!(
        "{private}/uploads/{}/source",
        upload.1["id"].as_str().unwrap()
    );
    assert_eq!(
        t.req("GET", &source, Value::Null, &member).await.1["bytes"],
        VALID.len()
    );
    assert_eq!(
        t.req("GET", &private, Value::Null, &owner).await.0,
        StatusCode::NOT_FOUND
    );
    for (method, target, body, actor) in [
        (
            "PATCH",
            workspace.clone(),
            json!({"name":"Forbidden rename","version":2}),
            &owner,
        ),
        (
            "POST",
            format!("{workspace}/programs"),
            json!({"id":Uuid::new_v4(),"title":"Forbidden draft","target_m":1000,"currency":"CZK","reward_minor":100,"max_participants":1}),
            &owner,
        ),
        (
            "POST",
            format!("{workspace}/invitations"),
            json!({"id":Uuid::new_v4(),"email":"another@example.test","role":"MEMBER"}),
            &owner,
        ),
        (
            "PATCH",
            format!("{workspace}/members/{}", member.user),
            json!({"role":"ADMIN"}),
            &owner,
        ),
        (
            "POST",
            format!("{path}/join"),
            json!({"id":Uuid::new_v4()}),
            &member,
        ),
        (
            "POST",
            "/api/organization-invitations/accept".to_string(),
            json!({"token":invite.1["token"]}),
            &invited,
        ),
    ] {
        let result = t.req(method, &target, body, actor).await;
        assert_eq!(
            result.0,
            StatusCode::CONFLICT,
            "{method} {target}: {}",
            result.1
        );
        assert_eq!(result.1["error"], "WORKSPACE_ARCHIVED");
    }
    assert_eq!(
        t.raw(&format!("{private}/upload"), VALID, &member).await.1["error"],
        "WORKSPACE_ARCHIVED"
    );
    assert_eq!(
        t.req("DELETE", &source, json!({}), &owner).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        t.req("DELETE", &source, json!({}), &invited).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        t.req("DELETE", &source, json!({}), &member).await.0,
        StatusCode::OK
    );
    assert_eq!(
        t.req("DELETE", &source, json!({}), &member).await.0,
        StatusCode::OK
    );
    assert_eq!(
        t.req("GET", &source, Value::Null, &member).await.1["error"],
        "SOURCE_FILE_REMOVED"
    );
    let erased = t.req("GET", &private, Value::Null, &member).await.1;
    assert_eq!(
        erased["uploads"][0]["decision"],
        before["uploads"][0]["decision"]
    );
    assert_eq!(erased["enrollment"], before["enrollment"]);
    assert!(erased["uploads"][0]["content_deleted_at"].is_string());
    let ready = t.req("GET", &path, Value::Null, &owner).await;
    assert_eq!(ready.1["closure"]["reason"], "WORKSPACE_ARCHIVED");
    assert_eq!(
        t.req(
            "POST",
            &format!("{workspace}/restore"),
            json!({"version":1}),
            &owner
        )
        .await
        .1["error"],
        "BUSINESS_VERSION_CHANGED"
    );
    let restored = t
        .req(
            "POST",
            &format!("{workspace}/restore"),
            json!({"version":2}),
            &owner,
        )
        .await;
    assert_eq!(restored.0, StatusCode::OK);
    assert!(restored.1["archived_at"].is_null());
    assert_eq!(restored.1["version"], 3);
    assert_eq!(
        t.req(
            "POST",
            &format!("{workspace}/restore"),
            json!({"version":2}),
            &owner
        )
        .await
        .1,
        restored.1
    );
    assert_eq!(t.balance(&owner).await, owner_balance);
    assert_eq!(t.balance(&member).await, member_balance);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM prototype_local_movements WHERE business_program_id=$1"
        )
        .bind(p)
        .fetch_one(&t.pool)
        .await
        .unwrap(),
        ledger_before
    );
    assert_eq!(
        t.req(
            "POST",
            "/api/organization-invitations/accept",
            json!({"token":invite.1["token"]}),
            &invited
        )
        .await
        .0,
        StatusCode::OK
    );
    for actor in [&member, &invited] {
        assert_eq!(
            t.req(
                "DELETE",
                &format!("{workspace}/members/{}", actor.user),
                json!({}),
                &owner
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    let managed = t.req("GET", &workspace, Value::Null, &owner).await;
    assert_eq!(
        managed.1["management"]["delete_reason"],
        "BUSINESS_HISTORY_MUST_BE_RETAINED"
    );
    let deleted = t
        .req(
            "DELETE",
            &workspace,
            json!({"password":PASSWORD,"confirmation":"Manual company"}),
            &owner,
        )
        .await;
    assert_eq!(deleted.0, StatusCode::CONFLICT);
    assert_eq!(deleted.1["error"], managed.1["management"]["delete_reason"]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM prototype_local_movements WHERE business_program_id=$1"
        )
        .bind(p)
        .fetch_one(&t.pool)
        .await
        .unwrap(),
        ledger_before
    );
    t.finish().await;
}

#[tokio::test]
async fn workspace_archive_guards_roles_versions_reserved_credits_and_restore_capacity() {
    let t = TestApp::new().await;
    let owner = t.account("archive-cap-owner@example.test").await;
    let admin = t.account("archive-cap-admin@example.test").await;
    let org = t.org(&owner).await;
    t.member(org, &admin, "ADMIN").await;
    let workspace = format!("/api/organizations/{org}");
    let p = t.draft(org, &owner, 1).await;
    assert_eq!(
        t.req("GET", &workspace, Value::Null, &admin).await.1["management"]["archive_reason"],
        "OWNER_REQUIRED"
    );
    assert_eq!(
        t.req(
            "POST",
            &format!("{workspace}/archive"),
            json!({"version":1}),
            &admin
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.req(
            "POST",
            &format!("{workspace}/archive"),
            json!({"version":0}),
            &owner
        )
        .await
        .1["error"],
        "BUSINESS_VERSION_CHANGED"
    );
    // Defensive guard also covers inconsistent retained program accounting, without touching a real wallet.
    sqlx::query("UPDATE company_programs SET budget_units=1,reserved_units=1 WHERE id=$1")
        .bind(p)
        .execute(&t.pool)
        .await
        .unwrap();
    assert_eq!(
        t.req("GET", &workspace, Value::Null, &owner).await.1["management"]["archive_reason"],
        "WORKSPACE_RESERVED_CREDITS"
    );
    assert_eq!(
        t.req(
            "POST",
            &format!("{workspace}/archive"),
            json!({"version":1}),
            &owner
        )
        .await
        .1["error"],
        "WORKSPACE_RESERVED_CREDITS"
    );
    sqlx::query("UPDATE company_programs SET budget_units=0,reserved_units=0 WHERE id=$1")
        .bind(p)
        .execute(&t.pool)
        .await
        .unwrap();
    let mut wrong_csrf = owner.clone();
    wrong_csrf.csrf = "invalid".into();
    assert_eq!(
        t.req(
            "POST",
            &format!("{workspace}/archive"),
            json!({"version":1}),
            &wrong_csrf
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let archive_path = format!("{workspace}/archive");
    let (a, b) = tokio::join!(
        t.req("POST", &archive_path, json!({"version":1}), &owner),
        t.req("POST", &archive_path, json!({"version":1}), &owner)
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    assert_eq!(a.1, b.1);
    let mut active = Vec::new();
    for _ in 0..10 {
        active.push(t.org(&owner).await);
    }
    let listed = t
        .req("GET", "/api/organizations", Value::Null, &owner)
        .await
        .1;
    assert_eq!(listed["organizations"].as_array().unwrap().len(), 10);
    assert_eq!(
        listed["archived_organizations"].as_array().unwrap().len(),
        1
    );
    let limit = t
        .req(
            "POST",
            &format!("{workspace}/restore"),
            json!({"version":2}),
            &owner,
        )
        .await;
    assert_eq!(limit.0, StatusCode::BAD_REQUEST);
    assert_eq!(limit.1["error"], "BUSINESS_OWNERSHIP_LIMIT");
    assert_eq!(
        t.req(
            "POST",
            &format!("/api/organizations/{}/archive", active[0]),
            json!({"version":1}),
            &owner
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        t.req(
            "POST",
            &format!("{workspace}/restore"),
            json!({"version":2}),
            &owner
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM organization_events WHERE organization_id=$1 AND kind='organization_archived'").bind(org).fetch_one(&t.pool).await.unwrap(), 1);
    for _ in 0..9 {
        t.org(&admin).await;
    }
    let old = t.org(&admin).await;
    assert_eq!(
        t.req(
            "POST",
            &format!("/api/organizations/{old}/archive"),
            json!({"version":1}),
            &admin
        )
        .await
        .0,
        StatusCode::OK
    );
    // Archived history does not consume one of the ten active ownership places.
    assert_eq!(
        t.req(
            "POST",
            &format!("{workspace}/transfer-owner"),
            json!({"user_id":admin.user,"version":3}),
            &owner
        )
        .await
        .0,
        StatusCode::OK
    );
    t.member(active[1], &admin, "MEMBER").await;
    assert_eq!(
        t.req(
            "POST",
            &format!("/api/organizations/{}/transfer-owner", active[1]),
            json!({"user_id":admin.user,"version":1}),
            &owner
        )
        .await
        .1["error"],
        "BUSINESS_OWNERSHIP_LIMIT"
    );
    t.finish().await;
}

#[tokio::test]
async fn paid_partial_capacity_keeps_promised_enrollment_window_open_then_refunds_unused_places() {
    let t = TestApp::new().await;
    let owner = t.account("partial-cap-owner@example.test").await;
    let runner = t.account("partial-cap-runner@example.test").await;
    let org = t.org(&owner).await;
    t.member(org, &runner, "MEMBER").await;
    t.grant(&owner).await;
    let p = t.draft(org, &owner, 2).await;
    t.publish(org, p, &owner).await;
    let e = t.join(org, p, &runner).await;
    let private = ep(org, p, e);
    assert_eq!(
        t.raw(&format!("{private}/upload"), VALID, &runner).await.0,
        StatusCode::OK
    );
    assert_eq!(
        t.req("POST", &format!("{private}/claim"), json!({}), &runner)
            .await
            .0,
        StatusCode::OK
    );
    let path = base(org, p);
    let readiness = t.req("GET", &path, Value::Null, &owner).await.1["closure"].clone();
    assert_eq!(readiness["participant_count"], 1);
    assert_eq!(readiness["rewarded_count"], 1);
    assert_eq!(readiness["unpaid_rewards"], 0);
    assert_eq!(readiness["reason"], "BUSINESS_UPLOAD_WINDOW_OPEN");
    assert_eq!(
        t.req(
            "POST",
            &format!("{path}/close"),
            json!({"version":2}),
            &owner
        )
        .await
        .1["error"],
        readiness["reason"]
    );
    let before = t.req("GET", &private, Value::Null, &runner).await.1;
    assert_eq!(t.req("GET", &private, Value::Null, &runner).await.1, before);
    let unchanged = t.req("GET", &path, Value::Null, &owner).await.1;
    assert_eq!(unchanged["program"]["version"], 2);
    assert_eq!(unchanged["program"]["paid_units"], 5_000_000);
    t.expire_upload(p).await;
    assert_eq!(
        t.req("GET", &path, Value::Null, &owner).await.1["closure"]["allowed"],
        true
    );
    let closed = t
        .req(
            "POST",
            &format!("{path}/close"),
            json!({"version":2}),
            &owner,
        )
        .await;
    assert_eq!(closed.0, StatusCode::OK);
    assert_eq!(closed.1["returned_units"], 5_000_000);
    assert_eq!(closed.1["paid_units"], 5_000_000);
    assert_eq!(t.balance(&owner).await["available"], 95_000_000);
    assert_eq!(t.balance(&owner).await["locked"], 0);
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
    state: AppState,
    pool: PgPool,
    admin: PgPool,
    schema: String,
}
impl TestApp {
    async fn new() -> Self {
        Self::with_connections(8).await
    }
    async fn with_connections(connections: u32) -> Self {
        let url = std::env::var("TEST_DATABASE_URL")
            .expect("TEST_DATABASE_URL required; business DB tests never silently skipped");
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
            .max_connections(connections)
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
        let app = router(state.clone(), "missing-dist");
        Self {
            app,
            state,
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
        s: Option<&Session>,
        raw: bool,
    ) -> (StatusCode, Value, String) {
        let mut b = Request::builder()
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
        if let Some(s) = s {
            b = b
                .header("cookie", &s.cookie)
                .header("x-csrf-token", &s.csrf);
        }
        let mut req = b.body(Body::from(bytes)).unwrap();
        req.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:6000".parse::<std::net::SocketAddr>().unwrap(),
        ));
        let response = self.app.clone().oneshot(req).await.unwrap();
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
        input: Value,
        s: &Session,
    ) -> (StatusCode, Value) {
        let (status, data, _) = self
            .bytes(method, path, input.to_string().into_bytes(), Some(s), false)
            .await;
        (status, data)
    }
    async fn raw(&self, path: &str, input: &[u8], s: &Session) -> (StatusCode, Value) {
        let (status, data, _) = self
            .bytes("POST", path, input.to_vec(), Some(s), true)
            .await;
        (status, data)
    }
    async fn account(&self, email: &str) -> Session {
        let result = self
            .bytes(
                "POST",
                "/api/auth/register",
                json!({"email":email,"password":PASSWORD,"display_name":"Business runner"})
                    .to_string()
                    .into_bytes(),
                None,
                false,
            )
            .await;
        assert_eq!(result.0, StatusCode::OK, "{}", result.1);
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
        assert_eq!(status, StatusCode::OK);
        let mut s = Session {
            user: Uuid::nil(),
            cookie: cookie.split(';').next().unwrap().into(),
            csrf: body["csrf_token"].as_str().unwrap().into(),
        };
        let (_, session) = self.req("GET", "/api/auth/session", Value::Null, &s).await;
        s.user = Uuid::parse_str(session["user"]["id"].as_str().unwrap()).unwrap();
        s
    }
    async fn org(&self, s: &Session) -> Uuid {
        let id = Uuid::new_v4();
        let response = self
            .req(
                "POST",
                "/api/organizations",
                json!({"id":id,"name":"Manual company"}),
                s,
            )
            .await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.1);
        id
    }
    async fn member(&self, org: Uuid, s: &Session, role: &str) {
        sqlx::query(
            "INSERT INTO organization_members(organization_id,user_id,role) VALUES($1,$2,$3)",
        )
        .bind(org)
        .bind(s.user)
        .bind(role)
        .execute(&self.pool)
        .await
        .unwrap();
    }
    async fn draft(&self, org: Uuid, s: &Session, cap: i32) -> Uuid {
        let id = Uuid::new_v4();
        let response=self.req("POST",&format!("/api/organizations/{org}/programs"),json!({"id":id,"title":"A voluntary run","target_m":3000,"currency":"CZK","reward_minor":25000,"max_participants":cap}),s).await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.1);
        id
    }
    async fn grant(&self, s: &Session) {
        let r = self
            .req(
                "POST",
                "/api/prototype/local/grant",
                json!({"id":Uuid::new_v4()}),
                s,
            )
            .await;
        assert_eq!(r.0, StatusCode::OK, "{}", r.1);
    }
    fn publish_input() -> Value {
        let now = Utc::now();
        json!({"version":1,"reward_units":5_000_000,"profile":"REPLAY","starts_at":now,"ends_at":now+Duration::minutes(10),"upload_deadline":now+Duration::minutes(10),"review_deadline":now+Duration::minutes(15)})
    }
    async fn publish(&self, org: Uuid, p: Uuid, s: &Session) {
        let r = self
            .req(
                "POST",
                &format!("/api/organizations/{org}/programs/{p}/publish"),
                Self::publish_input(),
                s,
            )
            .await;
        assert_eq!(r.0, StatusCode::OK, "{}", r.1);
    }
    async fn join(&self, org: Uuid, p: Uuid, s: &Session) -> Uuid {
        let id = Uuid::new_v4();
        let r = self
            .req(
                "POST",
                &format!("/api/organizations/{org}/programs/{p}/join"),
                json!({"id":id}),
                s,
            )
            .await;
        assert_eq!(r.0, StatusCode::OK, "{}", r.1);
        Uuid::parse_str(r.1["id"].as_str().unwrap()).unwrap()
    }
    async fn balance(&self, s: &Session) -> Value {
        let r = self
            .req("GET", "/api/prototype/local/balance", Value::Null, s)
            .await;
        assert_eq!(r.0, StatusCode::OK);
        r.1
    }
    async fn expire_upload(&self, p: Uuid) {
        sqlx::query("UPDATE company_programs SET starts_at=now()-interval '30 minutes',ends_at=now()-interval '20 minutes',upload_deadline=now()-interval '10 minutes',review_deadline=now()+interval '10 minutes' WHERE id=$1").bind(p).execute(&self.pool).await.unwrap();
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
fn base(org: Uuid, p: Uuid) -> String {
    format!("/api/organizations/{org}/programs/{p}")
}
fn ep(org: Uuid, p: Uuid, e: Uuid) -> String {
    format!("{}/enrollments/{e}", base(org, p))
}

#[tokio::test]
async fn funded_join_reward_and_unused_refund_are_atomic_and_preserve_czk_planning() {
    let t = TestApp::new().await;
    let owner = t.account("fund-owner@example.test").await;
    let runner = t.account("fund-runner@example.test").await;
    let other = t.account("fund-other@example.test").await;
    let org = t.org(&owner).await;
    t.member(org, &runner, "MEMBER").await;
    t.member(org, &other, "MEMBER").await;
    t.grant(&owner).await;
    let p = t.draft(org, &owner, 2).await;
    let path = base(org, p);
    let publish = TestApp::publish_input();
    let publishpath = format!("{path}/publish");
    let (a, b) = tokio::join!(
        t.req("POST", &publishpath, publish.clone(), &owner),
        t.req("POST", &publishpath, publish, &owner)
    );
    assert_eq!(a.0, StatusCode::OK, "{}", a.1);
    assert_eq!(b.0, StatusCode::OK, "{}", b.1);
    assert_eq!(a.1["currency"], "CZK");
    assert_eq!(a.1["reward_minor"], 25000);
    assert_eq!(a.1["budget_units"], 10_000_000);
    assert_eq!(t.balance(&owner).await["available"], 90_000_000);
    let joinid = Uuid::new_v4();
    let joinpath = format!("{path}/join");
    let (a, b) = tokio::join!(
        t.req("POST", &joinpath, json!({"id":joinid}), &runner),
        t.req("POST", &joinpath, json!({"id":joinid}), &runner)
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    assert_eq!(a.1["id"], b.1["id"]);
    let enrol = Uuid::parse_str(a.1["id"].as_str().unwrap()).unwrap();
    t.join(org, p, &other).await;
    let uploadpath = format!("{}/upload", ep(org, p, enrol));
    let uploaded = t.raw(&uploadpath, VALID, &runner).await;
    assert_eq!(uploaded.0, StatusCode::OK, "{}", uploaded.1);
    assert_eq!(uploaded.1["decision"], "ACCEPTED");
    let claim = format!("{}/claim", ep(org, p, enrol));
    let (a, b) = tokio::join!(
        t.req("POST", &claim, json!({}), &runner),
        t.req("POST", &claim, json!({}), &runner)
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    assert_eq!(t.balance(&runner).await["available"], 5_000_000);
    assert_eq!(t.balance(&owner).await["locked"], 5_000_000);
    let retry = t.raw(&uploadpath, VALID, &runner).await;
    assert_eq!(retry.0, StatusCode::OK);
    assert_eq!(retry.1["id"], uploaded.1["id"]);
    assert_eq!(retry.1["duplicate"], true);
    let readiness = t.req("GET", &path, Value::Null, &owner).await;
    assert_eq!(readiness.1["closure"]["allowed"], false);
    assert_eq!(
        readiness.1["closure"]["reason"],
        "BUSINESS_UPLOAD_WINDOW_OPEN"
    );
    assert_eq!(readiness.1["closure"]["participant_count"], 2);
    assert_eq!(readiness.1["closure"]["rewarded_count"], 1);
    assert!(readiness.1["closure"]["available_at"].is_string());
    let before = t
        .req(
            "POST",
            &format!("{path}/close"),
            json!({"version":2}),
            &owner,
        )
        .await;
    assert_eq!(before.0, StatusCode::CONFLICT);
    assert_eq!(before.1["error"], readiness.1["closure"]["reason"]);
    t.expire_upload(p).await;
    assert_eq!(
        t.req("GET", &path, Value::Null, &owner).await.1["closure"]["allowed"],
        true
    );
    let closeinput = json!({"version":2});
    let closepath = format!("{path}/close");
    let (a, b) = tokio::join!(
        t.req("POST", &closepath, closeinput.clone(), &owner),
        t.req("POST", &closepath, closeinput, &owner)
    );
    assert_eq!(a.0, StatusCode::OK, "{}", a.1);
    assert_eq!(b.0, StatusCode::OK);
    assert_eq!(a.1["paid_units"], 5_000_000);
    assert_eq!(a.1["returned_units"], 5_000_000);
    assert_eq!(a.1["reserved_units"], 0);
    assert_eq!(t.balance(&owner).await["available"], 95_000_000);
    assert_eq!(t.balance(&owner).await["locked"], 0);
    assert_eq!(t.balance(&other).await["forfeited"], 0);
    let(rows,total,business):(i64,i64,i64)=sqlx::query_as("SELECT count(*),sum(wallet_delta+locked_delta+recipient_delta+issued_delta+business_delta)::bigint,sum(business_delta)::bigint FROM prototype_local_movements WHERE business_program_id=$1").bind(p).fetch_one(&t.pool).await.unwrap();
    assert_eq!((rows, total, business), (4, 0, 0));
    assert_eq!(
        t.req(
            "POST",
            &format!("{path}/archive"),
            json!({"version":3}),
            &owner
        )
        .await
        .0,
        StatusCode::OK
    );
    t.finish().await;
}

#[tokio::test]
async fn capacity_and_employer_wallet_cannot_be_overspent_by_concurrent_programs_or_joins() {
    let t = TestApp::new().await;
    let owner = t.account("cap-owner@example.test").await;
    let a = t.account("cap-a@example.test").await;
    let b = t.account("cap-b@example.test").await;
    let org = t.org(&owner).await;
    t.member(org, &a, "MEMBER").await;
    t.member(org, &b, "MEMBER").await;
    t.grant(&owner).await;
    let p = t.draft(org, &owner, 1).await;
    t.publish(org, p, &owner).await;
    let joinpath = format!("{}/join", base(org, p));
    let (x, y) = tokio::join!(
        t.req("POST", &joinpath, json!({"id":Uuid::new_v4()}), &a),
        t.req("POST", &joinpath, json!({"id":Uuid::new_v4()}), &b)
    );
    assert!(matches!(
        (x.0, y.0),
        (StatusCode::OK, StatusCode::CONFLICT) | (StatusCode::CONFLICT, StatusCode::OK)
    ));
    let p1 = t.draft(org, &owner, 15).await;
    let p2 = t.draft(org, &owner, 15).await;
    let body = TestApp::publish_input();
    let path1 = format!("{}/publish", base(org, p1));
    let path2 = format!("{}/publish", base(org, p2));
    let (x, y) = tokio::join!(
        t.req("POST", &path1, body.clone(), &owner),
        t.req("POST", &path2, body, &owner)
    );
    assert!(matches!(
        (x.0, y.0),
        (StatusCode::OK, StatusCode::BAD_REQUEST) | (StatusCode::BAD_REQUEST, StatusCode::OK)
    ));
    assert_eq!(t.balance(&owner).await["available"], 20_000_000);
    assert_eq!(t.balance(&owner).await["locked"], 80_000_000);
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM prototype_commands")
        .fetch_one(&t.pool)
        .await
        .unwrap();
    assert_eq!(
        pending, 0,
        "B2B local funding must never invoke a chain command"
    );
    t.finish().await;
}

#[tokio::test]
async fn manual_invites_are_email_bound_expiring_revocable_and_owner_controls_roles() {
    let t = TestApp::new().await;
    let owner = t.account("invite-owner@example.test").await;
    let member = t.account("invite-member@example.test").await;
    let outsider = t.account("invite-outsider@example.test").await;
    let org = t.org(&owner).await;
    let input = json!({"id":Uuid::new_v4(),"email":" invite-member@example.test ","role":"MEMBER"});
    let path = format!("/api/organizations/{org}/invitations");
    let made = t.req("POST", &path, input.clone(), &owner).await;
    assert_eq!(made.0, StatusCode::OK);
    let token = made.1["token"].as_str().unwrap().to_string();
    let repeated = t.req("POST", &path, input, &owner).await;
    assert_eq!(repeated.0, StatusCode::OK);
    assert!(repeated.1["token"].is_null());
    assert_eq!(
        t.req(
            "POST",
            "/api/organization-invitations/accept",
            json!({"token":token}),
            &outsider
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (a, b) = tokio::join!(
        t.req(
            "POST",
            "/api/organization-invitations/accept",
            json!({"token":token}),
            &member
        ),
        t.req(
            "POST",
            "/api/organization-invitations/accept",
            json!({"token":token}),
            &member
        )
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    let mpath = format!("/api/organizations/{org}/members/{}", member.user);
    assert_eq!(
        t.req("PATCH", &mpath, json!({"role":"ADMIN"}), &member)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.req("PATCH", &mpath, json!({"role":"ADMIN"}), &owner)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.req(
            "POST",
            &path,
            json!({"id":Uuid::new_v4(),"email":"invite-outsider@example.test","role":"ADMIN"}),
            &member
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let expired = Uuid::new_v4();
    let inv = t
        .req(
            "POST",
            &path,
            json!({"id":expired,"email":"invite-outsider@example.test","role":"MEMBER"}),
            &owner,
        )
        .await;
    sqlx::query(
        "UPDATE organization_invitations SET expires_at=now()-interval '1 second' WHERE id=$1",
    )
    .bind(expired)
    .execute(&t.pool)
    .await
    .unwrap();
    assert_eq!(
        t.req(
            "POST",
            "/api/organization-invitations/accept",
            json!({"token":inv.1["token"]}),
            &outsider
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let revoked = Uuid::new_v4();
    let inv = t
        .req(
            "POST",
            &path,
            json!({"id":revoked,"email":"invite-outsider@example.test","role":"MEMBER"}),
            &owner,
        )
        .await;
    assert_eq!(
        t.req(
            "POST",
            &format!("/api/organizations/{org}/invitations/{revoked}/revoke"),
            json!({}),
            &owner
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        t.req(
            "POST",
            "/api/organization-invitations/accept",
            json!({"token":inv.1["token"]}),
            &outsider
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let dto = t
        .req(
            "GET",
            &format!("/api/organizations/{org}"),
            Value::Null,
            &owner,
        )
        .await
        .1;
    assert!(!dto.to_string().contains(&token));
    assert!(!dto.to_string().contains("token_hash"));
    assert_eq!(
        t.req(
            "POST",
            &format!("/api/organizations/{org}/transfer-owner"),
            json!({"user_id":member.user,"version":1}),
            &owner
        )
        .await
        .0,
        StatusCode::OK
    );
    let dto = t
        .req(
            "GET",
            &format!("/api/organizations/{org}"),
            Value::Null,
            &member,
        )
        .await
        .1;
    assert_eq!(dto["organization"]["role"], "OWNER");
    t.finish().await;
}

#[tokio::test]
async fn company_admins_see_progress_but_never_private_telemetry_and_cannot_override_goals() {
    let t = TestApp::new().await;
    let owner = t.account("privacy-owner@example.test").await;
    let runner = t.account("privacy-runner@example.test").await;
    let operator = t.account("privacy-operator@example.test").await;
    let stranger = t.account("privacy-stranger@example.test").await;
    sqlx::query("INSERT INTO prototype_operators(user_id) VALUES($1)")
        .bind(operator.user)
        .execute(&t.pool)
        .await
        .unwrap();
    let org = t.org(&owner).await;
    t.member(org, &runner, "MEMBER").await;
    t.grant(&owner).await;
    let p = t.draft(org, &owner, 1).await;
    t.publish(org, p, &owner).await;
    let e = t.join(org, p, &runner).await;
    let private = ep(org, p, e);
    let short = t.raw(&format!("{private}/upload"), SHORT, &runner).await;
    assert_eq!(short.0, StatusCode::OK);
    assert_eq!(short.1["decision"], "REJECTED");
    let review=t.req("POST",&format!("{private}/review"),json!({"upload_id":short.1["id"],"accept":true,"reason":"A manual claim cannot override the distance."}),&operator).await;
    assert_eq!(review.0, StatusCode::BAD_REQUEST);
    let upload = t.raw(&format!("{private}/upload"), REVIEW, &runner).await;
    assert_eq!(upload.0, StatusCode::OK);
    assert_eq!(upload.1["decision"], "REVIEW_REQUIRED");
    for s in [&owner, &stranger] {
        assert_eq!(
            t.req("GET", &private, Value::Null, s).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            t.req(
                "GET",
                &format!(
                    "{private}/uploads/{}/source",
                    upload.1["id"].as_str().unwrap()
                ),
                Value::Null,
                s
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
    let safe = t.req("GET", &base(org, p), Value::Null, &owner).await;
    assert_eq!(safe.0, StatusCode::OK);
    assert_eq!(safe.1["participants"][0]["assessment"], "REVIEW_REQUIRED");
    for secret in ["heart_rate", "telemetry", "activity", "fingerprint", "<gpx"] {
        assert!(!safe.1.to_string().contains(secret));
    }
    let source = t.req("GET", &private, Value::Null, &runner).await;
    assert_eq!(source.0, StatusCode::OK);
    assert_eq!(
        source.1["uploads"][1]["activity"]["source_authenticity"],
        "unverified"
    );
    t.expire_upload(p).await;
    let readiness = t.req("GET", &base(org, p), Value::Null, &owner).await;
    assert_eq!(readiness.1["closure"]["reason"], "BUSINESS_REVIEW_PENDING");
    assert_eq!(readiness.1["closure"]["pending_reviews"], 1);
    assert_eq!(readiness.1["closure"]["unpaid_rewards"], 0);
    assert_eq!(
        t.req(
            "POST",
            &format!("{}/close", base(org, p)),
            json!({"version":2}),
            &owner
        )
        .await
        .1["error"],
        "BUSINESS_REVIEW_PENDING"
    );
    assert_eq!(t.req("POST",&format!("{private}/review"),json!({"upload_id":upload.1["id"],"accept":true,"reason":"Reviewed the sample recording and accepted its plausibility."}),&owner).await.0,StatusCode::FORBIDDEN);
    assert_eq!(t.req("POST",&format!("{private}/review"),json!({"upload_id":upload.1["id"],"accept":true,"reason":"Reviewed the sample recording and accepted its plausibility."}),&operator).await.0,StatusCode::OK);
    let readiness = t.req("GET", &base(org, p), Value::Null, &owner).await;
    assert_eq!(
        readiness.1["closure"]["reason"],
        "BUSINESS_ACCEPTED_REWARDS_MUST_BE_PAID"
    );
    assert_eq!(readiness.1["closure"]["unpaid_rewards"], 1);
    assert!(readiness.1["closure"]["available_at"].is_null());
    assert_eq!(
        t.req(
            "POST",
            &format!("{}/close", base(org, p)),
            json!({"version":2}),
            &owner
        )
        .await
        .1["error"],
        "BUSINESS_ACCEPTED_REWARDS_MUST_BE_PAID"
    );
    let q = t
        .req("GET", "/api/business/review", Value::Null, &operator)
        .await;
    assert_eq!(q.1["enrollments"].as_array().unwrap().len(), 1);
    assert_eq!(
        t.req("POST", &format!("{private}/claim"), json!({}), &operator)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.req(
            "POST",
            &format!("{}/close", base(org, p)),
            json!({"version":2}),
            &owner
        )
        .await
        .0,
        StatusCode::OK
    );
    t.finish().await;
}

#[tokio::test]
async fn unpublished_terms_tenants_and_active_claims_are_protected() {
    let t = TestApp::new().await;
    let owner = t.account("terms-owner@example.test").await;
    let admin = t.account("terms-admin@example.test").await;
    let runner = t.account("terms-runner@example.test").await;
    let other = t.account("terms-other@example.test").await;
    let org = t.org(&owner).await;
    t.member(org, &admin, "ADMIN").await;
    t.member(org, &runner, "MEMBER").await;
    t.grant(&owner).await;
    let p = t.draft(org, &owner, 1).await;
    let path = base(org, p);
    assert_eq!(
        t.req("GET", &path, Value::Null, &runner).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        t.req(
            "POST",
            &format!("{path}/publish"),
            TestApp::publish_input(),
            &admin
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.req(
            "POST",
            &format!("{path}/publish"),
            TestApp::publish_input(),
            &other
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    t.publish(org, p, &owner).await;
    t.join(org, p, &runner).await;
    assert_eq!(t.req("PATCH",&path,json!({"title":"Changed terms","target_m":1000,"currency":"CZK","reward_minor":100,"max_participants":1,"version":2}),&owner).await.0,StatusCode::CONFLICT);
    assert_eq!(
        t.req(
            "POST",
            &format!("{path}/archive"),
            json!({"version":2}),
            &owner
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        t.req(
            "DELETE",
            &format!("/api/organizations/{org}/members/{}", runner.user),
            json!({}),
            &owner
        )
        .await
        .1["error"],
        "ACTIVE_BUSINESS_ENROLLMENT"
    );
    assert_eq!(
        t.req(
            "DELETE",
            "/api/account",
            json!({"password":PASSWORD,"confirmation":"DELETE"}),
            &runner
        )
        .await
        .1["error"],
        "ACTIVE_BUSINESS_MUST_SETTLE"
    );
    assert_eq!(
        t.req(
            "POST",
            &format!("{path}/close"),
            json!({"version":2}),
            &owner
        )
        .await
        .1["error"],
        "BUSINESS_UPLOAD_WINDOW_OPEN"
    );
    t.expire_upload(p).await;
    assert_eq!(
        t.req(
            "POST",
            &format!("{path}/close"),
            json!({"version":2}),
            &owner
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        t.req(
            "DELETE",
            &format!("/api/organizations/{org}/members/{}", runner.user),
            json!({}),
            &owner
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        t.req(
            "DELETE",
            "/api/account",
            json!({"password":PASSWORD,"confirmation":"DELETE"}),
            &runner
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(t.balance(&owner).await["available"], 100_000_000);
    t.finish().await;
}

#[tokio::test]
async fn qualifying_evidence_is_reserved_across_b2c_and_business_programs() {
    let t = TestApp::new().await;
    let owner = t.account("reuse-owner@example.test").await;
    let runner = t.account("reuse-runner@example.test").await;
    let org = t.org(&owner).await;
    t.member(org, &runner, "MEMBER").await;
    t.grant(&owner).await;
    t.grant(&runner).await;
    let p = t.draft(org, &owner, 1).await;
    t.publish(org, p, &owner).await;
    let e = t.join(org, p, &runner).await;
    let c = Uuid::new_v4();
    let body = json!({"id":c,"title":"Personal run","target_m":3000,"amount_units":5_000_000,"network":"LOCAL","profile":"REPLAY","starts_at":Utc::now()});
    assert_eq!(
        t.req("POST", "/api/prototype/challenges", body, &runner)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.req(
            "POST",
            &format!("/api/prototype/challenges/{c}/local-action"),
            json!({"action":"DEPOSIT"}),
            &runner
        )
        .await
        .0,
        StatusCode::OK
    );
    let personal = format!("/api/prototype/challenges/{c}/upload");
    let business = format!("{}/upload", ep(org, p, e));
    let (a, b) = tokio::join!(
        t.raw(&personal, VALID, &runner),
        t.raw(&business, VALID, &runner)
    );
    assert!(
        matches!(
            (a.0, b.0),
            (StatusCode::OK, StatusCode::CONFLICT) | (StatusCode::CONFLICT, StatusCode::OK)
        ),
        "{a:?} {b:?}"
    );
    let blocked = if a.0 == StatusCode::CONFLICT {
        a.1
    } else {
        b.1
    };
    assert_eq!(blocked["error"], "ACTIVITY_ALREADY_USED");
    let p2 = t.draft(org, &owner, 1).await;
    t.publish(org, p2, &owner).await;
    let e2 = t.join(org, p2, &runner).await;
    assert_eq!(
        t.raw(&format!("{}/upload", ep(org, p2, e2)), VALID, &runner)
            .await
            .1["error"],
        "ACTIVITY_ALREADY_USED"
    );
    let files:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM company_uploads WHERE goal_result='MET')+(SELECT count(*) FROM prototype_uploads WHERE goal_result='MET')").fetch_one(&t.pool).await.unwrap();
    assert_eq!(files, 1);
    t.finish().await;
}

#[tokio::test]
async fn participant_export_and_source_removal_do_not_expose_tokens_or_other_employees() {
    let t = TestApp::new().await;
    let owner = t.account("export-owner@example.test").await;
    let runner = t.account("export-runner@example.test").await;
    let org = t.org(&owner).await;
    t.member(org, &runner, "MEMBER").await;
    t.grant(&owner).await;
    let p = t.draft(org, &owner, 1).await;
    t.publish(org, p, &owner).await;
    let e = t.join(org, p, &runner).await;
    let path = ep(org, p, e);
    let upload = t.raw(&format!("{path}/upload"), VALID, &runner).await;
    assert_eq!(upload.0, StatusCode::OK);
    let source = format!("{path}/uploads/{}/source", upload.1["id"].as_str().unwrap());
    assert_eq!(
        t.req("DELETE", &source, json!({}), &runner).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        t.req("POST", &format!("{path}/claim"), json!({}), &runner)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.req("DELETE", &source, json!({}), &runner).await.0,
        StatusCode::OK
    );
    assert_eq!(
        t.req("DELETE", &source, json!({}), &runner).await.0,
        StatusCode::OK
    );
    assert_eq!(
        t.req("GET", &source, Value::Null, &runner).await.0,
        StatusCode::CONFLICT
    );
    let export = t
        .req("GET", "/api/account/export", Value::Null, &runner)
        .await;
    assert_eq!(export.0, StatusCode::OK);
    assert_eq!(
        export.1["business_enrollments"].as_array().unwrap().len(),
        1
    );
    assert_eq!(export.1["business_uploads"].as_array().unwrap().len(), 1);
    for secret in [
        "token_hash",
        "<gpx",
        "telemetry",
        "heart_rate",
        "fingerprint",
        "content",
    ] {
        assert!(!export.1["business_uploads"].to_string().contains(secret));
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM prototype_local_movements WHERE business_enrollment_id=$1",
    )
    .bind(e)
    .fetch_one(&t.pool)
    .await
    .unwrap();
    assert_eq!(count, 2);
    t.finish().await;
}

type CapturedBusinessUpload = (
    truhabit_api::auth::Auth,
    axum::http::HeaderMap,
    truhabit_api::business::RecordedBusinessUpload,
);
async fn capture_business_upload(
    t: &TestApp,
    s: &Session,
    org: Uuid,
    p: Uuid,
    e: Uuid,
) -> CapturedBusinessUpload {
    type Sender = std::sync::Arc<
        tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<CapturedBusinessUpload>>>,
    >;
    async fn capture(
        auth: truhabit_api::auth::Auth,
        axum::Extension(sender): axum::Extension<Sender>,
        headers: axum::http::HeaderMap,
        recorded: truhabit_api::business::RecordedBusinessUpload,
    ) -> StatusCode {
        assert!(
            sender
                .lock()
                .await
                .take()
                .unwrap()
                .send((auth, headers, recorded))
                .is_ok()
        );
        StatusCode::NO_CONTENT
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    let app = Router::new()
        .route(
            "/capture/{org}/{program}/{enrollment}",
            axum::routing::post(capture),
        )
        .layer(axum::Extension(std::sync::Arc::new(
            tokio::sync::Mutex::new(Some(send)),
        )))
        .layer(axum::extract::DefaultBodyLimit::max(
            truhabit_evidence::upload::MAX_BYTES,
        ))
        .with_state(t.state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/capture/{org}/{p}/{e}"))
                .header("cookie", &s.cookie)
                .header("x-csrf-token", &s.csrf)
                .body(Body::from(VALID.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    receive.await.unwrap()
}
async fn deliver_business_upload(
    t: &TestApp,
    org: Uuid,
    p: Uuid,
    e: Uuid,
    captured: CapturedBusinessUpload,
) -> (StatusCode, Value) {
    let (auth, headers, recorded) = captured;
    match truhabit_api::business::upload(
        auth,
        axum::extract::State(t.state.clone()),
        axum::extract::Path((org, p, e)),
        axum::extract::Query(truhabit_api::prototype::UploadQuery { session: None }),
        headers,
        recorded,
    )
    .await
    {
        Ok(axum::Json(value)) => (StatusCode::OK, value),
        Err(error) => (error.status, json!({"error":error.message})),
    }
}
async fn business_deadline_fixture() -> (
    TestApp,
    Session,
    Session,
    Uuid,
    Uuid,
    Uuid,
    chrono::DateTime<Utc>,
) {
    let t = TestApp::new().await;
    let owner = t.account("deadline-owner@example.test").await;
    let runner = t.account("deadline-runner@example.test").await;
    let org = t.org(&owner).await;
    t.member(org, &runner, "MEMBER").await;
    t.grant(&owner).await;
    let p = t.draft(org, &owner, 1).await;
    t.publish(org, p, &owner).await;
    let e = t.join(org, p, &runner).await;
    let deadline = Utc::now() + Duration::seconds(2);
    sqlx::query("UPDATE company_programs SET starts_at=$1,ends_at=$2,upload_deadline=$3,review_deadline=$4 WHERE id=$5").bind(deadline-Duration::minutes(20)).bind(deadline-Duration::minutes(10)).bind(deadline).bind(deadline+Duration::minutes(5)).bind(p).execute(&t.pool).await.unwrap();
    (t, owner, runner, org, p, e, deadline)
}
async fn wait_deadline(deadline: chrono::DateTime<Utc>) {
    let limit = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while Utc::now() <= deadline {
        assert!(tokio::time::Instant::now() < limit);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
#[tokio::test]
async fn company_complete_body_cannot_lose_earned_reward_to_closure_before_handler_scheduling() {
    let (t, owner, runner, org, p, e, deadline) = business_deadline_fixture().await;
    // This invokes the actual routed production extractor; only subsequent handler scheduling is paused.
    let captured = capture_business_upload(&t, &runner, org, p, e).await;
    assert!(captured.2.received_at() <= deadline);
    wait_deadline(deadline).await;
    let closepath = format!("{}/close", base(org, p));
    let mut close = Box::pin(t.req("POST", &closepath, json!({"version":2}), &owner));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut close)
            .await
            .is_err(),
        "Program closure bypassed admitted body locks"
    );
    let upload = deliver_business_upload(&t, org, p, e, captured).await;
    let closed = close.await;
    let outcome:(String,String,i64)=sqlx::query_as("SELECT p.state,e.assessment,p.reserved_units FROM company_programs p JOIN company_enrollments e ON e.program_id=p.id WHERE e.id=$1").bind(e).fetch_one(&t.pool).await.unwrap();
    t.finish().await;
    assert_eq!(upload.0, StatusCode::OK, "{}", upload.1);
    assert_eq!(closed.0, StatusCode::CONFLICT);
    assert_eq!(closed.1["error"], "BUSINESS_ACCEPTED_REWARDS_MUST_BE_PAID");
    assert_eq!(outcome, ("PUBLISHED".into(), "MET".into(), 5_000_000));
}
#[tokio::test]
async fn company_late_body_and_abandoned_admission_release_budget_without_inventing_evidence() {
    let (t, owner, runner, org, p, e, deadline) = business_deadline_fixture().await;
    let captured = capture_business_upload(&t, &runner, org, p, e).await;
    drop(captured);
    wait_deadline(deadline).await;
    let late = capture_business_upload(&t, &runner, org, p, e).await;
    assert!(late.2.received_at() > deadline);
    let upload = deliver_business_upload(&t, org, p, e, late).await;
    assert_eq!(upload.0, StatusCode::CONFLICT);
    assert_eq!(upload.1["error"], "BUSINESS_UPLOAD_CLOSED");
    let close = t
        .req(
            "POST",
            &format!("{}/close", base(org, p)),
            json!({"version":2}),
            &owner,
        )
        .await;
    assert_eq!(close.0, StatusCode::OK, "{}", close.1);
    assert_eq!(close.1["returned_units"], 5_000_000);
    assert_eq!(t.balance(&owner).await["available"], 100_000_000);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM company_uploads WHERE enrollment_id=$1")
            .bind(e)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    t.finish().await;
    assert_eq!(count, 0);
}

#[tokio::test]
async fn company_body_limits_csrf_and_fit_retries_use_only_one_pool_lease_and_the_chosen_session() {
    let t = TestApp::with_connections(1).await;
    let owner = t.account("fit-owner@example.test").await;
    let runner = t.account("fit-runner@example.test").await;
    let org = t.org(&owner).await;
    t.member(org, &runner, "MEMBER").await;
    t.grant(&owner).await;
    let p = t.draft(org, &owner, 1).await;
    t.publish(org, p, &owner).await;
    let e = t.join(org, p, &runner).await;
    let path = format!("{}/upload", ep(org, p, e));
    let mut invalid = runner.clone();
    invalid.csrf = "wrong".into();
    assert_eq!(t.raw(&path, VALID, &invalid).await.0, StatusCode::FORBIDDEN);
    let oversized = vec![0; truhabit_evidence::upload::MAX_BYTES + 1];
    assert_eq!(
        t.raw(&path, &oversized, &runner).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let fit = include_bytes!("../../../web/public/prototype/multi-session.fit");
    assert_eq!(t.raw(&path, fit, &runner).await.0, StatusCode::BAD_REQUEST);
    let chosen = format!("{path}?session=0");
    let upload = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        t.raw(&chosen, fit, &runner),
    )
    .await
    .unwrap();
    assert_eq!(upload.0, StatusCode::OK, "{}", upload.1);
    assert_eq!(upload.1["decision"], "ACCEPTED");
    let attempts: i32 = sqlx::query_scalar("SELECT count FROM rate_limits WHERE key_hash=$1")
        .bind(truhabit_api::crypto::digest(format!(
            "business-upload:{}",
            runner.user
        )))
        .fetch_one(&t.pool)
        .await
        .unwrap();
    assert_eq!(
        attempts, 2,
        "Rejected multi-session selection must consume a persisted attempt; CSRF/body-size rejection must not consume a parser attempt"
    );
    assert_eq!(
        t.req(
            "POST",
            &format!("{}/claim", ep(org, p, e)),
            json!({}),
            &runner
        )
        .await
        .0,
        StatusCode::OK
    );
    sqlx::query("UPDATE rate_limits SET count=21 WHERE key_hash=$1")
        .bind(truhabit_api::crypto::digest(format!(
            "business-upload:{}",
            runner.user
        )))
        .execute(&t.pool)
        .await
        .unwrap();
    let retry = t.raw(&chosen, fit, &runner).await;
    assert_eq!(retry.0, StatusCode::OK);
    assert_eq!(retry.1["duplicate"], true);
    assert_eq!(retry.1["id"], upload.1["id"]);
    assert_eq!(
        t.raw(&format!("{path}?session=1"), fit, &runner).await.1["error"],
        "Příliš mnoho pokusů. Zkuste to prosím později."
    );
    let (rows, session): (i64, Option<i32>) = sqlx::query_as(
        "SELECT count(*),max(session_index) FROM company_uploads WHERE enrollment_id=$1",
    )
    .bind(e)
    .fetch_one(&t.pool)
    .await
    .unwrap();
    t.finish().await;
    assert_eq!((rows, session), (1, Some(0)));
}
