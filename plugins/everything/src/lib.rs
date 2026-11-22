//! Client for a running Everything (voidtools) instance, shared by this plugin and others
//! that search files through it, e.g. dialog-jump.
use std::path::PathBuf;

#[cfg(windows)]
mod ipc;
// Only the Windows search path ranks results; elsewhere the module is kept for its tests.
#[cfg_attr(not(windows), allow(dead_code))]
mod relevance;

#[cfg(windows)]
const MAX_RESULTS: usize = 50;
/// Used when the host does not say how long it waits (`timeoutMs`).
#[cfg(windows)]
const DEFAULT_BUDGET: std::time::Duration = std::time::Duration::from_millis(900);
/// Kept free before the host's deadline for ranking and writing the response.
#[cfg(windows)]
const RESPONSE_MARGIN: std::time::Duration = std::time::Duration::from_millis(100);
/// A tier is only started when at least this much of the budget is left.
#[cfg(windows)]
const MIN_TIER_TIME: std::time::Duration = std::time::Duration::from_millis(150);

/// One Everything result: a file, folder or drive.
#[derive(Debug, PartialEq)]
pub struct Hit {
    pub path: PathBuf,
    pub folder: bool,
    pub drive: bool,
}

#[derive(Debug)]
pub enum SearchError {
    NotRunning,
    /// Everything did not answer within the search budget, usually while it is busy
    /// updating its index after many file changes.
    Busy,
    /// The host sent a newer query; this result would be discarded.
    Cancelled,
    Failed(String),
}

/// Ranked hits, and whether some tiers were skipped because Everything was too slow.
pub struct Found {
    pub hits: Vec<Hit>,
    pub incomplete: bool,
}

#[cfg(windows)]
pub fn query_everything(query: &str) -> Result<Found, SearchError> {
    use std::time::Instant;
    let deadline = mytools_plugin_api::deadline()
        .map(|deadline| deadline.checked_sub(RESPONSE_MARGIN).unwrap_or(deadline))
        .unwrap_or_else(|| Instant::now() + DEFAULT_BUDGET);
    let tiers = relevance::tiers(query);
    let tier_count = tiers.len();
    let mut hits = Vec::new();
    let mut first_error = None;
    let mut finished = 0;
    for (text, limit) in tiers {
        if mytools_plugin_api::cancelled() {
            return Err(SearchError::Cancelled);
        }
        // Everything keeps running a query after we stop waiting for it, so a query that
        // cannot finish in time would only delay the next search; skip it instead.
        if finished > 0 && deadline.saturating_duration_since(Instant::now()) < MIN_TIER_TIME {
            break;
        }
        match ipc::search(&text, limit, deadline) {
            Ok(found) => {
                hits.extend(found);
                finished += 1;
            }
            Err(error @ (SearchError::NotRunning | SearchError::Cancelled)) => return Err(error),
            Err(SearchError::Busy) => {
                first_error.get_or_insert(SearchError::Busy);
                break; // Further queries would queue behind the unfinished one.
            }
            // A failing tier (e.g. unsupported syntax) still leaves the others to run.
            Err(error) => {
                first_error.get_or_insert(error);
                finished += 1;
            }
        }
    }
    match first_error {
        Some(error) if hits.is_empty() => Err(error),
        _ => Ok(Found {
            hits: relevance::rank_hits(hits, query, MAX_RESULTS),
            incomplete: finished < tier_count,
        }),
    }
}

#[cfg(not(windows))]
pub fn query_everything(_query: &str) -> Result<Found, SearchError> {
    Err(SearchError::Failed("unsupported".into()))
}

