use mytools_core::{AliasTable, Catalog, LoadOptions, Notice, SearchOptions, SearchResults};
use std::{
    sync::{
        Arc, RwLock,
        atomic::{AtomicU32, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

/// How often the idle worker re-runs a search that waits for a plugin, applies requests
/// from the settings window and forwards problem notices.
const TICK: Duration = Duration::from_millis(250);

#[derive(Clone)]
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
    /// Plugin problems found in the background, such as a start that timed out.
    Notices(Vec<Notice>),
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
    aliases: Arc<RwLock<Option<AliasTable>>>,
    thread: Option<JoinHandle<()>>,
}

impl SearchWorker {
    pub fn start(options: LoadOptions) -> (Self, async_channel::Receiver<Event>) {
        let (requests, receiver) = mpsc::channel::<Option<Query>>();
        let (events, results) = async_channel::unbounded();
        let latest = Arc::new(AtomicU64::new(0));
        let worker_latest = latest.clone();
        let aliases = Arc::new(RwLock::new(None));
        let worker_aliases = aliases.clone();
        let thread = thread::spawn(move || {
            let mut worker = Worker {
                catalog: Catalog::load_with(&options),
                options,
                events,
                latest: worker_latest,
                aliases: worker_aliases,
                waiting: None,
            };
            worker.publish_aliases();
            'requests: loop {
                let mut query = match receiver.recv_timeout(TICK) {
                    Ok(Some(query)) => query,
                    Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if !worker.tick() {
                            break;
                        }
                        continue;
                    }
                };
                // Typed input settles for a moment; an explicit retry runs at once.
                while query.attempt == 0 {
                    match receiver.recv_timeout(Duration::from_millis(120)) {
                        Ok(Some(newer)) => query = newer,
                        Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break 'requests,
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                    }
                }
                let generation = query.generation;
                if worker
                    .events
                    .send_blocking(Event::Started { generation })
                    .is_err()
                {
                    break;
                }
                worker.reload_if_changed();
                if !worker.search(query) {
                    break;
                }
            }
            worker.catalog.shutdown();
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

/// The state owned by the worker thread.
struct Worker {
    catalog: Catalog,
    options: LoadOptions,
    events: async_channel::Sender<Event>,
    latest: Arc<AtomicU64>,
    aliases: Arc<RwLock<Option<AliasTable>>>,
    /// The last search, if it waited for a starting plugin, and the plugin states it saw.
    waiting: Option<(Query, u64)>,
}

impl Worker {
    /// Applies what was saved in Settings: enabled plugins, aliases and startup policies.
    fn reload_if_changed(&mut self) {
        if !self.catalog.settings_changed() {
            return;
        }
        self.catalog.shutdown();
        self.catalog = Catalog::load_with(&self.options);
        self.publish_aliases();
        // The new catalog counts revisions afresh; repeat a waiting search regardless.
        if let Some((_, revision)) = &mut self.waiting {
            *revision = u64::MAX;
        }
    }

    fn publish_aliases(&self) {
        *self.aliases.write().unwrap_or_else(|e| e.into_inner()) = Some(self.catalog.alias_table());
    }

    /// Runs one search and reports it; `false` once the UI is gone.
    fn search(&mut self, query: Query) -> bool {
        let generation = query.generation;
        let revision = self.catalog.revision();
        let latest = self.latest.clone();
        let is_stale = || latest.load(Ordering::SeqCst) != generation;
        let results = self.catalog.search_with(
            &query.text,
            &SearchOptions {
                attempt: query.attempt,
                cancelled: &is_stale,
            },
        );
        if results.cancelled {
            return true; // The newer query is already queued.
        }
        self.waiting = results.waiting.then_some((query, revision));
        self.events
            .send_blocking(Event::Completed(Completed {
                generation,
                results,
            }))
            .is_ok()
            && self.forward_notices()
    }

    /// Background work between searches; `false` once the UI is gone.
    fn tick(&mut self) -> bool {
        self.reload_if_changed();
        self.catalog.apply_requests();
        if !self.forward_notices() {
            return false;
        }
        // A plugin the shown results waited for changed state: show its results now.
        if let Some((query, revision)) = &self.waiting
            && *revision != self.catalog.revision()
            && self.latest.load(Ordering::SeqCst) == query.generation
        {
            let query = query.clone();
            return self.search(query);
        }
        true
    }

    fn forward_notices(&self) -> bool {
        let notices = self.catalog.take_notices();
        notices.is_empty() || self.events.send_blocking(Event::Notices(notices)).is_ok()
    }
}

/// Sends queries to the worker from the UI thread.
#[derive(Clone)]
pub struct QuerySender {
    requests: mpsc::Sender<Option<Query>>,
    latest: Arc<AtomicU64>,
    aliases: Arc<RwLock<Option<AliasTable>>>,
}

impl QuerySender {
    /// The plugin an alias query will search; `None` for global search or while the
    /// catalog is still loading.
    pub fn plugin_for(&self, text: &str) -> Option<String> {
        self.aliases
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()?
            .plugin_for(text)
            .map(str::to_owned)
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
