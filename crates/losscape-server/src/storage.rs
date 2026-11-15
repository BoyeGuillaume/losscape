//! Where checkpoint files are kept, through `object_store`: a folder (`/data/checkpoints`,
//! `file:///…`) or an S3-compatible store (`s3://bucket/prefix`, credentials and endpoint from
//! the usual `AWS_*` variables). Keys are made of ids and numbers only, never of names users
//! give, so nothing they send can reach outside a checkpoint's place.

use std::sync::Arc;

use bytes::Bytes;
use futures_util::stream::BoxStream;
use futures_util::{StreamExt, TryStreamExt};
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt};
use tokio::sync::Semaphore;

use crate::error::ApiError;

/// Parts written at once: each is held in memory while it's written.
const CONCURRENT_WRITES: usize = 16;

pub struct Storage {
    store: Arc<dyn ObjectStore>,
    writes: Semaphore,
    /// For logs: where, without credentials.
    pub description: String,
}

#[derive(Debug, thiserror::Error)]
#[error("checkpoint storage {location}: {reason}")]
pub struct StorageError {
    location: String,
    reason: String,
}

impl Storage {
    /// Opens `location`, creating the folder of a local one.
    pub fn open(location: &str) -> Result<Self, StorageError> {
        let failed = |reason: String| StorageError {
            location: location.to_owned(),
            reason,
        };
        let (store, description): (Arc<dyn ObjectStore>, String) = if location.starts_with("s3://")
        {
            let store = AmazonS3Builder::from_env()
                .with_url(location)
                .build()
                .map_err(|e| failed(e.to_string()))?;
            (Arc::new(store), location.to_owned())
        } else {
            let folder = location.strip_prefix("file://").unwrap_or(location);
            if !folder.starts_with('/') {
                return Err(failed(
                    "use an absolute path, file:///… or s3://bucket/…".into(),
                ));
            }
            std::fs::create_dir_all(folder).map_err(|e| failed(e.to_string()))?;
            let store = LocalFileSystem::new_with_prefix(folder)
                .map_err(|e| failed(e.to_string()))?
                .with_automatic_cleanup(true);
            (Arc::new(store), format!("folder {folder}"))
        };
        Ok(Self {
            store,
            writes: Semaphore::new(CONCURRENT_WRITES),
            description,
        })
    }

    pub async fn put(&self, key: &Path, data: Bytes) -> Result<(), ApiError> {
        let _permit = self.writes.acquire().await.map_err(|_| ApiError::busy())?;
        self.store
            .put(key, data.into())
            .await
            .map(|_| ())
            .map_err(|err| unavailable("writing", key, err))
    }

    /// The objects at `keys`, one after the other, as one stream.
    pub fn read(&self, keys: Vec<Path>) -> BoxStream<'static, std::io::Result<Bytes>> {
        let store = self.store.clone();
        futures_util::stream::iter(keys)
            .then(move |key| {
                let store = store.clone();
                async move {
                    match store.get(&key).await {
                        Ok(object) => Ok(object.into_stream()),
                        Err(err) => {
                            tracing::warn!(%key, %err, "reading checkpoint storage failed");
                            Err(object_store::Error::Generic {
                                store: "checkpoints",
                                source: "reading failed".into(),
                            })
                        }
                    }
                }
            })
            .try_flatten()
            .map_err(std::io::Error::other)
            .boxed()
    }

    /// Deletes everything under `prefix`. Returns how many objects.
    pub async fn delete_prefix(&self, prefix: &Path) -> Result<u64, ApiError> {
        let listed = self
            .store
            .list(Some(prefix))
            .map_ok(|meta| meta.location)
            .boxed();
        let mut deleted = self.store.delete_stream(listed);
        let mut count = 0;
        while let Some(result) = deleted.next().await {
            match result {
                Ok(_) => count += 1,
                Err(object_store::Error::NotFound { .. }) => {}
                Err(err) => return Err(unavailable("deleting", prefix, err)),
            }
        }
        Ok(count)
    }
}

fn unavailable(doing: &str, key: &Path, err: object_store::Error) -> ApiError {
    tracing::warn!(%key, %err, "{doing} checkpoint storage failed");
    ApiError::unavailable("checkpoint storage unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_reads_and_deletes_by_prefix() {
        let dir = std::env::temp_dir().join(format!("losscape-storage-{}", uuid::Uuid::now_v7()));
        let storage = Storage::open(dir.to_str().unwrap()).unwrap();
        let key = |s: &str| Path::from(s);
        storage
            .put(&key("1/a/0"), Bytes::from("hello "))
            .await
            .unwrap();
        storage
            .put(&key("1/a/1"), Bytes::from("world"))
            .await
            .unwrap();
        storage
            .put(&key("1/b/0"), Bytes::from("other"))
            .await
            .unwrap();

        let read: Vec<Bytes> = storage
            .read(vec![key("1/a/0"), key("1/a/1")])
            .try_collect()
            .await
            .unwrap();
        assert_eq!(read.concat(), b"hello world");

        assert_eq!(storage.delete_prefix(&key("1/a")).await.unwrap(), 2);
        assert_eq!(storage.delete_prefix(&key("1/a")).await.unwrap(), 0, "gone");
        let rest: Vec<Bytes> = storage
            .read(vec![key("1/b/0")])
            .try_collect()
            .await
            .unwrap();
        assert_eq!(rest.concat(), b"other", "another prefix is left alone");
        assert!(dir.join("1/b/0").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn refuses_relative_folders() {
        assert!(Storage::open("data/checkpoints").is_err());
    }
}
