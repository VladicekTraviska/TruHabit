use crate::{
    AppState,
    auth::{Auth, lock_user, rate_limit},
    crypto,
    error::ApiError,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Serialize, Deserialize, FromRow, Debug)]
pub struct Goal {
    pub id: Uuid,
    pub user_id: Uuid,
    pub target_m: i32,
    pub pledge_cents: i32,
    pub currency: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub state: String,
    pub policy_version: String,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalInput {
    pub target_m: i32,
    pub pledge_cents: i32,
    pub starts_at: DateTime<Utc>,
    #[serde(default = "default_currency", skip_serializing_if = "is_usd")]
    pub currency: String,
}
fn default_currency() -> String {
    "USD".into()
}
fn is_usd(value: &String) -> bool {
    value == "USD"
}
fn validate(input: &GoalInput) -> Result<(), ApiError> {
    truhabit_rules::plan::validate_currency(
        input.target_m,
        input.pledge_cents,
        &input.currency,
        input.starts_at.timestamp(),
        Utc::now().timestamp(),
    )
    .map_err(|_| {
        ApiError::bad(
            "Cíl musí mít 1–5 km, plánovanou částku 50–1 000 Kč nebo 1–50 USD a začátek za 5 minut až 30 dní.",
        )
    })
}
#[derive(Deserialize)]
pub struct Page {
    after: Option<Uuid>,
}
pub async fn list(
    auth: Auth,
    State(state): State<AppState>,
    Query(page): Query<Page>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rows = if let Some(id) = page.after {
        let time: DateTime<Utc> =
            sqlx::query_scalar("SELECT created_at FROM goals WHERE id=$1 AND user_id=$2")
                .bind(id)
                .bind(auth.user.id)
                .fetch_optional(&state.pool)
                .await?
                .ok_or_else(ApiError::not_found)?;
        sqlx::query_as::<_,Goal>("SELECT * FROM goals WHERE user_id=$1 AND (created_at,id)<($2,$3) ORDER BY created_at DESC,id DESC LIMIT 51").bind(auth.user.id).bind(time).bind(id).fetch_all(&state.pool).await?
    } else {
        sqlx::query_as::<_, Goal>(
            "SELECT * FROM goals WHERE user_id=$1 ORDER BY created_at DESC,id DESC LIMIT 51",
        )
        .bind(auth.user.id)
        .fetch_all(&state.pool)
        .await?
    };
    let next = if rows.len() > 50 {
        Some(rows[49].id)
    } else {
        None
    };
    Ok(Json(
        serde_json::json!({"goals":rows.into_iter().take(50).collect::<Vec<_>>(),"next_cursor":next}),
    ))
}
pub async fn get(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let goal = sqlx::query_as::<_, Goal>("SELECT * FROM goals WHERE id=$1 AND user_id=$2")
        .bind(id)
        .bind(auth.user.id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let events: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT to_jsonb(e) FROM goal_events e WHERE goal_id=$1 AND actor_id=$2 ORDER BY id",
    )
    .bind(id)
    .bind(auth.user.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(serde_json::json!({"goal":goal,"events":events})))
}
pub async fn create(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<GoalInput>,
) -> Result<Json<Goal>, ApiError> {
    auth.csrf(&headers)?;
    rate_limit(&state, &format!("goals:{}", auth.user.id), 30, 3600).await?;
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or_else(|| ApiError::bad("Chybí UUID Idempotency-Key."))?;
    let hash = crypto::digest(serde_json::to_vec(&input).map_err(|_| ApiError::internal())?);
    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, auth.user.id).await?;
    if let Some((old, id)) = sqlx::query_as::<_, (String, Uuid)>(
        "SELECT request_hash,resource_id FROM idempotency_keys WHERE user_id=$1 AND key=$2",
    )
    .bind(auth.user.id)
    .bind(key)
    .fetch_optional(&mut *tx)
    .await?
    {
        if old != hash {
            return Err(ApiError::conflict(
                "Stejný klíč již patří jinému požadavku.",
            ));
        }
        let goal = sqlx::query_as::<_, Goal>("SELECT * FROM goals WHERE id=$1 AND user_id=$2")
            .bind(id)
            .bind(auth.user.id)
            .fetch_one(&mut *tx)
            .await?;
        return Ok(Json(goal));
    }
    // Validate after idempotency lookup: a delayed retry must recover an already-created goal.
    validate(&input)?;
    let goal=sqlx::query_as::<_,Goal>("INSERT INTO goals(id,user_id,target_m,pledge_cents,starts_at,ends_at,policy_version,currency) VALUES($1,$2,$3,$4,$5,$6,$7,$8) RETURNING *")
        .bind(Uuid::new_v4()).bind(auth.user.id).bind(input.target_m).bind(input.pledge_cents).bind(input.starts_at).bind(input.starts_at+Duration::hours(24)).bind(truhabit_rules::plan::POLICY_VERSION).bind(&input.currency).fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO goal_events(goal_id,actor_id,kind,detail) VALUES($1,$2,'created',$3)")
        .bind(goal.id)
        .bind(auth.user.id)
        .bind(serde_json::to_value(&input).map_err(|_| ApiError::internal())?)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO idempotency_keys(user_id,key,request_hash,resource_id) VALUES($1,$2,$3,$4)",
    )
    .bind(auth.user.id)
    .bind(key)
    .bind(hash)
    .bind(goal.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(goal))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    target_m: i32,
    pledge_cents: i32,
    starts_at: DateTime<Utc>,
    version: i32,
    #[serde(default = "default_currency")]
    currency: String,
}
pub async fn update(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Update>,
) -> Result<Json<Goal>, ApiError> {
    auth.csrf(&headers)?;
    let mut tx = state.pool.begin().await?;
    let old =
        sqlx::query_as::<_, Goal>("SELECT * FROM goals WHERE id=$1 AND user_id=$2 FOR UPDATE")
            .bind(id)
            .bind(auth.user.id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(ApiError::not_found)?;
    if old.state != "DRAFT" || old.version != input.version {
        return Err(ApiError::conflict(
            "Cíl se změnil nebo byl archivován. Načtěte ho znovu.",
        ));
    }
    if old.currency != input.currency {
        return Err(ApiError::bad(
            "Měnu existujícího cíle nelze změnit. Vytvořte nový cíl.",
        ));
    }
    let values = GoalInput {
        target_m: input.target_m,
        pledge_cents: input.pledge_cents,
        starts_at: input.starts_at,
        currency: input.currency,
    };
    validate(&values)?;
    let goal=sqlx::query_as::<_,Goal>("UPDATE goals SET target_m=$1,pledge_cents=$2,starts_at=$3,ends_at=$4,version=version+1,updated_at=now() WHERE id=$5 AND user_id=$6 RETURNING *")
        .bind(input.target_m).bind(input.pledge_cents).bind(input.starts_at).bind(input.starts_at+Duration::hours(24)).bind(id).bind(auth.user.id).fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO goal_events(goal_id,actor_id,kind,detail) VALUES($1,$2,'updated',$3)")
        .bind(id)
        .bind(auth.user.id)
        .bind(serde_json::to_value(values).map_err(|_| ApiError::internal())?)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(goal))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    version: i32,
}
pub async fn archive(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Archive>,
) -> Result<Json<Goal>, ApiError> {
    auth.csrf(&headers)?;
    let mut tx = state.pool.begin().await?;
    let old =
        sqlx::query_as::<_, Goal>("SELECT * FROM goals WHERE id=$1 AND user_id=$2 FOR UPDATE")
            .bind(id)
            .bind(auth.user.id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(ApiError::not_found)?;
    if old.state == "ARCHIVED" {
        return Ok(Json(old));
    }
    if old.version != input.version {
        return Err(ApiError::conflict(
            "Cíl se mezitím změnil. Obnovte jeho detail.",
        ));
    }
    let goal=sqlx::query_as::<_,Goal>("UPDATE goals SET state='ARCHIVED',version=version+1,updated_at=now() WHERE id=$1 AND user_id=$2 RETURNING *").bind(id).bind(auth.user.id).fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO goal_events(goal_id,actor_id,kind) VALUES($1,$2,'archived')")
        .bind(id)
        .bind(auth.user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(goal))
}
pub async fn fund(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    auth.csrf(&headers)?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM goals WHERE id=$1 AND user_id=$2)")
            .bind(id)
            .bind(auth.user.id)
            .fetch_one(&state.pool)
            .await?;
    if !exists {
        return Err(ApiError::not_found());
    }
    Err(ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "FUNDING_UNAVAILABLE",
        "Vklady zatím nejsou dostupné. Žádné peníze nebyly přijaty ani uzamčeny.",
    ))
}
