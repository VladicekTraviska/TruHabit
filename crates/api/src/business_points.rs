//! Simulated company benefit points. This ledger never touches personal credits or a chain signer.
use crate::{
    AppState,
    auth::{self, Auth},
    business::Enrollment,
    crypto,
    error::ApiError,
    organizations::{self, Organization, Program},
    prototype,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use chrono::{DateTime, Months, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(crate) const SCALE: i64 = 1_000_000;

pub(crate) async fn balances(
    tx: &mut Transaction<'_, Postgres>,
    org: Uuid,
    user: Uuid,
) -> Result<(i64, i64, i64, i64, i64), ApiError> {
    Ok(sqlx::query_as(
        "SELECT COALESCE(sum(pool_delta),0)::bigint,COALESCE(sum(reserved_delta),0)::bigint,
         COALESCE(sum(employee_delta) FILTER(WHERE kind='REWARD'),0)::bigint,
         COALESCE(sum(employee_delta) FILTER(WHERE user_id=$2),0)::bigint,
         COALESCE(sum(stake_delta) FILTER(WHERE user_id=$2),0)::bigint
         FROM business_point_movements WHERE organization_id=$1",
    )
    .bind(org)
    .bind(user)
    .fetch_one(&mut **tx)
    .await?)
}

async fn summary(
    tx: &mut Transaction<'_, Postgres>,
    org: &Organization,
    user: Uuid,
) -> Result<Value, ApiError> {
    let (pool, reserved, awarded, available, staked) = balances(tx, org.id, user).await?;
    let movements: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id',id,'kind',kind,'program_id',program_id,'enrollment_id',enrollment_id,
         'amount_points',CASE WHEN kind='STAKE_LOCK' THEN -employee_delta ELSE employee_delta END,
         'employee_delta',employee_delta,'stake_delta',stake_delta,'created_at',created_at)
         FROM business_point_movements WHERE organization_id=$1 AND user_id=$2
         ORDER BY created_at DESC,id DESC LIMIT 100",
    ).bind(org.id).bind(user).fetch_all(&mut **tx).await?;
    let mut result = json!({"unit":"POINTS","simulation":true,"real_money":false,
        "own_available_points":available,"own_staked_points":staked,"movements":movements});
    if org.role != "MEMBER" {
        result["pool_available_points"] = json!(pool);
        result["pool_reserved_points"] = json!(reserved);
        result["total_awarded_points"] = json!(awarded);
    }
    Ok(result)
}

pub async fn balance(
    auth: Auth,
    State(state): State<AppState>,
    Path(org): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    let mut tx = state.pool.begin().await?;
    let workspace = organizations::access(&mut tx, auth.user.id, org, false).await?;
    let result = summary(&mut tx, &workspace, auth.user.id).await?;
    tx.commit().await?;
    Ok(Json(result))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TopUp {
    id: Uuid,
    points: i64,
}

pub async fn top_up(
    auth: Auth,
    State(state): State<AppState>,
    Path(org): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<TopUp>,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    if input.id.is_nil() || !(1..=10_000_000).contains(&input.points) {
        return Err(ApiError::bad("INVALID_COMPANY_POINT_TOP_UP"));
    }
    auth::rate_limit(
        &state,
        &format!("company-points-top-up:{}", auth.user.id),
        30,
        3600,
    )
    .await?;
    let hash = crypto::digest(serde_json::to_vec(&input).map_err(|_| ApiError::internal())?);
    let mut tx = state.pool.begin().await?;
    auth::lock_user(&mut tx, auth.user.id).await?;
    let workspace = organizations::access(&mut tx, auth.user.id, org, true).await?;
    if workspace.role != "OWNER" {
        return Err(ApiError::forbidden());
    }
    let existing: Option<(Uuid, Option<Uuid>, String, Option<String>)> = sqlx::query_as(
        "SELECT organization_id,actor_id,kind,request_hash FROM business_point_movements WHERE id=$1",
    ).bind(input.id).fetch_optional(&mut *tx).await?;
    if let Some((old_org, actor, kind, old_hash)) = existing {
        if old_org != org
            || actor != Some(auth.user.id)
            || kind != "TOP_UP"
            || old_hash.as_deref() != Some(&hash)
        {
            return Err(ApiError::conflict("BUSINESS_POINT_ID_CONFLICT"));
        }
    } else {
        let issued: i64 = sqlx::query_scalar("SELECT COALESCE(-sum(issued_delta),0)::bigint FROM business_point_movements WHERE organization_id=$1")
            .bind(org).fetch_one(&mut *tx).await?;
        if issued + input.points > 100_000_000 {
            return Err(ApiError::bad("COMPANY_POINT_ISSUANCE_LIMIT"));
        }
        let inserted = sqlx::query("INSERT INTO business_point_movements(id,organization_id,actor_id,kind,pool_delta,issued_delta,request_hash) VALUES($1,$2,$3,'TOP_UP',$4,$5,$6) ON CONFLICT(id) DO NOTHING")
            .bind(input.id).bind(org).bind(auth.user.id).bind(input.points).bind(-input.points).bind(hash).execute(&mut *tx).await?;
        if inserted.rows_affected() != 1 {
            return Err(ApiError::conflict("BUSINESS_POINT_ID_CONFLICT"));
        }
        organizations::event(
            &mut tx,
            org,
            auth.user.id,
            "company_points_added",
            json!({"points":input.points,"simulation":true}),
        )
        .await?;
    }
    let result = summary(&mut tx, &workspace, auth.user.id).await?;
    tx.commit().await?;
    Ok(Json(result))
}

#[derive(Default)]
struct Delta {
    pool: i64,
    reserved: i64,
    employee: i64,
    stake: i64,
}
async fn record(
    tx: &mut Transaction<'_, Postgres>,
    p: &Program,
    e: Option<&Enrollment>,
    actor: Uuid,
    kind: &str,
    delta: Delta,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO business_point_movements(id,organization_id,program_id,enrollment_id,user_id,actor_id,kind,pool_delta,reserved_delta,employee_delta,stake_delta) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
        .bind(Uuid::new_v4()).bind(p.organization_id).bind(p.id).bind(e.map(|v|v.id))
        .bind(e.and_then(|v|v.user_id)).bind(actor).bind(kind).bind(delta.pool).bind(delta.reserved).bind(delta.employee).bind(delta.stake)
        .execute(&mut **tx).await?;
    Ok(())
}

pub(crate) async fn fund(
    tx: &mut Transaction<'_, Postgres>,
    p: &Program,
    actor: Uuid,
    points: i64,
) -> Result<(), ApiError> {
    if !(1..=10_000_000).contains(&points) {
        return Err(ApiError::bad("INVALID_BUSINESS_POINT_BUDGET"));
    }
    let available = balances(tx, p.organization_id, actor).await?.0;
    if available < points {
        return Err(ApiError::bad("INSUFFICIENT_COMPANY_POINTS"));
    }
    record(
        tx,
        p,
        None,
        actor,
        "FUND",
        Delta {
            pool: -points,
            reserved: points,
            ..Delta::default()
        },
    )
    .await
}

pub(crate) async fn pledge(
    tx: &mut Transaction<'_, Postgres>,
    p: &Program,
    e: &Enrollment,
    actor: Uuid,
) -> Result<(), ApiError> {
    if p.point_stake == 0 {
        return Ok(());
    }
    let available = balances(tx, p.organization_id, actor).await?.3;
    if available < p.point_stake {
        return Err(ApiError::bad("INSUFFICIENT_EMPLOYEE_POINTS"));
    }
    record(
        tx,
        p,
        Some(e),
        actor,
        "STAKE_LOCK",
        Delta {
            employee: -p.point_stake,
            stake: p.point_stake,
            ..Delta::default()
        },
    )
    .await
}

pub(crate) fn provisional(p: &Program, at: DateTime<Utc>) -> i64 {
    if !p.monthly_decline {
        return p.point_reward;
    }
    let (Some(start), Some(end)) = (p.starts_at, p.ends_at) else {
        return p.point_reward;
    };
    let duration = (end - start).num_milliseconds();
    if duration <= 0 {
        return 0;
    }
    let remaining = (end - at).num_milliseconds().clamp(0, duration);
    ((i128::from(p.point_reward) * i128::from(remaining)) / i128::from(duration)) as i64
}

pub(crate) fn enrollment_json(
    p: &Program,
    e: &Enrollment,
    now: DateTime<Utc>,
) -> Result<Value, ApiError> {
    let mut result = serde_json::to_value(e).map_err(|_| ApiError::internal())?;
    if p.uses_points() {
        result["provisional_points"] = json!(if e.state == "ENROLLED" {
            provisional(p, now)
        } else {
            0
        });
    }
    Ok(result)
}

pub(crate) async fn settle(
    tx: &mut Transaction<'_, Postgres>,
    p: &Program,
    enrollment_id: Uuid,
    actor: Uuid,
) -> Result<Value, ApiError> {
    let e: Enrollment = sqlx::query_as(
        "SELECT * FROM company_enrollments WHERE id=$1 AND program_id=$2 FOR UPDATE",
    )
    .bind(enrollment_id)
    .bind(p.id)
    .fetch_one(&mut **tx)
    .await?;
    if e.state == "REWARDED" {
        return Ok(
            json!({"ok":true,"state":"REWARDED","reward_paid":true,"awarded_points":e.awarded_points,"stake_returned_points":e.staked_points,"unit":"POINTS","simulation":true}),
        );
    }
    if p.state != "PUBLISHED"
        || e.state != "ENROLLED"
        || e.assessment != "MET"
        || e.user_id.is_none()
    {
        return Err(ApiError::conflict("BUSINESS_REWARD_NOT_AVAILABLE"));
    }
    let (received, ended): (DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
        "SELECT received_at,(activity->>'ends_at')::timestamptz FROM company_uploads
         WHERE enrollment_id=$1 AND decision='ACCEPTED' AND goal_result='MET' ORDER BY received_at,id LIMIT 1",
    ).bind(e.id).fetch_optional(&mut **tx).await?.ok_or_else(||ApiError::conflict("BUSINESS_REWARD_NOT_AVAILABLE"))?;
    let at = if p.profile.as_deref() == Some("REPLAY") {
        received
    } else {
        ended
    };
    let award = provisional(p, at);
    let maximum = e.reward_units / SCALE;
    if award < 0 || award > maximum || p.reserved_units < e.reward_units {
        return Err(ApiError::internal());
    }
    let unused = maximum - award;
    record(
        tx,
        p,
        Some(&e),
        actor,
        "REWARD",
        Delta {
            pool: unused,
            reserved: -maximum,
            employee: award,
            ..Delta::default()
        },
    )
    .await?;
    if e.staked_points > 0 {
        record(
            tx,
            p,
            Some(&e),
            actor,
            "STAKE_REFUND",
            Delta {
                employee: e.staked_points,
                stake: -e.staked_points,
                ..Delta::default()
            },
        )
        .await?;
    }
    sqlx::query("UPDATE company_enrollments SET state='REWARDED',paid_at=now(),awarded_points=$2 WHERE id=$1")
        .bind(e.id).bind(award).execute(&mut **tx).await?;
    sqlx::query("UPDATE company_programs SET reserved_units=reserved_units-$1,paid_units=paid_units+$2,returned_units=returned_units+$3,updated_at=now() WHERE id=$4")
        .bind(e.reward_units).bind(award*SCALE).bind(unused*SCALE).bind(p.id).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO company_enrollment_events(enrollment_id,actor_id,kind,detail) VALUES($1,$2,'points_awarded',$3)")
        .bind(e.id).bind(actor).bind(json!({"awarded_points":award,"stake_returned_points":e.staked_points,"simulation":true})).execute(&mut **tx).await?;
    organizations::event(
        tx,
        p.organization_id,
        actor,
        "points_awarded",
        json!({"program_id":p.id,"enrollment_id":e.id,"awarded_points":award,"simulation":true}),
    )
    .await?;
    Ok(
        json!({"ok":true,"state":"REWARDED","reward_paid":true,"awarded_points":award,"stake_returned_points":e.staked_points,"unit":"POINTS","simulation":true}),
    )
}

pub(crate) async fn release(
    tx: &mut Transaction<'_, Postgres>,
    p: &Program,
    actor: Uuid,
    points: i64,
) -> Result<(), ApiError> {
    let enrollments: Vec<Enrollment> = sqlx::query_as("SELECT * FROM company_enrollments WHERE program_id=$1 AND state='ENROLLED' AND staked_points>0 ORDER BY id FOR UPDATE")
        .bind(p.id).fetch_all(&mut **tx).await?;
    for e in enrollments {
        record(
            tx,
            p,
            Some(&e),
            actor,
            "STAKE_FORFEIT",
            Delta {
                pool: e.staked_points,
                stake: -e.staked_points,
                ..Delta::default()
            },
        )
        .await?;
        sqlx::query("INSERT INTO company_enrollment_events(enrollment_id,actor_id,kind,detail) VALUES($1,$2,'stake_forfeited',$3)")
            .bind(e.id).bind(actor).bind(json!({"points":e.staked_points,"destination":"company_pool","simulation":true})).execute(&mut **tx).await?;
    }
    record(
        tx,
        p,
        None,
        actor,
        "RELEASE",
        Delta {
            pool: points,
            reserved: -points,
            ..Delta::default()
        },
    )
    .await
}

pub(crate) async fn user_has_points(
    tx: &mut Transaction<'_, Postgres>,
    org: Uuid,
    user: Uuid,
) -> Result<bool, ApiError> {
    let (_, _, _, available, staked) = balances(tx, org, user).await?;
    Ok(available > 0 || staked > 0)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NextCycle {
    id: Uuid,
    version: i32,
}
pub async fn next_cycle(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(input): Json<NextCycle>,
) -> Result<Json<Program>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    if input.id.is_nil() || input.id == id {
        return Err(ApiError::bad("BUSINESS_ID_REQUIRED"));
    }
    let hash = crypto::digest(format!(
        "next-company-cycle:{org}:{id}:{}:{}",
        input.id, input.version
    ));
    let mut tx = state.pool.begin().await?;
    auth::lock_user(&mut tx, auth.user.id).await?;
    organizations::access(&mut tx, auth.user.id, org, true).await?;
    let previous: Program = sqlx::query_as(
        "SELECT * FROM company_programs WHERE organization_id=$1 AND id=$2 FOR UPDATE",
    )
    .bind(org)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::not_found)?;
    let old: Option<(Uuid, Option<String>)> = sqlx::query_as(
        "SELECT organization_id,cycle_creation_hash FROM company_programs WHERE id=$1",
    )
    .bind(input.id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((old_org, old_hash)) = old {
        if old_org != org || old_hash.as_deref() != Some(&hash) {
            return Err(ApiError::conflict("BUSINESS_POINT_ID_CONFLICT"));
        }
        let saved = sqlx::query_as::<_, Program>("SELECT * FROM company_programs WHERE id=$1")
            .bind(input.id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(Json(saved));
    }
    if previous.template != "MONTHLY_BUDGET" || previous.published_at.is_none() {
        return Err(ApiError::conflict("BUSINESS_MONTHLY_CYCLE_REQUIRED"));
    }
    if previous.version != input.version {
        return Err(ApiError::conflict("BUSINESS_VERSION_CHANGED"));
    }
    let child: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM company_programs WHERE previous_cycle_id=$1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    if child.is_some() {
        return Err(ApiError::conflict("BUSINESS_NEXT_CYCLE_EXISTS"));
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM company_programs WHERE organization_id=$1")
            .bind(org)
            .fetch_one(&mut *tx)
            .await?;
    if count >= 100 {
        return Err(ApiError::bad("BUSINESS_PROGRAM_LIMIT"));
    }
    let start = previous
        .starts_at
        .and_then(|v| v.checked_add_months(Months::new(1)))
        .ok_or_else(|| ApiError::bad("INVALID_BUSINESS_CYCLE"))?;
    let end = previous
        .ends_at
        .and_then(|v| v.checked_add_months(Months::new(1)))
        .ok_or_else(|| ApiError::bad("INVALID_BUSINESS_CYCLE"))?;
    if end <= start {
        return Err(ApiError::bad("INVALID_BUSINESS_CYCLE"));
    }
    let next:Program=sqlx::query_as("INSERT INTO company_programs(id,organization_id,title,target_m,currency,reward_minor,max_participants,creation_hash,template,point_reward,point_stake,monthly_decline,previous_cycle_id,cycle_starts_at,cycle_ends_at,cycle_creation_hash) VALUES($1,$2,$3,$4,'CZK',100,$5,$6,'MONTHLY_BUDGET',$7,0,true,$8,$9,$10,$6) ON CONFLICT(id) DO NOTHING RETURNING *")
        .bind(input.id).bind(org).bind(&previous.title).bind(previous.target_m).bind(previous.max_participants).bind(&hash).bind(previous.point_reward).bind(id).bind(start).bind(end)
        .fetch_optional(&mut *tx).await?.ok_or_else(||ApiError::conflict("BUSINESS_POINT_ID_CONFLICT"))?;
    organizations::event(
        &mut tx,
        org,
        auth.user.id,
        "monthly_cycle_prepared",
        json!({"program_id":next.id,"previous_cycle_id":id,"simulation":true}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(next))
}
