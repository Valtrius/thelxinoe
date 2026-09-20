//! Every Docker mutation requires this process to hold the deployment writer lease.
use std::{
    fs::File,
    sync::atomic::{AtomicBool, Ordering},
};
static ACTIVE: AtomicBool = AtomicBool::new(false);
pub static HANDOFF: tokio::sync::Notify = tokio::sync::Notify::const_new();
pub fn active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}
pub struct Lease {
    runtime: File,
    deployment: File,
}
impl Lease {
    pub fn acquire(runtime: &std::path::Path) -> anyhow::Result<Self> {
        let root = crate::store::root();
        Self::acquire_in(runtime, &root)
    }
    fn acquire_in(runtime: &std::path::Path, root: &std::path::Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(root)?;
        let open = |path| {
            std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(path)
        };
        let runtime = open(runtime.join("controller.lock"))?;
        fs2::FileExt::try_lock_exclusive(&runtime)?;
        let deployment = open(root.join("mutation.lock"))?;
        fs2::FileExt::try_lock_exclusive(&deployment)?;
        ACTIVE.store(true, Ordering::SeqCst);
        Ok(Self {
            runtime,
            deployment,
        })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::SeqCst);
        let _ = fs2::FileExt::unlock(&self.deployment);
        let _ = fs2::FileExt::unlock(&self.runtime);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mutation_lease_excludes_a_second_writer_and_releases_cleanly() {
        let root = std::env::temp_dir().join(thelxinoe_core::id());
        std::fs::create_dir_all(root.join("runtime")).unwrap();
        let first = Lease::acquire_in(&root.join("runtime"), &root.join("deployment")).unwrap();
        assert!(active());
        assert!(Lease::acquire_in(&root.join("runtime"), &root.join("deployment")).is_err());
        drop(first);
        assert!(!active());
        let next = Lease::acquire_in(&root.join("runtime"), &root.join("deployment")).unwrap();
        assert!(active());
        drop(next);
        std::fs::remove_dir_all(root).unwrap();
    }
}
