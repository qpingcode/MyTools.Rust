use mytools_core::{Catalog, SearchResults};
use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Duration,
};

pub struct Query {
    pub generation: u64,
    pub text: String,
}

pub struct Completed {
    pub generation: u64,
    pub results: SearchResults,
}

/// A single background worker owns the catalog and its resident plugin processes.
/// New input replaces queued input before searching; the UI also rejects stale results.
pub struct SearchWorker {
    pub requests: mpsc::Sender<Option<Query>>,
    thread: Option<JoinHandle<()>>,
}

impl SearchWorker {
    pub fn start() -> (Self, async_channel::Receiver<Completed>) {
        let (requests, receiver) = mpsc::channel::<Option<Query>>();
        let (completed, results) = async_channel::unbounded();
        let thread = thread::spawn(move || {
            let catalog = Catalog::load();
            'requests: while let Ok(Some(mut query)) = receiver.recv() {
                loop {
                    match receiver.recv_timeout(Duration::from_millis(120)) {
                        Ok(Some(newer)) => query = newer,
                        Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break 'requests,
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                    }
                }
                let results = catalog.search(&query.text);
                if completed
                    .send_blocking(Completed {
                        generation: query.generation,
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
                thread: Some(thread),
            },
            results,
        )
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
