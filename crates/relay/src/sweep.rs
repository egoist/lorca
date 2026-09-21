//! Hourly housekeeping beyond `Store::tick`: sealed envelopes nobody consumed, the marks of
//! groups deleted long ago, and, once a day, file objects whose row is gone (a delete the file
//! store refused, a relay that stopped between an upload and its row).

use std::sync::Arc;
use std::time::Duration;

use crate::db::{now, Store};
use crate::metrics::METRICS;
use crate::routes::{valid_id, ApiResult};
use crate::store::{self, FileStore};

/// A Runner asks for ten minutes at most; an envelope this old has nobody waiting on it.
const SEALED_TTL: i64 = 7 * 86_400;
/// Longer than a Device stays away and still uploads into a chat deleted meanwhile.
const DELETED_GROUP_TTL: i64 = 180 * 86_400;
/// An object goes up before its row, so a young one may only be early.
const ORPHAN_AGE: i64 = 86_400;

pub fn spawn(db: Arc<dyn Store>, files: Arc<FileStore>) {
    tokio::spawn(async move {
        let mut hourly = tokio::time::interval_at(tokio::time::Instant::now() + Duration::from_secs(600), Duration::from_secs(3600));
        for hour in 0u64.. {
            hourly.tick().await;
            match db.sweep(now() - SEALED_TTL, now() - DELETED_GROUP_TTL).await {
                Ok(envelopes) => {
                    METRICS.swept_envelopes.add(envelopes);
                    METRICS.sweep_at.set(now() as u64);
                    if envelopes > 0 {
                        tracing::info!(envelopes, "swept stale envelopes");
                    }
                }
                Err(error) => {
                    METRICS.sweep_failures.add(1);
                    tracing::warn!(?error, "sweeping");
                }
            }
            if hour % 24 == 0 {
                match orphans(db.as_ref(), &files, now() - ORPHAN_AGE).await {
                    Ok((seen, removed)) => {
                        METRICS.file_objects.set(seen as u64);
                        METRICS.swept_orphans.add(removed as u64);
                        METRICS.file_sweep_at.set(now() as u64);
                        if removed > 0 {
                            tracing::info!(seen, removed, "removed orphaned file objects");
                        }
                    }
                    Err(error) => {
                        METRICS.sweep_failures.add(1);
                        tracing::warn!(?error, "sweeping the file store");
                    }
                }
            }
        }
    });
}

/// Removes the objects written before `before` that have no row. Returns how many objects
/// the store listed and how many went.
pub async fn orphans(db: &dyn Store, files: &FileStore, before: i64) -> ApiResult<(usize, usize)> {
    let (mut seen, mut removed, mut page) = (0, 0, None);
    loop {
        let (listed, next) = files.list(page).await?;
        seen += listed.len();
        // Only `<identity>/<blob id>` written a while ago: a bucket may hold other things.
        let old: Vec<(String, String)> = listed
            .iter()
            .filter(|object| object.modified.is_some_and(|at| at < before))
            .filter_map(|object| object.key.split_once('/'))
            .filter(|(identity, id)| valid_id(identity) && valid_id(id))
            .map(|(identity, id)| (identity.to_string(), id.to_string()))
            .collect();
        for (identity, id) in db.orphans(&old).await? {
            let key = store::key(&identity, &id);
            match files.delete(&key).await {
                Ok(()) => removed += 1,
                Err(error) => tracing::warn!(?error, key, "removing an orphaned file object"),
            }
        }
        match next {
            Some(next) => page = Some(next),
            None => return Ok((seen, removed)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Local, NewBlob, Payload};

    /// A directory always; a bucket too when `LORCA_RELAY_TEST_S3` names an endpoint, e.g.
    /// `http://127.0.0.1:59000` with `LORCA_RELAY_TEST_S3_KEYS=access:secret` (MinIO).
    async fn stores() -> Vec<FileStore> {
        let mut stores = vec![FileStore::Local { dir: std::env::temp_dir().join(format!("lorca-relay-sweep-{}", uuid::Uuid::new_v4())) }];
        if let Ok(endpoint) = std::env::var("LORCA_RELAY_TEST_S3") {
            let keys = std::env::var("LORCA_RELAY_TEST_S3_KEYS").unwrap_or_default();
            let (access, secret) = keys.split_once(':').unwrap_or_default();
            let prefix = format!("sweep-{}", uuid::Uuid::new_v4().simple());
            let s3 = store::S3::new(endpoint, "lorca-test".into(), "us-east-1".into(), prefix, access.into(), secret.into());
            s3.create_bucket().await;
            stores.push(FileStore::S3(s3));
        }
        stores
    }

    #[tokio::test]
    async fn an_old_object_with_no_row_goes() {
        for files in stores().await {
            let path = std::env::temp_dir().join(format!("lorca-relay-sweep-{}.db", uuid::Uuid::new_v4()));
            let db = crate::db::open(path.to_str().unwrap(), Arc::new(Local::default())).await.unwrap();
            let who = format!("identity-{}", uuid::Uuid::new_v4().simple());
            db.register_identity(&who, "content", "machine", "box", "attestation").await.map_err(|e| format!("{e:?}")).unwrap();
            let kept = NewBlob { identity_pubkey: who.clone(), id: "kept".into(), kind: "file".into(), recipient_machine_pubkey: None, slot: None, group: None, payload: Payload::InFileStore { size: 1 } };
            db.insert_blob(kept, 0).await.map_err(|e| format!("{e:?}")).unwrap();
            for key in [store::key(&who, "kept"), store::key(&who, "lost"), store::key("stranger", "theirs")] {
                files.put(&key, b"x".to_vec()).await.map_err(|e| format!("{e:?}")).unwrap();
            }

            // Young objects are left alone, whatever the database says.
            let swept = orphans(db.as_ref(), &files, now() - 3600).await.map_err(|e| format!("{e:?}")).unwrap();
            assert_eq!(swept, (3, 0), "{}", files.describe());
            let swept = orphans(db.as_ref(), &files, now() + 3600).await.map_err(|e| format!("{e:?}")).unwrap();
            assert_eq!(swept, (3, 1), "{}", files.describe());

            let mut left: Vec<String> = files.list(None).await.map_err(|e| format!("{e:?}")).unwrap().0.into_iter().map(|object| object.key).collect();
            left.sort();
            let mut expected = vec![store::key(&who, "kept"), store::key("stranger", "theirs")];
            expected.sort();
            assert_eq!(left, expected, "{}", files.describe());
        }
    }
}
