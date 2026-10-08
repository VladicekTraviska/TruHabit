use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{Duration, Timelike, Utc};
use ed25519_dalek::{Signer, SigningKey};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use truhabit_api::{AppState, MIGRATOR, config::Config, crypto, mail, router};
use uuid::Uuid;

const PASSWORD: &str = "a long unique test passphrase 123";

fn company_program(id: Uuid) -> Value {
    json!({"id":id,"title":"Team running","target_m":3000,"currency":"CZK","reward_minor":40000,"max_participants":25})
}

#[tokio::test]
async fn goal_currencies_preserve_legacy_retries_and_reject_reinterpretation() {
    let t = TestApp::new(false).await;
    let s = t.account("currency@example.com").await;
    let key = Uuid::new_v4().to_string();
    let mut old_input =
        json!({"target_m":3000,"pledge_cents":1000,"starts_at":Utc::now()+Duration::hours(1)});
    let (status, old, _) = t
        .request(
            "POST",
            "/api/goals",
            old_input.clone(),
            Some(&s),
            Some(&key),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(old["currency"], "USD");
    let hash: String =
        sqlx::query_scalar("SELECT request_hash FROM idempotency_keys WHERE user_id=$1 AND key=$2")
            .bind(s.user)
            .bind(Uuid::parse_str(&key).unwrap())
            .fetch_one(&t.pool)
            .await
            .unwrap();
    // Serialization is in the historical struct order; adding explicit USD must produce this same hash.
    let historical = serde_json::to_string(&truhabit_api::goals::GoalInput {
        target_m: 3000,
        pledge_cents: 1000,
        starts_at: serde_json::from_value(old_input["starts_at"].clone()).unwrap(),
        currency: "USD".into(),
    })
    .unwrap();
    assert!(!historical.contains("currency"));
    assert_eq!(hash, crypto::digest(historical));
    old_input["currency"] = json!("USD");
    let (_, repeated, _) = t
        .request(
            "POST",
            "/api/goals",
            old_input.clone(),
            Some(&s),
            Some(&key),
        )
        .await;
    assert_eq!(old["id"], repeated["id"]);
    old_input["currency"] = json!("CZK");
    old_input["pledge_cents"] = json!(10000);
    old_input["version"] = json!(1);
    assert_eq!(
        t.request(
            "PATCH",
            &format!("/api/goals/{}", old["id"].as_str().unwrap()),
            old_input.clone(),
            Some(&s),
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    old_input.as_object_mut().unwrap().remove("version");
    let (status, new, _) = t
        .request(
            "POST",
            "/api/goals",
            old_input.clone(),
            Some(&s),
            Some(&Uuid::new_v4().to_string()),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(new["currency"], "CZK");
    assert_eq!(new["pledge_cents"], 10000);
    old_input["currency"] = json!("EUR");
    assert_eq!(
        t.request(
            "POST",
            "/api/goals",
            old_input,
            Some(&s),
            Some(&Uuid::new_v4().to_string())
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    t.finish().await;
}

#[tokio::test]
async fn organization_creation_is_atomic_idempotent_and_cannot_claim_another_owner() {
    let t = TestApp::new(false).await;
    let a = t.account("owner@example.com").await;
    let b = t.account("outsider@example.com").await;
    let id = Uuid::new_v4();
    let input = json!({"id":id,"name":"Acme"});
    let (first, second) = tokio::join!(
        t.request("POST", "/api/organizations", input.clone(), Some(&a), None),
        t.request("POST", "/api/organizations", input.clone(), Some(&a), None)
    );
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(second.0, StatusCode::OK);
    assert_eq!(first.1["id"], second.1["id"]);
    assert_eq!(first.1["role"], "OWNER");
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM organization_events WHERE organization_id=$1")
            .bind(id)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        t.request(
            "POST",
            "/api/organizations",
            json!({"id":id,"name":"Changed"}),
            Some(&a),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        t.request("POST", "/api/organizations", input.clone(), Some(&b), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let mut forged = input;
    forged["id"] = json!(Uuid::new_v4());
    forged["owner_id"] = json!(b.user);
    assert_eq!(
        t.request("POST", "/api/organizations", forged, Some(&a), None)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    t.finish().await;
}

#[tokio::test]
async fn organization_authorization_separates_tenants_roles_and_personal_goals() {
    let t = TestApp::new(false).await;
    let a = t.account("a@example.com").await;
    let b = t.account("b@example.com").await;
    let id = Uuid::new_v4();
    let path = format!("/api/organizations/{id}");
    assert_eq!(
        t.request(
            "POST",
            "/api/organizations",
            json!({"id":id,"name":"Private team"}),
            Some(&a),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let program = company_program(Uuid::new_v4());
    assert_eq!(
        t.request(
            "POST",
            &format!("{path}/programs"),
            program.clone(),
            Some(&a),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    for method in ["GET", "PATCH", "DELETE"] {
        let body = match method {
            "PATCH" => json!({"name":"Attack","version":1}),
            "DELETE" => json!({"password":PASSWORD,"confirmation":"Private team"}),
            _ => Value::Null,
        };
        assert_eq!(
            t.request(method, &path, body, Some(&b), None).await.0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        t.request("GET", &path, Value::Null, None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        t.request("GET", "/api/organizations", Value::Null, Some(&b), None)
            .await
            .1["organizations"],
        json!([])
    );
    let mut bad_csrf = a.clone();
    bad_csrf.csrf = "wrong".into();
    assert_eq!(
        t.request(
            "POST",
            &format!("{path}/programs"),
            program.clone(),
            Some(&bad_csrf),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    sqlx::query(
        "INSERT INTO organization_members(organization_id,user_id,role) VALUES($1,$2,'MEMBER')",
    )
    .bind(id)
    .bind(b.user)
    .execute(&t.pool)
    .await
    .unwrap();
    let detail = t.request("GET", &path, Value::Null, Some(&b), None).await;
    assert_eq!(detail.0, StatusCode::OK);
    assert_eq!(detail.1["programs"], json!([]));
    assert_eq!(detail.1["events"], json!([]));
    assert_eq!(
        t.request("POST", &format!("{path}/programs"), program, Some(&b), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let goal = t.goal(&a, &Uuid::new_v4().to_string()).await;
    assert_eq!(
        t.request(
            "GET",
            &format!("/api/goals/{}", goal["id"].as_str().unwrap()),
            Value::Null,
            Some(&b),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    sqlx::query(
        "UPDATE organization_members SET role='ADMIN' WHERE organization_id=$1 AND user_id=$2",
    )
    .bind(id)
    .bind(b.user)
    .execute(&t.pool)
    .await
    .unwrap();
    assert_eq!(
        t.request("GET", &path, Value::Null, Some(&b), None).await.1["programs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        t.request(
            "DELETE",
            &path,
            json!({"password":PASSWORD,"confirmation":"Private team"}),
            Some(&b),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.request(
            "GET",
            &format!("/api/goals/{}", goal["id"].as_str().unwrap()),
            Value::Null,
            Some(&b),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    t.finish().await;
}

#[tokio::test]
async fn programs_reject_cross_tenant_ids_stale_writes_and_over_budget_plans() {
    let t = TestApp::new(false).await;
    let a = t.account("programs@example.com").await;
    let org = Uuid::new_v4();
    let other = Uuid::new_v4();
    for id in [org, other] {
        assert_eq!(
            t.request(
                "POST",
                "/api/organizations",
                json!({"id":id,"name":"My team"}),
                Some(&a),
                None
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    let id = Uuid::new_v4();
    let input = company_program(id);
    let path = format!("/api/organizations/{org}/programs");
    let (first, second) = tokio::join!(
        t.request("POST", &path, input.clone(), Some(&a), None),
        t.request("POST", &path, input.clone(), Some(&a), None)
    );
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(second.0, StatusCode::OK);
    let mut edit = input.clone();
    edit.as_object_mut().unwrap().remove("id");
    edit["version"] = json!(1);
    edit["title"] = json!("Updated team");
    assert_eq!(
        t.request(
            "PATCH",
            &format!("/api/organizations/{other}/programs/{id}"),
            edit.clone(),
            Some(&a),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let route = format!("{path}/{id}");
    let (one, two) = tokio::join!(
        t.request("PATCH", &route, edit.clone(), Some(&a), None),
        t.request("PATCH", &route, edit.clone(), Some(&a), None)
    );
    assert!([one.0, two.0].contains(&StatusCode::OK));
    assert!([one.0, two.0].contains(&StatusCode::CONFLICT));
    let archive = format!("{route}/archive");
    assert_eq!(
        t.request("POST", &archive, json!({"version":1}), Some(&a), None)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        t.request("POST", &archive, json!({"version":2}), Some(&a), None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.request("POST", &archive, json!({"version":2}), Some(&a), None)
            .await
            .1["version"],
        3
    );
    edit["version"] = json!(3);
    assert_eq!(
        t.request("PATCH", &route, edit, Some(&a), None).await.0,
        StatusCode::CONFLICT
    );
    for (field, value) in [
        ("currency", json!("USD")),
        ("reward_minor", json!(i64::MAX)),
        ("max_participants", json!(10000)),
        ("target_m", json!(0)),
    ] {
        let mut bad = company_program(Uuid::new_v4());
        bad[field] = value;
        assert_eq!(
            t.request("POST", &path, bad, Some(&a), None).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    let (_, detail, _) = t
        .request(
            "GET",
            &format!("/api/organizations/{org}"),
            Value::Null,
            Some(&a),
            None,
        )
        .await;
    assert_eq!(detail["programs"].as_array().unwrap().len(), 1);
    assert_eq!(detail["events"].as_array().unwrap().len(), 4);
    t.finish().await;
}

#[tokio::test]
async fn company_deletion_requires_owner_reauthentication_and_preserves_personal_data() {
    let t = TestApp::new(false).await;
    let a = t.account("delete-company@example.com").await;
    let b = t.account("other-company@example.com").await;
    let goal = t.goal(&a, &Uuid::new_v4().to_string()).await;
    let org = Uuid::new_v4();
    let path = format!("/api/organizations/{org}");
    t.request(
        "POST",
        "/api/organizations",
        json!({"id":org,"name":"Delete me"}),
        Some(&a),
        None,
    )
    .await;
    t.request(
        "POST",
        &format!("{path}/programs"),
        company_program(Uuid::new_v4()),
        Some(&a),
        None,
    )
    .await;
    let exported = t
        .request("GET", "/api/account/export", Value::Null, Some(&a), None)
        .await
        .1;
    assert_eq!(exported["organizations"].as_array().unwrap().len(), 1);
    assert_eq!(exported["organization_events"].as_array().unwrap().len(), 2);
    assert!(!exported.to_string().contains("creation_hash"));
    assert_eq!(
        t.request(
            "DELETE",
            "/api/account",
            json!({"password":PASSWORD,"confirmation":"DELETE"}),
            Some(&a),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        t.request(
            "DELETE",
            &path,
            json!({"password":"wrong","confirmation":"Delete me"}),
            Some(&a),
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        t.request(
            "DELETE",
            &path,
            json!({"password":PASSWORD,"confirmation":"wrong"}),
            Some(&a),
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    sqlx::query(
        "INSERT INTO organization_members(organization_id,user_id,role) VALUES($1,$2,'MEMBER')",
    )
    .bind(org)
    .bind(b.user)
    .execute(&t.pool)
    .await
    .unwrap();
    assert_eq!(
        t.request(
            "DELETE",
            &path,
            json!({"password":PASSWORD,"confirmation":"Delete me"}),
            Some(&a),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    sqlx::query("DELETE FROM organization_members WHERE organization_id=$1 AND user_id=$2")
        .bind(org)
        .bind(b.user)
        .execute(&t.pool)
        .await
        .unwrap();
    assert_eq!(
        t.request(
            "DELETE",
            &path,
            json!({"password":PASSWORD,"confirmation":"Delete me"}),
            Some(&a),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        t.request(
            "GET",
            &format!("/api/goals/{}", goal["id"].as_str().unwrap()),
            Value::Null,
            Some(&a),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM company_programs WHERE organization_id=$1")
            .bind(org)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        t.request(
            "DELETE",
            "/api/account",
            json!({"password":PASSWORD,"confirmation":"DELETE"}),
            Some(&a),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        t.request("GET", "/api/auth/session", Value::Null, Some(&b), None)
            .await
            .0,
        StatusCode::OK
    );
    t.finish().await;
}
struct TestApp {
    app: Router,
    state: AppState,
    pool: PgPool,
    admin: PgPool,
    schema: String,
}

#[tokio::test]
async fn closed_devnet_recovery_snapshot_refresh_is_idempotent_and_exports_transaction_lineage() {
    type PersistedCommand = (Uuid, String, String, Value, Option<String>, Option<String>);
    let t = TestApp::new(false).await;
    let owner = t.account("recovery-history@example.com").await;
    let outsider = t.account("recovery-outsider@example.com").await;
    let id = t.prototype(&owner, 5_000_000, "REPLAY").await;
    // This is a disposable persisted recovery snapshot, not an RPC or
    // ACTIVE -> EXPIRED test. Node proof tests and real Devnet E2E cover that
    // transition; these assertions cover repeat refresh and retained lineage.
    sqlx::query("UPDATE prototype_challenges SET network='DEVNET',state='EXPIRED',closed_at=now(),chain=$1 WHERE id=$2")
        .bind(json!({"owner":"test-wallet-public-key","terms_hash":"immutable-test-terms"}))
        .bind(id).execute(&t.pool).await.unwrap();
    let deposit = Uuid::new_v4();
    let superseded = Uuid::new_v4();
    let recovered = Uuid::new_v4();
    let deposit_signature = "D".repeat(88);
    let superseded_signature = "S".repeat(88);
    let timeout_signature = "T".repeat(88);
    for (command, action, status, signature, payload, signed) in [
        (
            deposit,
            "DEPOSIT",
            "CONFIRMED",
            deposit_signature.clone(),
            json!({"message":"private-deposit-message"}),
            Some("private-deposit-envelope"),
        ),
        (
            superseded,
            "SUCCESS",
            "FAILED",
            superseded_signature.clone(),
            json!({"message":"private-original-message"}),
            Some("private-original-envelope"),
        ),
        (
            recovered,
            "TIMEOUT",
            "CONFIRMED",
            timeout_signature.clone(),
            json!({"recovered":true,"proof":{"action":"TIMEOUT","signature":timeout_signature,"amount_units":5_000_000}}),
            None,
        ),
    ] {
        sqlx::query("INSERT INTO prototype_commands(id,challenge_id,action,status,signature,payload,signed_transaction) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(command).bind(id).bind(action).bind(status).bind(signature).bind(payload).bind(signed)
            .execute(&t.pool).await.unwrap();
    }
    for (kind, detail) in [
        (
            "transfer_confirmed",
            json!({"action":"DEPOSIT","signature":deposit_signature,"slot":120}),
        ),
        (
            "superseded_by_verified_settlement",
            json!({"command_id":superseded,"original_action":"SUCCESS","original_signature":superseded_signature,"action":"TIMEOUT","signature":timeout_signature}),
        ),
        (
            "transfer_confirmed",
            json!({"action":"TIMEOUT","signature":timeout_signature,"slot":124,"source":"external_chain_recovery"}),
        ),
    ] {
        sqlx::query(
            "INSERT INTO prototype_events(challenge_id,actor_id,kind,detail) VALUES($1,$2,$3,$4)",
        )
        .bind(id)
        .bind(owner.user)
        .bind(kind)
        .bind(detail)
        .execute(&t.pool)
        .await
        .unwrap();
    }
    let before: Vec<PersistedCommand> = sqlx::query_as(
        "SELECT id,action,status,payload,signed_transaction,signature FROM prototype_commands WHERE challenge_id=$1 ORDER BY id",
    ).bind(id).fetch_all(&t.pool).await.unwrap();
    let events_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM prototype_events WHERE challenge_id=$1")
            .bind(id)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    let path = format!("/api/prototype/challenges/{id}/refresh");
    let (first, second) = tokio::join!(
        t.request("POST", &path, json!({}), Some(&owner), None),
        t.request("POST", &path, json!({}), Some(&owner), None),
    );
    for response in [
        first,
        second,
        t.request("POST", &path, json!({}), Some(&owner), None)
            .await,
    ] {
        assert_eq!(response.0, StatusCode::OK);
        assert_eq!(response.1["status"], "NO_PENDING_TRANSACTION");
    }
    assert_eq!(
        t.request("POST", &path, json!({}), Some(&outsider), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let after: Vec<PersistedCommand> = sqlx::query_as(
        "SELECT id,action,status,payload,signed_transaction,signature FROM prototype_commands WHERE challenge_id=$1 ORDER BY id",
    ).bind(id).fetch_all(&t.pool).await.unwrap();
    assert_eq!(
        before, after,
        "Refresh must preserve original envelopes and recovered command identity"
    );
    let events_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM prototype_events WHERE challenge_id=$1")
            .bind(id)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(
        events_before, events_after,
        "A closed refresh must not duplicate financial history"
    );
    let local_movements: i64 =
        sqlx::query_scalar("SELECT count(*) FROM prototype_local_movements WHERE challenge_id=$1")
            .bind(id)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(
        local_movements, 0,
        "Devnet recovery must not invent simulation credits"
    );
    let (_, detail, _) = t
        .request(
            "GET",
            &format!("/api/prototype/challenges/{id}"),
            Value::Null,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(detail["challenge"]["state"], "EXPIRED");
    assert_eq!(detail["commands"].as_array().unwrap().len(), 3);
    assert_eq!(
        detail["events"].as_array().unwrap().len() as i64,
        events_before
    );
    let (status, export, _) = t
        .request(
            "GET",
            "/api/account/export",
            Value::Null,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let exported = export.to_string();
    assert!(exported.contains(&deposit_signature));
    assert!(exported.contains(&superseded_signature));
    assert!(exported.contains(&timeout_signature));
    assert!(exported.contains("superseded_by_verified_settlement"));
    for private in [
        "private-deposit-message",
        "private-original-message",
        "private-deposit-envelope",
        "private-original-envelope",
    ] {
        assert!(!exported.contains(private));
        assert!(!detail.to_string().contains(private));
    }
    t.finish().await;
}
#[derive(Clone)]
struct Session {
    cookie: String,
    csrf: String,
    user: Uuid,
}
impl TestApp {
    /// Invoke the actual handler with a deterministic server receipt fixture.
    /// This tests API policy at an exact instant, not physical network timing.
    async fn recorded_upload(
        &self,
        s: &Session,
        id: Uuid,
        received_at: chrono::DateTime<Utc>,
        bytes: Vec<u8>,
    ) -> (StatusCode, Value) {
        use axum::extract::{FromRequestParts, Path, Query, State};
        let request = Request::builder()
            .header("cookie", &s.cookie)
            .header("x-csrf-token", &s.csrf)
            .body(Body::from(bytes))
            .unwrap();
        let (mut parts, body) = request.into_parts();
        let auth = truhabit_api::auth::Auth::from_request_parts(&mut parts, &self.state)
            .await
            .unwrap();
        let headers = parts.headers.clone();
        let mut tx = self.pool.begin().await.unwrap();
        truhabit_api::auth::lock_user(&mut tx, s.user)
            .await
            .unwrap();
        truhabit_api::prototype::owned(&mut tx, id, s.user)
            .await
            .unwrap();
        let mut recorded = truhabit_api::prototype::RecordedUpload::read_admitted(
            Request::from_parts(parts, body),
            &self.state,
            tx,
        )
        .await
        .unwrap();
        // Only this test helper supplies a deterministic policy boundary.
        // HTTP uses the extractor's actual server receipt and admitted locks.
        recorded.received_at = received_at;
        match truhabit_api::prototype::upload(
            auth,
            State(self.state.clone()),
            Path(id),
            Query(truhabit_api::prototype::UploadQuery { session: None }),
            headers,
            recorded,
        )
        .await
        {
            Ok(axum::Json(value)) => (StatusCode::OK, value),
            Err(error) => (
                error.status,
                json!({"code":error.code,"error":error.message}),
            ),
        }
    }
    async fn prototype(&self, s: &Session, amount: i64, profile: &str) -> Uuid {
        let id = Uuid::new_v4();
        let input = json!({"id":id,"title":"Test run","target_m":3000,"amount_units":amount,"network":"LOCAL","profile":profile,"starts_at":Utc::now()+Duration::minutes(10)});
        let (status, body, _) = self
            .request("POST", "/api/prototype/challenges", input, Some(s), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        id
    }
    async fn prototype_action(
        &self,
        s: &Session,
        id: Uuid,
        action: &str,
    ) -> (StatusCode, Value, String) {
        self.request(
            "POST",
            &format!("/api/prototype/challenges/{id}/local-action"),
            json!({"action":action}),
            Some(s),
            None,
        )
        .await
    }
    async fn prototype_grant(&self, s: &Session) {
        let (status, body, _) = self
            .request(
                "POST",
                "/api/prototype/local/grant",
                json!({"id":Uuid::new_v4()}),
                Some(s),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    async fn raw(
        &self,
        method: &str,
        path: &str,
        bytes: Vec<u8>,
        session: Option<&Session>,
    ) -> (StatusCode, Vec<u8>) {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:8787")
            .header("origin", "http://127.0.0.1:8787")
            .header("x-truhabit-request", "web")
            .header("content-type", "application/octet-stream");
        if let Some(s) = session {
            builder = builder
                .header("cookie", &s.cookie)
                .header("x-csrf-token", &s.csrf);
        }
        let mut request = builder.body(Body::from(bytes)).unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:6000".parse::<std::net::SocketAddr>().unwrap(),
        ));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        (status, body)
    }
    async fn new(mail_enabled: bool) -> Self {
        Self::with_pool_size(mail_enabled, 4).await
    }
    async fn with_pool_size(mail_enabled: bool, connections: u32) -> Self {
        let url=std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required. Run scripts/verify.ps1; PostgreSQL tests are never silently skipped.");
        let parsed = url::Url::parse(&url).unwrap();
        assert!(
            parsed.path().ends_with("_test"),
            "Tests refuse to use a non-test database"
        );
        let admin = PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .unwrap();
        let schema = format!("test_{}", Uuid::new_v4().simple());
        // Identifier consists only of our fixed prefix and a generated UUID, never user input.
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .unwrap();
        let search_path = schema.clone();
        let pool = PgPoolOptions::new()
            .max_connections(connections)
            .after_connect(move |c, _| {
                let query = search_path.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('search_path',$1,false)")
                        .bind(&query)
                        .execute(&mut *c)
                        .await?;
                    sqlx::query("SELECT set_config('application_name',$1,false)")
                        .bind(&query)
                        .execute(&mut *c)
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
            mail_key: mail_enabled.then_some([42; 32]),
            smtp_host: mail_enabled.then(|| "smtp.example.invalid".into()),
            smtp_user: mail_enabled.then(|| "test".into()),
            smtp_password: mail_enabled.then(|| "test-only".into()),
            mail_from: mail_enabled.then(|| "TruHabit <support@example.invalid>".into()),
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
    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Value,
        session: Option<&Session>,
        key: Option<&str>,
    ) -> (StatusCode, Value, String) {
        let origin = url::Url::parse(&self.state.config.origin).unwrap();
        let host = &origin[url::Position::BeforeHost..url::Position::AfterPort];
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("host", host)
            .header("origin", &self.state.config.origin)
            .header("x-truhabit-request", "web")
            .header("content-type", "application/json");
        if let Some(s) = session {
            builder = builder
                .header("cookie", &s.cookie)
                .header("x-csrf-token", &s.csrf);
        }
        if let Some(k) = key {
            builder = builder.header("idempotency-key", k);
        }
        let mut request = builder.body(Body::from(body.to_string())).unwrap();
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
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| json!({"text":String::from_utf8_lossy(&bytes)})),
            cookie,
        )
    }
    async fn account(&self, email: &str) -> Session {
        let (status, body, _) = self
            .request(
                "POST",
                "/api/auth/register",
                json!({"email":email,"password":PASSWORD,"display_name":"Běžec"}),
                None,
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        self.login(email, PASSWORD).await
    }
    async fn login(&self, email: &str, password: &str) -> Session {
        let (status, body, cookie) = self
            .request(
                "POST",
                "/api/auth/login",
                json!({"email":email,"password":password}),
                None,
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        let mut s = Session {
            cookie: cookie.split(';').next().unwrap().into(),
            csrf: body["csrf_token"].as_str().unwrap().into(),
            user: Uuid::nil(),
        };
        let (_, body, _) = self
            .request("GET", "/api/auth/session", Value::Null, Some(&s), None)
            .await;
        s.user = Uuid::parse_str(body["user"]["id"].as_str().unwrap()).unwrap();
        s
    }
    async fn goal(&self, s: &Session, key: &str) -> Value {
        let body =
            json!({"target_m":5000,"pledge_cents":1000,"starts_at":Utc::now()+Duration::hours(1)});
        let (status, result, _) = self
            .request("POST", "/api/goals", body, Some(s), Some(key))
            .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        result
    }
    async fn finish(self) {
        self.pool.close().await;
        assert!(self.schema.starts_with("test_") && self.schema.len() == 37);
        assert!(self.schema[5..].bytes().all(|b| b.is_ascii_hexdigit()));
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

#[tokio::test]
async fn prototype_refund_is_atomic_and_export_retains_ledger_and_private_file_controls() {
    let t = TestApp::new(false).await;
    let s = t.account("prototype@example.com").await;
    t.prototype_grant(&s).await;
    let id = t.prototype(&s, 5_000_000, "REPLAY").await;
    assert_eq!(
        t.prototype_action(&s, id, "DEPOSIT").await.0,
        StatusCode::OK
    );
    assert_eq!(
        t.prototype_action(&s, id, "SUCCESS").await.0,
        StatusCode::CONFLICT
    );
    let source = include_bytes!("../../../web/public/prototype/valid-run.gpx");
    let upload_path = format!("/api/prototype/challenges/{id}/upload");
    let (status, bytes) = t.raw("POST", &upload_path, source.to_vec(), Some(&s)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let upload: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(upload["decision"], "ACCEPTED");
    let (_, duplicate) = t.raw("POST", &upload_path, source.to_vec(), Some(&s)).await;
    let duplicate: Value = serde_json::from_slice(&duplicate).unwrap();
    assert_eq!(duplicate["id"], upload["id"]);
    assert_eq!(duplicate["duplicate"], true);
    let file_path = format!(
        "/api/prototype/challenges/{id}/uploads/{}/source",
        upload["id"].as_str().unwrap()
    );
    assert_eq!(t.raw("GET", &file_path, vec![], Some(&s)).await.1, source);
    assert_eq!(
        t.request("DELETE", &file_path, Value::Null, Some(&s), None)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        t.request(
            "DELETE",
            "/api/account",
            json!({"password":PASSWORD,"confirmation":"DELETE"}),
            Some(&s),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (a, b) = tokio::join!(
        t.prototype_action(&s, id, "SUCCESS"),
        t.prototype_action(&s, id, "SUCCESS")
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    let (_, balance, _) = t
        .request(
            "GET",
            "/api/prototype/local/balance",
            Value::Null,
            Some(&s),
            None,
        )
        .await;
    assert_eq!(balance["available"], 100_000_000);
    assert_eq!(balance["locked"], 0);
    assert_eq!(balance["forfeited"], 0);
    let (_, export, _) = t
        .request("GET", "/api/account/export", Value::Null, Some(&s), None)
        .await;
    assert_eq!(export["prototype_movements"].as_array().unwrap().len(), 3);
    assert_eq!(export["prototype_challenges"][0]["state"], "REFUNDED");
    assert!(!export.to_string().contains("<gpx"));
    assert_eq!(
        t.request("DELETE", &file_path, Value::Null, Some(&s), None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.request("DELETE", &file_path, Value::Null, Some(&s), None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.raw("GET", &file_path, vec![], Some(&s)).await.0,
        StatusCode::CONFLICT
    );
    let (_, detail, _) = t
        .request(
            "GET",
            &format!("/api/prototype/challenges/{id}"),
            Value::Null,
            Some(&s),
            None,
        )
        .await;
    assert_eq!(detail["uploads"].as_array().unwrap().len(), 1);
    assert!(!detail["uploads"][0]["content_deleted_at"].is_null());
    assert_eq!(
        t.request(
            "DELETE",
            "/api/account",
            json!({"password":PASSWORD,"confirmation":"DELETE"}),
            Some(&s),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    t.finish().await;
}

#[tokio::test]
async fn prototype_recorded_receipt_accepts_exact_deadline_and_preserves_it_after_processing_delay()
{
    let t = TestApp::new(false).await;
    let runner = t.account("receipt-boundary@example.com").await;
    t.prototype_grant(&runner).await;
    let id = t.prototype(&runner, 5_000_000, "REPLAY").await;
    t.prototype_action(&runner, id, "DEPOSIT").await;
    let deadline = (Utc::now() - Duration::minutes(1))
        .with_nanosecond(0)
        .unwrap();
    sqlx::query("UPDATE prototype_challenges SET starts_at=$1,ends_at=$2,upload_deadline=$3,refund_after=$4 WHERE id=$5")
        .bind(deadline-Duration::hours(2)).bind(deadline-Duration::hours(1))
        .bind(deadline).bind(deadline+Duration::minutes(10)).bind(id)
        .execute(&t.pool).await.unwrap();
    let sample = include_bytes!("../../../web/public/prototype/valid-run.gpx").to_vec();
    // The timestamp is explicitly a server receipt fixture. Handler execution
    // is already a minute past cutoff; no real HTTP arrival at an exact µs is claimed.
    let (status, accepted) = t
        .recorded_upload(&runner, id, deadline, sample.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(accepted["decision"], "ACCEPTED");
    let stored: chrono::DateTime<Utc> =
        sqlx::query_scalar("SELECT received_at FROM prototype_uploads WHERE challenge_id=$1")
            .bind(id)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(stored, deadline);
    let processed: chrono::DateTime<Utc> = sqlx::query_scalar("SELECT created_at FROM prototype_events WHERE challenge_id=$1 AND kind='activity_uploaded'")
        .bind(id).fetch_one(&t.pool).await.unwrap();
    assert!(processed > deadline);
    let (status, duplicate) = t
        .recorded_upload(
            &runner,
            id,
            deadline + Duration::microseconds(1),
            sample.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(duplicate["id"], accepted["id"]);
    assert_eq!(duplicate["duplicate"], true);
    let mut new_bytes = sample;
    new_bytes.push(b' ');
    let (status, rejected) = t
        .recorded_upload(&runner, id, deadline + Duration::microseconds(1), new_bytes)
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(rejected["error"], "UPLOAD_WINDOW_CLOSED");
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM prototype_uploads WHERE challenge_id=$1")
            .bind(id)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    let (_, detail, _) = t
        .request(
            "GET",
            &format!("/api/prototype/challenges/{id}"),
            Value::Null,
            Some(&runner),
            None,
        )
        .await;
    assert_eq!(detail["challenge"]["state"], "ACTIVE");
    assert_eq!(detail["challenge"]["assessment"], "MET");
    t.finish().await;
}

#[tokio::test]
async fn prototype_complete_body_waiting_on_rate_limiter_cannot_be_overtaken_by_failure() {
    let t = TestApp::new(false).await;
    let runner = t.account("receipt-race@example.com").await;
    let operator = t.account("receipt-race-operator@example.com").await;
    sqlx::query("INSERT INTO prototype_operators(user_id) VALUES($1)")
        .bind(operator.user)
        .execute(&t.pool)
        .await
        .unwrap();
    t.prototype_grant(&runner).await;
    let id = t.prototype(&runner, 5_000_000, "REPLAY").await;
    t.prototype_action(&runner, id, "DEPOSIT").await;
    let deadline = (Utc::now() + Duration::seconds(5))
        .with_nanosecond(0)
        .unwrap();
    sqlx::query("UPDATE prototype_challenges SET starts_at=$1,ends_at=$2,upload_deadline=$3,refund_after=$4 WHERE id=$5")
        .bind(deadline-Duration::hours(2)).bind(deadline-Duration::hours(1))
        .bind(deadline).bind(deadline+Duration::minutes(5)).bind(id)
        .execute(&t.pool).await.unwrap();
    let rate_key = crypto::digest(format!("prototype-upload:{}", runner.user));
    sqlx::query(
        "INSERT INTO rate_limits(key_hash,count,reset_at) VALUES($1,0,now()+interval '1 hour')",
    )
    .bind(&rate_key)
    .execute(&t.pool)
    .await
    .unwrap();
    let mut limiter = t.pool.begin().await.unwrap();
    sqlx::query("SELECT count FROM rate_limits WHERE key_hash=$1 FOR UPDATE")
        .bind(rate_key)
        .fetch_one(&mut *limiter)
        .await
        .unwrap();
    let path = format!("/api/prototype/challenges/{id}/upload");
    let mut upload = Box::pin(t.raw(
        "POST",
        &path,
        include_bytes!("../../../web/public/prototype/valid-run.gpx").to_vec(),
        Some(&runner),
    ));
    // An observed blocked limiter query proves the body extractor completed.
    // The test schedules an interval around cutoff; it does not claim a packet
    // arrived on a physically exact boundary.
    loop {
        tokio::select! {
            result = &mut upload => panic!("Upload unexpectedly finished before limiter release: {}", result.0),
            _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
        }
        let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=$1 AND wait_event_type='Lock' AND query LIKE 'INSERT INTO rate_limits%')")
            .bind(&t.schema).fetch_one(&t.admin).await.unwrap();
        if blocked {
            break;
        }
        assert!(
            Utc::now() < deadline,
            "Could not establish the blocked intake before cutoff"
        );
    }
    assert!(Utc::now() < deadline);
    while Utc::now() <= deadline {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let mut failure = Box::pin(t.prototype_action(&operator, id, "FAILURE"));
    // The corrected lock order must keep FAILURE behind this intake. Poll it
    // while releasing the deliberate limiter block, then await both results.
    let early_failure = tokio::select! {
        result = &mut failure => Some(result),
        _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => None,
    };
    limiter.rollback().await.unwrap();
    let upload_response = upload.await;
    let failure_response = match early_failure {
        Some(value) => value,
        None => (&mut failure).await,
    };
    let assessment: String =
        sqlx::query_scalar("SELECT assessment FROM prototype_challenges WHERE id=$1")
            .bind(id)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    drop(failure);
    t.finish().await;
    assert_eq!(
        failure_response.0,
        StatusCode::CONFLICT,
        "Failure overtook the complete pre-cutoff body: {}",
        failure_response.1
    );
    assert_eq!(
        upload_response.0,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&upload_response.1)
    );
    assert_eq!(assessment, "MET");
}

#[tokio::test]
async fn prototype_upload_savepoint_counts_invalid_attempts_and_needs_only_one_pool_connection() {
    // A limiter taking a second lease while holding user/challenge locks would
    // stall this single-connection pool. The production path uses one lease.
    let t = TestApp::with_pool_size(false, 1).await;
    let runner = t.account("receipt-rate-limit@example.com").await;
    t.prototype_grant(&runner).await;
    let id = t.prototype(&runner, 5_000_000, "REPLAY").await;
    t.prototype_action(&runner, id, "DEPOSIT").await;
    let path = format!("/api/prototype/challenges/{id}/upload");
    for _ in 0..20 {
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            t.raw("POST", &path, b"<gpx broken".to_vec(), Some(&runner)),
        )
        .await
        .expect("Upload must not request a second pool connection");
        assert_eq!(response.0, StatusCode::BAD_REQUEST);
    }
    let response = t
        .raw(
            "POST",
            &path,
            include_bytes!("../../../web/public/prototype/valid-run.gpx").to_vec(),
            Some(&runner),
        )
        .await;
    assert_eq!(response.0, StatusCode::TOO_MANY_REQUESTS);
    let limit: Value = serde_json::from_slice(&response.1).unwrap();
    assert_eq!(limit["code"], "RATE_LIMITED");
    let rate_key = crypto::digest(format!("prototype-upload:{}", runner.user));
    let count: i32 = sqlx::query_scalar("SELECT count FROM rate_limits WHERE key_hash=$1")
        .bind(&rate_key)
        .fetch_one(&t.pool)
        .await
        .unwrap();
    assert_eq!(
        count, 21,
        "Parser rollback must not roll back the rate counter"
    );
    let uploads: i64 =
        sqlx::query_scalar("SELECT count(*) FROM prototype_uploads WHERE challenge_id=$1")
            .bind(id)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(
        uploads, 0,
        "Invalid parser work must leave no file or verdict"
    );
    sqlx::query("UPDATE rate_limits SET reset_at=now()-interval '1 second' WHERE key_hash=$1")
        .bind(&rate_key)
        .execute(&t.pool)
        .await
        .unwrap();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        t.raw(
            "POST",
            &path,
            include_bytes!("../../../web/public/prototype/valid-run.gpx").to_vec(),
            Some(&runner),
        ),
    )
    .await
    .expect("One pool lease must suffice for accepted parser work too");
    assert_eq!(response.0, StatusCode::OK);
    let count: i32 = sqlx::query_scalar("SELECT count FROM rate_limits WHERE key_hash=$1")
        .bind(rate_key)
        .fetch_one(&t.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let (_, balance, _) = t
        .request(
            "GET",
            "/api/prototype/local/balance",
            Value::Null,
            Some(&runner),
            None,
        )
        .await;
    assert_eq!(balance["available"], 95_000_000);
    assert_eq!(balance["locked"], 5_000_000);
    assert_eq!(balance["forfeited"], 0);
    t.finish().await;
}

#[tokio::test]
async fn prototype_open_manual_review_after_cutoff_blocks_failure_and_timeout_refunds_once() {
    let t = TestApp::new(false).await;
    let runner = t.account("receipt-open-review@example.com").await;
    let operator = t.account("receipt-review-operator@example.com").await;
    sqlx::query("INSERT INTO prototype_operators(user_id) VALUES($1)")
        .bind(operator.user)
        .execute(&t.pool)
        .await
        .unwrap();
    t.prototype_grant(&runner).await;
    let id = t.prototype(&runner, 5_000_000, "REPLAY").await;
    t.prototype_action(&runner, id, "DEPOSIT").await;
    let deadline = (Utc::now() - Duration::minutes(1))
        .with_nanosecond(0)
        .unwrap();
    sqlx::query("UPDATE prototype_challenges SET starts_at=$1,ends_at=$2,upload_deadline=$3,refund_after=$4 WHERE id=$5")
        .bind(deadline-Duration::hours(2)).bind(deadline-Duration::hours(1))
        .bind(deadline).bind(deadline+Duration::minutes(10)).bind(id)
        .execute(&t.pool).await.unwrap();
    let (status, upload) = t
        .recorded_upload(
            &runner,
            id,
            deadline,
            include_bytes!("../../../web/public/prototype/review-run.gpx").to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(upload["decision"], "REVIEW_REQUIRED");
    assert_eq!(
        t.prototype_action(&operator, id, "FAILURE").await.0,
        StatusCode::CONFLICT
    );
    // Check the Devnet preparation guard without invoking RPC or a signer:
    // unresolved review must reject the request before the worker is called.
    sqlx::query("UPDATE prototype_challenges SET network='DEVNET' WHERE id=$1")
        .bind(id)
        .execute(&t.pool)
        .await
        .unwrap();
    let response = t
        .request(
            "POST",
            &format!("/api/prototype/challenges/{id}/prepare"),
            json!({"action":"FAILURE"}),
            Some(&operator),
            None,
        )
        .await;
    assert_eq!(response.0, StatusCode::CONFLICT);
    assert_eq!(response.1["error"], "ACTION_NOT_AVAILABLE");
    let commands: i64 =
        sqlx::query_scalar("SELECT count(*) FROM prototype_commands WHERE challenge_id=$1")
            .bind(id)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(commands, 0);
    sqlx::query("UPDATE prototype_challenges SET network='LOCAL',refund_after=now()-interval '1 second' WHERE id=$1")
        .bind(id).execute(&t.pool).await.unwrap();
    assert_eq!(
        t.prototype_action(&runner, id, "TIMEOUT").await.0,
        StatusCode::OK
    );
    assert_eq!(
        t.prototype_action(&runner, id, "TIMEOUT").await.0,
        StatusCode::OK
    );
    let transfers: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM prototype_local_movements WHERE challenge_id=$1 AND action='TIMEOUT'",
    )
    .bind(id)
    .fetch_one(&t.pool)
    .await
    .unwrap();
    assert_eq!(transfers, 1);
    let (_, balance, _) = t
        .request(
            "GET",
            "/api/prototype/local/balance",
            Value::Null,
            Some(&runner),
            None,
        )
        .await;
    assert_eq!(balance["available"], 100_000_000);
    assert_eq!(balance["locked"], 0);
    assert_eq!(balance["forfeited"], 0);
    let (_, detail, _) = t
        .request(
            "GET",
            &format!("/api/prototype/challenges/{id}"),
            Value::Null,
            Some(&runner),
            None,
        )
        .await;
    assert_eq!(detail["challenge"]["state"], "EXPIRED");
    assert_eq!(detail["uploads"][0]["decision"], "REVIEW_REQUIRED");
    t.finish().await;
}

#[tokio::test]
async fn prototype_manual_rejection_after_cutoff_is_required_before_failure_can_proceed() {
    let t = TestApp::new(false).await;
    let runner = t.account("receipt-rejected-review@example.com").await;
    let operator = t.account("receipt-rejecting-operator@example.com").await;
    sqlx::query("INSERT INTO prototype_operators(user_id) VALUES($1)")
        .bind(operator.user)
        .execute(&t.pool)
        .await
        .unwrap();
    t.prototype_grant(&runner).await;
    let id = t.prototype(&runner, 5_000_000, "REPLAY").await;
    t.prototype_action(&runner, id, "DEPOSIT").await;
    let (status, bytes) = t
        .raw(
            "POST",
            &format!("/api/prototype/challenges/{id}/upload"),
            include_bytes!("../../../web/public/prototype/review-run.gpx").to_vec(),
            Some(&runner),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let upload: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(upload["decision"], "REVIEW_REQUIRED");
    sqlx::query("UPDATE prototype_challenges SET starts_at=now()-interval '2 hours',ends_at=now()-interval '1 hour',upload_deadline=now()-interval '1 minute',refund_after=now()+interval '5 minutes' WHERE id=$1")
        .bind(id).execute(&t.pool).await.unwrap();
    assert_eq!(
        t.prototype_action(&operator, id, "FAILURE").await.0,
        StatusCode::CONFLICT
    );
    let response = t.request("POST", &format!("/api/prototype/challenges/{id}/review"), json!({"upload_id":upload["id"],"accept":false,"reason":"Rejected synthetic anomalous activity after inspection"}), Some(&operator), None).await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(
        t.prototype_action(&operator, id, "FAILURE").await.0,
        StatusCode::OK
    );
    let (_, detail, _) = t
        .request(
            "GET",
            &format!("/api/prototype/challenges/{id}"),
            Value::Null,
            Some(&runner),
            None,
        )
        .await;
    assert_eq!(detail["challenge"]["state"], "FORFEITED");
    assert!(
        detail["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["kind"] == "manual_review" && event["detail"]["accepted"] == false)
    );
    let (_, balance, _) = t
        .request(
            "GET",
            "/api/prototype/local/balance",
            Value::Null,
            Some(&runner),
            None,
        )
        .await;
    assert_eq!(balance["available"], 95_000_000);
    assert_eq!(balance["forfeited"], 5_000_000);
    t.finish().await;
}

#[tokio::test]
async fn prototype_permissions_invalid_upload_and_dedup_never_forfeit() {
    let t = TestApp::new(false).await;
    let a = t.account("a@example.com").await;
    let b = t.account("b@example.com").await;
    t.prototype_grant(&a).await;
    let id = t.prototype(&a, 5_000_000, "REPLAY").await;
    let id2 = t.prototype(&a, 5_000_000, "REPLAY").await;
    assert_eq!(
        t.prototype_action(&a, id, "DEPOSIT").await.0,
        StatusCode::OK
    );
    assert_eq!(
        t.prototype_action(&a, id2, "DEPOSIT").await.0,
        StatusCode::OK
    );
    let path = format!("/api/prototype/challenges/{id}");
    let upload = format!("{path}/upload");
    assert_eq!(
        t.request("GET", &path, Value::Null, Some(&b), None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        t.raw(
            "POST",
            &upload,
            b"<!DOCTYPE x [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><x/>".to_vec(),
            Some(&a)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        t.raw("POST", &upload, vec![0; 16 * 1024 * 1024 + 1], Some(&a))
            .await
            .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let source = include_bytes!("../../../web/public/prototype/valid-run.fit");
    assert_eq!(
        t.raw("POST", &upload, source.to_vec(), Some(&b)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        t.raw("POST", &upload, source.to_vec(), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let mut bad = a.clone();
    bad.csrf = "invalid".into();
    assert_eq!(
        t.raw("POST", &upload, source.to_vec(), Some(&bad)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.raw("POST", &upload, source.to_vec(), Some(&a)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        t.raw(
            "POST",
            &format!("/api/prototype/challenges/{id2}/upload"),
            source.to_vec(),
            Some(&a)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        t.prototype_action(&a, id, "FAILURE").await.0,
        StatusCode::FORBIDDEN
    );
    let (_, detail, _) = t.request("GET", &path, Value::Null, Some(&a), None).await;
    assert_eq!(detail["challenge"]["state"], "ACTIVE");
    let file_path = format!(
        "{path}/uploads/{}/source",
        detail["uploads"][0]["id"].as_str().unwrap()
    );
    assert_eq!(
        t.raw("GET", &file_path, vec![], Some(&b)).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, body, _) = t
        .request(
            "POST",
            &format!("{path}/prepare"),
            json!({"action":"DEPOSIT"}),
            Some(&a),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "INVALID_INPUT");
    assert_eq!(body["error"], "WRONG_NETWORK");

    // A Devnet draft can be saved before wallet onboarding, but repeated
    // activation attempts must not create commands or alter financial state.
    let devnet = Uuid::new_v4();
    let (status, body, _) = t
        .request(
            "POST",
            "/api/prototype/challenges",
            json!({"id":devnet,"title":"Wallet prerequisite","target_m":3000,"amount_units":5_000_000,"network":"DEVNET","profile":"REPLAY","starts_at":Utc::now()+Duration::minutes(10)}),
            Some(&b),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let devnet_path = format!("/api/prototype/challenges/{devnet}");
    let (status, before, _) = t
        .request("GET", &devnet_path, Value::Null, Some(&b), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(before["challenge"]["state"], "DRAFT");
    assert!(before["challenge"]["chain"].is_null());
    assert!(before["commands"].as_array().unwrap().is_empty());
    assert_eq!(before["events"].as_array().unwrap().len(), 1);
    assert_eq!(before["events"][0]["kind"], "created");
    for _ in 0..3 {
        let (status, body, _) = t
            .request(
                "POST",
                &format!("{devnet_path}/prepare"),
                json!({"action":"DEPOSIT"}),
                Some(&b),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "INVALID_INPUT");
        assert_eq!(body["error"], "LINK_PHANTOM_FIRST");
    }
    let (status, after, _) = t
        .request("GET", &devnet_path, Value::Null, Some(&b), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after, before);
    let movements: i64 =
        sqlx::query_scalar("SELECT count(*) FROM prototype_local_movements WHERE challenge_id=$1")
            .bind(devnet)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(movements, 0);
    t.finish().await;
}

#[tokio::test]
async fn prototype_short_attempt_can_be_replaced_and_review_cannot_override_distance() {
    let t = TestApp::new(false).await;
    let s = t.account("runner@example.com").await;
    let op = t.account("operator@example.com").await;
    sqlx::query("INSERT INTO prototype_operators(user_id) VALUES($1)")
        .bind(op.user)
        .execute(&t.pool)
        .await
        .unwrap();
    t.prototype_grant(&s).await;
    let id = t.prototype(&s, 5_000_000, "REPLAY").await;
    t.prototype_action(&s, id, "DEPOSIT").await;
    let path = format!("/api/prototype/challenges/{id}");
    let upload = format!("{path}/upload");
    let (status, bytes) = t
        .raw(
            "POST",
            &upload,
            include_bytes!("../../../web/public/prototype/short-run.gpx").to_vec(),
            Some(&s),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let short: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(short["decision"], "REJECTED");
    let approve =
        json!({"upload_id":short["id"],"accept":true,"reason":"Manual review of evidence"});
    assert_eq!(
        t.request(
            "POST",
            &format!("{path}/review"),
            approve.clone(),
            Some(&s),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.request("POST", &format!("{path}/review"), approve, Some(&op), None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        t.prototype_action(&s, id, "SUCCESS").await.0,
        StatusCode::CONFLICT
    );
    let (status, bytes) = t
        .raw(
            "POST",
            &upload,
            include_bytes!("../../../web/public/prototype/review-run.gpx").to_vec(),
            Some(&s),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let suspect: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(suspect["decision"], "REVIEW_REQUIRED");
    assert_eq!(
        t.prototype_action(&s, id, "SUCCESS").await.0,
        StatusCode::CONFLICT
    );
    let accept = json!({"upload_id":suspect["id"],"accept":true,"reason":"Accepted synthetic test after manual inspection"});
    assert_eq!(
        t.request("POST", &format!("{path}/review"), accept, Some(&op), None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.prototype_action(&s, id, "SUCCESS").await.0,
        StatusCode::OK
    );
    let request = json!({"reason":"Please double check the recorded outcome"});
    assert_eq!(
        t.request(
            "POST",
            &format!("{path}/review-request"),
            request.clone(),
            Some(&op),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (status, first, _) = t
        .request(
            "POST",
            &format!("{path}/review-request"),
            request.clone(),
            Some(&s),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, repeat, _) = t
        .request(
            "POST",
            &format!("{path}/review-request"),
            request,
            Some(&s),
            None,
        )
        .await;
    assert_eq!(first["id"], repeat["id"]);
    assert_eq!(
        t.request(
            "POST",
            &format!("{path}/review-request/respond"),
            json!({"reason":"The recorded outcome is consistent"}),
            Some(&s),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.request(
            "POST",
            &format!("{path}/review-request/respond"),
            json!({"reason":"The recorded outcome is consistent"}),
            Some(&op),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, detail, _) = t.request("GET", &path, Value::Null, Some(&s), None).await;
    assert_eq!(detail["challenge"]["state"], "REFUNDED");
    assert_eq!(detail["review_request"]["status"], "CLOSED");
    t.finish().await;
}

#[tokio::test]
async fn prototype_concurrent_deposits_never_overdraw_and_cancel_or_timeout_refund_once() {
    let t = TestApp::new(false).await;
    let s = t.account("race@example.com").await;
    t.prototype_grant(&s).await;
    let a = t.prototype(&s, 50_000_000, "LIVE").await;
    let b = t.prototype(&s, 50_000_000, "LIVE").await;
    let c = t.prototype(&s, 50_000_000, "LIVE").await;
    let results = tokio::join!(
        t.prototype_action(&s, a, "DEPOSIT"),
        t.prototype_action(&s, b, "DEPOSIT"),
        t.prototype_action(&s, c, "DEPOSIT")
    );
    let results = [(a, results.0), (b, results.1), (c, results.2)];
    assert_eq!(
        results.iter().filter(|r| r.1.0 == StatusCode::OK).count(),
        2
    );
    assert_eq!(
        results
            .iter()
            .filter(|r| r.1.0 == StatusCode::BAD_REQUEST)
            .count(),
        1
    );
    for (id, r) in results {
        if r.0 == StatusCode::OK {
            let (x, y) = tokio::join!(
                t.prototype_action(&s, id, "CANCEL"),
                t.prototype_action(&s, id, "CANCEL")
            );
            assert_eq!(x.0, StatusCode::OK);
            assert_eq!(y.0, StatusCode::OK);
        }
    }
    let timeout = t.prototype(&s, 5_000_000, "REPLAY").await;
    t.prototype_action(&s, timeout, "DEPOSIT").await;
    assert_eq!(
        t.prototype_action(&s, timeout, "CANCEL").await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        t.prototype_action(&s, timeout, "TIMEOUT").await.0,
        StatusCode::CONFLICT
    );
    sqlx::query("UPDATE prototype_challenges SET starts_at=now()-interval '3 hours',ends_at=now()-interval '2 hours',upload_deadline=now()-interval '1 hour',refund_after=now()-interval '1 minute' WHERE id=$1").bind(timeout).execute(&t.pool).await.unwrap();
    assert_eq!(
        t.prototype_action(&s, timeout, "TIMEOUT").await.0,
        StatusCode::OK
    );
    assert_eq!(
        t.prototype_action(&s, timeout, "SUCCESS").await.0,
        StatusCode::CONFLICT
    );
    let (_, balance, _) = t
        .request(
            "GET",
            "/api/prototype/local/balance",
            Value::Null,
            Some(&s),
            None,
        )
        .await;
    assert_eq!(balance["available"], 100_000_000);
    assert_eq!(balance["locked"], 0);
    t.finish().await;
}

#[tokio::test]
async fn prototype_failure_requires_operator_and_closed_window_and_preserves_principal() {
    let t = TestApp::new(false).await;
    let s = t.account("runner@example.com").await;
    let op = t.account("operator@example.com").await;
    sqlx::query("INSERT INTO prototype_operators(user_id) VALUES($1)")
        .bind(op.user)
        .execute(&t.pool)
        .await
        .unwrap();
    t.prototype_grant(&s).await;
    let id = t.prototype(&s, 5_000_000, "REPLAY").await;
    t.prototype_action(&s, id, "DEPOSIT").await;
    assert_eq!(
        t.prototype_action(&op, id, "FAILURE").await.0,
        StatusCode::CONFLICT
    );
    sqlx::query("UPDATE prototype_challenges SET starts_at=now()-interval '2 hours',ends_at=now()-interval '1 hour',upload_deadline=now()-interval '1 minute',refund_after=now()+interval '5 minutes' WHERE id=$1").bind(id).execute(&t.pool).await.unwrap();
    assert_eq!(
        t.raw(
            "POST",
            &format!("/api/prototype/challenges/{id}/upload"),
            include_bytes!("../../../web/public/prototype/valid-run.fit").to_vec(),
            Some(&s)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        t.prototype_action(&op, id, "FAILURE").await.0,
        StatusCode::OK
    );
    assert_eq!(
        t.prototype_action(&op, id, "FAILURE").await.0,
        StatusCode::OK
    );
    assert_eq!(
        t.prototype_action(&s, id, "TIMEOUT").await.0,
        StatusCode::CONFLICT
    );
    let (_, balance, _) = t
        .request(
            "GET",
            "/api/prototype/local/balance",
            Value::Null,
            Some(&s),
            None,
        )
        .await;
    assert_eq!(balance["available"], 95_000_000);
    assert_eq!(balance["locked"], 0);
    assert_eq!(balance["forfeited"], 5_000_000);
    t.finish().await;
}

#[tokio::test]
async fn prototype_historical_live_upload_is_not_met_and_fit_session_is_explicit() {
    let t = TestApp::new(false).await;
    let s = t.account("live@example.com").await;
    t.prototype_grant(&s).await;
    let live = t.prototype(&s, 5_000_000, "LIVE").await;
    t.prototype_action(&s, live, "DEPOSIT").await;
    let (status, bytes) = t
        .raw(
            "POST",
            &format!("/api/prototype/challenges/{live}/upload"),
            include_bytes!("../../../web/public/prototype/valid-run.gpx").to_vec(),
            Some(&s),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let result: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(result["reason"], "OUTSIDE_ACTIVITY_WINDOW");
    let (_, detail, _) = t
        .request(
            "GET",
            &format!("/api/prototype/challenges/{live}"),
            Value::Null,
            Some(&s),
            None,
        )
        .await;
    assert_eq!(detail["challenge"]["assessment"], "NOT_MET");
    assert!(
        detail["uploads"][0]["activity"]["starts_at"]
            .as_str()
            .unwrap()
            .starts_with("2026-09-01")
    );

    // Recovery is a separate agreement: an immutable NOT_MET historical
    // attempt may qualify in REPLAY without changing the original LIVE stake.
    let recovered = t.prototype(&s, 5_000_000, "REPLAY").await;
    t.prototype_action(&s, recovered, "DEPOSIT").await;
    let source = include_bytes!("../../../web/public/prototype/valid-run.gpx").to_vec();
    let recovered_path = format!("/api/prototype/challenges/{recovered}/upload");
    let (status, bytes) = t
        .raw("POST", &recovered_path, source.clone(), Some(&s))
        .await;
    assert_eq!(status, StatusCode::OK);
    let accepted: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(accepted["decision"], "ACCEPTED");
    assert_eq!(accepted["reason"], "DISTANCE_AND_WINDOW_MET");
    let (status, bytes) = t
        .raw("POST", &recovered_path, source.clone(), Some(&s))
        .await;
    assert_eq!(status, StatusCode::OK);
    let repeated: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(repeated["id"], accepted["id"]);
    assert_eq!(repeated["duplicate"], true);

    // Prefer this challenge's earlier record over the later global reservation.
    let (status, bytes) = t
        .raw(
            "POST",
            &format!("/api/prototype/challenges/{live}/upload"),
            source.clone(),
            Some(&s),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let repeated: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(repeated["id"], result["id"]);
    assert_eq!(repeated["duplicate"], true);
    assert_eq!(
        t.prototype_action(&s, recovered, "SUCCESS").await.0,
        StatusCode::OK
    );
    let (_, balance, _) = t
        .request(
            "GET",
            "/api/prototype/local/balance",
            Value::Null,
            Some(&s),
            None,
        )
        .await;
    assert_eq!(balance["available"], 95_000_000);
    assert_eq!(balance["locked"], 5_000_000);
    assert_eq!(balance["forfeited"], 0);
    let (_, unchanged_live, _) = t
        .request(
            "GET",
            &format!("/api/prototype/challenges/{live}"),
            Value::Null,
            Some(&s),
            None,
        )
        .await;
    assert_eq!(unchanged_live, detail);

    // Qualifying evidence cannot earn a second refund, including a differently
    // encoded file with the same parsed activity fingerprint.
    let another = t.prototype(&s, 5_000_000, "REPLAY").await;
    t.prototype_action(&s, another, "DEPOSIT").await;
    let mut reencoded = source.clone();
    reencoded.extend_from_slice(b"\n ");
    for bytes in [source, reencoded] {
        let (status, bytes) = t
            .raw(
                "POST",
                &format!("/api/prototype/challenges/{another}/upload"),
                bytes,
                Some(&s),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
        let blocked: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(blocked["error"], "ACTIVITY_ALREADY_USED");
    }
    let (count, wallet, locked, recipient): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT count(*),sum(wallet_delta)::bigint,sum(locked_delta)::bigint,sum(recipient_delta)::bigint FROM prototype_local_movements WHERE challenge_id=$1",
    ).bind(live).fetch_one(&t.pool).await.unwrap();
    assert_eq!(
        (count, wallet, locked, recipient),
        (1, -5_000_000, 5_000_000, 0)
    );

    let replay = t.prototype(&s, 5_000_000, "REPLAY").await;
    t.prototype_action(&s, replay, "DEPOSIT").await;
    let path = format!("/api/prototype/challenges/{replay}/upload");
    let fit = include_bytes!("../../../web/public/prototype/multi-session.fit").to_vec();
    let (status, bytes) = t.raw("POST", &path, fit.clone(), Some(&s)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(String::from_utf8_lossy(&bytes).contains("SELECT_FIT_SESSION"));
    let (status, bytes) = t
        .raw("POST", &format!("{path}?session=0"), fit, Some(&s))
        .await;
    assert_eq!(status, StatusCode::OK);
    let result: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(result["decision"], "ACCEPTED");
    assert_eq!(
        t.prototype_action(&s, replay, "SUCCESS").await.0,
        StatusCode::OK
    );
    t.finish().await;
}

#[tokio::test]
async fn prototype_qualifying_evidence_stays_reserved_after_manual_rejection() {
    let t = TestApp::new(false).await;
    let runner = t.account("reserved-runner@example.com").await;
    let operator = t.account("reserved-reviewer@example.com").await;
    sqlx::query("INSERT INTO prototype_operators(user_id) VALUES($1)")
        .bind(operator.user)
        .execute(&t.pool)
        .await
        .unwrap();
    t.prototype_grant(&runner).await;
    let first = t.prototype(&runner, 5_000_000, "REPLAY").await;
    let second = t.prototype(&runner, 5_000_000, "REPLAY").await;
    t.prototype_action(&runner, first, "DEPOSIT").await;
    t.prototype_action(&runner, second, "DEPOSIT").await;
    let source = include_bytes!("../../../web/public/prototype/review-run.gpx").to_vec();
    let first_path = format!("/api/prototype/challenges/{first}");
    let (status, bytes) = t
        .raw(
            "POST",
            &format!("{first_path}/upload"),
            source.clone(),
            Some(&runner),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let upload: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(upload["decision"], "REVIEW_REQUIRED");
    let (status, body, _) = t
        .request(
            "POST", &format!("{first_path}/review"),
            json!({"upload_id":upload["id"],"accept":false,"reason":"Recorded measurements remain unverified"}),
            Some(&operator), None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, detail, _) = t
        .request("GET", &first_path, Value::Null, Some(&runner), None)
        .await;
    assert_eq!(detail["uploads"][0]["goal_result"], "MET");
    assert_eq!(detail["uploads"][0]["decision"], "REJECTED");
    assert_eq!(detail["challenge"]["assessment"], "NOT_MET");

    let mut reencoded = source.clone();
    reencoded.extend_from_slice(b"\n ");
    for bytes in [source, reencoded] {
        let (status, bytes) = t
            .raw(
                "POST",
                &format!("/api/prototype/challenges/{second}/upload"),
                bytes,
                Some(&runner),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
        let blocked: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(blocked["error"], "ACTIVITY_ALREADY_USED");
    }
    let (_, second_detail, _) = t
        .request(
            "GET",
            &format!("/api/prototype/challenges/{second}"),
            Value::Null,
            Some(&runner),
            None,
        )
        .await;
    assert!(second_detail["uploads"].as_array().unwrap().is_empty());
    assert_eq!(second_detail["challenge"]["state"], "ACTIVE");
    assert_eq!(second_detail["challenge"]["assessment"], "UNKNOWN");
    let (_, balance, _) = t
        .request(
            "GET",
            "/api/prototype/local/balance",
            Value::Null,
            Some(&runner),
            None,
        )
        .await;
    assert_eq!(balance["available"], 90_000_000);
    assert_eq!(balance["locked"], 10_000_000);
    assert_eq!(balance["forfeited"], 0);
    t.finish().await;
}

#[tokio::test]
async fn prototype_concurrent_qualifying_uploads_reserve_one_run_without_settlement() {
    let t = TestApp::new(false).await;
    let runner = t.account("upload-reservation-race@example.com").await;
    t.prototype_grant(&runner).await;
    let first = t.prototype(&runner, 5_000_000, "REPLAY").await;
    let second = t.prototype(&runner, 5_000_000, "REPLAY").await;
    for id in [first, second] {
        assert_eq!(
            t.prototype_action(&runner, id, "DEPOSIT").await.0,
            StatusCode::OK
        );
    }
    let (_, before, _) = t
        .request(
            "GET",
            "/api/prototype/local/balance",
            Value::Null,
            Some(&runner),
            None,
        )
        .await;

    // Hold the shared intake lock until both requests actually reach it. This
    // proves overlap rather than relying on the scheduler to race two uploads.
    let mut gate = t.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(runner.user)
        .fetch_one(&mut *gate)
        .await
        .unwrap();
    let first_path = format!("/api/prototype/challenges/{first}/upload");
    let second_path = format!("/api/prototype/challenges/{second}/upload");
    let source = include_bytes!("../../../web/public/prototype/valid-run.gpx").to_vec();
    let mut attempts = Box::pin(async {
        tokio::join!(
            t.raw("POST", &first_path, source.clone(), Some(&runner)),
            t.raw("POST", &second_path, source.clone(), Some(&runner)),
        )
    });
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        tokio::select! {
            _ = &mut attempts => panic!("Uploads bypassed the held user intake lock"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
        }
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity WHERE application_name=$1 AND wait_event_type='Lock' AND query LIKE 'SELECT password_hash FROM users%FOR NO KEY UPDATE'",
        )
        .bind(&t.schema)
        .fetch_one(&t.admin)
        .await
        .unwrap();
        if waiting == 2 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Both uploads did not reach the user intake lock"
        );
    }
    gate.rollback().await.unwrap();
    let (one, two) = tokio::time::timeout(std::time::Duration::from_secs(5), attempts)
        .await
        .expect("Concurrent uploads must complete after the user lock is released");
    let responses = [one, two];
    assert_eq!(
        responses.iter().filter(|r| r.0 == StatusCode::OK).count(),
        1
    );
    assert_eq!(
        responses
            .iter()
            .filter(|r| r.0 == StatusCode::CONFLICT)
            .count(),
        1
    );
    for (status, bytes) in responses {
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        if status == StatusCode::OK {
            assert_eq!(body["decision"], "ACCEPTED");
        } else {
            assert_eq!(body["error"], "ACTIVITY_ALREADY_USED");
        }
    }
    let uploads: (i64, i64) = sqlx::query_as(
        "SELECT count(*),count(*) FILTER (WHERE goal_result='MET') FROM prototype_uploads WHERE user_id=$1",
    )
    .bind(runner.user)
    .fetch_one(&t.pool)
    .await
    .unwrap();
    assert_eq!(uploads, (1, 1));
    let challenges: Vec<(String, i64)> =
        sqlx::query_as("SELECT state,amount_units FROM prototype_challenges WHERE user_id=$1")
            .bind(runner.user)
            .fetch_all(&t.pool)
            .await
            .unwrap();
    assert_eq!(challenges.len(), 2);
    assert!(
        challenges
            .iter()
            .all(|c| c == &("ACTIVE".into(), 5_000_000))
    );
    let (_, after, _) = t
        .request(
            "GET",
            "/api/prototype/local/balance",
            Value::Null,
            Some(&runner),
            None,
        )
        .await;
    assert_eq!(after, before);
    assert_eq!(after["available"], 90_000_000);
    assert_eq!(after["locked"], 10_000_000);
    assert_eq!(after["forfeited"], 0);
    let (movements, wallet, locked, recipient): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT count(*),sum(wallet_delta)::bigint,sum(locked_delta)::bigint,sum(recipient_delta)::bigint FROM prototype_local_movements WHERE challenge_id IN ($1,$2)",
    )
    .bind(first)
    .bind(second)
    .fetch_one(&t.pool)
    .await
    .unwrap();
    assert_eq!(
        (movements, wallet, locked, recipient),
        (2, -10_000_000, 10_000_000, 0)
    );
    let commands: i64 =
        sqlx::query_scalar("SELECT count(*) FROM prototype_commands WHERE challenge_id IN ($1,$2)")
            .bind(first)
            .bind(second)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(commands, 0);
    t.finish().await;
}

#[tokio::test]
async fn accounts_are_private_sessions_hashed_and_logout_revokes_cookie() {
    let t = TestApp::new(false).await;
    let a = t.account("alice@example.com").await;
    let b = t.account("bob@example.com").await;
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1")
        .bind(a.user)
        .fetch_one(&t.pool)
        .await
        .unwrap();
    assert!(hash.starts_with("$argon2id$"));
    assert!(!hash.contains(PASSWORD));
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM sessions WHERE user_id=$1")
        .bind(a.user)
        .fetch_one(&t.pool)
        .await
        .unwrap();
    let raw = a.cookie.split_once('=').unwrap().1;
    assert_ne!(stored, raw);
    assert_eq!(stored, crypto::digest(raw));
    let goal = t.goal(&a, &Uuid::new_v4().to_string()).await;
    let id = goal["id"].as_str().unwrap();
    assert_eq!(
        t.request("GET", "/api/goals", Value::Null, None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        t.request(
            "GET",
            &format!("/api/goals/{id}"),
            Value::Null,
            Some(&b),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        t.request(
            "POST",
            &format!("/api/goals/{id}/archive"),
            json!({"version":1}),
            Some(&b),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, list, _) = t
        .request("GET", "/api/goals", Value::Null, Some(&b), None)
        .await;
    assert_eq!(list["goals"].as_array().unwrap().len(), 0);
    assert_eq!(
        t.request("POST", "/api/auth/logout", json!({}), Some(&a), None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.request("GET", "/api/auth/session", Value::Null, Some(&a), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    t.finish().await;
}

#[tokio::test]
async fn csrf_origin_and_unauthenticated_mutations_are_rejected() {
    let t = TestApp::new(false).await;
    let a = t.account("csrf@example.com").await;
    let mut bad = a.clone();
    bad.csrf = "wrong".into();
    assert_eq!(
        t.request(
            "PATCH",
            "/api/account",
            json!({"display_name":"attacker"}),
            Some(&bad),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let request = Request::builder()
        .method("POST")
        .uri("/api/auth/logout")
        .header("host", "127.0.0.1:8787")
        .header("origin", "https://attacker.example")
        .header("x-truhabit-request", "web")
        .header("cookie", &a.cookie)
        .header("x-csrf-token", &a.csrf)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        t.app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.request("GET", "/api/demo", Value::Null, Some(&a), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        t.request("POST", "/api/clock", json!({}), Some(&a), None)
            .await
            .0,
        StatusCode::METHOD_NOT_ALLOWED
    );
    t.finish().await;
}

#[tokio::test]
async fn auth_recovery_logout_login_cycles_preserve_the_account_and_rotate_csrf() {
    let t = TestApp::new(false).await;
    let email = "repeat-login@example.com";
    let mut session = t.account(email).await;
    let user = session.user;
    let goal = t.goal(&session, &Uuid::new_v4().to_string()).await;
    for _ in 0..12 {
        let (status, _, cleared) = t
            .request("POST", "/api/auth/logout", json!({}), Some(&session), None)
            .await;
        assert_eq!(status, StatusCode::OK);
        assert!(cleared.starts_with(&format!("{}=;", t.state.config.cookie_name())));
        assert!(cleared.contains("Max-Age=0"));
        assert_eq!(
            t.request(
                "GET",
                "/api/auth/session",
                Value::Null,
                Some(&session),
                None
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        let old = session;
        session = t.login(email, PASSWORD).await;
        assert_eq!(session.user, user);
        assert_ne!(session.cookie, old.cookie);
        assert_ne!(session.csrf, old.csrf);
        let mut stale_csrf = session.clone();
        stale_csrf.csrf = old.csrf;
        assert_eq!(
            t.request(
                "PATCH",
                "/api/account",
                json!({"display_name":"Stale request"}),
                Some(&stale_csrf),
                None
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        let (status, goals, _) = t
            .request("GET", "/api/goals", Value::Null, Some(&session), None)
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(goals["goals"][0]["id"], goal["id"]);
    }
    let ip_attempts: i32 = sqlx::query_scalar("SELECT count FROM rate_limits WHERE key_hash=$1")
        .bind(crypto::digest("login-ip:127.0.0.1"))
        .fetch_one(&t.pool)
        .await
        .unwrap();
    assert_eq!(ip_attempts, 13);
    let account_limits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_limits WHERE key_hash=$1")
            .bind(crypto::digest(format!("login-account:{email}")))
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(account_limits, 0);
    t.finish().await;
}

#[tokio::test]
async fn auth_recovery_deleted_email_can_be_registered_with_a_new_password_and_stale_cookie() {
    let t = TestApp::new(true).await;
    let email = "recreated@example.com";
    let old = t.account(email).await;
    let (status, body, cleared) = t
        .request(
            "DELETE",
            "/api/account",
            json!({"password":PASSWORD,"confirmation":"DELETE"}),
            Some(&old),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(cleared.starts_with(&format!("{}=;", t.state.config.cookie_name())));
    assert!(cleared.contains("Max-Age=0"));
    for table in ["users", "sessions", "auth_tokens", "mail_outbox"] {
        let column = if table == "users" { "id" } else { "user_id" };
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table} WHERE {column}=$1"
        )))
        .bind(old.user)
        .fetch_one(&t.pool)
        .await
        .unwrap();
        assert_eq!(count, 0, "Deleted account retained rows in {table}");
    }
    let new_password = "a replacement test passphrase 456";
    let (status, body, _) = t
        .request(
            "POST",
            "/api/auth/register",
            json!({"email":" RECREATED@EXAMPLE.COM ","password":new_password,"display_name":"New account"}),
            Some(&old),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body, cookie) = t
        .request(
            "POST",
            "/api/auth/login",
            json!({"email":email,"password":new_password}),
            Some(&old),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let recreated = Session {
        cookie: cookie.split(';').next().unwrap().into(),
        csrf: body["csrf_token"].as_str().unwrap().into(),
        user: Uuid::nil(),
    };
    let (status, body, _) = t
        .request(
            "GET",
            "/api/auth/session",
            Value::Null,
            Some(&recreated),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(body["user"]["id"], old.user.to_string());
    assert_eq!(body["user"]["display_name"], "New account");
    assert_eq!(body["user"]["email"], email);
    assert_eq!(
        t.request("GET", "/api/auth/session", Value::Null, Some(&old), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, body, _) = t
        .request(
            "POST",
            "/api/auth/login",
            json!({"email":email,"password":PASSWORD}),
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "INVALID_CREDENTIALS");
    t.finish().await;
}

#[tokio::test]
async fn auth_recovery_blocked_deletion_does_not_claim_duplicate_registration_created_an_account() {
    let t = TestApp::new(false).await;
    let email = "undeleted@example.com";
    let old = t.account(email).await;
    let (status, body, _) = t
        .request(
            "POST",
            "/api/organizations",
            json!({"id":Uuid::new_v4(),"name":"Owned workspace"}),
            Some(&old),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _, cookie) = t
        .request(
            "DELETE",
            "/api/account",
            json!({"password":PASSWORD,"confirmation":"DELETE"}),
            Some(&old),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(cookie.is_empty());
    let replacement_password = "a replacement test passphrase 456";
    let (status, body, _) = t
        .request(
            "POST",
            "/api/auth/register",
            json!({"email":email,"password":replacement_password,"display_name":"Replacement"}),
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "ACCOUNT_EXISTS");
    assert_eq!(
        t.request(
            "POST",
            "/api/auth/login",
            json!({"email":email,"password":replacement_password}),
            None,
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(t.login(email, PASSWORD).await.user, old.user);
    let (status, body, _) = t
        .request("GET", "/api/auth/session", Value::Null, Some(&old), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["user"]["display_name"], "Běžec");
    t.finish().await;
}

#[tokio::test]
async fn auth_recovery_production_duplicate_registration_remains_indistinguishable() {
    let mut t = TestApp::new(true).await;
    let mut config = (*t.state.config).clone();
    config.production = true;
    config.origin = "https://app.example.com".into();
    config.validate().unwrap();
    t.state.config = std::sync::Arc::new(config);
    t.app = router(t.state.clone(), "missing-dist");
    let registration =
        json!({"email":"private@example.com","password":PASSWORD,"display_name":"First"});
    let (created_status, created, _) = t
        .request("POST", "/api/auth/register", registration, None, None)
        .await;
    let (duplicate_status, duplicate, _) = t
        .request(
            "POST",
            "/api/auth/register",
            json!({"email":"private@example.com","password":"another private passphrase 456","display_name":"Duplicate"}),
            None,
            None,
        )
        .await;
    assert_eq!(created_status, StatusCode::OK);
    assert_eq!(duplicate_status, created_status);
    assert_eq!(duplicate, created);
    let session = t.login("private@example.com", PASSWORD).await;
    assert!(session.cookie.starts_with("__Host-truhabit="));
    t.finish().await;
}

#[tokio::test]
async fn auth_recovery_parallel_local_instances_keep_their_browser_sessions_separate() {
    let first = TestApp::new(false).await;
    let mut second = TestApp::new(false).await;
    let mut config = (*second.state.config).clone();
    config.origin = "http://127.0.0.1:8788".into();
    config.bind = "127.0.0.1:8788".parse().unwrap();
    second.state.config = std::sync::Arc::new(config);
    second.app = router(second.state.clone(), "missing-dist");
    let a = first.account("first-instance@example.com").await;
    let b = second.account("second-instance@example.com").await;
    assert_ne!(
        a.cookie.split_once('=').unwrap().0,
        b.cookie.split_once('=').unwrap().0
    );
    let jar = format!("{}; {}", a.cookie, b.cookie);
    let mut shared_a = a.clone();
    shared_a.cookie = jar.clone();
    let mut shared_b = b.clone();
    shared_b.cookie = jar;
    for (app, session) in [(&first, &shared_a), (&second, &shared_b)] {
        let (status, body, _) = app
            .request("GET", "/api/auth/session", Value::Null, Some(session), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["user"]["id"], session.user.to_string());
    }
    let (status, _, cleared) = first
        .request("POST", "/api/auth/logout", json!({}), Some(&shared_a), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        cleared.split_once('=').unwrap().0,
        a.cookie.split_once('=').unwrap().0
    );
    assert_eq!(
        second
            .request(
                "GET",
                "/api/auth/session",
                Value::Null,
                Some(&shared_b),
                None
            )
            .await
            .0,
        StatusCode::OK
    );
    first.finish().await;
    second.finish().await;
}

#[tokio::test]
async fn auth_recovery_rate_limit_reports_the_remaining_original_window() {
    let t = TestApp::new(false).await;
    let key = "register:127.0.0.1";
    let original_reset: chrono::DateTime<Utc> = sqlx::query_scalar(
        "INSERT INTO rate_limits(key_hash,count,reset_at) VALUES($1,10,now()+interval '30 seconds') RETURNING reset_at",
    )
    .bind(crypto::digest(key))
    .fetch_one(&t.pool)
    .await
    .unwrap();
    for _ in 0..2 {
        let error = truhabit_api::auth::rate_limit(&t.state, key, 10, 3600)
            .await
            .unwrap_err();
        let response = axum::response::IntoResponse::into_response(error);
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let seconds: u64 = response.headers()["retry-after"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((1..=30).contains(&seconds));
        let reset: chrono::DateTime<Utc> =
            sqlx::query_scalar("SELECT reset_at FROM rate_limits WHERE key_hash=$1")
                .bind(crypto::digest(key))
                .fetch_one(&t.pool)
                .await
                .unwrap();
        assert_eq!(
            reset, original_reset,
            "Blocked retries must not extend the wait"
        );
    }
    sqlx::query("UPDATE rate_limits SET reset_at=now()-interval '1 second' WHERE key_hash=$1")
        .bind(crypto::digest(key))
        .execute(&t.pool)
        .await
        .unwrap();
    truhabit_api::auth::rate_limit(&t.state, key, 10, 3600)
        .await
        .unwrap();
    let count: i32 = sqlx::query_scalar("SELECT count FROM rate_limits WHERE key_hash=$1")
        .bind(crypto::digest(key))
        .fetch_one(&t.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    t.finish().await;
}

#[tokio::test]
async fn concurrent_create_is_idempotent_and_cannot_claim_funding() {
    let t = TestApp::new(false).await;
    let a = t.account("goals@example.com").await;
    let key = Uuid::new_v4().to_string();
    let input =
        json!({"target_m":5000,"pledge_cents":1000,"starts_at":Utc::now()+Duration::hours(1)});
    let (first, second) = tokio::join!(
        t.request("POST", "/api/goals", input.clone(), Some(&a), Some(&key)),
        t.request("POST", "/api/goals", input.clone(), Some(&a), Some(&key))
    );
    assert_eq!(first.0, StatusCode::OK, "{}", first.1);
    assert_eq!(first.1, second.1);
    assert_eq!(first.1["state"], "DRAFT");
    let id = first.1["id"].as_str().unwrap();
    let mut changed = input.clone();
    changed["target_m"] = json!(3000);
    assert_eq!(
        t.request("POST", "/api/goals", changed, Some(&a), Some(&key))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (status, fund, _) = t
        .request(
            "POST",
            &format!("/api/goals/{id}/fund"),
            json!({}),
            Some(&a),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(fund["code"], "FUNDING_UNAVAILABLE");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM goals WHERE user_id=$1")
        .bind(a.user)
        .fetch_one(&t.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    t.finish().await;
}

#[tokio::test]
async fn stale_edits_do_not_overwrite_goals_and_archives_are_terminal() {
    let t = TestApp::new(false).await;
    let a = t.account("edits@example.com").await;
    let goal = t.goal(&a, &Uuid::new_v4().to_string()).await;
    let path = format!("/api/goals/{}", goal["id"].as_str().unwrap());
    let input = json!({"target_m":3000,"pledge_cents":500,"starts_at":Utc::now()+Duration::hours(2),"version":1});
    let (status, updated, _) = t
        .request("PATCH", &path, input.clone(), Some(&a), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["version"], 2);
    assert_eq!(
        t.request("PATCH", &path, input, Some(&a), None).await.0,
        StatusCode::CONFLICT
    );
    let archive = format!("{path}/archive");
    assert_eq!(
        t.request("POST", &archive, json!({"version":2}), Some(&a), None)
            .await
            .0,
        StatusCode::OK
    );
    let (status, again, _) = t
        .request("POST", &archive, json!({"version":2}), Some(&a), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["state"], "ARCHIVED");
    let (_, detail, _) = t.request("GET", &path, Value::Null, Some(&a), None).await;
    assert_eq!(detail["events"].as_array().unwrap().len(), 3);
    t.finish().await;
}

#[tokio::test]
async fn password_change_revokes_all_sessions_and_wrong_password_cannot_delete_account() {
    let t = TestApp::new(false).await;
    let a = t.account("password@example.com").await;
    let b = t.login("password@example.com", PASSWORD).await;
    assert_eq!(
        t.request(
            "DELETE",
            "/api/account",
            json!({"confirmation":"SMAZAT","password":"wrong"}),
            Some(&a),
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        t.request(
            "POST",
            "/api/auth/password",
            json!({"current_password":PASSWORD,"new_password":"A second long passphrase 456"}),
            Some(&a),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    for s in [&a, &b] {
        assert_eq!(
            t.request("GET", "/api/auth/session", Value::Null, Some(s), None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        t.request(
            "POST",
            "/api/auth/login",
            json!({"email":"password@example.com","password":PASSWORD}),
            None,
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let _ = t
        .login("password@example.com", "A second long passphrase 456")
        .await;
    t.finish().await;
}

#[tokio::test]
async fn verification_and_recovery_tokens_are_encrypted_single_use_and_reset_revokes_sessions() {
    let t = TestApp::new(true).await;
    let a = t.account("mail@example.com").await;
    let (id, encrypted): (Uuid, String) =
        sqlx::query_as("SELECT id,payload_encrypted FROM mail_outbox WHERE user_id=$1")
            .bind(a.user)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert!(!encrypted.contains("mail@example.com"));
    let payload = mail::decrypt(&[42; 32], id, &encrypted).unwrap();
    let token = payload
        .body
        .split("verify_email=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    assert_eq!(
        t.request(
            "POST",
            "/api/auth/verify-email",
            json!({"token":token}),
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        t.request(
            "POST",
            "/api/auth/verify-email",
            json!({"token":token}),
            None,
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        t.request(
            "POST",
            "/api/auth/forgot-password",
            json!({"email":"mail@example.com"}),
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let (id,encrypted):(Uuid,String)=sqlx::query_as("SELECT id,payload_encrypted FROM mail_outbox WHERE user_id=$1 ORDER BY created_at DESC LIMIT 1").bind(a.user).fetch_one(&t.pool).await.unwrap();
    let payload = mail::decrypt(&[42; 32], id, &encrypted).unwrap();
    let token = payload
        .body
        .split("reset_password=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    let body = json!({"token":token,"new_password":"Restored secret passphrase 789"});
    assert_eq!(
        t.request("POST", "/api/auth/reset-password", body.clone(), None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.request("POST", "/api/auth/reset-password", body, None, None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        t.request("GET", "/api/auth/session", Value::Null, Some(&a), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let _ = t
        .login("mail@example.com", "Restored secret passphrase 789")
        .await;
    t.finish().await;
}

#[tokio::test]
async fn wallet_signatures_are_bound_to_account_session_challenge_and_expiry() {
    let t = TestApp::new(false).await;
    let a = t.account("wallet@example.com").await;
    let b = t.account("otherwallet@example.com").await;
    let key = SigningKey::from_bytes(&[7; 32]);
    let address = bs58::encode(key.verifying_key().as_bytes()).into_string();
    let (status, challenge, _) = t
        .request(
            "POST",
            "/api/wallet/challenge",
            json!({"public_key":address}),
            Some(&a),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let message = challenge["message"].as_str().unwrap();
    assert!(message.contains("http://127.0.0.1:8787"));
    assert!(message.contains(&a.user.to_string()));
    let body = json!({"challenge_id":challenge["id"],"signature":STANDARD.encode(key.sign(message.as_bytes()).to_bytes())});
    assert_eq!(
        t.request("POST", "/api/wallet/link", body.clone(), Some(&b), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        t.request("POST", "/api/wallet/link", body.clone(), Some(&a), None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.request("POST", "/api/wallet/link", body, Some(&a), None)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (_, challenge, _) = t
        .request(
            "POST",
            "/api/wallet/challenge",
            json!({"public_key":address}),
            Some(&a),
            None,
        )
        .await;
    let body = json!({"challenge_id":challenge["id"],"signature":STANDARD.encode(key.sign(challenge["message"].as_str().unwrap().as_bytes()).to_bytes())});
    sqlx::query("UPDATE wallet_challenges SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(Uuid::parse_str(challenge["id"].as_str().unwrap()).unwrap())
        .execute(&t.pool)
        .await
        .unwrap();
    assert_eq!(
        t.request("POST", "/api/wallet/link", body, Some(&a), None)
            .await
            .0,
        StatusCode::CONFLICT
    );
    t.finish().await;
}

#[tokio::test]
async fn export_excludes_credentials_and_deletion_cascades_only_own_data() {
    let t = TestApp::new(false).await;
    let a = t.account("export@example.com").await;
    let b = t.account("survivor@example.com").await;
    t.goal(&a, &Uuid::new_v4().to_string()).await;
    t.goal(&b, &Uuid::new_v4().to_string()).await;
    let (status, export, _) = t
        .request("GET", "/api/account/export", Value::Null, Some(&a), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(export["goals"].as_array().unwrap().len(), 1);
    let text = export.to_string();
    assert!(!text.contains("password_hash"));
    assert!(!text.contains("csrf_token"));
    assert!(!text.contains("survivor@example.com"));
    assert_eq!(
        t.request(
            "DELETE",
            "/api/account",
            json!({"confirmation":"SMAZAT","password":PASSWORD}),
            Some(&a),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM goals WHERE user_id=$1")
        .bind(a.user)
        .fetch_one(&t.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        t.request("GET", "/api/auth/session", Value::Null, Some(&b), None)
            .await
            .0,
        StatusCode::OK
    );
    t.finish().await;
}

#[tokio::test]
async fn account_deletion_during_a_goal_edit_or_archive_never_deadlocks() {
    for archive in [false, true] {
        let t = std::sync::Arc::new(TestApp::new(false).await);
        let s = t.account("goal-deletion-race@example.test").await;
        let goal = t.goal(&s, &Uuid::new_v4().to_string()).await;
        let path = format!(
            "/api/goals/{}{}",
            goal["id"].as_str().unwrap(),
            if archive { "/archive" } else { "" }
        );
        let update = if archive {
            json!({"version":1})
        } else {
            json!({"version":1,"target_m":3000,"pledge_cents":1000,"starts_at":Utc::now()+Duration::hours(1)})
        };
        // Pause the real mutation after it owns the goal row, making the
        // edit/delete interleaving deterministic. This trigger and advisory
        // lock exist only in this test's disposable schema.
        let key = (Uuid::new_v4().as_u128() & (i64::MAX as u128)) as i64;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE FUNCTION pause_goal_mutation() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock({key}); RETURN NEW; END $$"
        )))
        .execute(&t.pool)
        .await
        .unwrap();
        sqlx::query("CREATE TRIGGER pause_goal_mutation BEFORE UPDATE ON goals FOR EACH ROW EXECUTE FUNCTION pause_goal_mutation()")
            .execute(&t.pool).await.unwrap();
        let mut pause = t.pool.begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(key)
            .execute(&mut *pause)
            .await
            .unwrap();
        let mutation = {
            let t = t.clone();
            let s = s.clone();
            tokio::spawn(async move {
                t.request(
                    if archive { "POST" } else { "PATCH" },
                    &path,
                    update,
                    Some(&s),
                    None,
                )
                .await
            })
        };
        let editing = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=$1 AND query LIKE 'UPDATE goals SET%' AND state='active' AND wait_event_type='Lock')")
                    .bind(&t.schema).fetch_one(&t.pool).await.unwrap();
                if blocked {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await;
        let deletion = {
            let t = t.clone();
            tokio::spawn(async move {
                t.request(
                    "DELETE",
                    "/api/account",
                    json!({"confirmation":"DELETE","password":PASSWORD}),
                    Some(&s),
                    None,
                )
                .await
            })
        };
        let deleting = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=$1 AND (query LIKE 'DELETE FROM users%' OR query LIKE 'SELECT password_hash FROM users%') AND state='active' AND wait_event_type='Lock')")
                    .bind(&t.schema).fetch_one(&t.pool).await.unwrap();
                if blocked {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await;
        pause.commit().await.unwrap();
        let (edited, deleted) = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            tokio::join!(mutation, deletion)
        })
        .await
        .expect("Goal edit and account deletion must finish without a lock cycle");
        let edited = edited.unwrap();
        let deleted = deleted.unwrap();
        let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
            .fetch_one(&t.pool)
            .await
            .unwrap();
        let t = match std::sync::Arc::try_unwrap(t) {
            Ok(t) => t,
            Err(_) => panic!("Concurrent requests retained the test app"),
        };
        t.finish().await;
        assert!(editing.is_ok(), "Goal mutation did not reach its pause");
        assert!(deleting.is_ok(), "Account deletion did not reach its lock");
        assert_eq!(
            (edited.0, deleted.0, remaining),
            (StatusCode::OK, StatusCode::OK, 0),
            "archive={archive}; edit={}, delete={}",
            edited.1,
            deleted.1
        );
    }
}

#[test]
fn authenticated_mail_encryption_rejects_tampering_and_swapped_job_identity() {
    let id = Uuid::new_v4();
    let payload = mail::MailPayload {
        to: "a@example.com".into(),
        subject: "Test".into(),
        body: "secret token".into(),
    };
    let encrypted = mail::encrypt(&[1; 32], id, &payload).unwrap();
    assert!(mail::decrypt(&[1; 32], Uuid::new_v4(), &encrypted).is_err());
    assert!(mail::decrypt(&[2; 32], id, &encrypted).is_err());
    assert_eq!(
        mail::decrypt(&[1; 32], id, &encrypted).unwrap().body,
        "secret token"
    );
}

#[tokio::test]
async fn idle_sessions_expire_and_login_rate_limits_cannot_be_bypassed_by_bad_passwords() {
    let t = TestApp::new(false).await;
    let a = t.account("expiry@example.com").await;
    sqlx::query("UPDATE sessions SET last_seen_at=now()-interval '121 minutes' WHERE user_id=$1")
        .bind(a.user)
        .execute(&t.pool)
        .await
        .unwrap();
    assert_eq!(
        t.request("GET", "/api/auth/session", Value::Null, Some(&a), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    for _ in 0..10 {
        assert_eq!(
            t.request(
                "POST",
                "/api/auth/login",
                json!({"email":"expiry@example.com","password":"incorrect password"}),
                None,
                None
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        t.request(
            "POST",
            "/api/auth/login",
            json!({"email":"expiry@example.com","password":PASSWORD}),
            None,
            None
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    t.finish().await;
}

type CapturedUpload = (
    truhabit_api::auth::Auth,
    axum::http::HeaderMap,
    truhabit_api::prototype::RecordedUpload,
);

/// The real production extractor takes the routed challenge Path and actual
/// receipt. A channel pauses only handler scheduling after body completion.
async fn capture_admitted_upload(t: &TestApp, s: &Session, id: Uuid) -> CapturedUpload {
    type Sender =
        std::sync::Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<CapturedUpload>>>>;
    async fn capture(
        auth: truhabit_api::auth::Auth,
        axum::Extension(sender): axum::Extension<Sender>,
        headers: axum::http::HeaderMap,
        recorded: truhabit_api::prototype::RecordedUpload,
    ) -> StatusCode {
        let sender = sender.lock().await.take().unwrap();
        assert!(sender.send((auth, headers, recorded)).is_ok());
        StatusCode::NO_CONTENT
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    let app = Router::new()
        .route("/admission/{id}", axum::routing::post(capture))
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
                .uri(format!("/admission/{id}"))
                .header("cookie", &s.cookie)
                .header("x-csrf-token", &s.csrf)
                .body(Body::from(
                    include_bytes!("../../../web/public/prototype/valid-run.gpx").to_vec(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    receive.await.unwrap()
}

async fn deliver_admitted_upload(
    t: &TestApp,
    id: Uuid,
    captured: CapturedUpload,
) -> (StatusCode, Value) {
    let (auth, headers, recorded) = captured;
    match truhabit_api::prototype::upload(
        auth,
        axum::extract::State(t.state.clone()),
        axum::extract::Path(id),
        axum::extract::Query(truhabit_api::prototype::UploadQuery { session: None }),
        headers,
        recorded,
    )
    .await
    {
        Ok(axum::Json(value)) => (StatusCode::OK, value),
        Err(error) => (
            error.status,
            json!({"code":error.code,"error":error.message}),
        ),
    }
}

async fn admission_fixture() -> (TestApp, Session, Session, Uuid, chrono::DateTime<Utc>) {
    let t = TestApp::new(false).await;
    let runner = t.account("admission-runner@example.test").await;
    let operator = t.account("admission-operator@example.test").await;
    sqlx::query("INSERT INTO prototype_operators(user_id) VALUES($1)")
        .bind(operator.user)
        .execute(&t.pool)
        .await
        .unwrap();
    t.prototype_grant(&runner).await;
    let id = t.prototype(&runner, 5_000_000, "REPLAY").await;
    assert_eq!(
        t.prototype_action(&runner, id, "DEPOSIT").await.0,
        StatusCode::OK
    );
    let deadline = (Utc::now() + Duration::seconds(3))
        .with_nanosecond(0)
        .unwrap();
    sqlx::query("UPDATE prototype_challenges SET starts_at=$1,ends_at=$2,upload_deadline=$3,refund_after=$4 WHERE id=$5")
        .bind(deadline-Duration::hours(2)).bind(deadline-Duration::hours(1))
        .bind(deadline).bind(deadline+Duration::minutes(5)).bind(id).execute(&t.pool).await.unwrap();
    (t, runner, operator, id, deadline)
}

async fn wait_past_admission_deadline(deadline: chrono::DateTime<Utc>) {
    let limit = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while Utc::now() <= deadline {
        assert!(tokio::time::Instant::now() < limit);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn prototype_admission_complete_body_retains_right_to_evaluation_before_handler_runs() {
    let (t, runner, operator, id, deadline) = admission_fixture().await;
    let captured = capture_admitted_upload(&t, &runner, id).await;
    assert!(captured.2.received_at <= deadline);
    wait_past_admission_deadline(deadline).await;
    let mut failure = Box::pin(t.prototype_action(&operator, id, "FAILURE"));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut failure)
            .await
            .is_err(),
        "Failure bypassed body admission locks"
    );
    let upload = deliver_admitted_upload(&t, id, captured).await;
    let failure = failure.await;
    let outcome: (String, String, i64) = sqlx::query_as("SELECT state,assessment,(SELECT count(*) FROM prototype_uploads WHERE challenge_id=$1) FROM prototype_challenges WHERE id=$1")
        .bind(id).fetch_one(&t.pool).await.unwrap();
    t.finish().await;
    assert_eq!(upload.0, StatusCode::OK, "{}", upload.1);
    assert_eq!(failure.0, StatusCode::CONFLICT, "{}", failure.1);
    assert_eq!(outcome, ("ACTIVE".into(), "MET".into(), 1));
}

#[tokio::test]
async fn prototype_admission_late_complete_body_does_not_block_failure() {
    let (t, runner, operator, id, deadline) = admission_fixture().await;
    wait_past_admission_deadline(deadline).await;
    let captured = capture_admitted_upload(&t, &runner, id).await;
    assert!(captured.2.received_at > deadline);
    let upload = deliver_admitted_upload(&t, id, captured).await;
    let failure = t.prototype_action(&operator, id, "FAILURE").await;
    t.finish().await;
    assert_eq!(upload.0, StatusCode::CONFLICT);
    assert_eq!(upload.1["error"], "UPLOAD_WINDOW_CLOSED");
    assert_eq!(failure.0, StatusCode::OK);
}

#[tokio::test]
async fn prototype_admission_drop_releases_locks_without_new_evidence() {
    let (t, runner, operator, id, deadline) = admission_fixture().await;
    let captured = capture_admitted_upload(&t, &runner, id).await;
    assert!(captured.2.received_at <= deadline);
    drop(captured);
    wait_past_admission_deadline(deadline).await;
    let failure = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        t.prototype_action(&operator, id, "FAILURE"),
    )
    .await
    .unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM prototype_uploads WHERE challenge_id=$1")
            .bind(id)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    t.finish().await;
    assert_eq!(failure.0, StatusCode::OK);
    assert_eq!(count, 0);
}

#[tokio::test]
async fn prototype_admission_timely_valid_evidence_invalidates_only_unsigned_failure() {
    let (t, runner, _operator, id, deadline) = admission_fixture().await;
    let command = Uuid::new_v4();
    // Legacy command fixture only: no worker invocation or chain broadcast.
    sqlx::query("UPDATE prototype_challenges SET network='DEVNET' WHERE id=$1")
        .bind(id)
        .execute(&t.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO prototype_commands(id,challenge_id,action,payload,status) VALUES($1,$2,'FAILURE','{}','PREPARED')").bind(command).bind(id).execute(&t.pool).await.unwrap();
    let captured = capture_admitted_upload(&t, &runner, id).await;
    assert!(captured.2.received_at <= deadline);
    wait_past_admission_deadline(deadline).await;
    let upload = deliver_admitted_upload(&t, id, captured).await;
    let outcome: (String, String, String) = sqlx::query_as("SELECT c.state,c.assessment,p.status FROM prototype_challenges c JOIN prototype_commands p ON p.challenge_id=c.id WHERE p.id=$1").bind(command).fetch_one(&t.pool).await.unwrap();
    t.finish().await;
    assert_eq!(upload.0, StatusCode::OK, "{}", upload.1);
    assert_eq!(outcome, ("ACTIVE".into(), "MET".into(), "FAILED".into()));
}

#[tokio::test]
async fn prototype_admission_signed_failure_blocks_new_evidence_until_reconciliation() {
    let (t, runner, _operator, id, _deadline) = admission_fixture().await;
    sqlx::query("UPDATE prototype_challenges SET network='DEVNET' WHERE id=$1")
        .bind(id)
        .execute(&t.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO prototype_commands(id,challenge_id,action,payload,status) VALUES($1,$2,'FAILURE','{}','SIGNED')").bind(Uuid::new_v4()).bind(id).execute(&t.pool).await.unwrap();
    let captured = capture_admitted_upload(&t, &runner, id).await;
    let upload = deliver_admitted_upload(&t, id, captured).await;
    let (assessment, count): (String,i64) = sqlx::query_as("SELECT assessment,(SELECT count(*) FROM prototype_uploads WHERE challenge_id=$1) FROM prototype_challenges WHERE id=$1").bind(id).fetch_one(&t.pool).await.unwrap();
    t.finish().await;
    assert_eq!(upload.0, StatusCode::CONFLICT);
    assert_eq!(upload.1["error"], "SETTLEMENT_PENDING");
    assert_eq!(assessment, "UNKNOWN");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn prototype_prepared_failure_is_reassessed_before_prepare_retry_or_first_signature() {
    let t = TestApp::new(false).await;
    let runner = t.account("prepared-runner@example.test").await;
    let operator = t.account("prepared-operator@example.test").await;
    sqlx::query("INSERT INTO prototype_operators(user_id) VALUES($1)")
        .bind(operator.user)
        .execute(&t.pool)
        .await
        .unwrap();
    for endpoint in ["prepare", "submit", "prepare-success"] {
        let id = t.prototype(&runner, 5_000_000, "REPLAY").await;
        sqlx::query("UPDATE prototype_challenges SET network='DEVNET',state='ACTIVE',assessment='MET' WHERE id=$1").bind(id).execute(&t.pool).await.unwrap();
        let command = Uuid::new_v4();
        sqlx::query("INSERT INTO prototype_commands(id,challenge_id,action,payload,status) VALUES($1,$2,'FAILURE','{}','PREPARED')").bind(command).bind(id).execute(&t.pool).await.unwrap();
        let input = if endpoint == "prepare" {
            json!({"action":"FAILURE"})
        } else if endpoint == "prepare-success" {
            json!({"action":"SUCCESS"})
        } else {
            json!({"command_id":command,"transaction":"no worker should receive this"})
        };
        let response = t
            .request(
                "POST",
                &format!(
                    "/api/prototype/challenges/{id}/{}",
                    if endpoint == "prepare-success" {
                        "prepare"
                    } else {
                        endpoint
                    }
                ),
                input,
                Some(&operator),
                None,
            )
            .await;
        assert_eq!(response.0, StatusCode::CONFLICT, "{}", response.1);
        assert_eq!(
            response.1["error"],
            if endpoint == "prepare-success" {
                "ACTION_STALE"
            } else {
                "ACTION_NOT_AVAILABLE"
            }
        );
        let (status, signature, signed): (String, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT status,signature,signed_transaction FROM prototype_commands WHERE id=$1",
        )
        .bind(command)
        .fetch_one(&t.pool)
        .await
        .unwrap();
        assert_eq!((status, signature, signed), ("FAILED".into(), None, None));
    }
    t.finish().await;
}

#[tokio::test]
async fn prototype_exact_upload_retry_recovers_after_settlement_and_exhausted_limits() {
    let t = TestApp::new(false).await;
    let runner = t.account("retry-runner@example.test").await;
    t.prototype_grant(&runner).await;
    let id = t.prototype(&runner, 5_000_000, "REPLAY").await;
    t.prototype_action(&runner, id, "DEPOSIT").await;
    let source = include_bytes!("../../../web/public/prototype/valid-run.gpx").to_vec();
    let path = format!("/api/prototype/challenges/{id}/upload");
    let (status, bytes) = t.raw("POST", &path, source.clone(), Some(&runner)).await;
    assert_eq!(status, StatusCode::OK);
    let original: Value = serde_json::from_slice(&bytes).unwrap();
    t.prototype_action(&runner, id, "SUCCESS").await;
    sqlx::query("UPDATE rate_limits SET count=21 WHERE key_hash=$1")
        .bind(crypto::digest(format!("prototype-upload:{}", runner.user)))
        .execute(&t.pool)
        .await
        .unwrap();
    let (status, bytes) = t.raw("POST", &path, source.clone(), Some(&runner)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let recovered: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(recovered["id"], original["id"]);
    assert_eq!(recovered["duplicate"], true);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM prototype_uploads WHERE challenge_id=$1")
            .bind(id)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    let mut changed = source;
    changed.push(b' ');
    assert_eq!(
        t.raw("POST", &path, changed, Some(&runner)).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    t.finish().await;
}

#[tokio::test]
async fn prototype_admission_body_limit_and_csrf_failures_release_the_single_pool_lease() {
    let t = TestApp::with_pool_size(false, 1).await;
    let runner = t.account("admission-limit@example.test").await;
    t.prototype_grant(&runner).await;
    let id = t.prototype(&runner, 5_000_000, "REPLAY").await;
    t.prototype_action(&runner, id, "DEPOSIT").await;
    let path = format!("/api/prototype/challenges/{id}/upload");
    let mut bad_csrf = runner.clone();
    bad_csrf.csrf = "wrong".into();
    let denied = t
        .raw(
            "POST",
            &path,
            vec![0; truhabit_evidence::upload::MAX_BYTES + 1],
            Some(&bad_csrf),
        )
        .await;
    assert_eq!(denied.0, StatusCode::FORBIDDEN);
    let oversized = t
        .raw(
            "POST",
            &path,
            vec![0; truhabit_evidence::upload::MAX_BYTES + 1],
            Some(&runner),
        )
        .await;
    assert_eq!(oversized.0, StatusCode::PAYLOAD_TOO_LARGE);
    let accepted = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        t.raw(
            "POST",
            &path,
            include_bytes!("../../../web/public/prototype/valid-run.gpx").to_vec(),
            Some(&runner),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        accepted.0,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&accepted.1)
    );
    t.finish().await;
}

#[tokio::test]
async fn prototype_exact_fit_retry_preserves_explicit_session_after_settlement() {
    let t = TestApp::new(false).await;
    let runner = t.account("fit-retry@example.test").await;
    t.prototype_grant(&runner).await;
    let id = t.prototype(&runner, 5_000_000, "REPLAY").await;
    t.prototype_action(&runner, id, "DEPOSIT").await;
    let path = format!("/api/prototype/challenges/{id}/upload");
    let source = include_bytes!("../../../web/public/prototype/multi-session.fit").to_vec();
    let (status, bytes) = t
        .raw(
            "POST",
            &format!("{path}?session=0"),
            source.clone(),
            Some(&runner),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let original: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        t.prototype_action(&runner, id, "SUCCESS").await.0,
        StatusCode::OK
    );
    let (status, bytes) = t
        .raw(
            "POST",
            &format!("{path}?session=0"),
            source.clone(),
            Some(&runner),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let retry: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(retry["id"], original["id"]);
    assert_eq!(retry["duplicate"], true);
    for suffix in ["", "?session=1"] {
        let (status, bytes) = t
            .raw(
                "POST",
                &format!("{path}{suffix}"),
                source.clone(),
                Some(&runner),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
        let rejected: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(rejected["error"], "UPLOAD_WINDOW_CLOSED");
    }
    let (count, selected): (i64, Option<i32>) = sqlx::query_as(
        "SELECT count(*),max(session_index) FROM prototype_uploads WHERE challenge_id=$1",
    )
    .bind(id)
    .fetch_one(&t.pool)
    .await
    .unwrap();
    t.finish().await;
    assert_eq!((count, selected), (1, Some(0)));
}
