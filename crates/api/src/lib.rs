pub mod auth;
pub mod business;
pub mod business_points;
pub mod config;
pub mod crypto;
pub mod dev_reset;
pub mod error;
pub mod goals;
pub mod mail;
pub mod network;
pub mod organizations;
pub mod prototype;
pub mod prototype_chain;
pub mod prototype_local;
pub mod wallet;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request},
    http::{HeaderValue, Method},
    middleware::{self, Next},
    response::Response,
    routing::{delete, get, patch, post},
};
use config::Config;
use error::ApiError;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use std::{path::Path, str::FromStr, sync::Arc};
use tokio::sync::Semaphore;
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub dummy_hash: Arc<String>,
    pub hashing: Arc<Semaphore>,
}
impl AppState {
    pub async fn new(pool: PgPool, config: Config) -> Result<Self, ApiError> {
        Ok(Self {
            pool,
            config: Arc::new(config),
            dummy_hash: Arc::new(crypto::hash_password(crypto::token()?).await?),
            hashing: Arc::new(Semaphore::new(4)),
        })
    }
}
pub async fn connect(config: &Config) -> Result<PgPool, sqlx::Error> {
    let mut options = PgConnectOptions::from_str(&config.database_url)?;
    if config.production {
        options = options.ssl_mode(PgSslMode::VerifyFull);
    }
    PgPoolOptions::new()
        .max_connections(12)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect_with(options)
        .await
}

pub fn router(state: AppState, dist: impl AsRef<Path>) -> Router {
    Router::new()
        .route("/api/prototype/local/balance",get(prototype_local::balance))
        .route("/api/prototype/local/grant",post(prototype_local::grant))
        .route("/api/prototype/challenges/{id}/local-action",post(prototype_local::action))
        .route("/api/prototype/balance",get(prototype_chain::balance))
        .route("/api/prototype/challenges/{id}/prepare",post(prototype_chain::prepare))
        .route("/api/prototype/challenges/{id}/submit",post(prototype_chain::submit))
        .route("/api/prototype/challenges/{id}/refresh",post(prototype_chain::refresh))
        .route("/api/prototype/challenges",get(prototype::list).post(prototype::create))
        .route("/api/prototype/challenges/{id}",get(prototype::detail))
        .route("/api/prototype/review",get(prototype::review_queue))
        .route("/api/prototype/challenges/{id}/review",post(prototype::review))
        .route("/api/prototype/challenges/{id}/review-request",post(prototype::request_review))
        .route("/api/prototype/challenges/{id}/review-request/respond",post(prototype::respond_review))
        .route("/api/prototype/challenges/{id}/uploads/{upload}/source",get(prototype::source_file).delete(prototype::remove_source))
        .route("/api/prototype/challenges/{id}/upload",post(prototype::upload).layer(DefaultBodyLimit::max(truhabit_evidence::upload::MAX_BYTES)))
        .route("/api/health",get(||async {Json(serde_json::json!({"status":"ok"}))}))
        .route("/api/readiness",get(readiness))
        .route("/api/organizations",get(organizations::list).post(organizations::create))
        .route("/api/organizations/{id}",get(organizations::get).patch(organizations::rename).delete(organizations::remove))
        .route("/api/organizations/{id}/archive",post(organizations::archive))
        .route("/api/organizations/{id}/restore",post(organizations::restore))
        .route("/api/organizations/{id}/programs",post(organizations::create_program))
        .route("/api/organizations/{id}/points",get(business_points::balance))
        .route("/api/organizations/{id}/points/top-up",post(business_points::top_up))
        .route("/api/organizations/{org}/programs/{id}/next-cycle",post(business_points::next_cycle))
        .route("/api/organizations/{org}/programs/{id}",get(business::detail).patch(organizations::update_program))
        .route("/api/organizations/{org}/programs/{id}/archive",post(organizations::archive_program))
        .route("/api/organizations/{id}/invitations",post(business::invite))
        .route("/api/organization-invitations/accept",post(business::accept_invitation))
        .route("/api/organizations/{org}/invitations/{id}/revoke",post(business::revoke_invitation))
        .route("/api/organizations/{org}/members/{user}",patch(business::set_role).delete(business::remove_member))
        .route("/api/organizations/{id}/transfer-owner",post(business::transfer_owner))
        .route("/api/organizations/{org}/programs/{id}/publish",post(business::publish))
        .route("/api/organizations/{org}/programs/{id}/join",post(business::join))
        .route("/api/organizations/{org}/programs/{id}/close",post(business::close))
        .route("/api/organizations/{org}/programs/{id}/enrollments/{enrol}",get(business::enrollment_detail))
        .route("/api/organizations/{org}/programs/{id}/enrollments/{enrol}/upload",post(business::upload).layer(DefaultBodyLimit::max(truhabit_evidence::upload::MAX_BYTES)))
        .route("/api/organizations/{org}/programs/{id}/enrollments/{enrol}/claim",post(business::claim))
        .route("/api/organizations/{org}/programs/{id}/enrollments/{enrol}/review",post(business::review))
        .route("/api/organizations/{org}/programs/{id}/enrollments/{enrol}/uploads/{upload}/source",get(business::source).delete(business::remove_source))
        .route("/api/business/review",get(business::review_queue))
        .route("/api/auth/register",post(auth::register))
        .route("/api/auth/login",post(auth::login))
        .route("/api/auth/session",get(auth::session))
        .route("/api/auth/logout",post(auth::logout))
        .route("/api/auth/logout-all",post(auth::logout_all))
        .route("/api/auth/password",post(auth::change_password))
        .route("/api/auth/forgot-password",post(auth::forgot_password))
        .route("/api/auth/reset-password",post(auth::reset_password))
        .route("/api/auth/verify-email",post(auth::verify_email))
        .route("/api/auth/resend-verification",post(auth::resend_verification))
        .route("/api/account",patch(auth::profile).delete(auth::delete_account))
        .route("/api/account/export",get(auth::export))
        .route("/api/account/dev-reset",get(dev_reset::preview).post(dev_reset::reset).layer(middleware::from_fn_with_state(state.clone(),dev_reset::gate)))
        .route("/api/goals",get(goals::list).post(goals::create))
        .route("/api/goals/{id}",get(goals::get).patch(goals::update))
        .route("/api/goals/{id}/archive",post(goals::archive))
        .route("/api/goals/{id}/fund",post(goals::fund))
        .route("/api/wallet/challenge",post(wallet::challenge))
        .route("/api/wallet/link",post(wallet::link))
        .route("/api/wallet",delete(wallet::unlink))
        .fallback_service(ServeDir::new(dist))
        .layer(DefaultBodyLimit::max(16*1024))
        .layer(SetResponseHeaderLayer::overriding(axum::http::header::CACHE_CONTROL,HeaderValue::from_static("no-store")))
        .layer(SetResponseHeaderLayer::overriding(axum::http::header::X_CONTENT_TYPE_OPTIONS,HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::overriding(axum::http::header::REFERRER_POLICY,HeaderValue::from_static("no-referrer")))
        .layer(SetResponseHeaderLayer::overriding(axum::http::header::CONTENT_SECURITY_POLICY,HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'self'")))
        .layer(middleware::from_fn_with_state(state.clone(),origin_guard))
        .with_state(state)
}

async fn origin_guard(
    axum::extract::State(state): axum::extract::State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let origin = url::Url::parse(&state.config.origin).map_err(|_| ApiError::internal())?;
    let expected = origin[url::Position::BeforeHost..url::Position::AfterPort].to_string();
    let host = request
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if host != expected && !(!state.config.production && host == "127.0.0.1:5173") {
        return Err(ApiError::forbidden());
    }
    let mutating = !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if mutating {
        let origin = request
            .headers()
            .get("origin")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        if !state.config.allowed_origin(origin)
            || request
                .headers()
                .get("x-truhabit-request")
                .and_then(|h| h.to_str().ok())
                != Some("web")
        {
            return Err(ApiError::forbidden());
        }
    }
    let mut response = next.run(request).await;
    if state.config.production {
        response.headers_mut().insert(
            axum::http::header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000"),
        );
    }
    Ok(response)
}

async fn readiness(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    sqlx::query("SELECT 1").execute(&state.pool).await?;
    Ok(Json(
        serde_json::json!({"accounts":true,"email_delivery":state.config.mail_enabled(),"activity_provider":false,"deposits":false,"dev_reset":dev_reset::enabled(&state.config),"reason":"Sportovní integrace a peněžní provoz zatím nejsou dostupné. Cíle můžete připravovat bez vkladu."}),
    ))
}
