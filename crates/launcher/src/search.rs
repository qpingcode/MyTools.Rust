use mytools_core::{AliasTable, Catalog, SearchOptions, SearchResults};
use std::{
    sync::{
        Arc, OnceLock,
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

/// What the worker reports back to the UI thread.
pub enum Event {
    /// Input has settled and plugins are being queried; typing alone never sends this.
    Started {
        generation: u64,
    },
    Completed(Completed),
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
    aliases: Arc<OnceLock<AliasTable>>,
    thread: Option<JoinHandle<()>>,
}

impl SearchWorker {
    pub fn start() -> (Self, async_channel::Receiver<Event>) {
        let (requests, receiver) = mpsc::channel::<Option<Query>>();
        let (events, results) = async_channel::unbounded();
        let latest = Arc::new(AtomicU64::new(0));
        let worker_latest = latest.clone();
        let aliases = Arc::new(OnceLock::new());
        let worker_aliases = aliases.clone();
        let thread = thread::spawn(move || {
            let catalog = Catalog::load();
            let _ = worker_aliases.set(catalog.alias_table());
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
                if events.send_blocking(Event::Started { generation }).is_err() {
                    break;
                }
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
                if events
                    .send_blocking(Event::Completed(Completed {
                        generation,
                        results,
                    }))
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
                aliases,
                thread: Some(thread),
            },
            results,
        )
    }

    pub fn sender(&self) -> QuerySender {
        QuerySender {
            requests: self.requests.clone(),
            latest: self.latest.clone(),
            aliases: self.aliases.clone(),
        }
    }
}

/// Sends queries to the worker from the UI thread.
#[derive(Clone)]
pub struct QuerySender {
    requests: mpsc::Sender<Option<Query>>,
    latest: Arc<AtomicU64>,
    aliases: Arc<OnceLock<AliasTable>>,
}

impl QuerySender {
    /// The plugin an alias query will search; `None` for global search or while the
    /// catalog is still loading.
    pub fn plugin_for(&self, text: &str) -> Option<String> {
        self.aliases.get()?.plugin_for(text).map(str::to_owned)
    }

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
