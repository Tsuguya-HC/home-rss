#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchOutcomeKind {
    FetchFailed,
    Unparseable,
    Stored,
    NotModified,
    StoreFailed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FetchFailureAction {
    Record { reason: String },
    Clear,
    Keep,
}

pub fn decide_fetch_failure_action(
    kind: FetchOutcomeKind,
    detail: &str,
    ever_fetched: bool,
) -> FetchFailureAction {
    match kind {
        FetchOutcomeKind::Stored | FetchOutcomeKind::NotModified => FetchFailureAction::Clear,
        FetchOutcomeKind::StoreFailed => FetchFailureAction::Keep,
        FetchOutcomeKind::FetchFailed | FetchOutcomeKind::Unparseable => {
            if ever_fetched {
                FetchFailureAction::Record {
                    reason: detail.chars().take(200).collect(),
                }
            } else {
                FetchFailureAction::Keep
            }
        }
    }
}
