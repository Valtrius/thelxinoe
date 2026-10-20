//! Service update stages, written by the controller and mirrored by the server. Both use
//! these lists, so a stage one of them adds is never missing from the other's checks.

/// Stages that end an attempt. Every other stage, the server's queued ones and any stage a
/// newer controller reports included, means the attempt still owns its service.
pub const FINISHED: &[&str] = &["committed", "rolled-back", "blocked", "runtime-failure"];
/// Unfinished stages that wait for a decision instead of working.
pub const WAITING: &[&str] = &["ready", "recovery-required"];

pub fn finished(stage: &str) -> bool {
    FINISHED.contains(&stage)
}

/// The stages as a SQL list, for `state IN (...)`. They are fixed literals.
pub fn sql(stages: &[&str]) -> String {
    stages
        .iter()
        .map(|stage| format!("'{stage}'"))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_mid_activation_are_unfinished() {
        for stage in [
            "queued",
            "submitting",
            "creating-replacement",
            "replacement-created",
            "activating",
            "ready",
            "recovery-required",
        ] {
            assert!(!finished(stage), "{stage}");
        }
        assert_eq!(
            sql(FINISHED),
            "'committed','rolled-back','blocked','runtime-failure'"
        );
    }
}
