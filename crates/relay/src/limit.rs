//! Rate limits: token buckets per client IP on the routes anyone can call, and per identity
//! on everything behind a bearer token. Both live in memory; the hosted relay is one process.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::Instant;

use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::routes::ApiError;
use crate::AppState;

pub struct RateLimiter {
    /// Tokens added per second; the bucket holds `burst` of them.
    rate: f64,
    burst: f64,
    buckets: Mutex<HashMap<String, Bucket>>,
}

struct Bucket {
    tokens: f64,
    refilled: Instant,
}

impl RateLimiter {
    /// `per_second` sustained, with a burst of `burst` requests. A rate of 0 disables the limit.
    pub fn new(per_second: f64, burst: u32) -> RateLimiter {
        RateLimiter { rate: per_second, burst: burst.max(1) as f64, buckets: Mutex::new(HashMap::new()) }
    }

    /// Takes one token for `key`; `Err` carries the seconds until one is back.
    pub fn check(&self, key: &str) -> Result<(), u64> {
        if self.rate <= 0.0 {
            return Ok(());
        }
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if buckets.len() >= 100_000 {
            let (rate, burst) = (self.rate, self.burst);
            buckets.retain(|_, b| b.tokens + now.duration_since(b.refilled).as_secs_f64() * rate < burst);
        }
        let bucket = buckets.entry(key.to_string()).or_insert(Bucket { tokens: self.burst, refilled: now });
        bucket.tokens = (bucket.tokens + now.duration_since(bucket.refilled).as_secs_f64() * self.rate).min(self.burst);
        bucket.refilled = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(((1.0 - bucket.tokens) / self.rate).ceil().max(1.0) as u64)
        }
    }
}

/// The caller's address: the last hop of `X-Forwarded-For` when the relay trusts its proxy,
/// otherwise the socket's peer.
fn client_ip(request: &Request, trust_proxy: bool) -> IpAddr {
    if trust_proxy {
        let forwarded = request
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse::<IpAddr>().ok());
        if let Some(ip) = forwarded {
            return ip;
        }
    }
    request.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip()).unwrap_or(IpAddr::from([0, 0, 0, 0]))
}

/// Middleware for the routes that need no token: registration, auth, and the pairing mailbox.
pub async fn per_ip(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let ip = client_ip(&request, state.trust_proxy);
    match state.ip_limiter.check(&ip.to_string()) {
        Ok(()) => next.run(request).await,
        Err(retry_after) => {
            tracing::debug!(%ip, path = %request.uri().path(), "rate limited");
            ApiError::too_many(retry_after).into_response()
        }
    }
}
