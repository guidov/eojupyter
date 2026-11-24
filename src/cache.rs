use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Instant, SystemTime};

#[derive(Clone)]
pub struct Blob {
    pub mime: String,
    pub data: Vec<u8>,
}

#[derive(Clone)]
pub struct CachedDoc {
    pub id: u64,
    pub modified: SystemTime,
    pub len: u64,
    pub payload_json: Arc<String>,
    pub blobs: Arc<Vec<Blob>>,
    pub blob_bytes: usize,
    pub last_used: Instant,
}

struct CacheInner {
    by_path: HashMap<PathBuf, CachedDoc>,
    path_by_id: HashMap<u64, PathBuf>,
    total_blob_bytes: usize,
}

pub struct DocCache {
    inner: RwLock<CacheInner>,
    next_id: AtomicU64,
    max_docs: usize,
    max_blob_bytes: usize,
}

impl DocCache {
    pub fn new(max_docs: usize, max_blob_bytes: usize) -> Self {
        Self {
            inner: RwLock::new(CacheInner {
                by_path: HashMap::new(),
                path_by_id: HashMap::new(),
                total_blob_bytes: 0,
            }),
            next_id: AtomicU64::new(1),
            max_docs,
            max_blob_bytes,
        }
    }

    pub fn alloc_doc_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn get(&self, path: &Path, modified: SystemTime, len: u64) -> Option<Arc<String>> {
        // Fast path read check
        {
            let guard = self.inner.read().ok()?;
            let doc = guard.by_path.get(path)?;
            if doc.modified != modified || doc.len != len {
                return None;
            }
        }
        // Update LRU timestamp
        let mut guard = self.inner.write().ok()?;
        let doc = guard.by_path.get_mut(path)?;
        if doc.modified == modified && doc.len == len {
            doc.last_used = Instant::now();
            Some(Arc::clone(&doc.payload_json))
        } else {
            None
        }
    }

    pub fn insert(
        &self,
        path: PathBuf,
        id: u64,
        modified: SystemTime,
        len: u64,
        payload_json: String,
        blobs: Vec<Blob>,
    ) -> Arc<String> {
        let blob_bytes: usize = blobs.iter().map(|b| b.data.len()).sum();
        let payload_arc = Arc::new(payload_json);
        let entry = CachedDoc {
            id,
            modified,
            len,
            payload_json: Arc::clone(&payload_arc),
            blobs: Arc::new(blobs),
            blob_bytes,
            last_used: Instant::now(),
        };

        if let Ok(mut guard) = self.inner.write() {
            if let Some(old) = guard.by_path.remove(&path) {
                guard.total_blob_bytes = guard.total_blob_bytes.saturating_sub(old.blob_bytes);
                guard.path_by_id.remove(&old.id);
            }
            guard.total_blob_bytes += blob_bytes;
            guard.path_by_id.insert(id, path.clone());
            guard.by_path.insert(path, entry);

            while guard.by_path.len() > self.max_docs
                || (guard.total_blob_bytes > self.max_blob_bytes && guard.by_path.len() > 1)
            {
                let oldest_path = guard
                    .by_path
                    .iter()
                    .min_by_key(|(_, v)| v.last_used)
                    .map(|(k, _)| k.clone());
                let Some(evict_path) = oldest_path else {
                    break;
                };
                if let Some(removed) = guard.by_path.remove(&evict_path) {
                    guard.total_blob_bytes =
                        guard.total_blob_bytes.saturating_sub(removed.blob_bytes);
                    guard.path_by_id.remove(&removed.id);
                }
            }
        }

        payload_arc
    }

    pub fn get_blob(&self, doc_id: u64, blob_idx: usize) -> Option<(String, Vec<u8>)> {
        let guard = self.inner.read().ok()?;
        let path = guard.path_by_id.get(&doc_id)?;
        let doc = guard.by_path.get(path)?;
        let blob = doc.blobs.get(blob_idx)?;
        Some((blob.mime.clone(), blob.data.clone()))
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.write() {
            guard.by_path.clear();
            guard.path_by_id.clear();
            guard.total_blob_bytes = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn cache_hits_invalidates_and_evicts() {
        let cache = DocCache::new(2, 1024 * 1024);
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let t1 = SystemTime::UNIX_EPOCH + Duration::from_secs(20);

        let p1 = PathBuf::from("/tmp/a.ipynb");
        let p2 = PathBuf::from("/tmp/b.ipynb");
        let p3 = PathBuf::from("/tmp/c.ipynb");

        let id1 = cache.alloc_doc_id();
        cache.insert(
            p1.clone(),
            id1,
            t0,
            100,
            "{\"doc\":1}".into(),
            vec![Blob {
                mime: "image/png".into(),
                data: vec![1, 2, 3],
            }],
        );
        assert_eq!(cache.get(&p1, t0, 100).as_deref().map(String::as_str), Some("{\"doc\":1}"));
        assert!(cache.get(&p1, t1, 100).is_none());
        assert_eq!(cache.get_blob(id1, 0).map(|b| b.1), Some(vec![1, 2, 3]));

        let id2 = cache.alloc_doc_id();
        cache.insert(p2.clone(), id2, t0, 100, "{\"doc\":2}".into(), vec![]);
        std::thread::sleep(Duration::from_millis(2));
        // Touch p1 so p2 becomes oldest
        let _ = cache.get(&p1, t0, 100);
        std::thread::sleep(Duration::from_millis(2));
        let id3 = cache.alloc_doc_id();
        cache.insert(p3.clone(), id3, t0, 100, "{\"doc\":3}".into(), vec![]);

        assert!(cache.get(&p2, t0, 100).is_none());
        assert!(cache.get(&p1, t0, 100).is_some());
        assert!(cache.get(&p3, t0, 100).is_some());
    }
}
