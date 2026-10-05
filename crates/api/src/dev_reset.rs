//! Explicit, authenticated reset of disposable development data. Never changes chain state.
use crate::{AppState, auth, config::Config, crypto, error::ApiError};
use axum::{
    Json,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use std::net::SocketAddr;
use uuid::Uuid;

pub fn enabled(config: &Config) -> bool {
    !config.production
        && config.bind.ip().is_loopback()
        && config.trusted_proxy_ip.is_none()
        && url::Url::parse(&config.origin).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
                && url.origin().ascii_serialization() == config.origin
        })
}

// Run before authentication: disabled deployments do not expose this endpoint at all.
pub async fn gate(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if !enabled(&state.config)
        || !request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .is_some_and(|peer| peer.0.ip().is_loopback())
    {
        return Err(ApiError::not_found());
    }
    Ok(next.run(request).await)
}

#[derive(Serialize, Default)]
struct Counts {
    goals: i64,
    personal_challenges: i64,
    personal_activity_files: i64,
    local_credit_movements: i64,
    owned_workspaces: i64,
    owned_programs: i64,
    owned_workspace_memberships: i64,
    owned_team_activity_files: i64,
    foreign_memberships: i64,
    foreign_enrollments: i64,
    foreign_activity_files: i64,
}

#[derive(Serialize)]
struct Impact {
    id: Uuid,
    name: String,
    member_count: i64,
    program_count: i64,
    activity_file_count: i64,
}

#[derive(Serialize)]
struct Blocker {
    code: &'static str,
    count: i64,
}

struct Snapshot {
    fingerprint: String,
    counts: Counts,
    impacts: Vec<Impact>,
    blockers: Vec<Blocker>,
}

// The account row always comes first, matching upload admission and personal settlement.
// Only this account's ledger can be erased. Workspace locks serialize other members'
// uploads/claims without acquiring a second user lock after an organization lock.
async fn lock_workspaces(tx: &mut Transaction<'_, Postgres>, user: Uuid) -> Result<(), ApiError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT o.id FROM organizations o WHERE o.owner_id=$1
         OR EXISTS(SELECT 1 FROM organization_members m WHERE m.organization_id=o.id AND m.user_id=$1)
         OR EXISTS(SELECT 1 FROM company_programs p LEFT JOIN company_enrollments e ON e.program_id=p.id
                   WHERE p.organization_id=o.id AND (p.funder_id=$1 OR e.user_id=$1))
         OR EXISTS(SELECT 1 FROM company_programs p JOIN prototype_local_movements m ON m.business_program_id=p.id
                   WHERE p.organization_id=o.id AND m.user_id=$1)
         ORDER BY o.id FOR UPDATE OF o",
    )
    .bind(user)
    .fetch_all(&mut **tx)
    .await?;
    Ok(())
}

async fn snapshot(tx: &mut Transaction<'_, Postgres>, user: Uuid) -> Result<Snapshot, ApiError> {
    // Hash states, versions, source erasure flags and command signatures, not just row
    // counts. Auth sessions, rate limits and security log entries are intentionally
    // excluded: reading the preview refreshes the session and consumes a rate limit.
    // Private sensor streams/source bytes are never selected or returned by this API.
    let data: Value = sqlx::query_scalar(
        r#"WITH owned AS (SELECT id FROM organizations WHERE owner_id=$1),
           programs AS (SELECT id FROM company_programs WHERE organization_id IN(SELECT id FROM owned)),
           enrolments AS (SELECT id FROM company_enrollments WHERE program_id IN(SELECT id FROM programs) OR user_id=$1),
           challenges AS (SELECT id FROM prototype_challenges WHERE user_id=$1),
           personal_goals AS (SELECT id FROM goals WHERE user_id=$1)
        SELECT jsonb_build_object(
          'user',$1::text,
          'goals',COALESCE((SELECT jsonb_agg(to_jsonb(g) ORDER BY g.id) FROM goals g WHERE g.user_id=$1),'[]'),
          'goal_events',COALESCE((SELECT jsonb_agg(to_jsonb(e) ORDER BY e.id) FROM goal_events e WHERE e.goal_id IN(SELECT id FROM personal_goals)),'[]'),
          'goal_keys',COALESCE((SELECT jsonb_agg(to_jsonb(k) ORDER BY k.key) FROM idempotency_keys k WHERE k.resource_id IN(SELECT id FROM personal_goals)),'[]'),
          'challenges',COALESCE((SELECT jsonb_agg(to_jsonb(c) ORDER BY c.id) FROM prototype_challenges c WHERE c.user_id=$1),'[]'),
          'commands',COALESCE((SELECT jsonb_agg((to_jsonb(c)-'signed_transaction'-'payload') || jsonb_build_object('signed',c.signed_transaction IS NOT NULL) ORDER BY c.id) FROM prototype_commands c WHERE c.challenge_id IN(SELECT id FROM challenges)),'[]'),
          'challenge_events',COALESCE((SELECT jsonb_agg(to_jsonb(e) ORDER BY e.id) FROM prototype_events e WHERE e.challenge_id IN(SELECT id FROM challenges)),'[]'),
          'personal_uploads',COALESCE((SELECT jsonb_agg(jsonb_build_object('id',u.id,'challenge_id',u.challenge_id,'file_hash',u.file_hash,'fingerprint',u.fingerprint,'session_index',u.session_index,'decision',u.decision,'goal_result',u.goal_result,'reason',u.reason,'received_at',u.received_at,'content_deleted_at',u.content_deleted_at,'has_content',u.content IS NOT NULL) ORDER BY u.id) FROM prototype_uploads u WHERE u.challenge_id IN(SELECT id FROM challenges)),'[]'),
          'reviews',COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM prototype_review_requests r WHERE r.challenge_id IN(SELECT id FROM challenges)),'[]'),
          'movements',COALESCE((SELECT jsonb_agg(to_jsonb(m) ORDER BY m.id) FROM prototype_local_movements m WHERE m.user_id=$1 OR m.business_program_id IN(SELECT id FROM programs)),'[]'),
          'workspaces',COALESCE((SELECT jsonb_agg(to_jsonb(o) ORDER BY o.id) FROM organizations o WHERE o.id IN(SELECT id FROM owned)),'[]'),
          'memberships',COALESCE((SELECT jsonb_agg(to_jsonb(m) ORDER BY m.organization_id,m.user_id) FROM organization_members m WHERE m.organization_id IN(SELECT id FROM owned) OR m.user_id=$1),'[]'),
          'invitations',COALESCE((SELECT jsonb_agg(to_jsonb(i)-'token_hash' ORDER BY i.id) FROM organization_invitations i WHERE i.organization_id IN(SELECT id FROM owned)),'[]'),
          'workspace_events',COALESCE((SELECT jsonb_agg(to_jsonb(e) ORDER BY e.id) FROM organization_events e WHERE e.organization_id IN(SELECT id FROM owned)),'[]'),
          'programs',COALESCE((SELECT jsonb_agg(to_jsonb(p) ORDER BY p.id) FROM company_programs p WHERE p.id IN(SELECT id FROM programs) OR p.funder_id=$1 OR EXISTS(SELECT 1 FROM company_enrollments e WHERE e.program_id=p.id AND e.user_id=$1) OR EXISTS(SELECT 1 FROM prototype_local_movements m WHERE m.business_program_id=p.id AND m.user_id=$1)),'[]'),
          'enrollments',COALESCE((SELECT jsonb_agg(to_jsonb(e) ORDER BY e.id) FROM company_enrollments e WHERE e.id IN(SELECT id FROM enrolments)),'[]'),
          'team_uploads',COALESCE((SELECT jsonb_agg(jsonb_build_object('id',u.id,'enrollment_id',u.enrollment_id,'user_id',u.user_id,'file_hash',u.file_hash,'fingerprint',u.fingerprint,'session_index',u.session_index,'decision',u.decision,'goal_result',u.goal_result,'reason',u.reason,'received_at',u.received_at,'content_deleted_at',u.content_deleted_at,'has_content',u.content IS NOT NULL) ORDER BY u.id) FROM company_uploads u WHERE u.enrollment_id IN(SELECT id FROM enrolments)),'[]'),
          'enrollment_events',COALESCE((SELECT jsonb_agg(to_jsonb(e) ORDER BY e.id) FROM company_enrollment_events e WHERE e.enrollment_id IN(SELECT id FROM enrolments)),'[]')
        )"#,
    )
    .bind(user)
    .fetch_one(&mut **tx)
    .await?;
    let fingerprint = crypto::digest(serde_json::to_vec(&data).map_err(|_| ApiError::internal())?);
    let row: (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
          (SELECT count(*) FROM goals WHERE user_id=$1),
          (SELECT count(*) FROM prototype_challenges WHERE user_id=$1),
          (SELECT count(content) FROM prototype_uploads WHERE user_id=$1),
          (SELECT count(*) FROM prototype_local_movements WHERE user_id=$1),
          (SELECT count(*) FROM organizations WHERE owner_id=$1),
          (SELECT count(*) FROM company_programs p JOIN organizations o ON o.id=p.organization_id WHERE o.owner_id=$1),
          (SELECT count(*) FROM organization_members m JOIN organizations o ON o.id=m.organization_id WHERE o.owner_id=$1 AND m.user_id<>$1),
          (SELECT count(u.content) FROM company_uploads u JOIN company_enrollments e ON e.id=u.enrollment_id JOIN company_programs p ON p.id=e.program_id JOIN organizations o ON o.id=p.organization_id WHERE o.owner_id=$1),
          (SELECT count(*) FROM organization_members m JOIN organizations o ON o.id=m.organization_id WHERE m.user_id=$1 AND o.owner_id<>$1),
          (SELECT count(*) FROM company_enrollments e JOIN company_programs p ON p.id=e.program_id JOIN organizations o ON o.id=p.organization_id WHERE e.user_id=$1 AND o.owner_id<>$1),
          (SELECT count(u.content) FROM company_uploads u JOIN company_enrollments e ON e.id=u.enrollment_id JOIN company_programs p ON p.id=e.program_id JOIN organizations o ON o.id=p.organization_id WHERE u.user_id=$1 AND o.owner_id<>$1)",
    ).bind(user).fetch_one(&mut **tx).await?;
    let counts = Counts {
        goals: row.0,
        personal_challenges: row.1,
        personal_activity_files: row.2,
        local_credit_movements: row.3,
        owned_workspaces: row.4,
        owned_programs: row.5,
        owned_workspace_memberships: row.6,
        owned_team_activity_files: row.7,
        foreign_memberships: row.8,
        foreign_enrollments: row.9,
        foreign_activity_files: row.10,
    };
    let impact_rows: Vec<(Uuid, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT o.id,o.name,
          (SELECT count(*) FROM organization_members m WHERE m.organization_id=o.id AND m.user_id<>$1),
          (SELECT count(*) FROM company_programs p WHERE p.organization_id=o.id),
          (SELECT count(u.content) FROM company_uploads u JOIN company_enrollments e ON e.id=u.enrollment_id JOIN company_programs p ON p.id=e.program_id WHERE p.organization_id=o.id)
         FROM organizations o WHERE o.owner_id=$1 ORDER BY o.id",
    ).bind(user).fetch_all(&mut **tx).await?;
    let impacts = impact_rows
        .into_iter()
        .map(|r| Impact {
            id: r.0,
            name: r.1,
            member_count: r.2,
            program_count: r.3,
            activity_file_count: r.4,
        })
        .collect();
    let mut blockers = Vec::new();
    for (code, sql) in [
        (
            "DEV_RESET_DEVNET_ESCROW_ACTIVE",
            "SELECT count(*) FROM prototype_challenges WHERE user_id=$1 AND network='DEVNET' AND state='ACTIVE'",
        ),
        (
            "DEV_RESET_DEVNET_COMMAND_PENDING",
            "SELECT count(*) FROM prototype_commands cmd JOIN prototype_challenges c ON c.id=cmd.challenge_id WHERE c.user_id=$1 AND c.network='DEVNET' AND cmd.status IN('PREPARED','SIGNED')",
        ),
        (
            "DEV_RESET_DEVNET_STATE_UNVERIFIED",
            "SELECT count(*) FROM prototype_challenges c WHERE c.user_id=$1 AND c.network='DEVNET' AND ((c.state='DRAFT' AND EXISTS(SELECT 1 FROM prototype_commands cmd WHERE cmd.challenge_id=c.id AND ((cmd.action='DEPOSIT' AND cmd.status='CONFIRMED') OR (cmd.status IN('FAILED','EXPIRED') AND (cmd.signed_transaction IS NOT NULL OR cmd.signature IS NOT NULL))))) OR (c.state NOT IN('DRAFT','ACTIVE') AND (c.closed_at IS NULL OR NOT EXISTS(SELECT 1 FROM prototype_commands cmd WHERE cmd.challenge_id=c.id AND cmd.status='CONFIRMED' AND length(cmd.signature)>0 AND cmd.action=CASE c.state WHEN 'REFUNDED' THEN 'SUCCESS' WHEN 'FORFEITED' THEN 'FAILURE' WHEN 'CANCELLED' THEN 'CANCEL' WHEN 'EXPIRED' THEN 'TIMEOUT' END))))",
        ),
        (
            "DEV_RESET_FOREIGN_PARTICIPATION_ACTIVE",
            "SELECT count(*) FROM company_programs p JOIN organizations o ON o.id=p.organization_id WHERE o.owner_id<>$1 AND ((p.state='PUBLISHED' AND (p.funder_id=$1 OR EXISTS(SELECT 1 FROM company_enrollments e WHERE e.program_id=p.id AND e.user_id=$1))) OR EXISTS(SELECT 1 FROM company_enrollments e WHERE e.program_id=p.id AND e.user_id=$1 AND e.assessment='MET' AND e.state<>'REWARDED'))",
        ),
        (
            "DEV_RESET_FOREIGN_FINANCIAL_HISTORY",
            "SELECT count(*) FROM prototype_local_movements m JOIN company_programs p ON p.id=m.business_program_id JOIN organizations o ON o.id=p.organization_id WHERE m.user_id=$1 AND o.owner_id<>$1",
        ),
        (
            "DEV_RESET_SHARED_FINANCIAL_HISTORY",
            "SELECT count(*) FROM company_programs p JOIN organizations o ON o.id=p.organization_id WHERE o.owner_id=$1 AND ((p.budget_units>0 AND (p.funder_id IS NULL OR p.funder_id<>$1)) OR EXISTS(SELECT 1 FROM prototype_local_movements m WHERE m.business_program_id=p.id AND m.user_id<>$1) OR EXISTS(SELECT 1 FROM company_enrollments e WHERE e.program_id=p.id AND e.user_id<>$1 AND e.assessment='MET'))",
        ),
    ] {
        let count: i64 = sqlx::query_scalar(sql)
            .bind(user)
            .fetch_one(&mut **tx)
            .await?;
        if count > 0 {
            blockers.push(Blocker { code, count });
        }
    }
    Ok(Snapshot {
        fingerprint,
        counts,
        impacts,
        blockers,
    })
}

pub async fn preview(
    State(state): State<AppState>,
    auth: auth::Auth,
) -> Result<Json<Value>, ApiError> {
    auth::rate_limit(
        &state,
        &format!("dev-reset-preview:{}", auth.user.id),
        30,
        300,
    )
    .await?;
    let mut tx = state.pool.begin().await?;
    auth::lock_user(&mut tx, auth.user.id).await?;
    lock_workspaces(&mut tx, auth.user.id).await?;
    let result = snapshot(&mut tx, auth.user.id).await?;
    tx.commit().await?;
    Ok(Json(json!({
        "enabled":true,"allowed":result.blockers.is_empty(),"fingerprint":result.fingerprint,
        "counts":result.counts,"owned_workspace_impacts":result.impacts,"blockers":result.blockers,
        "preserved":{"account":true,"credentials":true,"sessions":true,"linked_wallet":true,
            "blockchain":true,"other_accounts":true,"foreign_workspaces":true},
        "network":"LOCAL","real_money":false
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reset {
    password: String,
    confirmation: String,
    fingerprint: String,
}

pub async fn reset(
    State(state): State<AppState>,
    auth: auth::Auth,
    headers: HeaderMap,
    Json(input): Json<Reset>,
) -> Result<Json<Value>, ApiError> {
    auth.csrf(&headers)?;
    auth::rate_limit(&state, &format!("dev-reset:{}", auth.user.id), 5, 900).await?;
    if input.confirmation != "RESET" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "DEV_RESET_CONFIRMATION_REQUIRED",
            "DEV_RESET_CONFIRMATION_REQUIRED",
        ));
    }
    if input.fingerprint.len() != 64 || !input.fingerprint.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "DEV_RESET_INVALID_FINGERPRINT",
            "DEV_RESET_INVALID_FINGERPRINT",
        ));
    }
    let mut tx = state.pool.begin().await?;
    auth::reauthenticate(&state, &mut tx, auth.user.id, input.password).await?;
    lock_workspaces(&mut tx, auth.user.id).await?;
    let before = snapshot(&mut tx, auth.user.id).await?;
    if before.fingerprint != input.fingerprint {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "DEV_RESET_PREVIEW_CHANGED",
            "DEV_RESET_PREVIEW_CHANGED",
        ));
    }
    if !before.blockers.is_empty() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "DEV_RESET_BLOCKED",
            "DEV_RESET_BLOCKED",
        ));
    }
    // Only personal book entries are erased. Shared or foreign entries were guarded
    // above, including ownership transfers where the current owner was not the funder.
    for sql in [
        "DELETE FROM prototype_local_movements WHERE user_id=$1",
        "DELETE FROM goals WHERE user_id=$1",
        "DELETE FROM prototype_challenges WHERE user_id=$1",
        "DELETE FROM company_uploads u USING company_enrollments e,company_programs p,organizations o WHERE u.enrollment_id=e.id AND e.program_id=p.id AND p.organization_id=o.id AND u.user_id=$1 AND o.owner_id<>$1",
        "UPDATE company_enrollment_events ev SET actor_id=NULL FROM company_enrollments e,company_programs p,organizations o WHERE ev.enrollment_id=e.id AND e.program_id=p.id AND p.organization_id=o.id AND e.user_id=$1 AND o.owner_id<>$1 AND ev.actor_id=$1",
        "UPDATE company_enrollments e SET user_id=NULL FROM company_programs p,organizations o WHERE e.program_id=p.id AND p.organization_id=o.id AND e.user_id=$1 AND o.owner_id<>$1",
        "DELETE FROM organization_members m USING organizations o WHERE m.organization_id=o.id AND m.user_id=$1 AND o.owner_id<>$1",
        "DELETE FROM organizations WHERE owner_id=$1",
    ] {
        sqlx::query(sql)
            .bind(auth.user.id)
            .execute(&mut *tx)
            .await?;
    }
    auth::event(&mut tx, auth.user.id, "development_data_reset").await?;
    tx.commit().await?;
    Ok(Json(
        json!({"ok":true,"account_preserved":true,"deleted":before.counts,
        "remaining_local_balance":{"available":0,"locked":0,"forfeited":0},
        "network":"LOCAL","real_money":false}),
    ))
}
