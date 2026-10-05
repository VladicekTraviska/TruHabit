use crate::{AppState, crypto, error::ApiError};
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::Utc;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    transport::smtp::authentication::Credentials,
};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
pub struct MailPayload {
    pub to: String,
    pub subject: String,
    pub body: String,
}
pub fn encrypt(key: &[u8; 32], id: Uuid, payload: &MailPayload) -> Result<String, ApiError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| ApiError::internal())?;
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|_| ApiError::internal())?;
    let json = serde_json::to_vec(payload).map_err(|_| ApiError::internal())?;
    let encrypted = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &json,
                aad: id.as_bytes(),
            },
        )
        .map_err(|_| ApiError::internal())?;
    let mut bytes = nonce.to_vec();
    bytes.extend(encrypted);
    Ok(STANDARD.encode(bytes))
}
pub fn decrypt(key: &[u8; 32], id: Uuid, encoded: &str) -> Result<MailPayload, ApiError> {
    let bytes = STANDARD.decode(encoded).map_err(|_| ApiError::internal())?;
    if bytes.len() < 28 {
        return Err(ApiError::internal());
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| ApiError::internal())?;
    let json = cipher
        .decrypt(
            Nonce::from_slice(&bytes[..12]),
            Payload {
                msg: &bytes[12..],
                aad: id.as_bytes(),
            },
        )
        .map_err(|_| ApiError::internal())?;
    serde_json::from_slice(&json).map_err(|_| ApiError::internal())
}
pub async fn enqueue_token(
    state: &AppState,
    tx: &mut Transaction<'_, Postgres>,
    user: Uuid,
    email: &str,
    purpose: &str,
) -> Result<(), ApiError> {
    let key = state
        .config
        .mail_key
        .as_ref()
        .ok_or_else(ApiError::internal)?;
    let raw = crypto::token()?;
    let expiry = if purpose == "reset_password" {
        chrono::Duration::minutes(30)
    } else {
        chrono::Duration::hours(24)
    };
    sqlx::query("INSERT INTO auth_tokens(token_hash,user_id,purpose,expires_at) VALUES($1,$2,$3,$4) ON CONFLICT(user_id,purpose) DO UPDATE SET token_hash=excluded.token_hash,expires_at=excluded.expires_at,created_at=now()")
        .bind(crypto::digest(&raw)).bind(user).bind(purpose).bind(Utc::now()+expiry).execute(&mut **tx).await?;
    let (subject, instruction) = if purpose == "reset_password" {
        (
            "TruHabit password reset / Obnovení hesla",
            "Set a new password. This link expires in 30 minutes.\nNastavte nové heslo. Odkaz platí 30 minut.",
        )
    } else {
        (
            "Verify your TruHabit email / Ověření e-mailu",
            "Verify your email address. This link expires in 24 hours.\nPotvrďte svoji e-mailovou adresu. Odkaz platí 24 hodin.",
        )
    };
    let payload = MailPayload {
        to: email.into(),
        subject: subject.into(),
        body: format!(
            "{instruction}\n\n{}/#{purpose}={raw}\n\nIf you did not request this message, ignore it.\nPokud jste o tuto zprávu nežádali, ignorujte ji.",
            state.config.origin
        ),
    };
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO mail_outbox(id,user_id,payload_encrypted) VALUES($1,$2,$3)")
        .bind(id)
        .bind(user)
        .bind(encrypt(key, id, &payload)?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn deliver_one(state: &AppState) -> Result<bool, ApiError> {
    if !state.config.mail_enabled() {
        return Ok(false);
    }
    let lease = Uuid::new_v4();
    let row=sqlx::query_as::<_,(Uuid,String,i32)>("UPDATE mail_outbox SET lease_id=$1,lease_until=now()+interval '90 seconds',attempts=attempts+1 WHERE id=(SELECT id FROM mail_outbox WHERE sent_at IS NULL AND attempts<5 AND available_at<=now() AND (lease_until IS NULL OR lease_until<now()) ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING id,payload_encrypted,attempts")
        .bind(lease).fetch_optional(&state.pool).await?;
    let Some((id, encrypted, attempts)) = row else {
        return Ok(false);
    };
    let delivered = send(state, id, &encrypted).await.is_ok();
    if delivered {
        sqlx::query("UPDATE mail_outbox SET sent_at=now(),payload_encrypted='',lease_until=NULL,lease_id=NULL WHERE id=$1 AND lease_id=$2").bind(id).bind(lease).execute(&state.pool).await?;
    } else {
        eprintln!("Verification email delivery failed; retry tracked for job {id}.");
        let backoff = 30_f64 * 2_f64.powi(attempts.min(5));
        sqlx::query("UPDATE mail_outbox SET available_at=now()+make_interval(secs=>$1),lease_until=NULL,lease_id=NULL WHERE id=$2 AND lease_id=$3").bind(backoff).bind(id).bind(lease).execute(&state.pool).await?;
    }
    Ok(true)
}
async fn send(state: &AppState, id: Uuid, encrypted: &str) -> Result<(), ApiError> {
    let c = &state.config;
    let payload = decrypt(
        c.mail_key.as_ref().ok_or_else(ApiError::internal)?,
        id,
        encrypted,
    )?;
    let message = Message::builder()
        .from(
            c.mail_from
                .as_ref()
                .ok_or_else(ApiError::internal)?
                .parse()
                .map_err(|_| ApiError::internal())?,
        )
        .to(payload.to.parse().map_err(|_| ApiError::internal())?)
        .subject(payload.subject)
        .message_id(Some(format!(
            "{id}@{}",
            url::Url::parse(&c.origin)
                .map_err(|_| ApiError::internal())?
                .host_str()
                .ok_or_else(ApiError::internal)?
        )))
        .body(payload.body)
        .map_err(|_| ApiError::internal())?;
    let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(
        c.smtp_host.as_ref().ok_or_else(ApiError::internal)?,
    )
    .map_err(|_| ApiError::internal())?
    .credentials(Credentials::new(
        c.smtp_user.clone().ok_or_else(ApiError::internal)?,
        c.smtp_password.clone().ok_or_else(ApiError::internal)?,
    ))
    .timeout(Some(std::time::Duration::from_secs(15)))
    .build();
    transport
        .send(message)
        .await
        .map_err(|_| ApiError::internal())?;
    Ok(())
}
pub async fn cleanup(state: &AppState) -> Result<(), ApiError> {
    sqlx::query(
        "DELETE FROM sessions WHERE expires_at<=now() OR last_seen_at<now()-interval '2 hours'",
    )
    .execute(&state.pool)
    .await?;
    sqlx::query("DELETE FROM auth_tokens WHERE expires_at<=now()")
        .execute(&state.pool)
        .await?;
    sqlx::query("DELETE FROM wallet_challenges WHERE expires_at<now()-interval '1 day'")
        .execute(&state.pool)
        .await?;
    sqlx::query("DELETE FROM rate_limits WHERE reset_at<now()-interval '1 day'")
        .execute(&state.pool)
        .await?;
    sqlx::query("DELETE FROM mail_outbox WHERE created_at<now()-interval '7 days'")
        .execute(&state.pool)
        .await?;
    sqlx::query("DELETE FROM security_events WHERE created_at<now()-interval '90 days'")
        .execute(&state.pool)
        .await?;
    Ok(())
}
