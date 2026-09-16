//! Where `file` ciphertext lives when it is kept out of SQLite: a directory on disk, or an
//! S3-compatible bucket (AWS, R2, MinIO) spoken to with SigV4 and path-style URLs. Objects are
//! keyed `<identity pubkey>/<blob id>`; the row in `blobs` keeps the metadata.

use std::path::PathBuf;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::routes::{ApiError, ApiResult};

pub enum FileStore {
    Local { dir: PathBuf },
    S3(S3),
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

    pub async fn put(&self, key: &str, bytes: Vec<u8>) -> ApiResult<()> {
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

    pub async fn get(&self, key: &str) -> ApiResult<Option<Vec<u8>>> {
        match self {
            FileStore::Local { dir } => match tokio::fs::read(dir.join(key)).await {
                Ok(bytes) => Ok(Some(bytes)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(storage_error("reading", error)),
            },
            FileStore::S3(s3) => {
                let response = s3.request(reqwest::Method::GET, key, Vec::new()).await?;
                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    return Ok(None);
                }
                if !response.status().is_success() {
                    return Err(s3_error("get", response).await);
                }
                let bytes = response.bytes().await.map_err(|error| storage_error("reading", error))?;
                Ok(Some(bytes.to_vec()))
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
                let response = s3.request(reqwest::Method::DELETE, key, Vec::new()).await?;
                if !response.status().is_success() && response.status() != reqwest::StatusCode::NOT_FOUND {
                    return Err(s3_error("delete", response).await);
                }
                Ok(())
            }
        }
    }
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

    async fn request(&self, method: reqwest::Method, key: &str, body: Vec<u8>) -> ApiResult<reqwest::Response> {
        let path = self.object_path(key);
        let url = format!("{}{}", self.endpoint, path);
        let host = url::host_of(&self.endpoint);
        let payload_hash = hex(&Sha256::digest(&body));
        let (amz_date, date) = amz_timestamp(crate::db::now());
        let headers = [("host", host.as_str()), ("x-amz-content-sha256", payload_hash.as_str()), ("x-amz-date", amz_date.as_str())];
        let authorization = self.authorization(method.as_str(), &path, &headers, &payload_hash, &amz_date, &date);

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

    /// SigV4 `Authorization` for a request with no query string. `headers` are the signed
    /// ones, lowercase names in sorted order, values trimmed.
    fn authorization(&self, method: &str, path: &str, headers: &[(&str, &str)], payload_hash: &str, amz_date: &str, date: &str) -> String {
        let canonical_headers: String = headers.iter().map(|(name, value)| format!("{name}:{value}\n")).collect();
        let signed_headers = headers.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(";");
        let canonical_request = format!("{method}\n{path}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}");
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
        let authorization = s3.authorization("GET", "/test.txt", &headers, empty, "20130524T000000Z", "20130524");
        assert_eq!(
            authorization,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn paths() {
        let s3 = S3::new("https://x.r2.cloudflarestorage.com/".into(), "tinybot".into(), "auto".into(), "/files/".into(), "".into(), "".into());
        assert_eq!(s3.object_path("abc-_/id.1"), "/tinybot/files/abc-_/id.1");
        assert_eq!(url::host_of("https://x.r2.cloudflarestorage.com"), "x.r2.cloudflarestorage.com");
        assert_eq!(uri_encode("a b+c"), "a%20b%2Bc");
    }
}
