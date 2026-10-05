use crate::network::ClientIp;
use crate::{AppState, crypto, error::ApiError, mail};
use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::{HeaderMap, HeaderValue, StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

#[derive(Serialize, FromRow, Clone)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
#[derive(Clone)]
pub struct Auth {
    pub user: User,
    pub token_hash: String,
    pub csrf_token: String,
    pub session_created_at: DateTime<Utc>,
}
impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let name = state.config.cookie_name();
        let cookie = parts
            .headers
            .get("cookie")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| {
                s.split(';').find_map(|v| {
                    v.trim()
                        .split_once('=')
                        .filter(|(key, _)| *key == name)
                        .map(|(_, value)| value)
                })
            })
            .ok_or_else(ApiError::unauthorized)?;
        if cookie.len() != 64 || !cookie.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ApiError::unauthorized());
        }
        let token_hash = crypto::digest(cookie);
        let row=sqlx::query_as::<_,(Uuid,String,DateTime<Utc>)>("UPDATE sessions SET last_seen_at=now() WHERE token_hash=$1 AND expires_at>now() AND last_seen_at>now()-interval '2 hours' RETURNING user_id,csrf_token,created_at")
            .bind(&token_hash).fetch_optional(&state.pool).await?.ok_or_else(ApiError::unauthorized)?;
        let user = sqlx::query_as::<_, User>(
            "SELECT id,email,display_name,email_verified_at,created_at FROM users WHERE id=$1",
        )
        .bind(row.0)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
        Ok(Self {
            user,
            token_hash,
            csrf_token: row.1,
            session_created_at: row.2,
        })
    }
}
impl Auth {
    pub fn csrf(&self, headers: &HeaderMap) -> Result<(), ApiError> {
        if headers.get("x-csrf-token").and_then(|v| v.to_str().ok()) != Some(&self.csrf_token) {
            return Err(ApiError::forbidden());
        }
        Ok(())
    }
    pub fn recent(&self) -> Result<(), ApiError> {
        if self.session_created_at < Utc::now() - Duration::minutes(15) {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "REAUTH_REQUIRED",
                "Pro tuto změnu se nejprve znovu přihlaste.",
            ));
        }
        Ok(())
    }
}
pub async fn event(
    tx: &mut Transaction<'_, Postgres>,
    user: Uuid,
    kind: &str,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO security_events(user_id,kind) VALUES($1,$2)")
        .bind(user)
        .bind(kind)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
pub async fn rate_limit(
    state: &AppState,
    key: &str,
    limit: i32,
    seconds: i32,
) -> Result<(), ApiError> {
    let count:i32=sqlx::query_scalar("INSERT INTO rate_limits(key_hash,count,reset_at) VALUES($1,1,now()+make_interval(secs=>$2)) ON CONFLICT(key_hash) DO UPDATE SET count=CASE WHEN rate_limits.reset_at<=now() THEN 1 ELSE rate_limits.count+1 END, reset_at=CASE WHEN rate_limits.reset_at<=now() THEN excluded.reset_at ELSE rate_limits.reset_at END RETURNING count")
        .bind(crypto::digest(key)).bind(f64::from(seconds)).fetch_one(&state.pool).await?;
    if count > limit {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "RATE_LIMITED",
            "Příliš mnoho pokusů. Zkuste to prosím později.",
        ));
    }
    Ok(())
}
pub async fn lock_user(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<String, ApiError> {
    sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(ApiError::unauthorized)
}
pub(crate) async fn reauthenticate(
    state: &AppState,
    tx: &mut Transaction<'_, Postgres>,
    user: Uuid,
    password: String,
) -> Result<(), ApiError> {
    if password.len() > 512 {
        return Err(ApiError::bad("Heslo je příliš dlouhé."));
    }
    let hash = lock_user(tx, user).await?;
    let _permit = state
        .hashing
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::unavailable("Přihlášení je vytížené. Zkuste to znovu."))?;
    if !crypto::verify_password(password, hash).await? {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "INVALID_CREDENTIALS",
            "Současné heslo není správné.",
        ));
    }
    Ok(())
}
fn cookie(state: &AppState, value: &str, remove: bool) -> Result<HeaderValue, ApiError> {
    let age = if remove { 0 } else { 86400 };
    HeaderValue::from_str(&format!(
        "{}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={age}{}",
        state.config.cookie_name(),
        if state.config.production {
            "; Secure"
        } else {
            ""
        }
    ))
    .map_err(|_| ApiError::internal())
}
fn with_cookie(
    state: &AppState,
    body: serde_json::Value,
    value: &str,
    remove: bool,
) -> Result<Response, ApiError> {
    let mut response = Json(body).into_response();
    response.headers_mut().insert(
        axum::http::header::SET_COOKIE,
        cookie(state, value, remove)?,
    );
    Ok(response)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    email: String,
    password: String,
    display_name: String,
}
pub async fn register(
    State(state): State<AppState>,
    ClientIp(peer): ClientIp,
    Json(input): Json<Registration>,
) -> Result<Json<serde_json::Value>, ApiError> {
    rate_limit(&state, &format!("register:{}", peer), 10, 3600).await?;
    let email = input.email.trim().to_lowercase();
    let name = input.display_name.trim();
    if !crypto::valid_email(&email)
        || !crypto::valid_password(&input.password)
        || !(1..=80).contains(&name.chars().count())
        || name.chars().any(char::is_control)
    {
        return Err(ApiError::bad(
            "Zadejte platný e-mail, jméno do 80 znaků a heslo s 15 až 128 znaky.",
        ));
    }
    let _permit = state
        .hashing
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::unavailable("Registrace je vytížená. Zkuste to znovu."))?;
    let hash = crypto::hash_password(input.password).await?;
    let mut tx = state.pool.begin().await?;
    let id=sqlx::query_scalar::<_,Uuid>("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,$3,$4) ON CONFLICT(email) DO NOTHING RETURNING id")
        .bind(Uuid::new_v4()).bind(&email).bind(name).bind(hash).fetch_optional(&mut *tx).await?;
    if let Some(id) = id {
        event(&mut tx, id, "registered").await?;
        if state.config.mail_enabled() {
            mail::enqueue_token(&state, &mut tx, id, &email, "verify_email").await?;
        }
    }
    tx.commit().await?;
    Ok(Json(
        serde_json::json!({"message":"Pokud bylo možné účet založit, je připravený k přihlášení. U existujícího účtu použijte původní heslo.","email_delivery":state.config.mail_enabled()}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Login {
    email: String,
    password: String,
}
pub async fn login(
    State(state): State<AppState>,
    ClientIp(peer): ClientIp,
    Json(input): Json<Login>,
) -> Result<Response, ApiError> {
    let email = input.email.trim().to_lowercase();
    rate_limit(&state, &format!("login-ip:{}", peer), 40, 900).await?;
    rate_limit(&state, &format!("login-account:{email}"), 10, 900).await?;
    if input.password.len() > 512 || email.len() > 254 {
        return Err(ApiError::bad("Neplatný rozsah přihlašovacích údajů."));
    }
    let row =
        sqlx::query_as::<_, (Uuid, String)>("SELECT id,password_hash FROM users WHERE email=$1")
            .bind(email)
            .fetch_optional(&state.pool)
            .await?;
    let hash = row
        .as_ref()
        .map(|r| r.1.clone())
        .unwrap_or_else(|| (*state.dummy_hash).clone());
    let _permit = state
        .hashing
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::unavailable("Přihlášení je vytížené. Zkuste to znovu."))?;
    let valid = crypto::verify_password(input.password, hash).await?;
    let Some((user_id, original_hash)) = row.filter(|_| valid) else {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "INVALID_CREDENTIALS",
            "E-mail nebo heslo není správné.",
        ));
    };
    let raw = crypto::token()?;
    let csrf = crypto::token()?;
    let mut tx = state.pool.begin().await?;
    if lock_user(&mut tx, user_id).await? != original_hash {
        return Err(ApiError::unauthorized());
    }
    sqlx::query("DELETE FROM sessions WHERE user_id=$1 AND (expires_at<=now() OR last_seen_at<now()-interval '2 hours')").bind(user_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM sessions WHERE token_hash IN (SELECT token_hash FROM sessions WHERE user_id=$1 ORDER BY created_at DESC OFFSET 9)").bind(user_id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf_token,expires_at) VALUES($1,$2,$3,now()+interval '24 hours')")
        .bind(crypto::digest(&raw)).bind(user_id).bind(&csrf).execute(&mut *tx).await?;
    event(&mut tx, user_id, "logged_in").await?;
    tx.commit().await?;
    with_cookie(&state, serde_json::json!({"csrf_token":csrf}), &raw, false)
}

pub async fn session(
    auth: Auth,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let wallet: Option<String> =
        sqlx::query_scalar("SELECT public_key FROM wallets WHERE user_id=$1")
            .bind(auth.user.id)
            .fetch_optional(&state.pool)
            .await?;
    Ok(Json(
        serde_json::json!({"user":auth.user,"csrf_token":auth.csrf_token,"wallet":wallet}),
    ))
}
pub async fn logout(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    auth.csrf(&headers)?;
    sqlx::query("DELETE FROM sessions WHERE token_hash=$1")
        .bind(&auth.token_hash)
        .execute(&state.pool)
        .await?;
    with_cookie(&state, serde_json::json!({"ok":true}), "", true)
}
pub async fn logout_all(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    auth.csrf(&headers)?;
    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, auth.user.id).await?;
    sqlx::query("DELETE FROM sessions WHERE user_id=$1")
        .bind(auth.user.id)
        .execute(&mut *tx)
        .await?;
    event(&mut tx, auth.user.id, "all_sessions_revoked").await?;
    tx.commit().await?;
    with_cookie(&state, serde_json::json!({"ok":true}), "", true)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasswordChange {
    current_password: String,
    new_password: String,
}
pub async fn change_password(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<PasswordChange>,
) -> Result<Response, ApiError> {
    auth.csrf(&headers)?;
    rate_limit(&state, &format!("password:{}", auth.user.id), 5, 900).await?;
    if !crypto::valid_password(&input.new_password) {
        return Err(ApiError::bad("Nové heslo musí mít 15 až 128 znaků."));
    }
    let mut tx = state.pool.begin().await?;
    reauthenticate(&state, &mut tx, auth.user.id, input.current_password).await?;
    let _permit = state
        .hashing
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::unavailable("Zkuste to znovu."))?;
    let hash = crypto::hash_password(input.new_password).await?;
    sqlx::query("UPDATE users SET password_hash=$1,updated_at=now() WHERE id=$2")
        .bind(hash)
        .bind(auth.user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id=$1")
        .bind(auth.user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM auth_tokens WHERE user_id=$1 AND purpose='reset_password'")
        .bind(auth.user.id)
        .execute(&mut *tx)
        .await?;
    event(&mut tx, auth.user.id, "password_changed").await?;
    tx.commit().await?;
    with_cookie(&state, serde_json::json!({"ok":true}), "", true)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmailInput {
    email: String,
}
pub async fn forgot_password(
    State(state): State<AppState>,
    ClientIp(peer): ClientIp,
    Json(input): Json<EmailInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !state.config.mail_enabled() {
        return Err(ApiError::unavailable(
            "Obnova hesla e-mailem nyní není dostupná.",
        ));
    }
    let email = input.email.trim().to_lowercase();
    rate_limit(&state, &format!("recovery-ip:{}", peer), 10, 3600).await?;
    rate_limit(&state, &format!("recovery-email:{email}"), 3, 3600).await?;
    if !crypto::valid_email(&email) {
        return Err(ApiError::bad("Zadejte platný e-mail."));
    }
    let mut tx = state.pool.begin().await?;
    if let Some(id) =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE email=$1 FOR UPDATE")
            .bind(&email)
            .fetch_optional(&mut *tx)
            .await?
    {
        mail::enqueue_token(&state, &mut tx, id, &email, "reset_password").await?;
    }
    tx.commit().await?;
    Ok(Json(
        serde_json::json!({"message":"Pokud účet existuje, odešleme postup pro obnovení hesla."}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenInput {
    token: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResetInput {
    token: String,
    new_password: String,
}
async fn consume_token(
    tx: &mut Transaction<'_, Postgres>,
    token: String,
    purpose: &str,
) -> Result<Uuid, ApiError> {
    if token.len() != 64 {
        return Err(ApiError::bad("Odkaz je neplatný."));
    }
    let hash = crypto::digest(token);
    let id = sqlx::query_scalar::<_, Uuid>(
        "SELECT user_id FROM auth_tokens WHERE token_hash=$1 AND purpose=$2 AND expires_at>now()",
    )
    .bind(&hash)
    .bind(purpose)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| ApiError::bad("Odkaz je neplatný nebo již vypršel."))?;
    lock_user(tx, id).await?;
    let count = sqlx::query(
        "DELETE FROM auth_tokens WHERE token_hash=$1 AND purpose=$2 AND expires_at>now()",
    )
    .bind(hash)
    .bind(purpose)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if count != 1 {
        return Err(ApiError::bad("Odkaz už byl použit nebo vypršel."));
    }
    Ok(id)
}
pub async fn reset_password(
    State(state): State<AppState>,
    ClientIp(peer): ClientIp,
    Json(input): Json<ResetInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    rate_limit(&state, &format!("reset:{}", peer), 10, 900).await?;
    if !crypto::valid_password(&input.new_password) {
        return Err(ApiError::bad("Heslo musí mít 15 až 128 znaků."));
    }
    let mut tx = state.pool.begin().await?;
    let id = consume_token(&mut tx, input.token, "reset_password").await?;
    let _permit = state
        .hashing
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::unavailable("Zkuste to znovu."))?;
    let hash = crypto::hash_password(input.new_password).await?;
    sqlx::query("UPDATE users SET password_hash=$1,updated_at=now() WHERE id=$2")
        .bind(hash)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    event(&mut tx, id, "password_reset").await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({"ok":true})))
}
pub async fn verify_email(
    State(state): State<AppState>,
    ClientIp(peer): ClientIp,
    Json(input): Json<TokenInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    rate_limit(&state, &format!("verify:{}", peer), 15, 900).await?;
    let mut tx = state.pool.begin().await?;
    let id = consume_token(&mut tx, input.token, "verify_email").await?;
    sqlx::query("UPDATE users SET email_verified_at=now(),updated_at=now() WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    event(&mut tx, id, "email_verified").await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({"ok":true})))
}
pub async fn resend_verification(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    auth.csrf(&headers)?;
    if !state.config.mail_enabled() {
        return Err(ApiError::unavailable(
            "Ověřovací e-mail nyní nelze odeslat.",
        ));
    }
    rate_limit(&state, &format!("resend:{}", auth.user.id), 3, 3600).await?;
    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, auth.user.id).await?;
    if auth.user.email_verified_at.is_none() {
        mail::enqueue_token(
            &state,
            &mut tx,
            auth.user.id,
            &auth.user.email,
            "verify_email",
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(serde_json::json!({"ok":true})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    display_name: String,
}
pub async fn profile(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Profile>,
) -> Result<Json<serde_json::Value>, ApiError> {
    auth.csrf(&headers)?;
    let name = input.display_name.trim();
    if !(1..=80).contains(&name.chars().count()) || name.chars().any(char::is_control) {
        return Err(ApiError::bad("Jméno musí mít 1 až 80 znaků."));
    }
    sqlx::query("UPDATE users SET display_name=$1,updated_at=now() WHERE id=$2")
        .bind(name)
        .bind(auth.user.id)
        .execute(&state.pool)
        .await?;
    Ok(Json(serde_json::json!({"ok":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteAccount {
    password: String,
    confirmation: String,
}
pub async fn delete_account(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<DeleteAccount>,
) -> Result<Response, ApiError> {
    auth.csrf(&headers)?;
    rate_limit(&state, &format!("delete:{}", auth.user.id), 5, 900).await?;
    if input.confirmation != "SMAZAT" && input.confirmation != "DELETE" {
        return Err(ApiError::bad("Potvrďte smazání účtu zadáním SMAZAT."));
    }
    let mut tx = state.pool.begin().await?;
    reauthenticate(&state, &mut tx, auth.user.id, input.password).await?;
    let owns_company: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM organizations WHERE owner_id=$1)")
            .bind(auth.user.id)
            .fetch_one(&mut *tx)
            .await?;
    if owns_company {
        return Err(ApiError::conflict(
            "Nejdříve odstraňte své firemní prostory v sekci Pro firmy.",
        ));
    }
    let active_prototype: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM prototype_challenges c WHERE user_id=$1 AND (state='ACTIVE' OR EXISTS(SELECT 1 FROM prototype_commands p WHERE p.challenge_id=c.id AND p.status IN ('PREPARED','SIGNED'))))")
        .bind(auth.user.id).fetch_one(&mut *tx).await?;
    if active_prototype {
        return Err(ApiError::conflict("ACTIVE_PROTOTYPE_MUST_SETTLE"));
    }
    let active_business: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM company_programs WHERE funder_id=$1 AND state='PUBLISHED') OR EXISTS(SELECT 1 FROM company_enrollments e JOIN company_programs p ON p.id=e.program_id WHERE e.user_id=$1 AND p.state='PUBLISHED')")
        .bind(auth.user.id).fetch_one(&mut *tx).await?;
    if active_business {
        return Err(ApiError::conflict("ACTIVE_BUSINESS_MUST_SETTLE"));
    }
    sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(auth.user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    with_cookie(&state, serde_json::json!({"ok":true}), "", true)
}
pub async fn export(
    auth: Auth,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    rate_limit(&state, &format!("export:{}", auth.user.id), 5, 3600).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let account = sqlx::query_as::<_, User>(
        "SELECT id,email,display_name,email_verified_at,created_at FROM users WHERE id=$1",
    )
    .bind(auth.user.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::unauthorized)?;
    let goals: Vec<serde_json::Value> =
        sqlx::query_scalar("SELECT to_jsonb(g) FROM goals g WHERE user_id=$1 ORDER BY created_at")
            .bind(auth.user.id)
            .fetch_all(&mut *tx)
            .await?;
    let events: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT to_jsonb(e) FROM goal_events e WHERE actor_id=$1 ORDER BY created_at",
    )
    .bind(auth.user.id)
    .fetch_all(&mut *tx)
    .await?;
    let wallets: Vec<serde_json::Value> =
        sqlx::query_scalar("SELECT to_jsonb(w) FROM wallets w WHERE user_id=$1")
            .bind(auth.user.id)
            .fetch_all(&mut *tx)
            .await?;
    let security_events: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT to_jsonb(e) FROM security_events e WHERE user_id=$1 ORDER BY created_at",
    )
    .bind(auth.user.id)
    .fetch_all(&mut *tx)
    .await?;
    let sessions: Vec<serde_json::Value> = sqlx::query_scalar("SELECT jsonb_build_object('created_at',created_at,'last_seen_at',last_seen_at,'expires_at',expires_at) FROM sessions WHERE user_id=$1 ORDER BY created_at").bind(auth.user.id).fetch_all(&mut *tx).await?;
    let organizations: Vec<serde_json::Value> = sqlx::query_scalar("SELECT jsonb_build_object('id',o.id,'name',o.name,'owner_id',o.owner_id,'created_at',o.created_at,'role',CASE WHEN o.owner_id=$1 THEN 'OWNER' ELSE m.role END) FROM organizations o LEFT JOIN organization_members m ON m.organization_id=o.id AND m.user_id=$1 WHERE o.owner_id=$1 OR m.user_id=$1")
        .bind(auth.user.id).fetch_all(&mut *tx).await?;
    let organization_events: Vec<serde_json::Value> = sqlx::query_scalar("SELECT jsonb_build_object('organization_id',organization_id,'kind',kind,'detail',detail,'created_at',created_at) FROM organization_events WHERE actor_id=$1 ORDER BY id")
        .bind(auth.user.id).fetch_all(&mut *tx).await?;
    let prototype:Vec<serde_json::Value>=sqlx::query_scalar("SELECT to_jsonb(c)-'creation_hash' FROM prototype_challenges c WHERE user_id=$1 ORDER BY created_at").bind(auth.user.id).fetch_all(&mut *tx).await?;
    let prototype_uploads:Vec<serde_json::Value>=sqlx::query_scalar("SELECT to_jsonb(u)-'content' FROM prototype_uploads u WHERE user_id=$1 ORDER BY received_at").bind(auth.user.id).fetch_all(&mut *tx).await?;
    let prototype_events:Vec<serde_json::Value>=sqlx::query_scalar("SELECT to_jsonb(e)-'actor_id' FROM prototype_events e JOIN prototype_challenges c ON c.id=e.challenge_id WHERE c.user_id=$1 ORDER BY e.id").bind(auth.user.id).fetch_all(&mut *tx).await?;
    let prototype_movements: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT to_jsonb(m) FROM prototype_local_movements m WHERE user_id=$1 ORDER BY created_at",
    )
    .bind(auth.user.id)
    .fetch_all(&mut *tx)
    .await?;
    let prototype_commands:Vec<serde_json::Value>=sqlx::query_scalar("SELECT to_jsonb(p)-'signed_transaction'-'payload' FROM prototype_commands p JOIN prototype_challenges c ON c.id=p.challenge_id WHERE c.user_id=$1 ORDER BY p.created_at").bind(auth.user.id).fetch_all(&mut *tx).await?;
    let prototype_review_requests: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT to_jsonb(r) FROM prototype_review_requests r WHERE user_id=$1 ORDER BY created_at",
    )
    .bind(auth.user.id)
    .fetch_all(&mut *tx)
    .await?;
    let business_enrollments: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT to_jsonb(e) FROM company_enrollments e WHERE user_id=$1 ORDER BY created_at",
    )
    .bind(auth.user.id)
    .fetch_all(&mut *tx)
    .await?;
    let business_programs:Vec<serde_json::Value>=sqlx::query_scalar("SELECT to_jsonb(p)-'creation_hash' FROM company_programs p WHERE funder_id=$1 OR EXISTS(SELECT 1 FROM company_enrollments e WHERE e.program_id=p.id AND e.user_id=$1) ORDER BY created_at").bind(auth.user.id).fetch_all(&mut *tx).await?;
    let business_uploads:Vec<serde_json::Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'enrollment_id',enrollment_id,'goal_result',goal_result,'decision',decision,'reason',reason,'received_at',received_at,'format',activity->'format','distance_m',activity->'distance_m','starts_at',activity->'starts_at','ends_at',activity->'ends_at','source_authenticity',activity->'source_authenticity') FROM company_uploads WHERE user_id=$1 ORDER BY received_at").bind(auth.user.id).fetch_all(&mut *tx).await?;
    let business_events:Vec<serde_json::Value>=sqlx::query_scalar("SELECT to_jsonb(e)-'actor_id' FROM company_enrollment_events e JOIN company_enrollments n ON n.id=e.enrollment_id WHERE n.user_id=$1 ORDER BY e.id").bind(auth.user.id).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(
        serde_json::json!({"exported_at":Utc::now(),"account":account,"goals":goals,"events":events,"wallets":wallets,"security_events":security_events,"sessions":sessions,"organizations":organizations,"organization_events":organization_events,"prototype_challenges":prototype,"prototype_uploads":prototype_uploads,"prototype_events":prototype_events,"prototype_movements":prototype_movements,"prototype_commands":prototype_commands,"prototype_review_requests":prototype_review_requests,"business_enrollments":business_enrollments,"business_programs":business_programs,"business_uploads":business_uploads,"business_events":business_events}),
    ))
}
