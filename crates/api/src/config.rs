use std::{
    env,
    net::{IpAddr, SocketAddr},
};
use url::Url;

#[derive(Clone)]
pub struct Config {
    pub production: bool,
    pub origin: String,
    pub bind: SocketAddr,
    pub trusted_proxy_ip: Option<IpAddr>,
    pub database_url: String,
    pub mail_key: Option<[u8; 32]>,
    pub smtp_host: Option<String>,
    pub smtp_user: Option<String>,
    pub smtp_password: Option<String>,
    pub mail_from: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let production = match env::var("APP_ENV")
            .unwrap_or_else(|_| "development".into())
            .as_str()
        {
            "production" => true,
            "development" => false,
            _ => return Err("APP_ENV must be production or development".into()),
        };
        let origin = env::var("SITE_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:8787".into());
        let bind = env::var("BIND_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:8787".into())
            .parse()
            .map_err(|_| "Invalid BIND_ADDR")?;
        let database_url = env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required")?;
        let mail_key = env::var("MAIL_ENCRYPTION_KEY")
            .ok()
            .map(|v| {
                let bytes =
                    hex::decode(v).map_err(|_| "MAIL_ENCRYPTION_KEY must be 64 hex characters")?;
                <[u8; 32]>::try_from(bytes).map_err(|_| "MAIL_ENCRYPTION_KEY must be 32 bytes")
            })
            .transpose()?;
        let c = Self {
            production,
            origin,
            bind,
            trusted_proxy_ip: env::var("TRUSTED_PROXY_IP")
                .ok()
                .map(|v| v.parse().map_err(|_| "Invalid TRUSTED_PROXY_IP"))
                .transpose()?,
            database_url,
            mail_key,
            smtp_host: env::var("SMTP_HOST").ok(),
            smtp_user: env::var("SMTP_USERNAME").ok(),
            smtp_password: env::var("SMTP_PASSWORD").ok(),
            mail_from: env::var("MAIL_FROM").ok(),
        };
        c.validate()?;
        Ok(c)
    }
    pub fn validate(&self) -> Result<(), String> {
        let url = Url::parse(&self.origin).map_err(|_| "Invalid SITE_ORIGIN")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.origin().ascii_serialization() != self.origin
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "SITE_ORIGIN must be a canonical origin without path, query or credentials".into(),
            );
        }
        let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if self.production && (url.scheme() != "https" || local) {
            return Err("Production requires a public HTTPS SITE_ORIGIN".into());
        }
        if !self.production && (!local || !self.bind.ip().is_loopback()) {
            return Err("Development mode is restricted to loopback".into());
        }
        let db = Url::parse(&self.database_url).map_err(|_| "Invalid DATABASE_URL")?;
        if !matches!(db.scheme(), "postgres" | "postgresql") {
            return Err("PostgreSQL is required".into());
        }
        let mail_parts = [
            self.mail_key.is_some(),
            self.smtp_host.is_some(),
            self.smtp_user.is_some(),
            self.smtp_password.is_some(),
            self.mail_from.is_some(),
        ];
        if [
            &self.smtp_host,
            &self.smtp_user,
            &self.smtp_password,
            &self.mail_from,
        ]
        .iter()
        .any(|v| v.as_ref().is_some_and(|s| s.trim().is_empty()))
        {
            return Err("SMTP fields cannot be empty".into());
        }
        if self
            .mail_from
            .as_ref()
            .is_some_and(|v| v.parse::<lettre::message::Mailbox>().is_err())
        {
            return Err("Invalid MAIL_FROM mailbox".into());
        }
        if mail_parts.iter().any(|v| *v) && !mail_parts.iter().all(|v| *v) {
            return Err("Configure all SMTP fields and MAIL_ENCRYPTION_KEY together".into());
        }
        if self.production && !self.mail_enabled() {
            return Err(
                "Production registration requires configured verification email delivery".into(),
            );
        }
        Ok(())
    }
    pub fn mail_enabled(&self) -> bool {
        self.mail_key.is_some() && self.smtp_host.is_some() && self.mail_from.is_some()
    }
    pub fn cookie_name(&self) -> &'static str {
        if self.production {
            "__Host-truhabit"
        } else {
            "truhabit_local"
        }
    }
    pub fn allowed_origin(&self, origin: &str) -> bool {
        origin == self.origin || (!self.production && origin == "http://127.0.0.1:5173")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            production: false,
            origin: "http://127.0.0.1:8787".into(),
            bind: "127.0.0.1:8787".parse().unwrap(),
            trusted_proxy_ip: None,
            database_url: "postgres://app:placeholder@localhost/truhabit".into(),
            mail_key: None,
            smtp_host: None,
            smtp_user: None,
            smtp_password: None,
            mail_from: None,
        }
    }
    #[test]
    fn development_cannot_be_exposed_or_use_non_http_origin() {
        let mut c = config();
        assert!(c.validate().is_ok());
        c.bind = "0.0.0.0:8787".parse().unwrap();
        assert!(c.validate().is_err());
        c = config();
        c.origin = "ftp://127.0.0.1:8787".into();
        assert!(c.validate().is_err());
        c = config();
        c.origin = "https://public.example".into();
        assert!(c.validate().is_err());
        c = config();
        c.origin.push('/');
        assert!(c.validate().is_err());
    }
    #[test]
    fn production_requires_https_complete_mail_and_secure_cookie_name() {
        let mut c = config();
        c.production = true;
        assert!(c.validate().is_err());
        c.origin = "https://app.example.com".into();
        assert!(c.validate().is_err());
        c.mail_key = Some([1; 32]);
        c.smtp_host = Some("smtp.example.com".into());
        c.smtp_user = Some("sender".into());
        c.smtp_password = Some("test-placeholder".into());
        c.mail_from = Some("TruHabit <support@example.com>".into());
        assert!(c.validate().is_ok());
        assert_eq!(c.cookie_name(), "__Host-truhabit");
        assert!(!c.allowed_origin("http://127.0.0.1:5173"));
        c.mail_from = Some("invalid".into());
        assert!(c.validate().is_err());
        c.mail_from = Some("support@example.com".into());
        c.smtp_password = Some("".into());
        assert!(c.validate().is_err());
    }
}
