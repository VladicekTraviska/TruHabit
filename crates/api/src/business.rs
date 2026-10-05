//! Company prototype: voluntary participation and employer-funded points/legacy LOCAL rewards.
//! Fiat planning fields never enter this ledger. Company roles never grant source/biometry access.
use crate::{
    AppState,
    auth::{self, Auth},
    business_points, crypto,
    error::ApiError,
    organizations::{self, Organization, Program},
    prototype,
};
use axum::{
    Json,
    extract::{FromRequest, FromRequestParts, Path, Query, Request, State},
    http::HeaderMap,
    response::IntoResponse,
};
use chrono::{DateTime, Duration, SubsecRound, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Acquire, FromRow, Postgres, Transaction};
use uuid::Uuid;

#[derive(Serialize, FromRow, Clone)]
pub struct Enrollment {
    pub id: Uuid,
    pub program_id: Uuid,
    pub user_id: Option<Uuid>,
    pub state: String,
    pub assessment: String,
    pub reward_units: i64,
    pub created_at: DateTime<Utc>,
    pub paid_at: Option<DateTime<Utc>>,
    pub staked_points: i64,
    pub awarded_points: i64,
    pub consented_at: Option<DateTime<Utc>>,
    pub terms_version: Option<i32>,
}
async fn program(
    tx: &mut Transaction<'_, Postgres>,
    org: Uuid,
    id: Uuid,
) -> Result<Program, ApiError> {
    sqlx::query_as("SELECT * FROM company_programs WHERE organization_id=$1 AND id=$2 FOR UPDATE")
        .bind(org)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(ApiError::not_found)
}
async fn enrollment(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    program: Uuid,
) -> Result<Enrollment, ApiError> {
    sqlx::query_as("SELECT * FROM company_enrollments WHERE id=$1 AND program_id=$2 FOR UPDATE")
        .bind(id)
        .bind(program)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(ApiError::not_found)
}
async fn lock_people(tx: &mut Transaction<'_, Postgres>, a: Uuid, b: Uuid) -> Result<(), ApiError> {
    lock_users(tx, vec![a, b]).await
}
async fn lock_users(
    tx: &mut Transaction<'_, Postgres>,
    mut ids: Vec<Uuid>,
) -> Result<(), ApiError> {
    ids.sort();
    ids.dedup();
    for id in ids {
        auth::lock_user(tx, id).await?;
    }
    Ok(())
}
async fn operator_org(tx: &mut Transaction<'_, Postgres>, org: Uuid) -> Result<(), ApiError> {
    let found: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
            .bind(org)
            .fetch_optional(&mut **tx)
            .await?;
    found.ok_or_else(ApiError::not_found)?;
    Ok(())
}
async fn active_operator_org(
    tx: &mut Transaction<'_, Postgres>,
    org: Uuid,
) -> Result<(), ApiError> {
    let archived: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT archived_at FROM organizations WHERE id=$1 FOR UPDATE")
            .bind(org)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(ApiError::not_found)?;
    if archived.is_some() {
        return Err(ApiError::conflict("WORKSPACE_ARCHIVED"));
    }
    Ok(())
}
async fn private_access(
    tx: &mut Transaction<'_, Postgres>,
    actor: Uuid,
    org: Uuid,
    id: Uuid,
    enrol: Uuid,
    op: bool,
) -> Result<(Program, Enrollment, bool), ApiError> {
    let p = program(tx, org, id).await?;
    let e = enrollment(tx, enrol, id).await?;
    if e.user_id != Some(actor) && !op {
        return Err(ApiError::not_found());
    }
    Ok((p, e, op))
}
async fn enrollment_event(
    tx: &mut Transaction<'_, Postgres>,
    enrol: Uuid,
    actor: Uuid,
    kind: &str,
    detail: Value,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO company_enrollment_events(enrollment_id,actor_id,kind,detail) VALUES($1,$2,$3,$4)")
        .bind(enrol).bind(actor).bind(kind).bind(detail).execute(&mut **tx).await?;
    Ok(())
}
pub(crate) async fn organization_data(
    tx: &mut Transaction<'_, Postgres>,
    org: &Organization,
    user: Uuid,
) -> Result<Value, ApiError> {
    let admin = org.role != "MEMBER";
    let members: Vec<Value> = if admin {
        sqlx::query_scalar("SELECT jsonb_build_object('user_id',u.id,'display_name',u.display_name,'email',u.email,'role',CASE WHEN u.id=o.owner_id THEN 'OWNER' ELSE m.role END) FROM organizations o JOIN users u ON u.id=o.owner_id OR EXISTS(SELECT 1 FROM organization_members x WHERE x.organization_id=o.id AND x.user_id=u.id) LEFT JOIN organization_members m ON m.organization_id=o.id AND m.user_id=u.id WHERE o.id=$1 ORDER BY (u.id=o.owner_id) DESC,u.display_name,u.id LIMIT 1001").bind(org.id).fetch_all(&mut **tx).await?
    } else {
        vec![]
    };
    let invitations: Vec<Value> = if admin {
        sqlx::query_scalar("SELECT to_jsonb(i)-'token_hash'-'creation_hash'-'accepted_by' FROM organization_invitations i WHERE organization_id=$1 ORDER BY created_at DESC LIMIT 100").bind(org.id).fetch_all(&mut **tx).await?
    } else {
        vec![]
    };
    let enrollments:Vec<Value>=sqlx::query_scalar("SELECT to_jsonb(e) FROM company_enrollments e JOIN company_programs p ON p.id=e.program_id WHERE p.organization_id=$1 AND e.user_id=$2 ORDER BY e.created_at").bind(org.id).bind(user).fetch_all(&mut **tx).await?;
    Ok(json!({"members":members,"invitations":invitations,"enrollments":enrollments}))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Invite {
    id: Uuid,
    email: String,
    role: String,
}
pub async fn invite(
    auth: Auth,
    State(state): State<AppState>,
    Path(org): Path<Uuid>,
    headers: HeaderMap,
    Json(mut input): Json<Invite>,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    auth::rate_limit(
        &state,
        &format!("business-invite:{}", auth.user.id),
        30,
        3600,
    )
    .await?;
    input.email = input.email.trim().to_lowercase();
    if input.id.is_nil()
        || !crypto::valid_email(&input.email)
        || !matches!(input.role.as_str(), "ADMIN" | "MEMBER")
    {
        return Err(ApiError::bad("INVALID_BUSINESS_INVITATION"));
    }
    let hash = crypto::digest(serde_json::to_vec(&input).map_err(|_| ApiError::internal())?);
    let mut tx = state.pool.begin().await?;
    auth::lock_user(&mut tx, auth.user.id).await?;
    let o = organizations::access(&mut tx, auth.user.id, org, true).await?;
    if input.role == "ADMIN" && o.role != "OWNER" {
        return Err(ApiError::forbidden());
    }
    let old:Option<(String,Value)>=sqlx::query_as("SELECT creation_hash,to_jsonb(i)-'token_hash'-'creation_hash'-'accepted_by' FROM organization_invitations i WHERE id=$1 AND organization_id=$2").bind(input.id).bind(org).fetch_optional(&mut *tx).await?;
    if let Some((prior, invite)) = old {
        if prior != hash {
            return Err(ApiError::conflict("BUSINESS_ID_CONFLICT"));
        }
        return Ok(Json(
            json!({"invitation":invite,"token":null,"manual_delivery":true}),
        ));
    }
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM organization_invitations WHERE organization_id=$1 AND status='OPEN' AND expires_at>now()").bind(org).fetch_one(&mut *tx).await?;
    if count >= 100 {
        return Err(ApiError::bad("BUSINESS_INVITATION_LIMIT"));
    }
    // Never check whether the email is registered: invitation creation is not an account directory.
    let token = crypto::token()?;
    let saved:Option<Value>=sqlx::query_scalar("INSERT INTO organization_invitations(id,organization_id,email,role,token_hash,creation_hash) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(id) DO NOTHING RETURNING to_jsonb(organization_invitations)-'token_hash'-'creation_hash'-'accepted_by'").bind(input.id).bind(org).bind(input.email).bind(input.role).bind(crypto::digest(&token)).bind(hash).fetch_optional(&mut *tx).await?;
    let saved = saved.ok_or_else(|| ApiError::conflict("BUSINESS_ID_CONFLICT"))?;
    organizations::event(
        &mut tx,
        org,
        auth.user.id,
        "invitation_created",
        json!({"invitation_id":input.id}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"invitation":saved,"token":token,"manual_delivery":true}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Accept {
    token: String,
}
pub async fn accept_invitation(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Accept>,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    auth::rate_limit(
        &state,
        &format!("business-accept:{}", auth.user.id),
        20,
        3600,
    )
    .await?;
    if input.token.len() != 64 || !input.token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::not_found());
    }
    let mut tx = state.pool.begin().await?;
    auth::lock_user(&mut tx, auth.user.id).await?;
    let org: Option<Uuid> = sqlx::query_scalar(
        "SELECT organization_id FROM organization_invitations WHERE token_hash=$1 AND email=$2",
    )
    .bind(crypto::digest(&input.token))
    .bind(&auth.user.email)
    .fetch_optional(&mut *tx)
    .await?;
    let org = org.ok_or_else(ApiError::not_found)?;
    operator_org(&mut tx, org).await?;
    let (id,role,status,expires,accepted):(Uuid,String,String,DateTime<Utc>,Option<Uuid>)=sqlx::query_as("SELECT id,role,status,expires_at,accepted_by FROM organization_invitations WHERE token_hash=$1 AND organization_id=$2 AND email=$3 FOR UPDATE").bind(crypto::digest(&input.token)).bind(org).bind(&auth.user.email).fetch_optional(&mut *tx).await?.ok_or_else(ApiError::not_found)?;
    if status == "ACCEPTED" && accepted == Some(auth.user.id) {
        return Ok(Json(json!({"ok":true,"organization_id":org})));
    }
    if status != "OPEN" || expires <= Utc::now() {
        return Err(ApiError::not_found());
    }
    active_operator_org(&mut tx, org).await?;
    let owner: Uuid = sqlx::query_scalar("SELECT owner_id FROM organizations WHERE id=$1")
        .bind(org)
        .fetch_one(&mut *tx)
        .await?;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM organization_members WHERE organization_id=$1")
            .bind(org)
            .fetch_one(&mut *tx)
            .await?;
    if count >= 1000 {
        return Err(ApiError::bad("BUSINESS_MEMBER_LIMIT"));
    }
    if owner != auth.user.id {
        sqlx::query("INSERT INTO organization_members(organization_id,user_id,role) VALUES($1,$2,$3) ON CONFLICT(organization_id,user_id) DO NOTHING").bind(org).bind(auth.user.id).bind(role).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE organization_invitations SET status='ACCEPTED',accepted_by=$1 WHERE id=$2")
        .bind(auth.user.id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    organizations::event(
        &mut tx,
        org,
        auth.user.id,
        "invitation_accepted",
        json!({"invitation_id":id}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true,"organization_id":org})))
}
pub async fn revoke_invitation(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    let mut tx = state.pool.begin().await?;
    auth::lock_user(&mut tx, auth.user.id).await?;
    let o = organizations::access(&mut tx, auth.user.id, org, true).await?;
    let (role,status):(String,String)=sqlx::query_as("SELECT role,status FROM organization_invitations WHERE organization_id=$1 AND id=$2 FOR UPDATE").bind(org).bind(id).fetch_optional(&mut *tx).await?.ok_or_else(ApiError::not_found)?;
    if role == "ADMIN" && o.role != "OWNER" {
        return Err(ApiError::forbidden());
    }
    if status == "ACCEPTED" {
        return Err(ApiError::conflict("INVITATION_ALREADY_ACCEPTED"));
    }
    if status == "OPEN" {
        sqlx::query("UPDATE organization_invitations SET status='REVOKED' WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        organizations::event(
            &mut tx,
            org,
            auth.user.id,
            "invitation_revoked",
            json!({"invitation_id":id}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Role {
    role: String,
}
pub async fn set_role(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, user)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(input): Json<Role>,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    if !matches!(input.role.as_str(), "ADMIN" | "MEMBER") {
        return Err(ApiError::bad("INVALID_BUSINESS_ROLE"));
    }
    let mut tx = state.pool.begin().await?;
    lock_people(&mut tx, auth.user.id, user).await?;
    let o = organizations::access(&mut tx, auth.user.id, org, true).await?;
    if o.role != "OWNER" || user == o.owner_id {
        return Err(ApiError::forbidden());
    }
    let result = sqlx::query(
        "UPDATE organization_members SET role=$1 WHERE organization_id=$2 AND user_id=$3",
    )
    .bind(&input.role)
    .bind(org)
    .bind(user)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    organizations::event(
        &mut tx,
        org,
        auth.user.id,
        "member_role_changed",
        json!({"user_id":user,"role":input.role}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
pub async fn remove_member(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, user)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    let mut tx = state.pool.begin().await?;
    lock_people(&mut tx, auth.user.id, user).await?;
    let o = organizations::access(&mut tx, auth.user.id, org, true).await?;
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM organization_members WHERE organization_id=$1 AND user_id=$2",
    )
    .bind(org)
    .bind(user)
    .fetch_optional(&mut *tx)
    .await?;
    let role = role.ok_or_else(ApiError::not_found)?;
    if user == o.owner_id || (role == "ADMIN" && o.role != "OWNER") {
        return Err(ApiError::forbidden());
    }
    let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM company_enrollments e JOIN company_programs p ON p.id=e.program_id WHERE p.organization_id=$1 AND e.user_id=$2 AND p.state='PUBLISHED') OR EXISTS(SELECT 1 FROM company_programs WHERE organization_id=$1 AND funder_id=$2 AND state='PUBLISHED')").bind(org).bind(user).fetch_one(&mut *tx).await?;
    if active {
        return Err(ApiError::conflict("ACTIVE_BUSINESS_ENROLLMENT"));
    }
    if business_points::user_has_points(&mut tx, org, user).await? {
        return Err(ApiError::conflict("EMPLOYEE_POINTS_MUST_REMAIN_ACCESSIBLE"));
    }
    sqlx::query("DELETE FROM organization_members WHERE organization_id=$1 AND user_id=$2")
        .bind(org)
        .bind(user)
        .execute(&mut *tx)
        .await?;
    organizations::event(
        &mut tx,
        org,
        auth.user.id,
        "member_removed",
        json!({"user_id":user}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transfer {
    user_id: Uuid,
    version: i32,
}
pub async fn transfer_owner(
    auth: Auth,
    State(state): State<AppState>,
    Path(org): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Transfer>,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    let mut tx = state.pool.begin().await?;
    lock_people(&mut tx, auth.user.id, input.user_id).await?;
    let o = organizations::access(&mut tx, auth.user.id, org, true).await?;
    if o.role != "OWNER" {
        return Err(ApiError::forbidden());
    }
    if input.user_id == o.owner_id {
        return Ok(Json(json!({"ok":true})));
    }
    if input.version != o.version {
        return Err(ApiError::conflict("BUSINESS_VERSION_CHANGED"));
    }
    let member: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM organization_members WHERE organization_id=$1 AND user_id=$2)",
    )
    .bind(org)
    .bind(input.user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !member {
        return Err(ApiError::not_found());
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM organizations WHERE owner_id=$1 AND archived_at IS NULL",
    )
    .bind(input.user_id)
    .fetch_one(&mut *tx)
    .await?;
    if count >= 10 {
        return Err(ApiError::bad("BUSINESS_OWNERSHIP_LIMIT"));
    }
    sqlx::query("DELETE FROM organization_members WHERE organization_id=$1 AND user_id=$2")
        .bind(org)
        .bind(input.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO organization_members(organization_id,user_id,role) VALUES($1,$2,'ADMIN') ON CONFLICT(organization_id,user_id) DO UPDATE SET role='ADMIN'").bind(org).bind(auth.user.id).execute(&mut *tx).await?;
    sqlx::query("UPDATE organizations SET owner_id=$1,version=version+1 WHERE id=$2")
        .bind(input.user_id)
        .bind(org)
        .execute(&mut *tx)
        .await?;
    organizations::event(
        &mut tx,
        org,
        auth.user.id,
        "owner_transferred",
        json!({"new_owner_id":input.user_id}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}

pub async fn detail(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    let op = prototype::operator(&state, auth.user.id).await?;
    let mut tx = state.pool.begin().await?;
    let role = if op {
        operator_org(&mut tx, org).await?;
        "OPERATOR".to_string()
    } else {
        organizations::access(&mut tx, auth.user.id, org, false)
            .await?
            .role
    };
    let p = program(&mut tx, org, id).await?;
    if p.published_at.is_none() && role == "MEMBER" {
        return Err(ApiError::not_found());
    }
    let mut participants: Vec<Value> = if role != "MEMBER" {
        sqlx::query_scalar("SELECT to_jsonb(e)||jsonb_build_object('display_name',COALESCE(u.display_name,'Deleted account')) FROM company_enrollments e LEFT JOIN users u ON u.id=e.user_id WHERE program_id=$1 ORDER BY e.created_at LIMIT 10000").bind(id).fetch_all(&mut *tx).await?
    } else {
        vec![]
    };
    if p.uses_points() {
        let provisional = business_points::provisional(&p, Utc::now());
        for participant in &mut participants {
            participant["provisional_points"] = json!(if participant["state"] == "ENROLLED" {
                provisional
            } else {
                0
            });
        }
    }
    let own: Option<Enrollment> =
        sqlx::query_as("SELECT * FROM company_enrollments WHERE program_id=$1 AND user_id=$2")
            .bind(id)
            .bind(auth.user.id)
            .fetch_optional(&mut *tx)
            .await?;
    let organization:Value=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'name',name,'owner_id',owner_id,'version',version,'archived_at',archived_at,'role',$2::text) FROM organizations WHERE id=$1")
        .bind(org).bind(&role).fetch_one(&mut *tx).await?;
    let closure = closure(&mut tx, &p, !organization["archived_at"].is_null()).await?;
    tx.commit().await?;
    let own = own
        .as_ref()
        .map(|e| business_points::enrollment_json(&p, e, Utc::now()))
        .transpose()?;
    let mut response = json!({"organization":organization,"program":p,"participants":participants,"current_user_enrollment":own,"participant_count":closure.participant_count,"is_operator":op,"role":role,"network":"LOCAL","real_money":false,"server_now":Utc::now()});
    if p.uses_points() {
        response["unit"] = json!("POINTS");
        response["simulation"] = json!(true);
    }
    if role != "MEMBER" {
        response["closure"] = serde_json::to_value(closure).map_err(|_| ApiError::internal())?;
    }
    Ok(Json(response))
}
#[derive(Serialize)]
struct Closure {
    allowed: bool,
    reason: Option<&'static str>,
    checked_at: DateTime<Utc>,
    available_at: Option<DateTime<Utc>>,
    unpaid_rewards: i64,
    pending_reviews: i64,
    participant_count: i64,
    rewarded_count: i64,
}
async fn closure(
    tx: &mut Transaction<'_, Postgres>,
    p: &Program,
    archived: bool,
) -> Result<Closure, ApiError> {
    let (count,unpaid,review,rewarded):(i64,i64,i64,i64)=sqlx::query_as("SELECT count(*),count(*) FILTER(WHERE state='ENROLLED' AND assessment='MET'),count(*) FILTER(WHERE state='ENROLLED' AND assessment='REVIEW_REQUIRED'),count(*) FILTER(WHERE state='REWARDED') FROM company_enrollments WHERE program_id=$1")
        .bind(p.id).fetch_one(&mut **tx).await?;
    let now = Utc::now();
    let before_start = p.starts_at.is_some_and(|start| now < start) && count == 0;
    let completed = count == i64::from(p.max_participants) && rewarded == count;
    let (reason, available_at) = if archived {
        (Some("WORKSPACE_ARCHIVED"), None)
    } else if p.state != "PUBLISHED" {
        (Some("BUSINESS_PROGRAM_NOT_PUBLISHED"), None)
    } else if !before_start
        && !completed
        && p.upload_deadline.is_none_or(|deadline| now <= deadline)
    {
        (Some("BUSINESS_UPLOAD_WINDOW_OPEN"), p.upload_deadline)
    } else if unpaid > 0 {
        (Some("BUSINESS_ACCEPTED_REWARDS_MUST_BE_PAID"), None)
    } else if review > 0 && p.review_deadline.is_some_and(|deadline| now < deadline) {
        (Some("BUSINESS_REVIEW_PENDING"), p.review_deadline)
    } else {
        (None, None)
    };
    Ok(Closure {
        allowed: reason.is_none(),
        reason,
        checked_at: now,
        available_at,
        unpaid_rewards: unpaid,
        pending_reviews: review,
        participant_count: count,
        rewarded_count: rewarded,
    })
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Publish {
    version: i32,
    #[serde(default)]
    reward_units: i64,
    profile: String,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    upload_deadline: DateTime<Utc>,
    review_deadline: DateTime<Utc>,
}
pub async fn publish(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(input): Json<Publish>,
) -> Result<Json<Program>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    let mut tx = state.pool.begin().await?;
    auth::lock_user(&mut tx, auth.user.id).await?;
    let access = organizations::access(&mut tx, auth.user.id, org, true).await?;
    if access.role != "OWNER" {
        return Err(ApiError::forbidden());
    }
    let p = program(&mut tx, org, id).await?;
    let reward_units = if p.uses_points() {
        p.point_reward * business_points::SCALE
    } else {
        input.reward_units
    };
    let now = Utc::now();
    let (start, end, upload, review) = if input.profile == "REPLAY" {
        let start = now.with_nanosecond(0).ok_or_else(ApiError::internal)?;
        (
            start,
            start + Duration::minutes(10),
            start + Duration::minutes(10),
            start + Duration::minutes(15),
        )
    } else {
        (
            input.starts_at.trunc_subsecs(6),
            input.ends_at.trunc_subsecs(6),
            input.upload_deadline.trunc_subsecs(6),
            input.review_deadline.trunc_subsecs(6),
        )
    };
    if (!p.uses_points() && !(1_000_000..=50_000_000).contains(&reward_units))
        || !matches!(input.profile.as_str(), "LIVE" | "REPLAY")
        || end <= start
        || end > start + Duration::days(if p.monthly_decline { 32 } else { 30 })
        || upload < end
        || upload > end + Duration::days(7)
        || review <= upload
        || review > upload + Duration::days(7)
    {
        return Err(ApiError::bad("INVALID_BUSINESS_TERMS"));
    }
    if p.state == "PUBLISHED"
        && p.funder_id == Some(auth.user.id)
        && p.reward_units == Some(reward_units)
        && p.profile.as_deref() == Some(&input.profile)
        && (input.profile == "REPLAY"
            || (p.starts_at == Some(start)
                && p.ends_at == Some(end)
                && p.upload_deadline == Some(upload)
                && p.review_deadline == Some(review)))
    {
        return Ok(Json(p));
    }
    // An already funded publication can be retried after its start. The future
    // start requirement only applies when publishing a new draft.
    if input.profile == "LIVE"
        && (start < now + Duration::minutes(5) || start > now + Duration::days(30))
    {
        return Err(ApiError::bad("INVALID_BUSINESS_TERMS"));
    }
    if p.state != "DRAFT" || p.version != input.version {
        return Err(ApiError::conflict("BUSINESS_VERSION_CHANGED"));
    }
    let budget = reward_units
        .checked_mul(i64::from(p.max_participants))
        .ok_or_else(|| ApiError::bad("INVALID_BUSINESS_BUDGET"))?;
    if budget
        > if p.uses_points() {
            10_000_000 * business_points::SCALE
        } else {
            1_000_000_000
        }
    {
        return Err(ApiError::bad("INVALID_BUSINESS_BUDGET"));
    }
    if p.uses_points() {
        business_points::fund(&mut tx, &p, auth.user.id, budget / business_points::SCALE).await?;
    } else {
        let available:i64=sqlx::query_scalar("SELECT COALESCE(sum(wallet_delta),0)::bigint FROM prototype_local_movements WHERE user_id=$1").bind(auth.user.id).fetch_one(&mut *tx).await?;
        if available < budget {
            return Err(ApiError::bad("INSUFFICIENT_SIMULATION_CREDITS"));
        }
        sqlx::query("INSERT INTO prototype_local_movements(id,user_id,action,wallet_delta,locked_delta,recipient_delta,issued_delta,business_program_id) VALUES($1,$2,'BUSINESS_FUND',$3,$4,0,0,$5)").bind(Uuid::new_v4()).bind(auth.user.id).bind(-budget).bind(budget).bind(id).execute(&mut *tx).await?;
    }
    let p=sqlx::query_as::<_,Program>("UPDATE company_programs SET state='PUBLISHED',version=version+1,updated_at=now(),reward_units=$1,budget_units=$2,funder_id=$3,profile=$4,starts_at=$5,ends_at=$6,upload_deadline=$7,review_deadline=$8,published_at=now() WHERE id=$9 RETURNING *").bind(reward_units).bind(budget).bind(auth.user.id).bind(input.profile).bind(start).bind(end).bind(upload).bind(review).bind(id).fetch_one(&mut *tx).await?;
    organizations::event(
        &mut tx,
        org,
        auth.user.id,
        "program_published",
        json!({"program_id":id,"budget_units":budget,"network":"LOCAL"}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(p))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Join {
    id: Uuid,
    #[serde(default)]
    accept_terms: bool,
}
pub async fn join(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(input): Json<Join>,
) -> Result<Json<Enrollment>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    if input.id.is_nil() {
        return Err(ApiError::bad("BUSINESS_ID_REQUIRED"));
    }
    let mut tx = state.pool.begin().await?;
    auth::lock_user(&mut tx, auth.user.id).await?;
    let workspace = organizations::access(&mut tx, auth.user.id, org, false).await?;
    if workspace.archived_at.is_some() {
        return Err(ApiError::conflict("WORKSPACE_ARCHIVED"));
    }
    let p = program(&mut tx, org, id).await?;
    if let Some(old) = sqlx::query_as::<_, Enrollment>(
        "SELECT * FROM company_enrollments WHERE program_id=$1 AND user_id=$2",
    )
    .bind(id)
    .bind(auth.user.id)
    .fetch_optional(&mut *tx)
    .await?
    {
        return Ok(Json(old));
    }
    let now = Utc::now();
    if matches!(p.template.as_str(), "EMPLOYER_MATCH" | "MONTHLY_BUDGET") && !input.accept_terms {
        return Err(ApiError::bad("BUSINESS_EXPLICIT_CONSENT_REQUIRED"));
    }
    if p.state != "PUBLISHED" || p.ends_at.is_none_or(|end| now >= end) {
        return Err(ApiError::conflict("BUSINESS_JOIN_CLOSED"));
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM company_enrollments WHERE program_id=$1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if count >= i64::from(p.max_participants) {
        return Err(ApiError::conflict("BUSINESS_PROGRAM_FULL"));
    }
    let reward = p.reward_units.ok_or_else(ApiError::internal)?;
    if p.reserved_units + reward + p.paid_units + p.returned_units > p.budget_units {
        return Err(ApiError::conflict("BUSINESS_BUDGET_EXHAUSTED"));
    }
    let saved=sqlx::query_as::<_,Enrollment>("INSERT INTO company_enrollments(id,program_id,user_id,reward_units,staked_points,consented_at,terms_version) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(id) DO NOTHING RETURNING *").bind(input.id).bind(id).bind(auth.user.id).bind(reward).bind(p.point_stake).bind(if p.uses_points() && input.accept_terms {Some(now)} else {None}).bind(if p.uses_points() {Some(p.version)} else {None}).fetch_optional(&mut *tx).await?.ok_or_else(||ApiError::conflict("BUSINESS_ID_CONFLICT"))?;
    if p.uses_points() {
        business_points::pledge(&mut tx, &p, &saved, auth.user.id).await?;
    }
    sqlx::query(
        "UPDATE company_programs SET reserved_units=reserved_units+$1,updated_at=now() WHERE id=$2",
    )
    .bind(reward)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    enrollment_event(
        &mut tx,
        saved.id,
        auth.user.id,
        "joined",
        json!({"network":"LOCAL","reward_units":reward}),
    )
    .await?;
    organizations::event(
        &mut tx,
        org,
        auth.user.id,
        "participant_joined",
        json!({"program_id":id,"enrollment_id":saved.id}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(saved))
}
pub async fn enrollment_detail(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, id, enrol)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    let op = prototype::operator(&state, auth.user.id).await?;
    let mut tx = state.pool.begin().await?;
    let (p, e, op) = private_access(&mut tx, auth.user.id, org, id, enrol, op).await?;
    let uploads:Vec<Value>=sqlx::query_scalar("SELECT to_jsonb(u)-'content'-'file_hash'-'fingerprint'-'user_id' FROM company_uploads u WHERE enrollment_id=$1 ORDER BY received_at,id").bind(enrol).fetch_all(&mut *tx).await?;
    let events:Vec<Value>=sqlx::query_scalar("SELECT to_jsonb(e)-'actor_id' FROM company_enrollment_events e WHERE enrollment_id=$1 ORDER BY id").bind(enrol).fetch_all(&mut *tx).await?;
    let e = business_points::enrollment_json(&p, &e, Utc::now())?;
    tx.commit().await?;
    Ok(Json(
        json!({"program":p,"enrollment":e,"uploads":uploads,"events":events,"is_operator":op,"network":"LOCAL","real_money":false}),
    ))
}

/// Admission owns the same DB locks used by program closure before reading the finite request body.
pub struct RecordedBusinessUpload(prototype::RecordedUpload);
impl RecordedBusinessUpload {
    pub fn received_at(&self) -> DateTime<Utc> {
        self.0.received_at
    }
}
impl FromRequest<AppState> for RecordedBusinessUpload {
    type Rejection = ApiError;
    async fn from_request(request: Request, state: &AppState) -> Result<Self, Self::Rejection> {
        prototype::enabled(state)?;
        let (mut parts, body) = request.into_parts();
        let actor = Auth::from_request_parts(&mut parts, state).await?;
        actor.csrf(&parts.headers)?;
        let Path((org, id, enrol)) =
            Path::<(Uuid, Uuid, Uuid)>::from_request_parts(&mut parts, state)
                .await
                .map_err(|_| ApiError::bad("INVALID_BUSINESS_PATH"))?;
        let mut tx = state.pool.begin().await?;
        auth::lock_user(&mut tx, actor.user.id).await?;
        let workspace = organizations::access(&mut tx, actor.user.id, org, false).await?;
        if workspace.archived_at.is_some() {
            return Err(ApiError::conflict("WORKSPACE_ARCHIVED"));
        }
        program(&mut tx, org, id).await?;
        let e = enrollment(&mut tx, enrol, id).await?;
        if e.user_id != Some(actor.user.id) {
            return Err(ApiError::not_found());
        }
        let admitted =
            prototype::RecordedUpload::read_admitted(Request::from_parts(parts, body), state, tx)
                .await?;
        Ok(Self(admitted))
    }
}
pub async fn upload(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, id, enrol)): Path<(Uuid, Uuid, Uuid)>,
    Query(query): Query<prototype::UploadQuery>,
    headers: HeaderMap,
    recorded: RecordedBusinessUpload,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    let (bytes, received, mut tx) = recorded.0.into_parts();
    let p = program(&mut tx, org, id).await?;
    let e = enrollment(&mut tx, enrol, id).await?;
    if e.user_id != Some(auth.user.id) {
        return Err(ApiError::not_found());
    }
    let hash = crypto::digest(&bytes);
    let session = query
        .session
        .map(i32::try_from)
        .transpose()
        .map_err(|_| ApiError::bad("FIT_SESSION_REQUIRED"))?;
    // Exact retries are readonly, including after payout/closure. The chosen FIT session is part of the command.
    let old:Option<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'duplicate',true,'decision',decision,'reason',reason) FROM company_uploads WHERE enrollment_id=$1 AND file_hash=$2 AND session_index IS NOT DISTINCT FROM $3").bind(enrol).bind(&hash).bind(session).fetch_optional(&mut *tx).await?;
    if let Some(old) = old {
        return Ok(Json(old));
    }
    let attempts:i32=sqlx::query_scalar("INSERT INTO rate_limits(key_hash,count,reset_at) VALUES($1,1,now()+interval '1 hour') ON CONFLICT(key_hash) DO UPDATE SET count=CASE WHEN rate_limits.reset_at<=now() THEN 1 ELSE rate_limits.count+1 END,reset_at=CASE WHEN rate_limits.reset_at<=now() THEN excluded.reset_at ELSE rate_limits.reset_at END RETURNING count")
        .bind(crypto::digest(format!("business-upload:{}",auth.user.id))).fetch_one(&mut *tx).await?;
    if attempts > 20 {
        tx.commit().await?;
        return Err(ApiError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "RATE_LIMITED",
            "Příliš mnoho pokusů. Zkuste to prosím později.",
        ));
    }
    let mut work = tx.begin().await?;
    let result = process_upload(
        &state,
        &mut work,
        &p,
        &e,
        auth.user.id,
        BusinessIntake {
            bytes,
            received,
            session,
            hash,
        },
    )
    .await;
    if result.is_ok() {
        work.commit().await?;
    } else {
        work.rollback().await?;
    }
    tx.commit().await?;
    result
}

struct BusinessIntake {
    bytes: axum::body::Bytes,
    received: DateTime<Utc>,
    session: Option<i32>,
    hash: String,
}
async fn process_upload(
    state: &AppState,
    tx: &mut Transaction<'_, Postgres>,
    p: &Program,
    e: &Enrollment,
    user: Uuid,
    input: BusinessIntake,
) -> Result<Json<Value>, ApiError> {
    let BusinessIntake {
        bytes,
        received,
        session,
        hash,
    } = input;
    let enrol = e.id;
    if p.state != "PUBLISHED"
        || e.state != "ENROLLED"
        || p.upload_deadline.is_none_or(|deadline| received > deadline)
    {
        return Err(ApiError::conflict("BUSINESS_UPLOAD_CLOSED"));
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM company_uploads WHERE enrollment_id=$1")
            .bind(enrol)
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
    let saved = bytes.clone();
    let activity = tokio::task::spawn_blocking(move || {
        truhabit_evidence::upload::parse(&bytes, session.map(|v| v as usize))
    })
    .await
    .map_err(|_| ApiError::internal())?
    .map_err(|e| ApiError::bad(e.0))?;
    if activity.ends_at > received {
        return Err(ApiError::bad("ACTIVITY_IN_FUTURE"));
    }
    let in_window = p.profile.as_deref() == Some("REPLAY")
        || (Some(activity.starts_at) >= p.starts_at && Some(activity.ends_at) <= p.ends_at);
    let goal = if in_window && activity.distance_m >= f64::from(p.target_m) {
        "MET"
    } else {
        "NOT_MET"
    };
    if goal == "MET" {
        let used:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM company_uploads WHERE user_id=$1 AND goal_result='MET' AND (file_hash=$2 OR fingerprint=$3)) OR EXISTS(SELECT 1 FROM prototype_uploads WHERE user_id=$1 AND goal_result='MET' AND (file_hash=$2 OR fingerprint=$3))").bind(user).bind(&hash).bind(&activity.fingerprint).fetch_one(&mut **tx).await?;
        if used {
            return Err(ApiError::conflict("ACTIVITY_ALREADY_USED"));
        }
    }
    let suspicious = activity
        .reasons
        .iter()
        .any(|reason| reason != "MANUAL_UPLOAD_UNVERIFIED");
    let (decision, reason) = if goal == "NOT_MET" {
        (
            "REJECTED",
            if in_window {
                "DISTANCE_NOT_MET"
            } else {
                "OUTSIDE_ACTIVITY_WINDOW"
            },
        )
    } else if suspicious {
        ("REVIEW_REQUIRED", "ACTIVITY_REQUIRES_REVIEW")
    } else {
        ("ACCEPTED", "DISTANCE_AND_WINDOW_MET")
    };
    let upload = Uuid::new_v4();
    sqlx::query("INSERT INTO company_uploads(id,enrollment_id,user_id,file_hash,fingerprint,session_index,content,activity,goal_result,decision,reason,received_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)").bind(upload).bind(enrol).bind(user).bind(hash).bind(&activity.fingerprint).bind(session).bind(saved.to_vec()).bind(serde_json::to_value(activity).map_err(|_|ApiError::internal())?).bind(goal).bind(decision).bind(reason).bind(received).execute(&mut **tx).await?;
    recalculate(tx, enrol).await?;
    enrollment_event(
        tx,
        enrol,
        user,
        "activity_uploaded",
        json!({"upload_id":upload,"decision":decision,"reason":reason}),
    )
    .await?;
    let mut result = json!({"id":upload,"decision":decision,"reason":reason,"duplicate":false});
    if p.uses_points() && decision == "ACCEPTED" {
        let settlement = business_points::settle(tx, p, enrol, user).await?;
        for key in [
            "reward_paid",
            "awarded_points",
            "stake_returned_points",
            "unit",
            "simulation",
        ] {
            result[key] = settlement[key].clone();
        }
    }
    Ok(Json(result))
}
async fn recalculate(tx: &mut Transaction<'_, Postgres>, enrol: Uuid) -> Result<(), ApiError> {
    sqlx::query("UPDATE company_enrollments SET assessment=CASE WHEN EXISTS(SELECT 1 FROM company_uploads WHERE enrollment_id=$1 AND decision='ACCEPTED') THEN 'MET' WHEN EXISTS(SELECT 1 FROM company_uploads WHERE enrollment_id=$1 AND decision='REVIEW_REQUIRED') THEN 'REVIEW_REQUIRED' ELSE 'NOT_MET' END WHERE id=$1").bind(enrol).execute(&mut **tx).await?;
    Ok(())
}
pub async fn review_queue(
    auth: Auth,
    State(state): State<AppState>,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    if !prototype::operator(&state, auth.user.id).await? {
        return Err(ApiError::forbidden());
    }
    let enrollments:Vec<Value>=sqlx::query_scalar("SELECT to_jsonb(e)||jsonb_build_object('organization_id',p.organization_id,'program_title',p.title,'display_name',u.display_name) FROM company_enrollments e JOIN company_programs p ON p.id=e.program_id JOIN users u ON u.id=e.user_id WHERE p.state='PUBLISHED' AND e.state='ENROLLED' AND e.assessment IN ('MET','REVIEW_REQUIRED') ORDER BY e.created_at LIMIT 100").fetch_all(&state.pool).await?;
    Ok(Json(json!({"enrollments":enrollments})))
}
pub async fn review(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, id, enrol)): Path<(Uuid, Uuid, Uuid)>,
    headers: HeaderMap,
    Json(input): Json<prototype::Review>,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    if !prototype::operator(&state, auth.user.id).await? {
        return Err(ApiError::forbidden());
    }
    if !(5..=500).contains(&input.reason.trim().chars().count())
        || input.reason.chars().any(char::is_control)
    {
        return Err(ApiError::bad("REVIEW_REASON_REQUIRED"));
    }
    let mut tx = state.pool.begin().await?;
    let user: Uuid =
        sqlx::query_scalar("SELECT user_id FROM company_enrollments WHERE id=$1 AND program_id=$2")
            .bind(enrol)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(ApiError::not_found)?;
    lock_people(&mut tx, user, auth.user.id).await?;
    active_operator_org(&mut tx, org).await?;
    let p = program(&mut tx, org, id).await?;
    let e = enrollment(&mut tx, enrol, id).await?;
    if p.state != "PUBLISHED"
        || e.state != "ENROLLED"
        || p.review_deadline
            .is_none_or(|deadline| Utc::now() >= deadline)
    {
        return Err(ApiError::conflict("BUSINESS_REVIEW_CLOSED"));
    }
    let goal: String = sqlx::query_scalar(
        "SELECT goal_result FROM company_uploads WHERE id=$1 AND enrollment_id=$2",
    )
    .bind(input.upload_id)
    .bind(enrol)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::not_found)?;
    if input.accept && goal != "MET" {
        return Err(ApiError::bad("CANNOT_OVERRIDE_GOAL_PARAMETERS"));
    }
    sqlx::query("UPDATE company_uploads SET decision=$1,reason=$2 WHERE id=$3")
        .bind(if input.accept { "ACCEPTED" } else { "REJECTED" })
        .bind(input.reason.trim())
        .bind(input.upload_id)
        .execute(&mut *tx)
        .await?;
    recalculate(&mut tx, enrol).await?;
    enrollment_event(
        &mut tx,
        enrol,
        auth.user.id,
        "manual_review",
        json!({"upload_id":input.upload_id,"accepted":input.accept,"reason":input.reason.trim()}),
    )
    .await?;
    if p.uses_points() {
        let assessment: String =
            sqlx::query_scalar("SELECT assessment FROM company_enrollments WHERE id=$1")
                .bind(enrol)
                .fetch_one(&mut *tx)
                .await?;
        if assessment == "MET" {
            let result = business_points::settle(&mut tx, &p, enrol, auth.user.id).await?;
            tx.commit().await?;
            return Ok(Json(result));
        }
    }
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(FromRow)]
struct RewardHint {
    funder_id: Option<Uuid>,
    user_id: Option<Uuid>,
    state: String,
    reward_units: i64,
    template: String,
}

pub async fn claim(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, id, enrol)): Path<(Uuid, Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    let op = prototype::operator(&state, auth.user.id).await?;
    let mut tx = state.pool.begin().await?;
    let pair:Option<RewardHint>=sqlx::query_as("SELECT p.funder_id,e.user_id,e.state,e.reward_units,p.template FROM company_enrollments e JOIN company_programs p ON p.id=e.program_id WHERE p.organization_id=$1 AND p.id=$2 AND e.id=$3").bind(org).bind(id).bind(enrol).fetch_optional(&mut *tx).await?;
    let RewardHint {
        funder_id: funder,
        user_id: user,
        state: current,
        reward_units: reward,
        template,
    } = pair.ok_or_else(ApiError::not_found)?;
    if user != Some(auth.user.id) && !op {
        return Err(ApiError::not_found());
    }
    if template != "LEGACY" {
        if current == "REWARDED" {
            let e: Enrollment =
                sqlx::query_as("SELECT * FROM company_enrollments WHERE id=$1 AND program_id=$2")
                    .bind(enrol)
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await?;
            tx.commit().await?;
            return Ok(Json(
                json!({"ok":true,"state":"REWARDED","reward_paid":true,"awarded_points":e.awarded_points,"stake_returned_points":e.staked_points,"unit":"POINTS","simulation":true}),
            ));
        }
        let user = user.ok_or_else(ApiError::not_found)?;
        lock_people(&mut tx, user, auth.user.id).await?;
        active_operator_org(&mut tx, org).await?;
        let p = program(&mut tx, org, id).await?;
        let result = business_points::settle(&mut tx, &p, enrol, auth.user.id).await?;
        tx.commit().await?;
        return Ok(Json(result));
    }
    if current == "REWARDED" {
        return Ok(Json(
            json!({"ok":true,"state":"REWARDED","amount_units":reward,"network":"LOCAL"}),
        ));
    }
    let funder = funder.ok_or_else(ApiError::not_found)?;
    let user = user.ok_or_else(ApiError::not_found)?;
    lock_users(&mut tx, vec![funder, user, auth.user.id]).await?;
    active_operator_org(&mut tx, org).await?;
    let p = program(&mut tx, org, id).await?;
    let e = enrollment(&mut tx, enrol, id).await?;
    if e.state == "REWARDED" {
        return Ok(Json(
            json!({"ok":true,"state":"REWARDED","amount_units":e.reward_units,"network":"LOCAL"}),
        ));
    }
    if p.state != "PUBLISHED"
        || e.state != "ENROLLED"
        || e.assessment != "MET"
        || p.reserved_units < e.reward_units
    {
        return Err(ApiError::conflict("BUSINESS_REWARD_NOT_AVAILABLE"));
    }
    // Two balanced rows in one transaction transfer LOCAL credits; no token mint or chain signer.
    for (recipient, action, wallet, locked, business) in [
        (funder, "BUSINESS_PAY", 0, -e.reward_units, e.reward_units),
        (user, "BUSINESS_REWARD", e.reward_units, 0, -e.reward_units),
    ] {
        sqlx::query("INSERT INTO prototype_local_movements(id,user_id,action,wallet_delta,locked_delta,recipient_delta,issued_delta,business_delta,business_program_id,business_enrollment_id) VALUES($1,$2,$3,$4,$5,0,0,$6,$7,$8)").bind(Uuid::new_v4()).bind(recipient).bind(action).bind(wallet).bind(locked).bind(business).bind(id).bind(enrol).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE company_enrollments SET state='REWARDED',paid_at=now() WHERE id=$1")
        .bind(enrol)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE company_programs SET reserved_units=reserved_units-$1,paid_units=paid_units+$1,updated_at=now() WHERE id=$2").bind(e.reward_units).bind(id).execute(&mut *tx).await?;
    enrollment_event(
        &mut tx,
        enrol,
        auth.user.id,
        "reward_paid",
        json!({"amount_units":e.reward_units,"network":"LOCAL"}),
    )
    .await?;
    organizations::event(&mut tx,org,auth.user.id,"reward_paid",json!({"program_id":id,"enrollment_id":enrol,"amount_units":e.reward_units,"network":"LOCAL"})).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"ok":true,"state":"REWARDED","amount_units":e.reward_units,"network":"LOCAL"}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Close {
    version: i32,
}
pub async fn close(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(input): Json<Close>,
) -> Result<Json<Program>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    let mut tx = state.pool.begin().await?;
    let hint: Option<(Option<Uuid>, String)> = sqlx::query_as(
        "SELECT funder_id,template FROM company_programs WHERE id=$1 AND organization_id=$2",
    )
    .bind(id)
    .bind(org)
    .fetch_optional(&mut *tx)
    .await?;
    let (funder, template) = hint.ok_or_else(ApiError::not_found)?;
    let point_program = template != "LEGACY";
    if funder.is_none() && !point_program {
        organizations::access(&mut tx, auth.user.id, org, true).await?;
        let p = program(&mut tx, org, id).await?;
        if matches!(p.state.as_str(), "CLOSED" | "ARCHIVED") && p.published_at.is_some() {
            return Ok(Json(p));
        }
        return Err(ApiError::not_found());
    }
    let funder = funder.unwrap_or(auth.user.id);
    if point_program {
        auth::lock_user(&mut tx, auth.user.id).await?;
    } else {
        lock_people(&mut tx, funder, auth.user.id).await?;
    }
    organizations::access(&mut tx, auth.user.id, org, true).await?;
    let p = program(&mut tx, org, id).await?;
    if p.state == "CLOSED" || p.state == "ARCHIVED" {
        return Ok(Json(p));
    }
    if p.state != "PUBLISHED" || p.version != input.version {
        return Err(ApiError::conflict("BUSINESS_VERSION_CHANGED"));
    }
    if let Some(reason) = closure(&mut tx, &p, false).await?.reason {
        return Err(ApiError::conflict(reason));
    }
    let release = p.budget_units - p.paid_units - p.returned_units;
    if point_program {
        business_points::release(&mut tx, &p, auth.user.id, release / business_points::SCALE)
            .await?;
    } else {
        sqlx::query("INSERT INTO prototype_local_movements(id,user_id,action,wallet_delta,locked_delta,recipient_delta,issued_delta,business_program_id) VALUES($1,$2,'BUSINESS_RELEASE',$3,$4,0,0,$5)").bind(Uuid::new_v4()).bind(funder).bind(release).bind(-release).bind(id).execute(&mut *tx).await?;
    }
    sqlx::query(
        "UPDATE company_enrollments SET state='CLOSED' WHERE program_id=$1 AND state='ENROLLED'",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let p=sqlx::query_as::<_,Program>("UPDATE company_programs SET state='CLOSED',reserved_units=0,returned_units=returned_units+$1,version=version+1,closed_at=now(),updated_at=now() WHERE id=$2 RETURNING *").bind(release).bind(id).fetch_one(&mut *tx).await?;
    organizations::event(
        &mut tx,
        org,
        auth.user.id,
        "program_closed",
        json!({"program_id":id,"returned_units":release,"network":"LOCAL"}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(p))
}
pub async fn source(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, id, enrol, upload)): Path<(Uuid, Uuid, Uuid, Uuid)>,
) -> Result<axum::response::Response, ApiError> {
    prototype::enabled(&state)?;
    let op = prototype::operator(&state, auth.user.id).await?;
    let mut tx = state.pool.begin().await?;
    private_access(&mut tx, auth.user.id, org, id, enrol, op).await?;
    let (content, activity): (Option<Vec<u8>>, Value) = sqlx::query_as(
        "SELECT content,activity FROM company_uploads WHERE id=$1 AND enrollment_id=$2",
    )
    .bind(upload)
    .bind(enrol)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::not_found)?;
    let bytes = content.ok_or_else(|| ApiError::conflict("SOURCE_FILE_REMOVED"))?;
    tx.commit().await?;
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "application/octet-stream"),
            (
                axum::http::header::CONTENT_DISPOSITION,
                if activity["format"] == "FIT" {
                    "attachment; filename=activity.fit"
                } else {
                    "attachment; filename=activity.gpx"
                },
            ),
        ],
        bytes,
    )
        .into_response())
}
pub async fn remove_source(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, id, enrol, upload)): Path<(Uuid, Uuid, Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    let mut tx = state.pool.begin().await?;
    auth::lock_user(&mut tx, auth.user.id).await?;
    // Own settled source erasure remains available in archived history; accounting is unchanged.
    operator_org(&mut tx, org).await?;
    let p = program(&mut tx, org, id).await?;
    let e = enrollment(&mut tx, enrol, id).await?;
    if e.user_id != Some(auth.user.id) {
        return Err(ApiError::not_found());
    }
    if p.state == "PUBLISHED" && e.state == "ENROLLED" {
        return Err(ApiError::conflict("SETTLE_BEFORE_FILE_REMOVAL"));
    }
    let changed=sqlx::query("UPDATE company_uploads SET content=NULL,content_deleted_at=now() WHERE enrollment_id=$1 AND id=$2 AND content IS NOT NULL").bind(enrol).bind(upload).execute(&mut *tx).await?;
    if changed.rows_affected() > 0 {
        enrollment_event(
            &mut tx,
            enrol,
            auth.user.id,
            "source_file_removed",
            json!({"upload_id":upload}),
        )
        .await?;
    } else {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM company_uploads WHERE enrollment_id=$1 AND id=$2)",
        )
        .bind(enrol)
        .bind(upload)
        .fetch_one(&mut *tx)
        .await?;
        if !exists {
            return Err(ApiError::not_found());
        }
    }
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
