use mytools_core::{Catalog, SearchOptions, SearchResults};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub struct Query {
    pub generation: u64,
    pub text: String,
    /// 0 for typed input; a retry asks plugins to wait longer.
    pub attempt: u32,
}

pub struct Completed {
    pub generation: u64,
    pub results: SearchResults,
}

/// The retry attempt for the next search the UI starts; consumed by that search.
static NEXT_ATTEMPT: AtomicU32 = AtomicU32::new(0);

/// Makes the next search of the current query a retry with a longer timeout.
pub fn request_retry(attempt: u32) {
    NEXT_ATTEMPT.store(attempt, Ordering::Relaxed);
}

pub fn take_retry_attempt() -> u32 {
    NEXT_ATTEMPT.swap(0, Ordering::Relaxed)
}

/// A single background worker owns the catalog and its resident plugin processes.
/// New input replaces queued input before searching and cancels a search that is still
/// waiting for plugins; the UI also rejects stale results.
pub struct SearchWorker {
    requests: mpsc::Sender<Option<Query>>,
    /// Generation of the newest query sent; a search for an older one stops waiting.
    latest: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl SearchWorker {
    pub fn start() -> (Self, async_channel::Receiver<Completed>) {
        let (requests, receiver) = mpsc::channel::<Option<Query>>();
        let (completed, results) = async_channel::unbounded();
        let latest = Arc::new(AtomicU64::new(0));
        let worker_latest = latest.clone();
        let thread = thread::spawn(move || {
            let catalog = Catalog::load();
            'requests: while let Ok(Some(mut query)) = receiver.recv() {
                // Typed input settles for a moment; an explicit retry runs at once.
                while query.attempt == 0 {
                    match receiver.recv_timeout(Duration::from_millis(120)) {
                        Ok(Some(newer)) => query = newer,
                        Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break 'requests,
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                    }
                }
                let generation = query.generation;
                let is_stale = || worker_latest.load(Ordering::SeqCst) != generation;
                let results = catalog.search_with(
                    &query.text,
                    &SearchOptions {
                        attempt: query.attempt,
                        cancelled: &is_stale,
                    },
                );
                if results.cancelled {
                    continue; // The newer query is already queued.
                }
                if completed
                    .send_blocking(Completed {
                        generation,
                        results,
                    })
                    .is_err()
                {
                    break;
                }
            }
            catalog.shutdown();
        });
        (
            Self {
                requests,
                latest,
                thread: Some(thread),
            },
            results,
        )
    }

    pub fn sender(&self) -> QuerySender {
        QuerySender {
            requests: self.requests.clone(),
            latest: self.latest.clone(),
        }
    }
}

/// Sends queries to the worker from the UI thread.
#[derive(Clone)]
pub struct QuerySender {
    requests: mpsc::Sender<Option<Query>>,
    latest: Arc<AtomicU64>,
}

impl QuerySender {
    pub fn send(&self, query: Query) {
        self.latest.store(query.generation, Ordering::SeqCst);
        let _ = self.requests.send(Some(query));
    }
}

impl Drop for SearchWorker {
    fn drop(&mut self) {
        let _ = self.requests.send(None);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
