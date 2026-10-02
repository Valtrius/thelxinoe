//! Independent service operations with an exclusive gate for deployment-wide work.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::{
    Mutex as AsyncMutex, OwnedMutexGuard, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock,
    TryLockError,
};

#[derive(Clone, Default)]
pub struct OperationLocks {
    all: Arc<RwLock<()>>,
    services: Arc<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
}

pub struct ServiceGuard {
    _services: Vec<OwnedMutexGuard<()>>,
    _all: OwnedRwLockReadGuard<()>,
}

impl OperationLocks {
    fn mutex(&self, kind: &str) -> Arc<AsyncMutex<()>> {
        self.services
            .lock()
            .unwrap()
            .entry(kind.into())
            .or_default()
            .clone()
    }

    pub async fn lock(&self) -> OwnedRwLockWriteGuard<()> {
        self.all.clone().write_owned().await
    }

    pub fn try_lock_owned(&self) -> Result<OwnedRwLockWriteGuard<()>, TryLockError> {
        self.all.clone().try_write_owned()
    }

    pub async fn service(&self, kind: &str) -> ServiceGuard {
        self.services(&[kind]).await
    }

    pub async fn services(&self, kinds: &[&str]) -> ServiceGuard {
        let all = self.all.clone().read_owned().await;
        let mut kinds = kinds.to_vec();
        kinds.sort_unstable();
        kinds.dedup();
        let mut services = Vec::with_capacity(kinds.len());
        for kind in kinds {
            services.push(self.mutex(kind).lock_owned().await);
        }
        ServiceGuard {
            _services: services,
            _all: all,
        }
    }

    pub fn try_service(&self, kind: &str) -> Result<ServiceGuard, TryLockError> {
        let all = self.all.clone().try_read_owned()?;
        let service = self.mutex(kind).try_lock_owned()?;
        Ok(ServiceGuard {
            _services: vec![service],
            _all: all,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::{Future, poll_fn},
        task::Poll,
        time::Duration,
    };

    #[tokio::test]
    async fn multiple_services_finish_with_a_queued_deployment_writer() {
        let locks = OperationLocks::default();
        let holder = locks.service("recyclarr").await;
        let mut operation = Box::pin(locks.services(&["sonarr", "recyclarr", "radarr", "radarr"]));
        assert!(
            poll_fn(|cx| Poll::Ready(operation.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        let mut deployment = Box::pin(locks.lock());
        assert!(
            poll_fn(|cx| Poll::Ready(deployment.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        drop(holder);
        let guard = tokio::time::timeout(Duration::from_secs(1), operation)
            .await
            .expect("A queued writer must not prevent the remaining service acquisitions");
        assert!(
            poll_fn(|cx| Poll::Ready(deployment.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(locks.try_service("radarr").is_err());
        assert!(locks.try_service("sonarr").is_err());
        drop(guard);
        tokio::time::timeout(Duration::from_secs(1), deployment)
            .await
            .expect("Deployment work must proceed after the service operation");
    }

    #[tokio::test]
    async fn services_run_independently_and_deployment_work_stays_exclusive() {
        let locks = OperationLocks::default();
        let radarr = locks.service("radarr").await;
        assert!(locks.try_service("radarr").is_err());
        let sonarr = locks.try_service("sonarr").unwrap();
        assert!(locks.try_lock_owned().is_err());
        drop(radarr);
        assert!(locks.try_service("radarr").is_ok());
        assert!(locks.try_service("sonarr").is_err());
        drop(sonarr);
        let deployment = locks.lock().await;
        assert!(locks.try_service("radarr").is_err());
        assert!(locks.try_service("sonarr").is_err());
        drop(deployment);
        assert!(locks.try_service("sonarr").is_ok());
    }
}
