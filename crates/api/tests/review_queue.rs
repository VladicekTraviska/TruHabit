//! Operator queue regressions against the HTTP router in disposable PostgreSQL schemas.
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
use truhabit_api::{AppState, MIGRATOR, config::Config, crypto, router};
use uuid::Uuid;

#[tokio::test]
async fn prototype_queue_skips_expired_and_pending_items_but_keeps_open_support() {
    let t = TestApp::new().await;
    let now = Utc::now();
    let mut expired = Uuid::nil();
    for i in 0..100 {
        expired = t
            .challenge(
                now - Duration::days(2) + Duration::seconds(i),
                now - Duration::hours(1),
                "ACTIVE",
                "UNKNOWN",
            )
            .await;
    }
    let mut pending = Vec::new();
    for status in ["PREPARED", "SIGNED"] {
        let id = t
            .challenge(
                now - Duration::days(1),
                now + Duration::hours(1),
                "ACTIVE",
                "REVIEW_REQUIRED",
            )
            .await;
        let upload = t.prototype_upload(id).await;
        sqlx::query("INSERT INTO prototype_commands(id,challenge_id,action,payload,status) VALUES($1,$2,'SUCCESS','{}',$3)")
            .bind(Uuid::new_v4()).bind(id).bind(status).execute(&t.pool).await.unwrap();
        pending.push((id, upload));
    }
    let support = t
        .challenge(
            now - Duration::days(1),
            now - Duration::hours(1),
            "EXPIRED",
            "UNKNOWN",
        )
        .await;
    sqlx::query("INSERT INTO prototype_review_requests(id,challenge_id,user_id,reason) VALUES($1,$2,$3,'Please review this settled outcome.')")
        .bind(Uuid::new_v4()).bind(support).bind(t.user).execute(&t.pool).await.unwrap();
    let current = t
        .challenge(now, now + Duration::hours(1), "ACTIVE", "REVIEW_REQUIRED")
        .await;
    let current_upload = t.prototype_upload(current).await;
    let unsettled_without_upload = t
        .challenge(now, now + Duration::hours(1), "ACTIVE", "UNKNOWN")
        .await;

    let queue = t.request("GET", "/api/prototype/review", Value::Null).await;
    let expired_review = t
        .request(
            "POST",
            &format!("/api/prototype/challenges/{expired}/review"),
            review_input(Uuid::new_v4()),
        )
        .await;
    let mut pending_reviews = Vec::new();
    for (id, upload) in pending {
        pending_reviews.push(
            t.request(
                "POST",
                &format!("/api/prototype/challenges/{id}/review"),
                review_input(upload),
            )
            .await,
        );
    }
    let current_review = t
        .request(
            "POST",
            &format!("/api/prototype/challenges/{current}/review"),
            review_input(current_upload),
        )
        .await;
    let support_response = t
        .request(
            "POST",
            &format!("/api/prototype/challenges/{support}/review-request/respond"),
            json!({"reason":"Reviewed the settled outcome and recorded the response."}),
        )
        .await;
    t.finish().await;

    assert_eq!(queue.0, StatusCode::OK, "{}", queue.1);
    let rows = queue.1["challenges"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        3,
        "Expired and pending items must not occupy the queue"
    );
    assert!(rows.iter().any(|row| row["id"] == current.to_string()));
    assert!(rows.iter().any(|row| row["id"] == support.to_string()));
    assert!(
        rows.iter()
            .any(|row| row["id"] == unsettled_without_upload.to_string())
    );
    assert_eq!(expired_review.0, StatusCode::CONFLICT);
    assert_eq!(expired_review.1["error"], "CHALLENGE_NOT_REVIEWABLE");
    for result in pending_reviews {
        assert_eq!(result.0, StatusCode::CONFLICT);
        assert_eq!(result.1["error"], "SETTLEMENT_PENDING");
    }
    assert_eq!(current_review.0, StatusCode::OK, "{}", current_review.1);
    assert_eq!(support_response.0, StatusCode::OK, "{}", support_response.1);
}

#[tokio::test]
async fn business_queue_skips_closed_review_windows_and_archived_workspaces() {
    let t = TestApp::new().await;
    let now = Utc::now();
    let org = t.organization(false).await;
    let mut expired = (Uuid::nil(), Uuid::nil());
    for i in 0..100 {
        expired = t
            .enrollment(
                org,
                now - Duration::days(2) + Duration::seconds(i),
                Some(now - Duration::hours(1)),
                "REVIEW_REQUIRED",
            )
            .await;
    }
    let null_deadline = t
        .enrollment(org, now - Duration::days(1), None, "REVIEW_REQUIRED")
        .await;
    let archived_org = t.organization(true).await;
    let archived = t
        .enrollment(
            archived_org,
            now - Duration::days(1),
            Some(now + Duration::hours(1)),
            "REVIEW_REQUIRED",
        )
        .await;
    let current = t
        .enrollment(org, now, Some(now + Duration::hours(1)), "REVIEW_REQUIRED")
        .await;
    let upload = t.company_upload(current.1).await;
    let accepted = t
        .enrollment(org, now, Some(now + Duration::hours(1)), "MET")
        .await;
    t.company_upload(accepted.1).await;

    let queue = t.request("GET", "/api/business/review", Value::Null).await;
    let expired_review = t
        .request(
            "POST",
            &business_review_path(org, expired),
            review_input(Uuid::new_v4()),
        )
        .await;
    let null_review = t
        .request(
            "POST",
            &business_review_path(org, null_deadline),
            review_input(Uuid::new_v4()),
        )
        .await;
    let archived_review = t
        .request(
            "POST",
            &business_review_path(archived_org, archived),
            review_input(Uuid::new_v4()),
        )
        .await;
    let current_review = t
        .request(
            "POST",
            &business_review_path(org, current),
            review_input(upload),
        )
        .await;
    t.finish().await;

    assert_eq!(queue.0, StatusCode::OK, "{}", queue.1);
    let rows = queue.1["enrollments"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        2,
        "Closed review windows must not occupy the queue"
    );
    assert!(rows.iter().any(|row| row["id"] == current.1.to_string()));
    assert!(rows.iter().any(|row| row["id"] == accepted.1.to_string()));
    for result in [expired_review, null_review] {
        assert_eq!(result.0, StatusCode::CONFLICT);
        assert_eq!(result.1["error"], "BUSINESS_REVIEW_CLOSED");
    }
    assert_eq!(archived_review.0, StatusCode::CONFLICT);
    assert_eq!(archived_review.1["error"], "WORKSPACE_ARCHIVED");
    assert_eq!(current_review.0, StatusCode::OK, "{}", current_review.1);
}

#[tokio::test]
async fn business_queue_keeps_accepted_legacy_reward_payable_after_review_deadline() {
    let t = TestApp::new().await;
    let participant = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,email,display_name,password_hash) SELECT $1,'accepted-legacy@example.test','Accepted participant',password_hash FROM users WHERE id=$2")
        .bind(participant).bind(t.user).execute(&t.pool).await.unwrap();
    let org = t.organization(false).await;
    let now = Utc::now();
    let (program, enrollment) = t
        .enrollment(org, now, Some(now - Duration::hours(1)), "MET")
        .await;
    sqlx::query("UPDATE company_programs SET funder_id=$1,reserved_units=5000000,published_at=now() WHERE id=$2")
        .bind(t.user).bind(program).execute(&t.pool).await.unwrap();
    sqlx::query("UPDATE company_enrollments SET user_id=$1 WHERE id=$2")
        .bind(participant)
        .bind(enrollment)
        .execute(&t.pool)
        .await
        .unwrap();
    let upload = t.company_upload(enrollment).await;
    sqlx::query("UPDATE company_uploads SET user_id=$1,decision='ACCEPTED',reason='DISTANCE_AND_WINDOW_MET' WHERE id=$2")
        .bind(participant).bind(upload).execute(&t.pool).await.unwrap();
    sqlx::query("INSERT INTO prototype_local_movements(id,user_id,action,wallet_delta,locked_delta,recipient_delta,issued_delta) VALUES($1,$2,'GRANT',5000000,0,0,-5000000)")
        .bind(Uuid::new_v4()).bind(t.user).execute(&t.pool).await.unwrap();
    sqlx::query("INSERT INTO prototype_local_movements(id,user_id,action,wallet_delta,locked_delta,recipient_delta,issued_delta,business_program_id) VALUES($1,$2,'BUSINESS_FUND',-5000000,5000000,0,0,$3)")
        .bind(Uuid::new_v4()).bind(t.user).bind(program).execute(&t.pool).await.unwrap();

    let queue = t.request("GET", "/api/business/review", Value::Null).await;
    let detail = t
        .request(
            "GET",
            &format!("/api/organizations/{org}/programs/{program}"),
            Value::Null,
        )
        .await;
    let claim_path =
        format!("/api/organizations/{org}/programs/{program}/enrollments/{enrollment}/claim");
    let paid = t.request("POST", &claim_path, json!({})).await;
    let repeated = t.request("POST", &claim_path, json!({})).await;
    let participant_available: i64 = sqlx::query_scalar(
        "SELECT COALESCE(sum(wallet_delta),0)::bigint FROM prototype_local_movements WHERE user_id=$1",
    ).bind(participant).fetch_one(&t.pool).await.unwrap();
    let movements: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM prototype_local_movements WHERE business_enrollment_id=$1",
    )
    .bind(enrollment)
    .fetch_one(&t.pool)
    .await
    .unwrap();
    let after_payment = t.request("GET", "/api/business/review", Value::Null).await;
    t.finish().await;

    assert_eq!(queue.0, StatusCode::OK, "{}", queue.1);
    assert!(
        queue.1["enrollments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == enrollment.to_string()),
        "Accepted unpaid legacy rewards remain actionable after the review deadline: {}",
        queue.1
    );
    assert_eq!(
        detail.1["closure"]["reason"],
        "BUSINESS_ACCEPTED_REWARDS_MUST_BE_PAID"
    );
    assert_eq!(paid.0, StatusCode::OK, "{}", paid.1);
    assert_eq!(paid.1["state"], "REWARDED");
    assert_eq!(repeated.0, StatusCode::OK, "{}", repeated.1);
    assert_eq!(participant_available, 5_000_000);
    assert_eq!(movements, 2, "One balanced transfer, including after retry");
    assert!(
        after_payment.1["enrollments"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

fn review_input(upload: Uuid) -> Value {
    json!({"upload_id":upload,"accept":true,"reason":"Reviewed the submitted evidence and accepted its plausibility."})
}

fn business_review_path(org: Uuid, (program, enrollment): (Uuid, Uuid)) -> String {
    format!("/api/organizations/{org}/programs/{program}/enrollments/{enrollment}/review")
}

struct TestApp {
    app: Router,
    pool: PgPool,
    admin: PgPool,
    schema: String,
    user: Uuid,
    cookie: String,
    csrf: String,
}

impl TestApp {
    async fn new() -> Self {
        let url = std::env::var("TEST_DATABASE_URL")
            .expect("TEST_DATABASE_URL required; review queue tests never silently skipped");
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
        let search_path = schema.clone();
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .after_connect(move |connection, _| {
                let search_path = search_path.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('search_path',$1,false)")
                        .bind(search_path)
                        .execute(connection)
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
        let user = Uuid::new_v4();
        let token = crypto::token().unwrap();
        let csrf = crypto::token().unwrap();
        sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,'review-operator@example.test','Review operator',$2)")
            .bind(user).bind(state.dummy_hash.as_str()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO prototype_operators(user_id) VALUES($1)")
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf_token,expires_at) VALUES($1,$2,$3,now()+interval '1 hour')")
            .bind(crypto::digest(&token)).bind(user).bind(&csrf).execute(&pool).await.unwrap();
        let cookie = format!("{}={token}", state.config.cookie_name());
        Self {
            app: router(state, "missing-dist"),
            pool,
            admin,
            schema,
            user,
            cookie,
            csrf,
        }
    }

    async fn request(&self, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:8787")
            .header("origin", "http://127.0.0.1:8787")
            .header("x-truhabit-request", "web")
            .header("content-type", "application/json")
            .header("cookie", &self.cookie)
            .header("x-csrf-token", &self.csrf)
            .body(Body::from(body.to_string()))
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:6000".parse::<std::net::SocketAddr>().unwrap(),
        ));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    async fn challenge(
        &self,
        created: DateTime<Utc>,
        refund: DateTime<Utc>,
        state: &str,
        assessment: &str,
    ) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO prototype_challenges(id,user_id,title,target_m,amount_units,profile,network,starts_at,ends_at,upload_deadline,refund_after,state,assessment,creation_hash,created_at) VALUES($1,$2,'Queue regression',3000,5000000,'REPLAY','LOCAL',$3,$4,$5,$6,$7,$8,'queue-test',$9)")
            .bind(id).bind(self.user).bind(refund-Duration::hours(3)).bind(refund-Duration::hours(2))
            .bind(refund-Duration::hours(1)).bind(refund).bind(state).bind(assessment).bind(created)
            .execute(&self.pool).await.unwrap();
        id
    }

    async fn prototype_upload(&self, challenge: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO prototype_uploads(id,challenge_id,user_id,file_hash,fingerprint,activity,goal_result,decision,reason) VALUES($1,$2,$3,$4,$4,'{}','MET','REVIEW_REQUIRED','ACTIVITY_REQUIRES_REVIEW')")
            .bind(id).bind(challenge).bind(self.user).bind(id.to_string()).execute(&self.pool).await.unwrap();
        id
    }

    async fn organization(&self, archived: bool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO organizations(id,owner_id,name,creation_hash,archived_at) VALUES($1,$2,'Queue organization','queue-test',$3)")
            .bind(id).bind(self.user).bind(archived.then(Utc::now)).execute(&self.pool).await.unwrap();
        id
    }

    async fn enrollment(
        &self,
        org: Uuid,
        created: DateTime<Utc>,
        deadline: Option<DateTime<Utc>>,
        assessment: &str,
    ) -> (Uuid, Uuid) {
        let program = Uuid::new_v4();
        let enrollment = Uuid::new_v4();
        sqlx::query("INSERT INTO company_programs(id,organization_id,title,target_m,currency,reward_minor,max_participants,state,creation_hash,reward_units,budget_units,profile,starts_at,ends_at,upload_deadline,review_deadline) VALUES($1,$2,'Queue program',3000,'CZK',25000,1,'PUBLISHED','queue-test',5000000,5000000,'REPLAY',$3,$4,$5,$6)")
            .bind(program).bind(org).bind(deadline.map(|d|d-Duration::hours(3)))
            .bind(deadline.map(|d|d-Duration::hours(2))).bind(deadline.map(|d|d-Duration::hours(1)))
            .bind(deadline).execute(&self.pool).await.unwrap();
        sqlx::query("INSERT INTO company_enrollments(id,program_id,user_id,assessment,reward_units,created_at) VALUES($1,$2,$3,$4,5000000,$5)")
            .bind(enrollment).bind(program).bind(self.user).bind(assessment).bind(created)
            .execute(&self.pool).await.unwrap();
        (program, enrollment)
    }

    async fn company_upload(&self, enrollment: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO company_uploads(id,enrollment_id,user_id,file_hash,fingerprint,activity,goal_result,decision,reason,received_at) VALUES($1,$2,$3,$4,$4,'{}','MET','REVIEW_REQUIRED','ACTIVITY_REQUIRES_REVIEW',now())")
            .bind(id).bind(enrollment).bind(self.user).bind(id.to_string()).execute(&self.pool).await.unwrap();
        id
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
