//! The edge must OVERWRITE X-Real-IP, preserve Host, and be the only public ingress.
use crate::{AppState, error::ApiError};
use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::{HeaderMap, request::Parts},
};
use std::net::{IpAddr, SocketAddr};

pub struct ClientIp(pub IpAddr);

fn resolve(peer: IpAddr, trusted: Option<IpAddr>, headers: &HeaderMap) -> Result<IpAddr, ApiError> {
    if trusted != Some(peer) {
        return Ok(peer);
    }
    let mut values = headers.get_all("x-real-ip").iter();
    let value = values
        .next()
        .and_then(|v| v.to_str().ok())
        .ok_or_else(ApiError::forbidden)?;
    if values.next().is_some() {
        return Err(ApiError::forbidden());
    }
    value.parse().map_err(|_| ApiError::forbidden())
}

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = ApiError;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .ok_or_else(ApiError::internal)?
            .0
            .ip();
        Ok(Self(resolve(
            peer,
            state.config.trusted_proxy_ip,
            &parts.headers,
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn untrusted_headers_cannot_change_rate_limit_identity() {
        let peer = "192.0.2.10".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "198.51.100.42".parse().unwrap());
        headers.insert("x-forwarded-for", "203.0.113.9".parse().unwrap());
        assert_eq!(resolve(peer, None, &headers).unwrap(), peer);
        assert_eq!(
            resolve(peer, Some("127.0.0.1".parse().unwrap()), &headers).unwrap(),
            peer
        );
    }
    #[test]
    fn trusted_proxy_requires_exactly_one_valid_ip() {
        let peer = "127.0.0.1".parse().unwrap();
        let mut headers = HeaderMap::new();
        assert!(resolve(peer, Some(peer), &headers).is_err());
        headers.insert("x-real-ip", "198.51.100.42, 192.0.2.10".parse().unwrap());
        assert!(resolve(peer, Some(peer), &headers).is_err());
        headers.insert("x-real-ip", "198.51.100.42".parse().unwrap());
        assert_eq!(
            resolve(peer, Some(peer), &headers).unwrap(),
            "198.51.100.42".parse::<IpAddr>().unwrap()
        );
        headers.append("x-real-ip", "198.51.100.43".parse().unwrap());
        assert!(resolve(peer, Some(peer), &headers).is_err());
    }
}
