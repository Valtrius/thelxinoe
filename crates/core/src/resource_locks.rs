use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

#[derive(Default)]
pub struct ResourceLocks(Mutex<HashMap<String, Weak<RwLock<()>>>>);
impl ResourceLocks {
    fn locks(&self, keys: &[&str]) -> Vec<Arc<RwLock<()>>> {
        let mut keys = keys.to_vec();
        keys.sort_unstable();
        keys.dedup();
        let mut locks = self.0.lock().unwrap();
        locks.retain(|_, lock| lock.strong_count() > 0);
        keys.into_iter()
            .map(|key| {
                let entry = locks.entry(key.into()).or_default();
                match entry.upgrade() {
                    Some(lock) => lock,
                    None => {
                        let lock = Arc::new(RwLock::new(()));
                        *entry = Arc::downgrade(&lock);
                        lock
                    }
                }
            })
            .collect()
    }
    pub async fn read(&self, keys: &[&str]) -> Vec<OwnedRwLockReadGuard<()>> {
        let mut guards = Vec::new();
        for lock in self.locks(keys) {
            guards.push(lock.read_owned().await);
        }
        guards
    }
    pub async fn write(&self, keys: &[&str]) -> Vec<OwnedRwLockWriteGuard<()>> {
        let mut guards = Vec::new();
        for lock in self.locks(keys) {
            guards.push(lock.write_owned().await);
        }
        guards
    }
}
