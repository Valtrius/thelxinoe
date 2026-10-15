//! Database checks for managers.ownership. Each runs immediately before an upstream mutation.
use super::*;
use thelxinoe_database::Database;

/// Owned, committed and released: both the integration flag and its ownership record agree.
const OPERATIONAL: &str = "EXISTS(SELECT 1 FROM stack_provisions p WHERE p.service_id=?1 AND p.state='complete' AND p.hold IS NULL) AND EXISTS(SELECT 1 FROM manager_services WHERE id=?1 AND enabled=1 AND operational=1 AND (?2 IS NULL OR generation=?2) UNION ALL SELECT 1 FROM support_services WHERE id=?1 AND operational=1 AND (?2 IS NULL OR generation=?2))";

pub(super) async fn operational(
    db: &Database,
    service: String,
    generation: String,
) -> anyhow::Result<bool> {
    db.read("managers.ownership.operational", move |db| {
        Ok(db.query_row(
            &format!("SELECT {OPERATIONAL}"),
            params![service, generation],
            |r| r.get(0),
        )?)
    })
    .await
}

pub(super) async fn operational_any(db: &Database, service: String) -> anyhow::Result<bool> {
    db.read("managers.ownership.operational_any", move |db| {
        Ok(db.query_row(
            &format!("SELECT {OPERATIONAL}"),
            params![service, None::<String>],
            |r| r.get(0),
        )?)
    })
    .await
}

/// Setup or reverification of an Owned service may configure only its exact held container,
/// whether it was installed or transferred. Provenance never grants or denies authority.
pub(super) async fn setup(
    db: &Database,
    provision: String,
    service: String,
    container: String,
) -> anyhow::Result<bool> {
    db.read("managers.ownership.setup", move |db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE id=?1 AND hold='setup' AND state='connecting' AND service_id=?2 AND container_id=?3)",
            params![provision, service, container],
            |r| r.get(0),
        )?)
    })
    .await
}

/// A committed transfer may run only reviewed setup actions against its exact replacement.
pub(super) async fn transfer(
    db: &Database,
    operation: String,
    action: String,
    service: String,
    container: String,
) -> anyhow::Result<bool> {
    db.read("managers.ownership.transfer", move |db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM service_transfers t JOIN stack_provisions p ON p.transfer_id=t.id WHERE t.id=?1 AND t.state='committed' AND t.service_id=?3 AND t.replacement_id=?4 AND p.hold='transfer' AND p.service_id=?3 AND p.container_id=?4 AND EXISTS(SELECT 1 FROM json_each(t.review,'$.setup_actions') a WHERE json_extract(a.value,'$.action')=?2))",
            params![operation, action, service, container],
            |r| r.get(0),
        )?)
    })
    .await
}
