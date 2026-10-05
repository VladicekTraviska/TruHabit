use crate::{
    AppState,
    auth::{Auth, event, lock_user, rate_limit},
    crypto,
    error::ApiError,
};
use axum::{Json, extract::State, http::HeaderMap};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChallengeInput {
    public_key: String,
}
pub fn public_key(value: &str) -> Result<VerifyingKey, ApiError> {
    if value.len() > 44 {
        return Err(ApiError::bad("Neplatná adresa Solana peněženky."));
    }
    let bytes: [u8; 32] = bs58::decode(value)
        .into_vec()
        .map_err(|_| ApiError::bad("Neplatná adresa peněženky."))?
        .try_into()
        .map_err(|_| ApiError::bad("Neplatná délka adresy."))?;
    let key =
        VerifyingKey::from_bytes(&bytes).map_err(|_| ApiError::bad("Neplatný veřejný klíč."))?;
    if key.is_weak() {
        return Err(ApiError::bad("Tento veřejný klíč není podporovaný."));
    }
    Ok(key)
}
pub async fn challenge(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ChallengeInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    auth.csrf(&headers)?;
    auth.recent()?;
    public_key(&input.public_key)?;
    rate_limit(&state, &format!("wallet:{}", auth.user.id), 10, 900).await?;
    let id = Uuid::new_v4();
    let nonce = crypto::token()?;
    let issued = Utc::now();
    let expires = issued + Duration::minutes(5);
    let message = format!(
        "TruHabit wallet ownership verification\nOrigin: {}\nAccount: {}\nAddress: {}\nNonce: {}\nIssued at: {}\nExpires at: {}\n\nThis signature links this wallet to your TruHabit account. It does not authorize a transaction or transfer funds.",
        state.config.origin,
        auth.user.id,
        input.public_key,
        nonce,
        issued.to_rfc3339(),
        expires.to_rfc3339()
    );
    sqlx::query("INSERT INTO wallet_challenges(id,user_id,session_hash,public_key,message,expires_at) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(id).bind(auth.user.id).bind(&auth.token_hash).bind(&input.public_key).bind(&message).bind(expires).execute(&state.pool).await?;
    Ok(Json(
        serde_json::json!({"id":id,"message":message,"expires_at":expires}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkInput {
    challenge_id: Uuid,
    signature: String,
}
pub async fn link(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<LinkInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    auth.csrf(&headers)?;
    auth.recent()?;
    rate_limit(&state, &format!("wallet-link:{}", auth.user.id), 10, 900).await?;
    if input.signature.len() > 100 {
        return Err(ApiError::bad("Neplatný podpis."));
    }
    let bytes = STANDARD
        .decode(input.signature)
        .map_err(|_| ApiError::bad("Neplatný podpis."))?;
    let signature =
        Signature::from_slice(&bytes).map_err(|_| ApiError::bad("Neplatná délka podpisu."))?;
    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, auth.user.id).await?;
    let (address,message,expires,used)=sqlx::query_as::<_,(String,String,DateTime<Utc>,Option<DateTime<Utc>>)>("SELECT public_key,message,expires_at,used_at FROM wallet_challenges WHERE id=$1 AND user_id=$2 AND session_hash=$3 FOR UPDATE")
        .bind(input.challenge_id).bind(auth.user.id).bind(&auth.token_hash).fetch_optional(&mut *tx).await?.ok_or_else(ApiError::not_found)?;
    if used.is_some() || expires <= Utc::now() {
        return Err(ApiError::conflict(
            "Výzva již byla použita nebo vypršela. Vytvořte novou.",
        ));
    }
    public_key(&address)?
        .verify_strict(message.as_bytes(), &signature)
        .map_err(|_| ApiError::bad("Podpis neodpovídá peněžence a výzvě."))?;
    if sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM wallets WHERE public_key=$1 AND user_id<>$2)",
    )
    .bind(&address)
    .bind(auth.user.id)
    .fetch_one(&mut *tx)
    .await?
    {
        return Err(ApiError::conflict(
            "Peněženku nelze propojit s tímto účtem.",
        ));
    }
    // No silent wallet replacement: explicitly unlink first while no funds are supported.
    let existing: Option<String> =
        sqlx::query_scalar("SELECT public_key FROM wallets WHERE user_id=$1")
            .bind(auth.user.id)
            .fetch_optional(&mut *tx)
            .await?;
    if existing.as_ref().is_some_and(|old| old != &address) {
        return Err(ApiError::conflict("Nejprve odpojte současnou peněženku."));
    }
    if existing.is_none() {
        let result = sqlx::query(
            "INSERT INTO wallets(id,user_id,public_key) VALUES($1,$2,$3) ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(auth.user.id)
        .bind(&address)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(ApiError::conflict(
                "Peněženku nelze propojit s tímto účtem.",
            ));
        }
    }
    sqlx::query("UPDATE wallet_challenges SET used_at=now() WHERE id=$1")
        .bind(input.challenge_id)
        .execute(&mut *tx)
        .await?;
    event(&mut tx, auth.user.id, "wallet_linked").await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({"public_key":address})))
}
pub async fn unlink(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    auth.csrf(&headers)?;
    auth.recent()?;
    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, auth.user.id).await?;
    sqlx::query("DELETE FROM wallets WHERE user_id=$1")
        .bind(auth.user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM wallet_challenges WHERE user_id=$1")
        .bind(auth.user.id)
        .execute(&mut *tx)
        .await?;
    event(&mut tx, auth.user.id, "wallet_unlinked").await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({"ok":true})))
}
