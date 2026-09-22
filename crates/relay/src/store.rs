//! Where `file` ciphertext lives when it is kept out of SQLite: a directory on disk, or an
//! S3-compatible bucket (AWS, R2, MinIO) spoken to with SigV4 and path-style URLs. Objects are
//! keyed `<identity pubkey>/<blob id>`; the row in `blobs` keeps the metadata.

use std::path::PathBuf;

use axum::body::Bytes;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::routes::{ApiError, ApiResult};

pub enum FileStore {
    Local { dir: PathBuf },
    S3(S3),
}

/// An object as a listing shows it.
pub struct Listed {
    pub key: String,
    /// Unix time of the last write; `None` when the store did not say in a form we read.
    pub modified: Option<i64>,
}

pub fn key(identity_pubkey: &str, id: &str) -> String {
    format!("{identity_pubkey}/{id}")
}

impl FileStore {
    pub fn describe(&self) -> String {
        match self {
            FileStore::Local { dir } => format!("dir {}", dir.display()),
            FileStore::S3(s3) => format!("s3 {}/{}", s3.endpoint, s3.bucket),
        }
    }

    pub async fn put(&self, key: &str, bytes: Bytes) -> ApiResult<()> {
        match self {
            FileStore::Local { dir } => {
                let path = dir.join(key);
                let parent = path.parent().expect("key has a directory").to_path_buf();
                let temp = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
                let result: std::io::Result<()> = async {
                    tokio::fs::create_dir_all(&parent).await?;
                    tokio::fs::write(&temp, &bytes).await?;
                    tokio::fs::rename(&temp, &path).await
                }
                .await;
                result.map_err(|error| storage_error("writing", error))
            }
            FileStore::S3(s3) => {
                let response = s3.request(reqwest::Method::PUT, key, bytes).await?;
                if !response.status().is_success() {
                    return Err(s3_error("put", response).await);
                }
                Ok(())
            }
        }
    }

    pub async fn get(&self, key: &str) -> ApiResult<Option<Bytes>> {
        match self {
            FileStore::Local { dir } => match tokio::fs::read(dir.join(key)).await {
                Ok(bytes) => Ok(Some(bytes.into())),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(storage_error("reading", error)),
            },
            FileStore::S3(s3) => {
                let response = s3.request(reqwest::Method::GET, key, Bytes::new()).await?;
                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    return Ok(None);
                }
                if !response.status().is_success() {
                    return Err(s3_error("get", response).await);
                }
                let bytes = response.bytes().await.map_err(|error| storage_error("reading", error))?;
                Ok(Some(bytes))
            }
        }
    }

    /// A page of the store's objects and, when there are more, what to pass for the next one.
    pub async fn list(&self, page: Option<String>) -> ApiResult<(Vec<Listed>, Option<String>)> {
        match self {
            FileStore::Local { dir } => {
                let dir = dir.clone();
                crate::db::blocking(move || Ok((list_dir(&dir).map_err(|error| storage_error("listing", error))?, None))).await
            }
            FileStore::S3(s3) => {
                let mut query = vec![("list-type", "2".to_string()), ("max-keys", "1000".to_string())];
                if !s3.prefix.is_empty() {
                    query.push(("prefix", format!("{}/", s3.prefix)));
                }
                if let Some(token) = page {
                    query.push(("continuation-token", token));
                }
                let response = s3.send(reqwest::Method::GET, &format!("/{}", uri_encode(&s3.bucket)), &query, Bytes::new()).await?;
                if !response.status().is_success() {
                    return Err(s3_error("list", response).await);
                }
                let body = response.text().await.map_err(|error| storage_error("listing", error))?;
                let strip = if s3.prefix.is_empty() { String::new() } else { format!("{}/", s3.prefix) };
                Ok(parse_listing(&body, &strip))
            }
        }
    }

    pub async fn delete(&self, key: &str) -> ApiResult<()> {
        match self {
            FileStore::Local { dir } => match tokio::fs::remove_file(dir.join(key)).await {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(storage_error("deleting", error)),
            },
            FileStore::S3(s3) => {
                let response = s3.request(reqwest::Method::DELETE, key, Bytes::new()).await?;
                if !response.status().is_success() && response.status() != reqwest::StatusCode::NOT_FOUND {
                    return Err(s3_error("delete", response).await);
                }
                Ok(())
            }
        }
    }
}

/// `<identity>/<id>` files under `dir`. A `.…tmp` file is a write in flight.
fn list_dir(dir: &std::path::Path) -> std::io::Result<Vec<Listed>> {
    let mut listed = Vec::new();
    let identities = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(listed),
        Err(error) => return Err(error),
    };
    for identity in identities {
        let identity = identity?;
        if !identity.file_type()?.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(identity.path())? {
            let file = file?;
            let (Some(identity), Some(name)) = (identity.file_name().to_str().map(str::to_string), file.file_name().to_str().map(str::to_string)) else { continue };
            if name.starts_with('.') || !file.file_type()?.is_file() {
                continue;
            }
            let modified = file.metadata()?.modified().ok().and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
            listed.push(Listed { key: key(&identity, &name), modified });
        }
    }
    Ok(listed)
}

/// The objects and the continuation token of a `ListObjectsV2` answer. Keys lose `strip`, the
/// bucket prefix; one outside it is left out.
fn parse_listing(xml: &str, strip: &str) -> (Vec<Listed>, Option<String>) {
    let mut listed = Vec::new();
    let mut rest = xml;
    while let Some((contents, after)) = element(rest, "Contents") {
        rest = after;
        let Some(key) = element(contents, "Key").map(|(key, _)| unescape(key)) else { continue };
        let Some(key) = key.strip_prefix(strip) else { continue };
        let modified = element(contents, "LastModified").and_then(|(stamp, _)| unix_from_iso8601(stamp));
        listed.push(Listed { key: key.to_string(), modified });
    }
    let truncated = element(xml, "IsTruncated").is_some_and(|(value, _)| value.trim() == "true");
    let next = element(xml, "NextContinuationToken").map(|(token, _)| unescape(token)).filter(|_| truncated);
    (listed, next)
}

/// The text of the first `<name>…</name>` and what follows it.
fn element<'a>(xml: &'a str, name: &str) -> Option<(&'a str, &'a str)> {
    let (open, close) = (format!("<{name}>"), format!("</{name}>"));
    let start = xml.find(&open)? + open.len();
    let end = start + xml[start..].find(&close)?;
    Some((&xml[start..end], &xml[end + close.len()..]))
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn storage_error(what: &str, error: impl std::fmt::Display) -> ApiError {
    tracing::error!(%error, "{what} a file object");
    ApiError::internal("File storage error")
}

async fn s3_error(what: &str, response: reqwest::Response) -> ApiError {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    tracing::error!(%status, body = %body.chars().take(300).collect::<String>(), "s3 {what}");
    ApiError::internal("File storage error")
}

// MARK: - S3

pub struct S3 {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub prefix: String,
    pub access_key: String,
    pub secret_key: String,
    http: reqwest::Client,
}

impl S3 {
    pub fn new(endpoint: String, bucket: String, region: String, prefix: String, access_key: String, secret_key: String) -> S3 {
        let endpoint = endpoint.trim_end_matches('/').to_string();
        let prefix = prefix.trim_matches('/').to_string();
        S3 { endpoint, bucket, region, prefix, access_key, secret_key, http: reqwest::Client::new() }
    }

    fn object_path(&self, key: &str) -> String {
        let mut path = format!("/{}", uri_encode(&self.bucket));
        if !self.prefix.is_empty() {
            for segment in self.prefix.split('/') {
                path.push('/');
                path.push_str(&uri_encode(segment));
            }
        }
        for segment in key.split('/') {
            path.push('/');
            path.push_str(&uri_encode(segment));
        }
        path
    }

    async fn request(&self, method: reqwest::Method, key: &str, body: Bytes) -> ApiResult<reqwest::Response> {
        self.send(method, &self.object_path(key), &[], body).await
    }

    #[cfg(test)]
    pub async fn create_bucket(&self) {
        let _ = self.send(reqwest::Method::PUT, &format!("/{}", uri_encode(&self.bucket)), &[], Bytes::new()).await;
    }

    /// A signed request to an encoded `path`, with `query` parameters in any order.
    async fn send(&self, method: reqwest::Method, path: &str, query: &[(&str, String)], body: Bytes) -> ApiResult<reqwest::Response> {
        let mut query: Vec<String> = query.iter().map(|(name, value)| format!("{}={}", uri_encode(name), uri_encode(value))).collect();
        query.sort();
        let query = query.join("&");
        let url = if query.is_empty() { format!("{}{}", self.endpoint, path) } else { format!("{}{}?{}", self.endpoint, path, query) };
        let host = url::host_of(&self.endpoint);
        let payload_hash = hex(&Sha256::digest(&body));
        let (amz_date, _) = amz_timestamp(crate::db::now());
        let headers = [("host", host.as_str()), ("x-amz-content-sha256", payload_hash.as_str()), ("x-amz-date", amz_date.as_str())];
        let authorization = self.authorization(method.as_str(), path, &query, &headers, &payload_hash, &amz_date);

        self.http
            .request(method, &url)
            .header("x-amz-date", amz_date)
            .header("x-amz-content-sha256", payload_hash)
            .header("authorization", authorization)
            .body(body)
            .send()
            .await
            .map_err(|error| storage_error("reaching object storage while", error))
    }

    /// SigV4 `Authorization`. `query` is the canonical query string (encoded, sorted by name);
    /// `headers` are the signed ones, lowercase names in sorted order, values trimmed.
    fn authorization(&self, method: &str, path: &str, query: &str, headers: &[(&str, &str)], payload_hash: &str, amz_date: &str) -> String {
        let date = &amz_date[..8];
        let canonical_headers: String = headers.iter().map(|(name, value)| format!("{name}:{value}\n")).collect();
        let signed_headers = headers.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(";");
        let canonical_request = format!("{method}\n{path}\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}");
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let string_to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", hex(&Sha256::digest(canonical_request.as_bytes())));
        let signing_key = hmac(
            &hmac(&hmac(&hmac(format!("AWS4{}", self.secret_key).as_bytes(), date.as_bytes()), self.region.as_bytes()), b"s3"),
            b"aws4_request",
        );
        let signature = hex(&hmac(&signing_key, string_to_sign.as_bytes()));
        format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}", self.access_key)
    }
}

mod url {
    /// `host[:port]` of an `http(s)://` endpoint, the way it goes into the `Host` header.
    pub fn host_of(endpoint: &str) -> String {
        let rest = endpoint.split_once("://").map(|(_, rest)| rest).unwrap_or(endpoint);
        rest.split('/').next().unwrap_or(rest).to_string()
    }
}

fn hmac(key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac key");
    mac.update(message);
    mac.finalize().into_bytes().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// RFC 3986 encoding of one path segment, as SigV4 wants it.
fn uri_encode(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Unix time of `2024-02-29T12:00:00.000Z`, the form S3 gives `LastModified` in.
fn unix_from_iso8601(stamp: &str) -> Option<i64> {
    let stamp = stamp.trim();
    let number = |range: std::ops::Range<usize>| stamp.get(range)?.parse::<i64>().ok();
    let (y, m, d) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !stamp.ends_with('Z') || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days since 1970-01-01 from a civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + hour * 3600 + minute * 60 + second)
}

/// `YYYYMMDDTHHMMSSZ` and `YYYYMMDD` for a unix time.
fn amz_timestamp(unix: i64) -> (String, String) {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let date = format!("{y:04}{m:02}{d:02}");
    (format!("{date}T{:02}{:02}{:02}Z", secs / 3600, secs % 3600 / 60, secs % 60), date)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps() {
        assert_eq!(amz_timestamp(0).0, "19700101T000000Z");
        assert_eq!(amz_timestamp(1_700_000_000).0, "20231114T221320Z");
        assert_eq!(amz_timestamp(1_709_164_800).1, "20240229");
    }

    /// The "GET Object" example from the S3 SigV4 documentation.
    #[test]
    fn sigv4_matches_the_aws_example() {
        let s3 = S3::new(
            "https://examplebucket.s3.amazonaws.com".into(),
            "examplebucket".into(),
            "us-east-1".into(),
            "".into(),
            "AKIAIOSFODNN7EXAMPLE".into(),
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
        );
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let headers = [
            ("host", "examplebucket.s3.amazonaws.com"),
            ("range", "bytes=0-9"),
            ("x-amz-content-sha256", empty),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let authorization = s3.authorization("GET", "/test.txt", "", &headers, empty, "20130524T000000Z");
        assert_eq!(
            authorization,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn iso8601_reads_back_what_amz_timestamp_writes() {
        assert_eq!(unix_from_iso8601("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(unix_from_iso8601("2023-11-14T22:13:20.000Z"), Some(1_700_000_000));
        assert_eq!(unix_from_iso8601("2024-02-29T00:00:00Z"), Some(1_709_164_800));
        assert_eq!(unix_from_iso8601("yesterday"), None);
    }

    #[test]
    fn a_listing_gives_keys_under_the_prefix_and_the_next_page() {
        let xml = "<ListBucketResult><IsTruncated>true</IsTruncated>\
            <Contents><Key>files/abc/photo.1</Key><LastModified>2023-11-14T22:13:20.000Z</LastModified></Contents>\
            <Contents><Key>elsewhere/x</Key><LastModified>2023-11-14T22:13:20.000Z</LastModified></Contents>\
            <Contents><Key>files/abc/b</Key><LastModified>soon</LastModified></Contents>\
            <NextContinuationToken>1a/b+c=&amp;</NextContinuationToken></ListBucketResult>";
        let (listed, next) = parse_listing(xml, "files/");
        assert_eq!(listed.iter().map(|o| (o.key.as_str(), o.modified)).collect::<Vec<_>>(), [("abc/photo.1", Some(1_700_000_000)), ("abc/b", None)]);
        assert_eq!(next.as_deref(), Some("1a/b+c=&"));
        assert_eq!(parse_listing("<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>", "").1, None);
    }

    #[test]
    fn paths() {
        let s3 = S3::new("https://x.r2.cloudflarestorage.com/".into(), "lorca".into(), "auto".into(), "/files/".into(), "".into(), "".into());
        assert_eq!(s3.object_path("abc-_/id.1"), "/lorca/files/abc-_/id.1");
        assert_eq!(url::host_of("https://x.r2.cloudflarestorage.com"), "x.r2.cloudflarestorage.com");
        assert_eq!(uri_encode("a b+c"), "a%20b%2Bc");
    }
}
