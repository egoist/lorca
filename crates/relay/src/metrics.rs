//! `GET /metrics`: what the relay can say about itself, in Prometheus' text format. The relay
//! cannot read what it stores, so counts and bytes are the whole view an operator has. The
//! route exists when `--metrics-token` is set and takes that token as a bearer.
//!
//! Totals come from the database and are the same from every relay process; they are read
//! at most once in `STATS_TTL`. Counters and the socket gauges are this process's own, since
//! it started, and carry its `instance`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{MatchedPath, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::db::Stats;
use crate::AppState;

const STATS_TTL: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Counter(AtomicU64);

impl Counter {
    pub const fn new() -> Counter {
        Counter(AtomicU64::new(0))
    }
    pub fn add(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }
    pub fn set(&self, n: u64) {
        self.0.store(n, Ordering::Relaxed);
    }
    fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// `(method, route, status)` to `(requests, seconds)`.
type Requests = BTreeMap<(String, String, u16), (u64, f64)>;

/// What this process counted since it started.
pub struct Metrics {
    pub rate_limited_ip: Counter,
    pub rate_limited_identity: Counter,
    /// Large uploads that waited out their 30 s and got `503`.
    pub uploads_refused: Counter,
    pub push_sent: [Counter; 2],
    pub push_gone: [Counter; 2],
    pub push_failed: [Counter; 2],
    pub swept_envelopes: Counter,
    pub swept_orphans: Counter,
    pub sweep_failures: Counter,
    /// Objects the last file-store sweep listed, and when it and the hourly sweep last ran.
    pub file_objects: Counter,
    pub file_sweep_at: Counter,
    pub sweep_at: Counter,
    requests: Mutex<Requests>,
}

pub static METRICS: Metrics = Metrics {
    rate_limited_ip: Counter::new(),
    rate_limited_identity: Counter::new(),
    uploads_refused: Counter::new(),
    push_sent: [Counter::new(), Counter::new()],
    push_gone: [Counter::new(), Counter::new()],
    push_failed: [Counter::new(), Counter::new()],
    swept_envelopes: Counter::new(),
    swept_orphans: Counter::new(),
    sweep_failures: Counter::new(),
    file_objects: Counter::new(),
    file_sweep_at: Counter::new(),
    sweep_at: Counter::new(),
    requests: Mutex::new(BTreeMap::new()),
};

const PLATFORMS: [&str; 2] = ["apns", "fcm"];

/// The slot of a platform in the `push_*` counters.
pub fn platform(name: &str) -> usize {
    (name == "fcm") as usize
}

/// Counts every answered request under its route template, so ids never become labels.
pub async fn count_requests(request: Request, next: Next) -> Response {
    let route = request.extensions().get::<MatchedPath>().map(|path| path.as_str().to_string());
    let method = request.method().to_string();
    let started = Instant::now();
    let response = next.run(request).await;
    let key = (method, route.unwrap_or_else(|| "unmatched".into()), response.status().as_u16());
    let mut requests = METRICS.requests.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = requests.entry(key).or_default();
    entry.0 += 1;
    entry.1 += started.elapsed().as_secs_f64();
    response
}

/// The database's totals, read at most once in `STATS_TTL` however often they are scraped.
#[derive(Default)]
pub struct StatsCache(tokio::sync::Mutex<Option<(Instant, Stats)>>);

impl StatsCache {
    async fn get(&self, state: &AppState) -> Option<Stats> {
        let mut cached = self.0.lock().await;
        if let Some((at, stats)) = cached.as_ref() {
            if at.elapsed() < STATS_TTL {
                return Some(stats.clone());
            }
        }
        match state.db.stats().await {
            Ok(stats) => {
                *cached = Some((Instant::now(), stats.clone()));
                Some(stats)
            }
            Err(error) => {
                tracing::warn!(?error, "reading stats");
                None
            }
        }
    }
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |diff, (x, y)| diff | (x ^ y)) == 0
}

pub async fn serve(State(state): State<AppState>, request: Request) -> Response {
    // Without a token the relay has no such route.
    let Some(token) = state.metrics_token.as_deref() else { return StatusCode::NOT_FOUND.into_response() };
    let given = request.headers().get(header::AUTHORIZATION).and_then(|value| value.to_str().ok()).and_then(|value| value.strip_prefix("Bearer "));
    if !given.is_some_and(|given| same(given.as_bytes(), token.as_bytes())) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let stats = state.stats.get(&state).await;
    let (sockets, identities) = state.local.hub.sockets();
    let body = render(stats.as_ref(), sockets, identities, &state.instance);
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")], body).into_response()
}

struct Page(String);

impl Page {
    fn family(&mut self, name: &str, kind: &str, help: &str) {
        self.0.push_str(&format!("# HELP lorca_relay_{name} {help}\n# TYPE lorca_relay_{name} {kind}\n"));
    }

    fn sample(&mut self, name: &str, labels: &[(&str, &str)], value: impl std::fmt::Display) {
        let labels: Vec<String> = labels.iter().map(|(key, value)| format!("{key}=\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))).collect();
        let labels = if labels.is_empty() { String::new() } else { format!("{{{}}}", labels.join(",")) };
        self.0.push_str(&format!("lorca_relay_{name}{labels} {value}\n"));
    }

    fn one(&mut self, name: &str, kind: &str, help: &str, labels: &[(&str, &str)], value: impl std::fmt::Display) {
        self.family(name, kind, help);
        self.sample(name, labels, value);
    }
}

fn render(stats: Option<&Stats>, sockets: usize, connected_identities: usize, instance: &str) -> String {
    let mut page = Page(String::new());
    let here = [("instance", instance)];
    page.one("info", "gauge", "The relay's version.", &[("version", env!("CARGO_PKG_VERSION")), ("instance", instance)], 1);

    if let Some(stats) = stats {
        page.one("identities", "gauge", "Registered identities.", &[], stats.identities);
        page.one("machines", "gauge", "Paired machines.", &[], stats.machines);
        page.one("active_machines", "gauge", "Machines seen in the last day, week, and thirty days.", &[("within", "1d")], stats.active_machines[0]);
        page.sample("active_machines", &[("within", "7d")], stats.active_machines[1]);
        page.sample("active_machines", &[("within", "30d")], stats.active_machines[2]);
        page.one("revoked_machines", "gauge", "Keys of unpaired machines.", &[], stats.revoked_machines);
        page.one("deleted_groups", "gauge", "Marks of deleted groups.", &[], stats.deleted_groups);
        page.family("blobs", "gauge", "Stored blobs by kind.");
        for (kind, count, _) in &stats.blobs {
            page.sample("blobs", &[("kind", kind)], count);
        }
        page.family("blob_bytes", "gauge", "Stored ciphertext by kind, files included.");
        for (kind, _, bytes) in &stats.blobs {
            page.sample("blob_bytes", &[("kind", kind)], bytes);
        }
        page.one("usage_bytes", "gauge", "What the identities' quotas count, all together.", &[], stats.usage_bytes);
        page.one("largest_identity_bytes", "gauge", "What the identity that stores the most stores.", &[], stats.largest_identity_bytes);
        page.family("push_tokens", "gauge", "Phones registered for pushes.");
        for (platform, count) in &stats.push_tokens {
            page.sample("push_tokens", &[("platform", platform)], count);
        }
    }

    page.one("sockets", "gauge", "Sync sockets open on this process.", &here, sockets);
    page.one("connected_identities", "gauge", "Identities with a socket on this process.", &here, connected_identities);

    page.family("requests_total", "counter", "Answered requests by route and status.");
    let requests = METRICS.requests.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
    for ((method, route, status), (count, _)) in &requests {
        page.sample("requests_total", &[("method", method), ("route", route), ("status", &status.to_string()), ("instance", instance)], count);
    }
    page.family("request_seconds_total", "counter", "Time spent answering, by route and status.");
    for ((method, route, status), (_, seconds)) in &requests {
        page.sample("request_seconds_total", &[("method", method), ("route", route), ("status", &status.to_string()), ("instance", instance)], format!("{seconds:.6}"));
    }

    page.family("rate_limited_total", "counter", "Requests refused by a rate limit.");
    page.sample("rate_limited_total", &[("by", "ip"), ("instance", instance)], METRICS.rate_limited_ip.get());
    page.sample("rate_limited_total", &[("by", "identity"), ("instance", instance)], METRICS.rate_limited_identity.get());
    page.one("uploads_refused_total", "counter", "Large uploads that found no place in 30 s.", &here, METRICS.uploads_refused.get());

    page.family("pushes_total", "counter", "Pushes handed to APNs and FCM, by what came of them.");
    for (index, name) in PLATFORMS.iter().enumerate() {
        for (result, counters) in [("sent", &METRICS.push_sent), ("gone", &METRICS.push_gone), ("failed", &METRICS.push_failed)] {
            page.sample("pushes_total", &[("platform", name), ("result", result), ("instance", instance)], counters[index].get());
        }
    }

    page.one("swept_envelopes_total", "counter", "Stale sealed envelopes the sweep dropped.", &here, METRICS.swept_envelopes.get());
    page.one("swept_orphans_total", "counter", "File objects with no row the sweep removed.", &here, METRICS.swept_orphans.get());
    page.one("sweep_failures_total", "counter", "Sweeps that ended in an error.", &here, METRICS.sweep_failures.get());
    page.one("file_objects", "gauge", "Objects the last file-store sweep listed.", &here, METRICS.file_objects.get());
    page.one("sweep_timestamp_seconds", "gauge", "When the hourly sweep last finished; 0 before the first.", &here, METRICS.sweep_at.get());
    page.one("file_sweep_timestamp_seconds", "gauge", "When the file-store sweep last finished; 0 before the first.", &here, METRICS.file_sweep_at.get());
    page.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_is_prometheus_text() {
        let stats = Stats { identities: 2, blobs: vec![("chat".into(), 3, 40), ("file".into(), 1, 9000)], push_tokens: vec![("apns".into(), 1)], ..Stats::default() };
        let page = render(Some(&stats), 4, 2, "abc");
        assert!(page.contains("# TYPE lorca_relay_blobs gauge\nlorca_relay_blobs{kind=\"chat\"} 3\nlorca_relay_blobs{kind=\"file\"} 1\n"), "{page}");
        assert!(page.contains("lorca_relay_blob_bytes{kind=\"file\"} 9000\n"));
        assert!(page.contains("lorca_relay_sockets{instance=\"abc\"} 4\n"));
        assert!(page.contains("lorca_relay_pushes_total{platform=\"fcm\",result=\"gone\",instance=\"abc\"} 0\n"));
        // Every sample line is `name{labels} value` under a family that was declared.
        for line in page.lines().filter(|line| !line.starts_with('#')) {
            let name = line.split(['{', ' ']).next().unwrap();
            assert!(page.contains(&format!("# TYPE {name} ")), "{line}");
            assert!(line.rsplit(' ').next().unwrap().parse::<f64>().is_ok(), "{line}");
        }
        assert!(!render(None, 0, 0, "abc").contains("lorca_relay_identities"));
    }

    #[test]
    fn tokens_compare_whole() {
        assert!(same(b"secret", b"secret"));
        assert!(!same(b"secret", b"secreT") && !same(b"secret", b"secre") && !same(b"", b"x"));
    }
}
