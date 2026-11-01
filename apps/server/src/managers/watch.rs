//! Services also change outside requests: containers stop, background checks fail and
//! updates run on schedule. One observer announces each change to the services view,
//! so the page does not poll for them.

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;

use super::*;

/// Fields that move on every attempt without changing what the page shows.
const UNSHOWN: [&str; 2] = ["updated_at", "next_attempt"];

async fn observe(state: &AppState) -> Value {
    let mut observed = view::listing(state).await;
    forget_unshown(&mut observed);
    observed
}

fn forget_unshown(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for field in UNSHOWN {
                fields.remove(field);
            }
            fields.values_mut().for_each(forget_unshown);
        }
        Value::Array(items) => items.iter_mut().for_each(forget_unshown),
        _ => {}
    }
}

/// Announces what differs from `seen`; the first observation only records.
async fn check(state: &AppState, seen: &mut Option<Value>) -> anyhow::Result<()> {
    let observed = observe(state).await;
    if seen.as_ref().is_some_and(|seen| *seen != observed) {
        state.emit(None, "stack.changed", json!({})).await?;
    }
    *seen = Some(observed);
    Ok(())
}

pub(crate) async fn run(state: AppState) -> anyhow::Result<()> {
    let mut seen = None;
    loop {
        // Without a listener nobody needs telling; the next check compares with what was seen.
        if state.events.receiver_count() > 0
            && let Err(error) = check(&state, &mut seen).await
        {
            tracing::warn!(%error, "A services change could not be announced; the next check retries");
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}
