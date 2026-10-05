use crate::{
    AppState,
    auth::{self, Auth},
    crypto,
    error::ApiError,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

#[derive(Serialize, FromRow)]
pub struct Organization {
    pub(crate) id: Uuid,
    pub(crate) owner_id: Uuid,
    name: String,
    pub(crate) version: i32,
    created_at: DateTime<Utc>,
    pub(crate) archived_at: Option<DateTime<Utc>>,
    pub(crate) role: String,
}
#[derive(Serialize, FromRow, Clone)]
pub struct Program {
    pub(crate) id: Uuid,
    pub(crate) organization_id: Uuid,
    pub(crate) title: String,
    pub(crate) target_m: i32,
    currency: String,
    reward_minor: i64,
    pub(crate) max_participants: i32,
    pub(crate) state: String,
    pub(crate) version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    pub(crate) reward_units: Option<i64>,
    pub(crate) budget_units: i64,
    pub(crate) reserved_units: i64,
    pub(crate) paid_units: i64,
    pub(crate) returned_units: i64,
    pub(crate) funder_id: Option<Uuid>,
    pub(crate) profile: Option<String>,
    pub(crate) starts_at: Option<DateTime<Utc>>,
    pub(crate) ends_at: Option<DateTime<Utc>>,
    pub(crate) upload_deadline: Option<DateTime<Utc>>,
    pub(crate) review_deadline: Option<DateTime<Utc>>,
    pub(crate) published_at: Option<DateTime<Utc>>,
    pub(crate) closed_at: Option<DateTime<Utc>>,
    pub(crate) template: String,
    pub(crate) point_reward: i64,
    pub(crate) point_stake: i64,
    pub(crate) monthly_decline: bool,
    pub(crate) previous_cycle_id: Option<Uuid>,
    pub(crate) cycle_starts_at: Option<DateTime<Utc>>,
    pub(crate) cycle_ends_at: Option<DateTime<Utc>>,
}
impl Program {
    pub(crate) fn uses_points(&self) -> bool {
        self.template != "LEGACY"
    }
}
fn title(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if !(2..=100).contains(&value.chars().count()) || value.chars().any(char::is_control) {
        return Err(ApiError::bad("Název musí mít 2 až 100 znaků."));
    }
    Ok(value.into())
}
pub(crate) async fn access(
    tx: &mut Transaction<'_, Postgres>,
    user: Uuid,
    id: Uuid,
    write: bool,
) -> Result<Organization, ApiError> {
    let org = sqlx::query_as::<_, Organization>(
        "SELECT o.*, CASE WHEN o.owner_id=$2 THEN 'OWNER' ELSE m.role END AS role
         FROM organizations o LEFT JOIN organization_members m ON m.organization_id=o.id AND m.user_id=$2
         WHERE o.id=$1 AND (o.owner_id=$2 OR m.user_id=$2) FOR UPDATE OF o")
        .bind(id).bind(user).fetch_optional(&mut **tx).await?.ok_or_else(ApiError::not_found)?;
    if write && org.role != "OWNER" && org.role != "ADMIN" {
        return Err(ApiError::forbidden());
    }
    if write && org.archived_at.is_some() {
        return Err(ApiError::conflict("WORKSPACE_ARCHIVED"));
    }
    Ok(org)
}
pub(crate) async fn event(
    tx: &mut Transaction<'_, Postgres>,
    org: Uuid,
    user: Uuid,
    kind: &str,
    detail: Value,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO organization_events(organization_id,actor_id,kind,detail) VALUES($1,$2,$3,$4)",
    )
    .bind(org)
    .bind(user)
    .bind(kind)
    .bind(detail)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
pub async fn list(auth: Auth, State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows = sqlx::query_as::<_, Organization>(
        "SELECT o.*, CASE WHEN o.owner_id=$1 THEN 'OWNER' ELSE m.role END AS role
         FROM organizations o LEFT JOIN organization_members m ON m.organization_id=o.id AND m.user_id=$1
         WHERE (o.owner_id=$1 OR m.user_id=$1) AND o.archived_at IS NULL ORDER BY o.created_at,o.id LIMIT 100")
        .bind(auth.user.id).fetch_all(&state.pool).await?;
    let archived = sqlx::query_as::<_, Organization>(
        "SELECT o.*, CASE WHEN o.owner_id=$1 THEN 'OWNER' ELSE m.role END AS role
         FROM organizations o LEFT JOIN organization_members m ON m.organization_id=o.id AND m.user_id=$1
         WHERE (o.owner_id=$1 OR m.user_id=$1) AND o.archived_at IS NOT NULL ORDER BY o.archived_at DESC,o.id LIMIT 100")
        .bind(auth.user.id).fetch_all(&state.pool).await?;
    Ok(Json(
        json!({"organizations":rows,"archived_organizations":archived,"is_operator":crate::prototype::operator(&state,auth.user.id).await?}),
    ))
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateOrganization {
    id: Uuid,
    name: String,
}
pub async fn create(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut input): Json<CreateOrganization>,
) -> Result<Json<Organization>, ApiError> {
    auth.csrf(&headers)?;
    auth::rate_limit(&state, &format!("org-create:{}", auth.user.id), 20, 3600).await?;
    input.name = title(&input.name)?;
    if input.id.is_nil() {
        return Err(ApiError::bad("Chybí identifikátor požadavku."));
    }
    let hash = crypto::digest(serde_json::to_vec(&input).map_err(|_| ApiError::internal())?);
    let mut tx = state.pool.begin().await?;
    auth::lock_user(&mut tx, auth.user.id).await?;
    let existing: Option<(Uuid, String)> =
        sqlx::query_as("SELECT owner_id,creation_hash FROM organizations WHERE id=$1")
            .bind(input.id)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some((owner, old)) = existing {
        if owner != auth.user.id {
            return Err(ApiError::not_found());
        }
        if old != hash {
            return Err(ApiError::conflict(
                "Identifikátor již patří jinému požadavku.",
            ));
        }
        return Ok(Json(access(&mut tx, auth.user.id, input.id, false).await?));
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM organizations WHERE owner_id=$1 AND archived_at IS NULL",
    )
    .bind(auth.user.id)
    .fetch_one(&mut *tx)
    .await?;
    if count >= 10 {
        return Err(ApiError::bad(
            "Jeden účet může vlastnit nejvýše 10 firemních prostorů.",
        ));
    }
    let inserted=sqlx::query("INSERT INTO organizations(id,owner_id,name,creation_hash) VALUES($1,$2,$3,$4) ON CONFLICT(id) DO NOTHING")
        .bind(input.id).bind(auth.user.id).bind(&input.name).bind(hash).execute(&mut *tx).await?;
    if inserted.rows_affected() != 1 {
        return Err(ApiError::conflict(
            "Identifikátor již patří jinému požadavku.",
        ));
    }
    event(
        &mut tx,
        input.id,
        auth.user.id,
        "organization_created",
        json!({"name":input.name}),
    )
    .await?;
    let org = access(&mut tx, auth.user.id, input.id, false).await?;
    tx.commit().await?;
    Ok(Json(org))
}
pub async fn get(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = state.pool.begin().await?;
    let org = access(&mut tx, auth.user.id, id, false).await?;
    let programs=sqlx::query_as::<_,Program>("SELECT * FROM company_programs WHERE organization_id=$1 AND ($2 OR published_at IS NOT NULL) ORDER BY created_at DESC,id DESC LIMIT 100")
        .bind(id).bind(org.role != "MEMBER").fetch_all(&mut *tx).await?;
    let events: Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'kind',kind,'detail',detail,'created_at',created_at) FROM organization_events WHERE organization_id=$1 ORDER BY id DESC LIMIT 50")
        .bind(id).fetch_all(&mut *tx).await?;
    let data = crate::business::organization_data(&mut tx, &org, auth.user.id).await?;
    let management = if org.role != "MEMBER" {
        Some(management(&mut tx, &org).await?)
    } else {
        None
    };
    tx.commit().await?;
    let mut response = json!({"organization":org,"programs":programs,"events":if org.role=="MEMBER" {vec![]} else {events},"members":data["members"],"invitations":data["invitations"],"current_user_enrollments":data["enrollments"],"network":"LOCAL","real_money":false});
    if let Some(management) = management {
        response["management"] = management;
    }
    Ok(Json(response))
}
async fn archive_reason(
    tx: &mut Transaction<'_, Postgres>,
    org: Uuid,
) -> Result<Option<&'static str>, ApiError> {
    let (published, reserved): (bool, bool) = sqlx::query_as("SELECT EXISTS(SELECT 1 FROM company_programs WHERE organization_id=$1 AND state='PUBLISHED'),EXISTS(SELECT 1 FROM company_programs WHERE organization_id=$1 AND (reserved_units>0 OR budget_units>paid_units+returned_units))")
        .bind(org).fetch_one(&mut **tx).await?;
    Ok(if published {
        Some("WORKSPACE_HAS_PUBLISHED_PROGRAMS")
    } else if reserved {
        Some("WORKSPACE_RESERVED_CREDITS")
    } else {
        None
    })
}
async fn delete_reason(
    tx: &mut Transaction<'_, Postgres>,
    org: &Organization,
) -> Result<Option<&'static str>, ApiError> {
    let (others, funded): (bool, bool) = sqlx::query_as("SELECT EXISTS(SELECT 1 FROM organization_members WHERE organization_id=$1 AND user_id<>$2),EXISTS(SELECT 1 FROM company_programs WHERE organization_id=$1 AND published_at IS NOT NULL) OR EXISTS(SELECT 1 FROM business_point_movements WHERE organization_id=$1)")
        .bind(org.id).bind(org.owner_id).fetch_one(&mut **tx).await?;
    Ok(if others {
        Some("WORKSPACE_HAS_OTHER_MEMBERS")
    } else if funded {
        Some("BUSINESS_HISTORY_MUST_BE_RETAINED")
    } else {
        None
    })
}
async fn management(
    tx: &mut Transaction<'_, Postgres>,
    org: &Organization,
) -> Result<Value, ApiError> {
    let role_reason = if org.role != "OWNER" {
        Some("OWNER_REQUIRED")
    } else if org.archived_at.is_some() {
        Some("WORKSPACE_ARCHIVED")
    } else {
        None
    };
    let delete = if role_reason.is_some() {
        role_reason
    } else {
        delete_reason(tx, org).await?
    };
    let archive = if role_reason.is_some() {
        role_reason
    } else {
        archive_reason(tx, org.id).await?
    };
    Ok(
        json!({"can_delete":delete.is_none(),"delete_reason":delete,"can_archive":archive.is_none(),"archive_reason":archive}),
    )
}
pub async fn archive(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Version>,
) -> Result<Json<Organization>, ApiError> {
    workspace_state(auth, state, id, headers, input, true).await
}
pub async fn restore(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Version>,
) -> Result<Json<Organization>, ApiError> {
    workspace_state(auth, state, id, headers, input, false).await
}
async fn workspace_state(
    auth: Auth,
    state: AppState,
    id: Uuid,
    headers: HeaderMap,
    input: Version,
    archived: bool,
) -> Result<Json<Organization>, ApiError> {
    auth.csrf(&headers)?;
    let mut tx = state.pool.begin().await?;
    auth::lock_user(&mut tx, auth.user.id).await?;
    let org = access(&mut tx, auth.user.id, id, false).await?;
    if org.role != "OWNER" {
        return Err(ApiError::forbidden());
    }
    if org.archived_at.is_some() == archived {
        return Ok(Json(org));
    }
    if org.version != input.version {
        return Err(ApiError::conflict("BUSINESS_VERSION_CHANGED"));
    }
    if archived {
        if let Some(reason) = archive_reason(&mut tx, id).await? {
            return Err(ApiError::conflict(reason));
        }
    } else {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM organizations WHERE owner_id=$1 AND archived_at IS NULL",
        )
        .bind(auth.user.id)
        .fetch_one(&mut *tx)
        .await?;
        if count >= 10 {
            return Err(ApiError::bad("BUSINESS_OWNERSHIP_LIMIT"));
        }
    }
    sqlx::query("UPDATE organizations SET archived_at=CASE WHEN $1 THEN now() ELSE NULL END,version=version+1 WHERE id=$2")
        .bind(archived).bind(id).execute(&mut *tx).await?;
    event(
        &mut tx,
        id,
        auth.user.id,
        if archived {
            "organization_archived"
        } else {
            "organization_restored"
        },
        json!({}),
    )
    .await?;
    let result = access(&mut tx, auth.user.id, id, false).await?;
    tx.commit().await?;
    Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rename {
    name: String,
    version: i32,
}
pub async fn rename(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Rename>,
) -> Result<Json<Organization>, ApiError> {
    auth.csrf(&headers)?;
    let name = title(&input.name)?;
    let mut tx = state.pool.begin().await?;
    auth::lock_user(&mut tx, auth.user.id).await?;
    let org = access(&mut tx, auth.user.id, id, true).await?;
    if org.version != input.version {
        return Err(ApiError::conflict(
            "Firemní prostor se změnil. Načtěte ho znovu.",
        ));
    }
    sqlx::query("UPDATE organizations SET name=$1,version=version+1 WHERE id=$2")
        .bind(&name)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    event(
        &mut tx,
        id,
        auth.user.id,
        "organization_renamed",
        json!({"name":name}),
    )
    .await?;
    let org = access(&mut tx, auth.user.id, id, false).await?;
    tx.commit().await?;
    Ok(Json(org))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Remove {
    password: String,
    confirmation: String,
}
pub async fn remove(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Remove>,
) -> Result<Json<Value>, ApiError> {
    auth.csrf(&headers)?;
    auth::rate_limit(&state, &format!("org-delete:{}", auth.user.id), 5, 900).await?;
    let mut tx = state.pool.begin().await?;
    // User then organization is the same lock order as account deletion and organization creation.
    auth::reauthenticate(&state, &mut tx, auth.user.id, input.password).await?;
    let org = access(&mut tx, auth.user.id, id, true).await?;
    if org.owner_id != auth.user.id {
        return Err(ApiError::forbidden());
    }
    if input.confirmation != org.name {
        return Err(ApiError::bad(
            "Potvrďte odstranění přesným názvem firemního prostoru.",
        ));
    }
    if let Some(reason) = delete_reason(&mut tx, &org).await? {
        return Err(ApiError::conflict(reason));
    }
    sqlx::query("DELETE FROM organizations WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    auth::event(&mut tx, auth.user.id, "organization_deleted").await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramInput {
    id: Uuid,
    title: String,
    target_m: i32,
    #[serde(default = "default_currency")]
    currency: String,
    #[serde(default = "default_planning_reward")]
    reward_minor: i64,
    max_participants: i32,
    #[serde(default = "default_template", skip_serializing_if = "legacy_template")]
    template: String,
    #[serde(default, skip_serializing_if = "zero_points")]
    point_reward: i64,
    #[serde(default, skip_serializing_if = "zero_points")]
    point_stake: i64,
}
fn default_currency() -> String {
    "CZK".into()
}
fn default_planning_reward() -> i64 {
    100
}
fn default_template() -> String {
    "LEGACY".into()
}
fn legacy_template(template: &str) -> bool {
    template == "LEGACY"
}
fn zero_points(points: &i64) -> bool {
    *points == 0
}
impl ProgramInput {
    fn validate(&mut self) -> Result<(), ApiError> {
        self.title = title(&self.title)?;
        if !matches!(
            self.template.as_str(),
            "LEGACY" | "ACTIVITY_POINTS" | "EMPLOYER_MATCH" | "EVENT" | "MONTHLY_BUDGET"
        ) {
            return Err(ApiError::bad("INVALID_BUSINESS_TEMPLATE"));
        }
        if self.template != "LEGACY" {
            if !(1..=100_000).contains(&self.point_reward)
                || !(0..=100_000).contains(&self.point_stake)
                || (self.template == "EMPLOYER_MATCH" && self.point_stake == 0)
                || (self.template != "EMPLOYER_MATCH" && self.point_stake != 0)
                || self
                    .point_reward
                    .checked_mul(i64::from(self.max_participants))
                    .is_none_or(|total| total > 10_000_000)
            {
                return Err(ApiError::bad("INVALID_BUSINESS_POINT_TERMS"));
            }
            // These retained fields are legacy planning metadata, never a points/CZK conversion.
            self.reward_minor = 100;
        } else if self.point_reward != 0 || self.point_stake != 0 {
            return Err(ApiError::bad("INVALID_BUSINESS_POINT_TERMS"));
        }
        if self.id.is_nil()
            || self.currency != "CZK"
            || !(1000..=5000).contains(&self.target_m)
            || !(100..=1000000).contains(&self.reward_minor)
            || !(1..=10000).contains(&self.max_participants)
            || self
                .reward_minor
                .checked_mul(i64::from(self.max_participants))
                .is_none_or(|n| n > 100000000)
        {
            return Err(ApiError::bad(
                "Program vyžaduje 1–5 km, 1–10 000 účastníků, odměnu 1–10 000 Kč a celkový plán do 1 000 000 Kč.",
            ));
        }
        Ok(())
    }
}
pub async fn create_program(
    auth: Auth,
    State(state): State<AppState>,
    Path(org): Path<Uuid>,
    headers: HeaderMap,
    Json(mut input): Json<ProgramInput>,
) -> Result<Json<Program>, ApiError> {
    auth.csrf(&headers)?;
    input.validate()?;
    let hash = crypto::digest(serde_json::to_vec(&input).map_err(|_| ApiError::internal())?);
    let mut tx = state.pool.begin().await?;
    auth::lock_user(&mut tx, auth.user.id).await?;
    access(&mut tx, auth.user.id, org, true).await?;
    if let Some((old,)) = sqlx::query_as::<_, (String,)>(
        "SELECT creation_hash FROM company_programs WHERE id=$1 AND organization_id=$2",
    )
    .bind(input.id)
    .bind(org)
    .fetch_optional(&mut *tx)
    .await?
    {
        if hash != old {
            return Err(ApiError::conflict(
                "Identifikátor již patří jinému požadavku.",
            ));
        }
        let row = sqlx::query_as::<_, Program>(
            "SELECT * FROM company_programs WHERE id=$1 AND organization_id=$2",
        )
        .bind(input.id)
        .bind(org)
        .fetch_one(&mut *tx)
        .await?;
        return Ok(Json(row));
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM company_programs WHERE organization_id=$1")
            .bind(org)
            .fetch_one(&mut *tx)
            .await?;
    if count >= 100 {
        return Err(ApiError::bad(
            "Prostor může mít nejvýše 100 návrhů programů.",
        ));
    }
    let row=sqlx::query_as::<_,Program>("INSERT INTO company_programs(id,organization_id,title,target_m,currency,reward_minor,max_participants,creation_hash,template,point_reward,point_stake,monthly_decline) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT(id) DO NOTHING RETURNING *")
        .bind(input.id).bind(org).bind(&input.title).bind(input.target_m).bind(&input.currency).bind(input.reward_minor).bind(input.max_participants).bind(hash).bind(&input.template).bind(input.point_reward).bind(input.point_stake).bind(input.template == "MONTHLY_BUDGET").fetch_optional(&mut *tx).await?.ok_or_else(||ApiError::conflict("Identifikátor již patří jinému požadavku."))?;
    event(
        &mut tx,
        org,
        auth.user.id,
        "program_created",
        json!({"program_id":row.id,"title":row.title}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(row))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramUpdate {
    title: String,
    target_m: i32,
    #[serde(default = "default_currency")]
    currency: String,
    #[serde(default = "default_planning_reward")]
    reward_minor: i64,
    max_participants: i32,
    version: i32,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    point_reward: Option<i64>,
    #[serde(default)]
    point_stake: Option<i64>,
}
pub async fn update_program(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(input): Json<ProgramUpdate>,
) -> Result<Json<Program>, ApiError> {
    auth.csrf(&headers)?;
    let mut tx = state.pool.begin().await?;
    auth::lock_user(&mut tx, auth.user.id).await?;
    access(&mut tx, auth.user.id, org, true).await?;
    let old = sqlx::query_as::<_, Program>(
        "SELECT * FROM company_programs WHERE id=$1 AND organization_id=$2",
    )
    .bind(id)
    .bind(org)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::not_found)?;
    if old.state != "DRAFT" || old.version != input.version {
        return Err(ApiError::conflict(
            "Program se změnil nebo byl archivován. Načtěte ho znovu.",
        ));
    }
    let mut values = ProgramInput {
        id,
        title: input.title,
        target_m: input.target_m,
        currency: input.currency,
        reward_minor: input.reward_minor,
        max_participants: input.max_participants,
        template: input.template.unwrap_or(old.template),
        point_reward: input.point_reward.unwrap_or(old.point_reward),
        point_stake: input.point_stake.unwrap_or(old.point_stake),
    };
    values.validate()?;
    let row=sqlx::query_as::<_,Program>("UPDATE company_programs SET title=$1,target_m=$2,reward_minor=$3,max_participants=$4,version=version+1,updated_at=now(),template=$7,point_reward=$8,point_stake=$9,monthly_decline=$10 WHERE id=$5 AND organization_id=$6 RETURNING *")
        .bind(values.title).bind(values.target_m).bind(values.reward_minor).bind(values.max_participants).bind(id).bind(org).bind(&values.template).bind(values.point_reward).bind(values.point_stake).bind(values.template == "MONTHLY_BUDGET").fetch_one(&mut *tx).await?;
    event(
        &mut tx,
        org,
        auth.user.id,
        "program_updated",
        json!({"program_id":id,"version":row.version}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(row))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Version {
    version: i32,
}
pub async fn archive_program(
    auth: Auth,
    State(state): State<AppState>,
    Path((org, id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(input): Json<Version>,
) -> Result<Json<Program>, ApiError> {
    auth.csrf(&headers)?;
    let mut tx = state.pool.begin().await?;
    auth::lock_user(&mut tx, auth.user.id).await?;
    access(&mut tx, auth.user.id, org, true).await?;
    let old = sqlx::query_as::<_, Program>(
        "SELECT * FROM company_programs WHERE id=$1 AND organization_id=$2",
    )
    .bind(id)
    .bind(org)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::not_found)?;
    if old.state == "ARCHIVED" {
        return Ok(Json(old));
    }
    if !matches!(old.state.as_str(), "DRAFT" | "CLOSED") {
        return Err(ApiError::conflict("CLOSE_BUSINESS_PROGRAM_FIRST"));
    }
    if old.version != input.version {
        return Err(ApiError::conflict(
            "Program se změnil nebo byl archivován. Načtěte ho znovu.",
        ));
    }
    let row=sqlx::query_as::<_,Program>("UPDATE company_programs SET state='ARCHIVED',version=version+1,updated_at=now() WHERE id=$1 AND organization_id=$2 RETURNING *").bind(id).bind(org).fetch_one(&mut *tx).await?;
    event(
        &mut tx,
        org,
        auth.user.id,
        "program_archived",
        json!({"program_id":id}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(row))
}
