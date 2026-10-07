//! Local Devnet prototype. Financial states are written only by the chain verifier.
use crate::{
    AppState,
    auth::{Auth, lock_user, rate_limit},
    crypto,
    error::ApiError,
};
use axum::{
    Json,
    body::Bytes,
    extract::{FromRequest, FromRequestParts, Path, Query, Request, State},
    http::HeaderMap,
};
use chrono::{DateTime, Duration, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Acquire, FromRow, Postgres, Transaction};
use uuid::Uuid;

pub fn enabled(state: &AppState) -> Result<(), ApiError> {
    if state.config.production {
        Err(ApiError::not_found())
    } else {
        Ok(())
    }
}
#[derive(Serialize, Deserialize, FromRow, Clone)]
pub struct Challenge {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: String,
    pub target_m: i32,
    pub amount_units: i64,
    pub profile: String,
    pub network: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub upload_deadline: DateTime<Utc>,
    pub refund_after: DateTime<Utc>,
    pub state: String,
    pub assessment: String,
    pub policy: String,
    pub chain: Option<Value>,
    #[serde(skip)]
    pub creation_hash: String,
    pub created_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub version: i32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub id: Uuid,
    pub title: String,
    pub target_m: i32,
    pub amount_units: i64,
    pub profile: String,
    pub network: String,
    pub starts_at: DateTime<Utc>,
}
pub async fn operator(state: &AppState, user: Uuid) -> Result<bool, ApiError> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM prototype_operators WHERE user_id=$1)")
            .bind(user)
            .fetch_one(&state.pool)
            .await?,
    )
}
pub async fn event(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    user: Uuid,
    kind: &str,
    detail: Value,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO prototype_events(challenge_id,actor_id,kind,detail) VALUES($1,$2,$3,$4)",
    )
    .bind(id)
    .bind(user)
    .bind(kind)
    .bind(detail)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
pub async fn owned(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    user: Uuid,
) -> Result<Challenge, ApiError> {
    sqlx::query_as("SELECT * FROM prototype_challenges WHERE id=$1 AND user_id=$2 FOR UPDATE")
        .bind(id)
        .bind(user)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(ApiError::not_found)
}
pub async fn list(auth: Auth, State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    enabled(&state)?;
    let challenges: Vec<Challenge> = sqlx::query_as(
        "SELECT * FROM prototype_challenges WHERE user_id=$1 ORDER BY created_at DESC LIMIT 100",
    )
    .bind(auth.user.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        json!({"challenges":challenges,"is_operator":operator(&state,auth.user.id).await?,"networks":["DEVNET","LOCAL"],"real_money":false}),
    ))
}
pub async fn create(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Create>,
) -> Result<Json<Challenge>, ApiError> {
    enabled(&state)?;
    auth.csrf(&headers)?;
    rate_limit(
        &state,
        &format!("prototype-create:{}", auth.user.id),
        30,
        3600,
    )
    .await?;
    let hash = crypto::digest(serde_json::to_vec(&input).map_err(|_| ApiError::internal())?);
    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, auth.user.id).await?;
    if let Some(old) =
        sqlx::query_as::<_, Challenge>("SELECT * FROM prototype_challenges WHERE id=$1")
            .bind(input.id)
            .fetch_optional(&mut *tx)
            .await?
    {
        if old.user_id != auth.user.id {
            return Err(ApiError::not_found());
        }
        if old.creation_hash != hash {
            return Err(ApiError::conflict("PROTOTYPE_ID_CONFLICT"));
        }
        return Ok(Json(old));
    }
    // Solana stores seconds. Freeze exactly the same instants in the database.
    let now = Utc::now()
        .with_nanosecond(0)
        .ok_or_else(ApiError::internal)?;
    if input.title.trim().is_empty()
        || !matches!(input.network.as_str(), "DEVNET" | "LOCAL")
        || input.title.chars().count() > 100
        || input.title.chars().any(char::is_control)
        || !(1000..=5000).contains(&input.target_m)
        || !(1_000_000..=50_000_000).contains(&input.amount_units)
        || !matches!(input.profile.as_str(), "LIVE" | "REPLAY")
    {
        return Err(ApiError::bad("INVALID_PROTOTYPE_PARAMETERS"));
    }
    let start = if input.profile == "REPLAY" {
        now
    } else {
        if input.starts_at < now + Duration::minutes(5)
            || input.starts_at > now + Duration::days(30)
        {
            return Err(ApiError::bad("START_MUST_BE_FUTURE"));
        }
        input
            .starts_at
            .with_nanosecond(0)
            .ok_or_else(ApiError::internal)?
    };
    let end = start
        + if input.profile == "REPLAY" {
            Duration::minutes(10)
        } else {
            Duration::days(1)
        };
    let upload = if input.profile == "REPLAY" {
        end
    } else {
        end + Duration::days(1)
    };
    let refund = upload
        + if input.profile == "REPLAY" {
            Duration::minutes(5)
        } else {
            Duration::days(1)
        };
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM prototype_challenges WHERE user_id=$1")
            .bind(auth.user.id)
            .fetch_one(&mut *tx)
            .await?;
    if count >= 100 {
        return Err(ApiError::bad("PROTOTYPE_LIMIT_REACHED"));
    }
    let c=sqlx::query_as::<_,Challenge>("INSERT INTO prototype_challenges(id,user_id,title,target_m,amount_units,profile,starts_at,ends_at,upload_deadline,refund_after,creation_hash,network) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) RETURNING *")
        .bind(input.id).bind(auth.user.id).bind(input.title.trim()).bind(input.target_m).bind(input.amount_units).bind(input.profile).bind(start).bind(end).bind(upload).bind(refund).bind(hash).bind(input.network).fetch_one(&mut *tx).await?;
    event(
        &mut tx,
        c.id,
        auth.user.id,
        "created",
        json!({"profile":c.profile,"policy":c.policy}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(c))
}
pub async fn detail(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    enabled(&state)?;
    let mut tx = state.pool.begin().await?;
    let c = sqlx::query_as::<_, Challenge>("SELECT * FROM prototype_challenges WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if c.user_id != auth.user.id && !operator(&state, auth.user.id).await? {
        return Err(ApiError::not_found());
    }
    let uploads:Vec<Value>=sqlx::query_scalar("SELECT to_jsonb(u)-'content'-'file_hash'-'fingerprint' FROM prototype_uploads u WHERE challenge_id=$1 ORDER BY received_at").bind(id).fetch_all(&mut *tx).await?;
    let events: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(e)-'actor_id' FROM prototype_events e WHERE challenge_id=$1 ORDER BY id",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    let commands:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'action',action,'status',status,'signature',signature,'created_at',created_at) FROM prototype_commands WHERE challenge_id=$1 ORDER BY created_at").bind(id).fetch_all(&mut *tx).await?;
    let review_request: Option<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(r)-'user_id' FROM prototype_review_requests r WHERE challenge_id=$1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"challenge":c,"uploads":uploads,"events":events,"commands":commands,"review_request":review_request}),
    ))
}
#[derive(Deserialize)]
pub struct UploadQuery {
    pub session: Option<usize>,
}
/// Admission holds the database locks before reading the body, including across
/// processes. A completed body therefore cannot lose to settlement while its
/// handler is waiting to be scheduled. Receipt is still the server's time after
/// the complete size-limited body, never a client-provided timestamp.
pub struct RecordedUpload {
    pub bytes: Bytes,
    pub received_at: DateTime<Utc>,
    admission: Transaction<'static, Postgres>,
}
impl RecordedUpload {
    /// Reuse for other evidence endpoints only after authenticating the caller
    /// and locking their funded record in the same order as its settlement.
    pub async fn read_admitted(
        request: Request,
        state: &AppState,
        admission: Transaction<'static, Postgres>,
    ) -> Result<Self, ApiError> {
        let bytes = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            Bytes::from_request(request, state),
        )
        .await
        .map_err(|_| {
            ApiError::new(
                axum::http::StatusCode::REQUEST_TIMEOUT,
                "UPLOAD_BODY_TIMEOUT",
                "UPLOAD_BODY_TIMEOUT",
            )
        })?
        .map_err(|error| ApiError::new(error.status(), "INVALID_UPLOAD_BODY", error.body_text()))?;
        Ok(Self {
            bytes,
            received_at: Utc::now(),
            admission,
        })
    }
    pub fn into_parts(self) -> (Bytes, DateTime<Utc>, Transaction<'static, Postgres>) {
        (self.bytes, self.received_at, self.admission)
    }
}
impl FromRequest<AppState> for RecordedUpload {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &AppState) -> Result<Self, Self::Rejection> {
        enabled(state)?;
        let (mut parts, body) = request.into_parts();
        let auth = Auth::from_request_parts(&mut parts, state).await?;
        auth.csrf(&parts.headers)?;
        let Path(id) = Path::<Uuid>::from_request_parts(&mut parts, state)
            .await
            .map_err(|_| ApiError::bad("INVALID_CHALLENGE_ID"))?;
        let mut tx = state.pool.begin().await?;
        lock_user(&mut tx, auth.user.id).await?;
        owned(&mut tx, id, auth.user.id).await?;
        Self::read_admitted(Request::from_parts(parts, body), state, tx).await
    }
}

fn upload_window_open(c: &Challenge, received: DateTime<Utc>) -> bool {
    c.state == "ACTIVE" && received <= c.upload_deadline && received < c.refund_after
}
pub async fn upload(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    recorded: RecordedUpload,
) -> Result<Json<Value>, ApiError> {
    enabled(&state)?;
    auth.csrf(&headers)?;
    let (bytes, received, mut tx) = recorded.into_parts();
    let c = owned(&mut tx, id, auth.user.id).await?;
    // A byte-identical retry only recovers an existing result; it admits no new
    // evidence and is safe even after settlement, cutoff, or exhausted limits.
    let hash = crypto::digest(&bytes);
    let prior: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM prototype_uploads WHERE challenge_id=$1 AND user_id=$2 AND file_hash=$3 AND session_index IS NOT DISTINCT FROM $4 ORDER BY received_at,id LIMIT 1",
    )
    .bind(id)
    .bind(auth.user.id)
    .bind(&hash)
    .bind(query.session.and_then(|index| i32::try_from(index).ok()))
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(upload) = prior.filter(|_| {
        query
            .session
            .is_none_or(|index| i32::try_from(index).is_ok())
    }) {
        tx.commit().await?;
        return Ok(Json(json!({"id":upload,"duplicate":true})));
    }
    // Use the same connection: taking another pool lease while holding the
    // locks would let simultaneous uploads exhaust the pool. Commit the counter
    // even when later validation fails, while rolling back only the savepoint.
    let count: i32 = sqlx::query_scalar("INSERT INTO rate_limits(key_hash,count,reset_at) VALUES($1,1,now()+interval '1 hour') ON CONFLICT(key_hash) DO UPDATE SET count=CASE WHEN rate_limits.reset_at<=now() THEN 1 ELSE rate_limits.count+1 END,reset_at=CASE WHEN rate_limits.reset_at<=now() THEN excluded.reset_at ELSE rate_limits.reset_at END RETURNING count")
        .bind(crypto::digest(format!("prototype-upload:{}", auth.user.id)))
        .fetch_one(&mut *tx).await?;
    if count > 20 {
        tx.commit().await?;
        return Err(ApiError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "RATE_LIMITED",
            "Příliš mnoho pokusů. Zkuste to prosím později.",
        ));
    }
    let mut work = tx.begin().await?;
    let result = process_upload(&state, &mut work, &c, received, bytes, query.session).await;
    if result.is_ok() {
        work.commit().await?;
    } else {
        work.rollback().await?;
    }
    tx.commit().await?;
    result
}

async fn process_upload(
    state: &AppState,
    tx: &mut Transaction<'_, Postgres>,
    c: &Challenge,
    received: DateTime<Utc>,
    bytes: Bytes,
    session: Option<usize>,
) -> Result<Json<Value>, ApiError> {
    let id = c.id;
    let user = c.user_id;
    if !upload_window_open(c, received) {
        return Err(ApiError::conflict("UPLOAD_WINDOW_CLOSED"));
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM prototype_uploads WHERE challenge_id=$1")
            .bind(id)
            .fetch_one(&mut **tx)
            .await?;
    if count >= 20 {
        return Err(ApiError::bad("UPLOAD_LIMIT_REACHED"));
    }
    let _permit = state
        .hashing
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::unavailable("PARSER_BUSY"))?;
    let hash = crypto::digest(&bytes);
    let saved = bytes.clone();
    let activity =
        tokio::task::spawn_blocking(move || truhabit_evidence::upload::parse(&bytes, session))
            .await
            .map_err(|_| ApiError::internal())?
            .map_err(|e| ApiError::bad(e.0))?;
    if activity.ends_at > received {
        return Err(ApiError::bad("ACTIVITY_IN_FUTURE"));
    }
    let pending: Option<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id,action,status FROM prototype_commands WHERE challenge_id=$1 AND status IN ('PREPARED','SIGNED') FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some((command, action, status)) = pending {
        if action != "FAILURE" || status != "PREPARED" {
            return Err(ApiError::conflict("SETTLEMENT_PENDING"));
        }
        // Recover an old prepared failure atomically with a timely valid body.
        // A signed envelope may have reached the network and cannot be revoked.
        sqlx::query("UPDATE prototype_commands SET status='FAILED' WHERE id=$1")
            .bind(command)
            .execute(&mut **tx)
            .await?;
        event(
            tx,
            id,
            user,
            "prepared_failure_invalidated_by_upload",
            json!({"command_id":command}),
        )
        .await?;
    }
    // A recorded attempt that cannot meet its immutable goal may be tried in a
    // separate challenge. Reserve qualifying evidence across the user's
    // challenges regardless of a later manual decision, and always prefer this
    // challenge's record so an old retry remains idempotent after such recovery.
    if let Some((other, upload)) = sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT challenge_id,id FROM prototype_uploads WHERE user_id=$1 AND (file_hash=$2 OR fingerprint=$3) AND (challenge_id=$4 OR goal_result='MET') ORDER BY (challenge_id=$4) DESC,received_at,id LIMIT 1",
    )
    .bind(user)
    .bind(&hash)
    .bind(&activity.fingerprint)
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?
    {
        if other == id {
            return Ok(Json(json!({"id":upload,"duplicate":true})));
        }
        return Err(ApiError::conflict("ACTIVITY_ALREADY_USED"));
    }
    let in_window = c.profile == "REPLAY"
        || (activity.starts_at >= c.starts_at && activity.ends_at <= c.ends_at);
    let goal = if in_window && activity.distance_m >= f64::from(c.target_m) {
        "MET"
    } else {
        "NOT_MET"
    };
    if goal == "MET" {
        let already_used: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM company_uploads WHERE user_id=$1 AND goal_result='MET' AND (file_hash=$2 OR fingerprint=$3))")
            .bind(user).bind(&hash).bind(&activity.fingerprint)
            .fetch_one(&mut **tx).await?;
        if already_used {
            return Err(ApiError::conflict("ACTIVITY_ALREADY_USED"));
        }
    }
    let suspicious = activity
        .reasons
        .iter()
        .any(|s| s != "MANUAL_UPLOAD_UNVERIFIED");
    let (decision, reason) = if goal == "NOT_MET" {
        (
            "REJECTED",
            if !in_window {
                "OUTSIDE_ACTIVITY_WINDOW"
            } else {
                "DISTANCE_NOT_MET"
            },
        )
    } else if suspicious {
        ("REVIEW_REQUIRED", "ACTIVITY_REQUIRES_REVIEW")
    } else {
        ("ACCEPTED", "DISTANCE_AND_WINDOW_MET")
    };
    let upload = Uuid::new_v4();
    sqlx::query("INSERT INTO prototype_uploads(id,challenge_id,user_id,file_hash,fingerprint,content,activity,goal_result,decision,reason,received_at,session_index) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
        .bind(upload).bind(id).bind(user).bind(hash).bind(&activity.fingerprint).bind(saved.to_vec()).bind(serde_json::to_value(&activity).map_err(|_|ApiError::internal())?).bind(goal).bind(decision).bind(reason).bind(received).bind(session.and_then(|index| i32::try_from(index).ok())).execute(&mut **tx).await?;
    recalculate(tx, id).await?;
    event(
        tx,
        id,
        user,
        "activity_uploaded",
        json!({"upload_id":upload,"decision":decision,"reason":reason}),
    )
    .await?;
    Ok(Json(
        json!({"id":upload,"decision":decision,"reason":reason}),
    ))
}
async fn recalculate(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<(), ApiError> {
    sqlx::query("UPDATE prototype_challenges SET assessment=CASE WHEN EXISTS(SELECT 1 FROM prototype_uploads WHERE challenge_id=$1 AND decision='ACCEPTED') THEN 'MET' WHEN EXISTS(SELECT 1 FROM prototype_uploads WHERE challenge_id=$1 AND decision='REVIEW_REQUIRED') THEN 'REVIEW_REQUIRED' ELSE 'NOT_MET' END,version=version+1 WHERE id=$1").bind(id).execute(&mut **tx).await?;
    Ok(())
}
pub async fn review_queue(
    auth: Auth,
    State(state): State<AppState>,
) -> Result<Json<Value>, ApiError> {
    enabled(&state)?;
    if !operator(&state, auth.user.id).await? {
        return Err(ApiError::forbidden());
    }
    // Active challenges also expose settlement verdicts, including those without uploads.
    let rows: Vec<Challenge> = sqlx::query_as(
        "SELECT * FROM prototype_challenges c
         WHERE (c.state='ACTIVE' AND c.refund_after>$1
                AND NOT EXISTS(SELECT 1 FROM prototype_commands p
                               WHERE p.challenge_id=c.id AND p.status IN ('PREPARED','SIGNED')))
            OR EXISTS(SELECT 1 FROM prototype_review_requests r
                      WHERE r.challenge_id=c.id AND r.status='OPEN')
         ORDER BY c.created_at,c.id LIMIT 100",
    )
    .bind(Utc::now())
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"challenges":rows})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub upload_id: Uuid,
    pub accept: bool,
    pub reason: String,
}
pub async fn review(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Review>,
) -> Result<Json<Value>, ApiError> {
    enabled(&state)?;
    auth.csrf(&headers)?;
    if !operator(&state, auth.user.id).await? {
        return Err(ApiError::forbidden());
    }
    if !(5..=500).contains(&input.reason.trim().chars().count())
        || input.reason.chars().any(char::is_control)
    {
        return Err(ApiError::bad("REVIEW_REASON_REQUIRED"));
    }
    let mut tx = state.pool.begin().await?;
    let owner: Uuid = sqlx::query_scalar("SELECT user_id FROM prototype_challenges WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(ApiError::not_found)?;
    lock_user(&mut tx, owner).await?;
    let c = owned(&mut tx, id, owner).await?;
    if c.state != "ACTIVE" || Utc::now() >= c.refund_after {
        return Err(ApiError::conflict("CHALLENGE_NOT_REVIEWABLE"));
    }
    let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM prototype_commands WHERE challenge_id=$1 AND status IN ('PREPARED','SIGNED'))").bind(id).fetch_one(&mut *tx).await?;
    if pending {
        return Err(ApiError::conflict("SETTLEMENT_PENDING"));
    }
    let goal: String = sqlx::query_scalar(
        "SELECT goal_result FROM prototype_uploads WHERE id=$1 AND challenge_id=$2",
    )
    .bind(input.upload_id)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::not_found)?;
    if input.accept && goal != "MET" {
        return Err(ApiError::bad("CANNOT_OVERRIDE_GOAL_PARAMETERS"));
    }
    sqlx::query("UPDATE prototype_uploads SET decision=$1,reason=$2 WHERE id=$3")
        .bind(if input.accept { "ACCEPTED" } else { "REJECTED" })
        .bind(input.reason.trim())
        .bind(input.upload_id)
        .execute(&mut *tx)
        .await?;
    recalculate(&mut tx, id).await?;
    event(
        &mut tx,
        id,
        auth.user.id,
        "manual_review",
        json!({"upload_id":input.upload_id,"accepted":input.accept,"reason":input.reason.trim()}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRequest {
    pub reason: String,
}

fn review_reason(reason: &str) -> Result<(), ApiError> {
    if !(5..=500).contains(&reason.trim().chars().count()) || reason.chars().any(char::is_control) {
        return Err(ApiError::bad("REVIEW_REASON_REQUIRED"));
    }
    Ok(())
}

/// A support request records a disputed outcome; it has no settlement authority.
pub async fn request_review(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<ReviewRequest>,
) -> Result<Json<Value>, ApiError> {
    enabled(&state)?;
    auth.csrf(&headers)?;
    review_reason(&input.reason)?;
    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, auth.user.id).await?;
    let c = owned(&mut tx, id, auth.user.id).await?;
    if !matches!(
        c.state.as_str(),
        "REFUNDED" | "FORFEITED" | "CANCELLED" | "EXPIRED"
    ) {
        return Err(ApiError::conflict("SETTLE_BEFORE_REVIEW_REQUEST"));
    }
    if let Some(old) = sqlx::query_scalar::<_, Value>(
        "SELECT to_jsonb(r)-'user_id' FROM prototype_review_requests r WHERE challenge_id=$1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    {
        return Ok(Json(old));
    }
    let saved:Value=sqlx::query_scalar("INSERT INTO prototype_review_requests(id,challenge_id,user_id,reason) VALUES($1,$2,$3,$4) RETURNING to_jsonb(prototype_review_requests)-'user_id'")
        .bind(Uuid::new_v4()).bind(id).bind(auth.user.id).bind(input.reason.trim()).fetch_one(&mut *tx).await?;
    event(
        &mut tx,
        id,
        auth.user.id,
        "review_requested",
        json!({"reason":input.reason.trim(),"settlement_unchanged":true}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(saved))
}

pub async fn respond_review(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<ReviewRequest>,
) -> Result<Json<Value>, ApiError> {
    enabled(&state)?;
    auth.csrf(&headers)?;
    review_reason(&input.reason)?;
    if !operator(&state, auth.user.id).await? {
        return Err(ApiError::forbidden());
    }
    let mut tx = state.pool.begin().await?;
    let owner: Uuid = sqlx::query_scalar("SELECT user_id FROM prototype_challenges WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(ApiError::not_found)?;
    lock_user(&mut tx, owner).await?;
    owned(&mut tx, id, owner).await?;
    let old:Value=sqlx::query_scalar("SELECT to_jsonb(r)-'user_id' FROM prototype_review_requests r WHERE challenge_id=$1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?.ok_or_else(ApiError::not_found)?;
    if old["status"] == "CLOSED" {
        return Ok(Json(old));
    }
    let saved:Value=sqlx::query_scalar("UPDATE prototype_review_requests SET status='CLOSED',resolution=$1,resolved_at=now() WHERE challenge_id=$2 RETURNING to_jsonb(prototype_review_requests)-'user_id'").bind(input.reason.trim()).bind(id).fetch_one(&mut *tx).await?;
    event(
        &mut tx,
        id,
        auth.user.id,
        "review_request_resolved",
        json!({"reason":input.reason.trim(),"settlement_unchanged":true}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(saved))
}

/// Source files are never exposed through the public static directory.
pub async fn source_file(
    auth: Auth,
    State(state): State<AppState>,
    Path((id, upload)): Path<(Uuid, Uuid)>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    enabled(&state)?;
    let row:(Uuid,Option<Vec<u8>>,Value)=sqlx::query_as("SELECT c.user_id,u.content,u.activity FROM prototype_uploads u JOIN prototype_challenges c ON c.id=u.challenge_id WHERE c.id=$1 AND u.id=$2").bind(id).bind(upload).fetch_optional(&state.pool).await?.ok_or_else(ApiError::not_found)?;
    if row.0 != auth.user.id && !operator(&state, auth.user.id).await? {
        return Err(ApiError::not_found());
    }
    let bytes = row
        .1
        .ok_or_else(|| ApiError::conflict("SOURCE_FILE_REMOVED"))?;
    let filename = if row.2["format"] == "FIT" {
        "attachment; filename=activity.fit"
    } else {
        "attachment; filename=activity.gpx"
    };
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "application/octet-stream"),
            (axum::http::header::CONTENT_DISPOSITION, filename),
        ],
        bytes,
    )
        .into_response())
}

pub async fn remove_source(
    auth: Auth,
    State(state): State<AppState>,
    Path((id, upload)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    enabled(&state)?;
    auth.csrf(&headers)?;
    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, auth.user.id).await?;
    let c = owned(&mut tx, id, auth.user.id).await?;
    if matches!(c.state.as_str(), "DRAFT" | "ACTIVE") {
        return Err(ApiError::conflict("SETTLE_BEFORE_FILE_REMOVAL"));
    }
    let row: (bool,) = sqlx::query_as(
        "SELECT content IS NULL FROM prototype_uploads WHERE id=$1 AND challenge_id=$2 FOR UPDATE",
    )
    .bind(upload)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::not_found)?;
    if !row.0 {
        sqlx::query(
            "UPDATE prototype_uploads SET content=NULL,content_deleted_at=now() WHERE id=$1",
        )
        .bind(upload)
        .execute(&mut *tx)
        .await?;
        event(
            &mut tx,
            id,
            auth.user.id,
            "source_file_removed",
            json!({"upload_id":upload}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
