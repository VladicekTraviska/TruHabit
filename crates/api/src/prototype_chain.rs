use crate::{
    AppState,
    auth::{Auth, lock_user},
    crypto,
    error::ApiError,
    prototype::{self, Challenge},
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::FromRow;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

fn worker_path(
    configured: Option<std::ffi::OsString>,
    installation: &std::path::Path,
    working_directory: &std::path::Path,
) -> Option<std::path::PathBuf> {
    // Explicit installation configuration must never fall back to another worker.
    let candidates = match configured {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            if !path.is_absolute() {
                return None;
            }
            vec![path]
        }
        None => vec![
            installation.join("prototype-chain/worker.mjs"),
            working_directory.join("prototype-chain/worker.mjs"),
        ],
    };
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(test)]
mod worker_tests {
    use super::worker_path;
    #[test]
    fn worker_installation_is_independent_of_compile_directory() {
        let root = std::env::temp_dir().join(format!("truhabit-worker-{}", uuid::Uuid::new_v4()));
        let installation = root.join("installation");
        let project = root.join("project");
        std::fs::create_dir_all(installation.join("prototype-chain")).unwrap();
        std::fs::create_dir_all(project.join("prototype-chain")).unwrap();
        let installed = installation.join("prototype-chain/worker.mjs");
        let development = project.join("prototype-chain/worker.mjs");
        std::fs::write(&installed, "// test installation").unwrap();
        std::fs::write(&development, "// test development").unwrap();
        assert_eq!(
            worker_path(None, &installation, &project),
            Some(installed.clone())
        );
        assert_eq!(
            worker_path(
                Some(development.clone().into_os_string()),
                &installation,
                &project
            ),
            Some(development.clone())
        );
        assert!(worker_path(Some("relative/worker.mjs".into()), &installation, &project).is_none());
        assert!(
            worker_path(
                Some(root.join("missing.mjs").into_os_string()),
                &installation,
                &project
            )
            .is_none()
        );
        std::fs::remove_file(installed).unwrap();
        assert_eq!(
            worker_path(None, &installation, &project),
            Some(development)
        );
        let resolved_root = root.canonicalize().unwrap();
        let temporary_root = std::env::temp_dir().canonicalize().unwrap();
        assert!(
            resolved_root.is_absolute() && resolved_root.parent() == Some(temporary_root.as_path())
        );
        std::fs::remove_dir_all(resolved_root).unwrap();
    }
}

async fn worker(input: Value) -> Result<Value, ApiError> {
    let executable =
        std::env::current_exe().map_err(|_| ApiError::unavailable("CHAIN_WORKER_UNAVAILABLE"))?;
    let installation = executable
        .parent()
        .ok_or_else(|| ApiError::unavailable("CHAIN_WORKER_UNAVAILABLE"))?;
    let working_directory =
        std::env::current_dir().map_err(|_| ApiError::unavailable("CHAIN_WORKER_UNAVAILABLE"))?;
    let path = worker_path(
        std::env::var_os("TRUHABIT_CHAIN_WORKER"),
        installation,
        &working_directory,
    )
    .ok_or_else(|| ApiError::unavailable("CHAIN_WORKER_UNAVAILABLE"))?;
    let node = match std::env::var_os("TRUHABIT_NODE_BIN") {
        Some(value) => {
            let value = std::path::PathBuf::from(value);
            if !value.is_absolute() || !value.is_file() {
                return Err(ApiError::unavailable("CHAIN_WORKER_UNAVAILABLE"));
            }
            value.into_os_string()
        }
        None => {
            let bundled = installation.join(if cfg!(windows) {
                "runtime/node/node.exe"
            } else {
                "runtime/node/node"
            });
            if bundled.is_file() {
                bundled.into_os_string()
            } else {
                "node".into()
            }
        }
    };
    let mut process = tokio::process::Command::new(node);
    process
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    process.creation_flags(0x08000000);
    let mut child = process
        .spawn()
        .map_err(|_| ApiError::unavailable("CHAIN_WORKER_UNAVAILABLE"))?;
    let mut stdin = child.stdin.take().ok_or_else(ApiError::internal)?;
    stdin
        .write_all(&serde_json::to_vec(&input).map_err(|_| ApiError::internal())?)
        .await
        .map_err(|_| ApiError::internal())?;
    drop(stdin);
    let output = tokio::time::timeout(std::time::Duration::from_secs(40), child.wait_with_output())
        .await
        .map_err(|_| ApiError::unavailable("CHAIN_RESPONSE_UNKNOWN"))?
        .map_err(|_| ApiError::unavailable("CHAIN_UNAVAILABLE"))?;
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| ApiError::unavailable("CHAIN_UNAVAILABLE"))?;
    if value["ok"] != true {
        return Err(ApiError::unavailable(
            value["code"].as_str().unwrap_or("CHAIN_UNAVAILABLE"),
        ));
    }
    Ok(value["result"].clone())
}
#[derive(FromRow)]
struct Command {
    id: Uuid,
    action: String,
    payload: Value,
    status: String,
    signed_transaction: Option<String>,
    signature: Option<String>,
}
fn failure_available(c: &Challenge, now: chrono::DateTime<Utc>) -> bool {
    c.state == "ACTIVE"
        && !matches!(c.assessment.as_str(), "MET" | "REVIEW_REQUIRED")
        // The contract reads Clock::unix_timestamp in whole seconds.
        && now.timestamp() > c.upload_deadline.timestamp()
        && now.timestamp() < c.refund_after.timestamp()
}
async fn invalidate_prepared_failure(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    c: &Challenge,
    cmd: &Command,
    actor: Uuid,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE prototype_commands SET status='FAILED' WHERE id=$1 AND status='PREPARED'")
        .bind(cmd.id)
        .execute(&mut **tx)
        .await?;
    prototype::event(
        tx,
        c.id,
        actor,
        "prepared_failure_invalidated",
        json!({"command_id":cmd.id,"assessment":c.assessment}),
    )
    .await
}
#[cfg(test)]
mod failure_tests {
    use super::*;
    #[test]
    fn failure_requires_the_next_chain_second_and_current_unmet_assessment() {
        let upload = chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let mut c = Challenge {
            id: Uuid::nil(),
            user_id: Uuid::nil(),
            title: "Test".into(),
            target_m: 3000,
            amount_units: 1_000_000,
            profile: "REPLAY".into(),
            network: "DEVNET".into(),
            starts_at: upload - chrono::Duration::hours(2),
            ends_at: upload - chrono::Duration::hours(1),
            upload_deadline: upload,
            refund_after: upload + chrono::Duration::minutes(5),
            state: "ACTIVE".into(),
            assessment: "UNKNOWN".into(),
            policy: "prototype-distance-v1".into(),
            chain: None,
            creation_hash: String::new(),
            created_at: upload,
            closed_at: None,
            version: 1,
        };
        assert!(!failure_available(
            &c,
            upload + chrono::Duration::milliseconds(999)
        ));
        assert!(failure_available(&c, upload + chrono::Duration::seconds(1)));
        assert!(!failure_available(&c, c.refund_after));
        c.assessment = "REVIEW_REQUIRED".into();
        assert!(!failure_available(
            &c,
            upload + chrono::Duration::seconds(1)
        ));
        c.assessment = "MET".into();
        assert!(!failure_available(
            &c,
            upload + chrono::Duration::seconds(1)
        ));
    }
}
fn settled_state(action: &str) -> Result<&'static str, ApiError> {
    match action {
        "SUCCESS" => Ok("REFUNDED"),
        "FAILURE" => Ok("FORFEITED"),
        "CANCEL" => Ok("CANCELLED"),
        "TIMEOUT" => Ok("EXPIRED"),
        _ => Err(ApiError::internal()),
    }
}
// The private verifier supplies an actual finalized settle + SPL transfer proof.
// Keep that transaction separate from a deposit or superseded original command.
async fn recovered_settlement(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    c: &Challenge,
    actor: Uuid,
    proof: &Value,
) -> Result<(), ApiError> {
    let action = proof["action"].as_str().ok_or_else(ApiError::internal)?;
    let state = settled_state(action)?;
    let signature = proof["signature"].as_str().ok_or_else(ApiError::internal)?;
    let slot = proof["slot"].as_u64().ok_or_else(ApiError::internal)?;
    if signature.is_empty() || proof["amount_units"].as_i64() != Some(c.amount_units) {
        return Err(ApiError::internal());
    }
    let existing: Option<(Uuid, String)> =
        sqlx::query_as("SELECT challenge_id,action FROM prototype_commands WHERE signature=$1")
            .bind(signature)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some((challenge, original_action)) = existing {
        if challenge != c.id || original_action != action {
            return Err(ApiError::conflict("CHAIN_MESSAGE_MISMATCH"));
        }
    } else {
        sqlx::query("INSERT INTO prototype_commands(id,challenge_id,action,payload,status,signature) VALUES($1,$2,$3,$4,'CONFIRMED',$5)")
            .bind(Uuid::new_v4()).bind(c.id).bind(action)
            .bind(json!({"recovered":true,"proof":proof})).bind(signature)
            .execute(&mut **tx).await?;
    }
    sqlx::query(
        "UPDATE prototype_challenges SET state=$1,version=version+1,closed_at=now() WHERE id=$2",
    )
    .bind(state)
    .bind(c.id)
    .execute(&mut **tx)
    .await?;
    prototype::event(
        tx, c.id, actor, "transfer_confirmed",
        json!({"action":action,"signature":signature,"slot":slot,"source":"external_chain_recovery","proof":proof}),
    ).await?;
    Ok(())
}
fn terms(c: &Challenge) -> Result<Value, ApiError> {
    if c.network != "DEVNET" {
        return Err(ApiError::bad("WRONG_NETWORK"));
    }
    let chain = c
        .chain
        .as_ref()
        .ok_or_else(|| ApiError::bad("LINK_PHANTOM_FIRST"))?;
    let mut data = serde_json::to_value(c).map_err(|_| ApiError::internal())?;
    data["owner"] = chain["owner"].clone();
    data["terms_hash"] = chain["terms_hash"].clone();
    Ok(data)
}
pub async fn balance(auth: Auth, State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    crate::auth::rate_limit(
        &state,
        &format!("prototype-balance:{}", auth.user.id),
        30,
        60,
    )
    .await?;
    let wallet: Option<String> =
        sqlx::query_scalar("SELECT public_key FROM wallets WHERE user_id=$1")
            .bind(auth.user.id)
            .fetch_optional(&state.pool)
            .await?;
    match wallet {
        Some(owner) => Ok(Json(
            worker(json!({"operation":"balance","owner":owner})).await?,
        )),
        None => Ok(Json(
            json!({"wallet":null,"network":"solana-devnet","deployed":false}),
        )),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    action: String,
}
pub async fn prepare(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Action>,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    crate::auth::rate_limit(
        &state,
        &format!("prototype-command:{}", auth.user.id),
        30,
        3600,
    )
    .await?;
    let is_operator = prototype::operator(&state, auth.user.id).await?;
    let mut tx = state.pool.begin().await?;
    // Consistent user -> challenge lock order with deletion and upload.
    let owner: Uuid = sqlx::query_scalar("SELECT user_id FROM prototype_challenges WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if owner != auth.user.id && !is_operator {
        return Err(ApiError::not_found());
    }
    lock_user(&mut tx, owner).await?;
    let mut c = prototype::owned(&mut tx, id, owner).await?;
    if c.network != "DEVNET" {
        return Err(ApiError::bad("WRONG_NETWORK"));
    }
    if let Some(cmd)=sqlx::query_as::<_,Command>("SELECT id,action,payload,status,signed_transaction,signature FROM prototype_commands WHERE challenge_id=$1 AND status IN ('PREPARED','SIGNED')").bind(id).fetch_optional(&mut *tx).await?{
        if cmd.action == "FAILURE" && cmd.status == "PREPARED" && !failure_available(&c, Utc::now()) {
            invalidate_prepared_failure(&mut tx, &c, &cmd, auth.user.id).await?;
            tx.commit().await?;
            return Err(ApiError::conflict(if input.action == "FAILURE" {"ACTION_NOT_AVAILABLE"} else {"ACTION_STALE"}));
        }
        if cmd.action!=input.action{return Err(ApiError::conflict("SETTLEMENT_PENDING"));}
        return Ok(Json(json!({"id":cmd.id,"payload":cmd.payload,"status":cmd.status})));
    }
    let now = Utc::now();
    match input.action.as_str() {
        "DEPOSIT" if c.state == "DRAFT" && owner == auth.user.id && now < c.ends_at => {}
        "SUCCESS"
            if c.state == "ACTIVE"
                && c.assessment == "MET"
                && now >= c.starts_at
                && now < c.refund_after => {}
        "FAILURE" if is_operator && failure_available(&c, now) => {}
        "CANCEL" if c.state == "ACTIVE" && owner == auth.user.id && now < c.starts_at => {}
        "TIMEOUT" if c.state == "ACTIVE" && now >= c.refund_after => {}
        _ => return Err(ApiError::conflict("ACTION_NOT_AVAILABLE")),
    }
    if c.chain.is_none() {
        let wallet: String = sqlx::query_scalar("SELECT public_key FROM wallets WHERE user_id=$1")
            .bind(owner)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| ApiError::bad("LINK_PHANTOM_FIRST"))?;
        let hash=crypto::digest(serde_json::to_vec(&json!({"id":c.id,"title":c.title,"owner":wallet,"target":c.target_m,"amount":c.amount_units,"profile":c.profile,"starts":c.starts_at,"ends":c.ends_at,"upload":c.upload_deadline,"refund":c.refund_after,"policy":c.policy})).map_err(|_|ApiError::internal())?);
        c.chain = Some(json!({"owner":wallet,"terms_hash":hash}));
    }
    let payload =
        worker(json!({"operation":"prepare","challenge":terms(&c)?,"action":input.action})).await?;
    let command = Uuid::new_v4();
    sqlx::query("UPDATE prototype_challenges SET chain=$1 WHERE id=$2")
        .bind(&payload["chain"])
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO prototype_commands(id,challenge_id,action,payload,status) VALUES($1,$2,$3,$4,'PREPARED')").bind(command).bind(id).bind(&input.action).bind(&payload).execute(&mut *tx).await?;
    prototype::event(
        &mut tx,
        id,
        auth.user.id,
        "transaction_prepared",
        json!({"command_id":command,"action":input.action}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"id":command,"payload":payload,"status":"PREPARED"}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submit {
    command_id: Uuid,
    transaction: String,
}
pub async fn submit(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Submit>,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    crate::auth::rate_limit(
        &state,
        &format!("prototype-submit:{}", auth.user.id),
        30,
        3600,
    )
    .await?;
    let is_operator = prototype::operator(&state, auth.user.id).await?;
    let mut tx = state.pool.begin().await?;
    let owner: Uuid = sqlx::query_scalar("SELECT user_id FROM prototype_challenges WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if owner != auth.user.id && !is_operator {
        return Err(ApiError::not_found());
    }
    lock_user(&mut tx, owner).await?;
    let c =
        sqlx::query_as::<_, Challenge>("SELECT * FROM prototype_challenges WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(ApiError::not_found)?;
    if c.user_id != auth.user.id && !is_operator {
        return Err(ApiError::not_found());
    }
    let cmd=sqlx::query_as::<_,Command>("SELECT id,action,payload,status,signed_transaction,signature FROM prototype_commands WHERE id=$1 AND challenge_id=$2 FOR UPDATE").bind(input.command_id).bind(id).fetch_optional(&mut *tx).await?.ok_or_else(ApiError::not_found)?;
    if cmd.action == "FAILURE" && !is_operator {
        return Err(ApiError::forbidden());
    }
    if !matches!(cmd.status.as_str(), "PREPARED" | "SIGNED") {
        return Ok(Json(json!({"status":cmd.status})));
    }
    // Recheck mutable evidence before the first signed envelope is persisted.
    // SIGNED is an unknown chain outcome and must instead be reconciled.
    if cmd.action == "FAILURE" && cmd.status == "PREPARED" && !failure_available(&c, Utc::now()) {
        invalidate_prepared_failure(&mut tx, &c, &cmd, auth.user.id).await?;
        tx.commit().await?;
        return Err(ApiError::conflict("ACTION_NOT_AVAILABLE"));
    }
    let checked = worker(
        json!({"operation":"validate","payload":cmd.payload,"transaction":input.transaction}),
    )
    .await?;
    let signature = checked["signature"]
        .as_str()
        .ok_or_else(ApiError::internal)?;
    if cmd.signature.as_deref().is_some_and(|old| old != signature) {
        return Err(ApiError::conflict("TRANSACTION_MISMATCH"));
    }
    sqlx::query("UPDATE prototype_commands SET status='SIGNED',signature=$1,signed_transaction=$2 WHERE id=$3").bind(signature).bind(checked["transaction"].as_str()).bind(cmd.id).execute(&mut *tx).await?;
    prototype::event(
        &mut tx,
        id,
        auth.user.id,
        "transaction_signed",
        json!({"command_id":cmd.id,"signature":signature}),
    )
    .await?;
    tx.commit().await?;
    // Persist the exact signed envelope before sending. A network error is an unknown outcome.
    let sent = worker(
        json!({"operation":"broadcast","payload":cmd.payload,"transaction":checked["transaction"]}),
    )
    .await
    .is_ok();
    Ok(Json(
        json!({"status":"SIGNED","broadcast_ack":sent,"signature":signature}),
    ))
}
pub async fn refresh(
    auth: Auth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    prototype::enabled(&state)?;
    auth.csrf(&headers)?;
    crate::auth::rate_limit(
        &state,
        &format!("prototype-refresh:{}", auth.user.id),
        60,
        300,
    )
    .await?;
    let is_operator = prototype::operator(&state, auth.user.id).await?;
    let mut tx = state.pool.begin().await?;
    let owner: Uuid = sqlx::query_scalar("SELECT user_id FROM prototype_challenges WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if owner != auth.user.id && !is_operator {
        return Err(ApiError::not_found());
    }
    lock_user(&mut tx, owner).await?;
    let c =
        sqlx::query_as::<_, Challenge>("SELECT * FROM prototype_challenges WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(ApiError::not_found)?;
    if c.user_id != auth.user.id && !is_operator {
        return Err(ApiError::not_found());
    }
    let cmd=sqlx::query_as::<_,Command>("SELECT id,action,payload,status,signed_transaction,signature FROM prototype_commands WHERE challenge_id=$1 AND status IN ('SIGNED','PREPARED') ORDER BY created_at DESC LIMIT 1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?;
    let Some(cmd) = cmd else {
        if c.network == "DEVNET" && c.state == "ACTIVE" && c.chain.is_some() {
            let result=worker(json!({"operation":"reconcile","challenge":terms(&c)?,"action":"STATE","payload":{},"signature":null})).await?;
            if result["state"] == "RECOVERED" {
                recovered_settlement(&mut tx, &c, auth.user.id, &result["settlement"]).await?;
            }
            tx.commit().await?;
            return Ok(Json(result));
        }
        return Ok(Json(json!({"status":"NO_PENDING_TRANSACTION"})));
    };
    let signature = cmd.signature;
    let result=worker(json!({"operation":"reconcile","challenge":terms(&c)?,"action":cmd.action,"payload":cmd.payload,"signature":signature})).await?;
    let status = result["state"].as_str().ok_or_else(ApiError::internal)?;
    if status == "CONFIRMED" {
        let state = match cmd.action.as_str() {
            "DEPOSIT" => "ACTIVE",
            action => settled_state(action)?,
        };
        sqlx::query("UPDATE prototype_challenges SET state=$1,version=version+1,closed_at=CASE WHEN $1='ACTIVE' THEN NULL ELSE now() END WHERE id=$2").bind(state).bind(id).execute(&mut *tx).await?;
        prototype::event(
            &mut tx,
            id,
            auth.user.id,
            "transfer_confirmed",
            json!({"action":cmd.action,"signature":result["signature"],"slot":result["slot"]}),
        )
        .await?;
        if let Some(proof) = result.get("settlement") {
            recovered_settlement(&mut tx, &c, auth.user.id, proof).await?;
        }
    } else if status == "RECOVERED" {
        sqlx::query("UPDATE prototype_commands SET status='FAILED' WHERE id=$1")
            .bind(cmd.id)
            .execute(&mut *tx)
            .await?;
        prototype::event(
            &mut tx, id, auth.user.id, "superseded_by_verified_settlement",
            json!({"command_id":cmd.id,"original_action":cmd.action,"original_signature":signature,"action":result["settlement"]["action"],"signature":result["settlement"]["signature"]}),
        ).await?;
        recovered_settlement(&mut tx, &c, auth.user.id, &result["settlement"]).await?;
    }
    if matches!(status, "CONFIRMED" | "FAILED" | "EXPIRED") {
        sqlx::query(
            "UPDATE prototype_commands SET status=$1,signature=COALESCE(signature,$3) WHERE id=$2",
        )
        .bind(status)
        .bind(cmd.id)
        .bind(result["signature"].as_str())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    if status == "PENDING"
        && let Some(encoded) = cmd.signed_transaction
    {
        let _ =
            worker(json!({"operation":"broadcast","payload":cmd.payload,"transaction":encoded}))
                .await;
    }
    Ok(Json(result))
}
