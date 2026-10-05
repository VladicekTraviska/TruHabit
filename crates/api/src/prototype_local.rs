//! Explicit simulation ledger. Never passed to a chain signer or called a blockchain balance.
use crate::{
    AppState,
    auth::{Auth, lock_user},
    error::ApiError,
    prototype,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub async fn balance(auth: Auth, State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    let (available,locked,forfeited):(i64,i64,i64)=sqlx::query_as("SELECT COALESCE(sum(wallet_delta),0)::bigint,COALESCE(sum(locked_delta),0)::bigint,COALESCE(sum(recipient_delta),0)::bigint FROM prototype_local_movements WHERE user_id=$1").bind(auth.user.id).fetch_one(&state.pool).await?;
    Ok(Json(
        json!({"network":"LOCAL","token_label":"Simulation credits","available":available,"locked":locked,"forfeited":forfeited,"real_money":false}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub id: Uuid,
}
pub async fn grant(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Grant>,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, auth.user.id).await?;
    if let Some((user, action)) = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT user_id,action FROM prototype_local_movements WHERE id=$1",
    )
    .bind(input.id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if user != auth.user.id || action != "GRANT" {
            return Err(ApiError::conflict("PROTOTYPE_ID_CONFLICT"));
        }
        return Ok(Json(json!({"ok":true})));
    }
    let recent:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM prototype_local_movements WHERE user_id=$1 AND action='GRANT' AND created_at>now()-interval '24 hours')").bind(auth.user.id).fetch_one(&mut *tx).await?;
    if recent {
        return Err(ApiError::conflict("SIMULATION_GRANT_DAILY_LIMIT"));
    }
    sqlx::query("INSERT INTO prototype_local_movements(id,user_id,action,wallet_delta,locked_delta,recipient_delta,issued_delta) VALUES($1,$2,'GRANT',100000000,0,0,-100000000)").bind(input.id).bind(auth.user.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub action: String,
}
pub async fn action(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Action>,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    let op = prototype::operator(&state, auth.user.id).await?;
    let mut tx = state.pool.begin().await?;
    let owner: Uuid = sqlx::query_scalar("SELECT user_id FROM prototype_challenges WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if owner != auth.user.id && !op {
        return Err(ApiError::not_found());
    }
    lock_user(&mut tx, owner).await?;
    let c = prototype::owned(&mut tx, id, owner).await?;
    if c.network != "LOCAL" {
        return Err(ApiError::bad("WRONG_NETWORK"));
    }
    if input.action == "FAILURE" && !op {
        return Err(ApiError::forbidden());
    }
    let done:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM prototype_local_movements WHERE challenge_id=$1 AND action=$2)").bind(id).bind(&input.action).fetch_one(&mut *tx).await?;
    if done {
        return Ok(Json(json!({"ok":true,"state":c.state})));
    }
    let now = Utc::now();
    let next = match input.action.as_str() {
        "DEPOSIT" if c.state == "DRAFT" && owner == auth.user.id && now < c.ends_at => "ACTIVE",
        "SUCCESS"
            if c.state == "ACTIVE"
                && c.assessment == "MET"
                && now >= c.starts_at
                && now < c.refund_after =>
        {
            "REFUNDED"
        }
        "FAILURE"
            if c.state == "ACTIVE"
                && op
                && !matches!(c.assessment.as_str(), "MET" | "REVIEW_REQUIRED")
                && now > c.upload_deadline
                && now < c.refund_after =>
        {
            "FORFEITED"
        }
        "CANCEL" if c.state == "ACTIVE" && owner == auth.user.id && now < c.starts_at => {
            "CANCELLED"
        }
        "TIMEOUT" if c.state == "ACTIVE" && now >= c.refund_after => "EXPIRED",
        _ => return Err(ApiError::conflict("ACTION_NOT_AVAILABLE")),
    };
    let (wallet, locked, recipient) = if input.action == "DEPOSIT" {
        let available:i64=sqlx::query_scalar("SELECT COALESCE(sum(wallet_delta),0)::bigint FROM prototype_local_movements WHERE user_id=$1").bind(owner).fetch_one(&mut *tx).await?;
        if available < c.amount_units {
            return Err(ApiError::bad("INSUFFICIENT_SIMULATION_CREDITS"));
        }
        (-c.amount_units, c.amount_units, 0)
    } else if input.action == "FAILURE" {
        (0, -c.amount_units, c.amount_units)
    } else {
        (c.amount_units, -c.amount_units, 0)
    };
    sqlx::query("INSERT INTO prototype_local_movements(id,user_id,challenge_id,action,wallet_delta,locked_delta,recipient_delta,issued_delta) VALUES($1,$2,$3,$4,$5,$6,$7,0)").bind(Uuid::new_v4()).bind(owner).bind(id).bind(&input.action).bind(wallet).bind(locked).bind(recipient).execute(&mut *tx).await?;
    sqlx::query("UPDATE prototype_challenges SET state=$1,version=version+1,closed_at=CASE WHEN $1='ACTIVE' THEN NULL ELSE now() END WHERE id=$2").bind(next).bind(id).execute(&mut *tx).await?;
    prototype::event(&mut tx,id,auth.user.id,"simulation_transfer",json!({"action":input.action,"amount_units":c.amount_units,"recipient":if input.action=="FAILURE"{"simulated-recipient"}else{"owner"},"network":"LOCAL"})).await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true,"state":next})))
}
