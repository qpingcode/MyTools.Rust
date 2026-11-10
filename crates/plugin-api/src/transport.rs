//! The resident plugin loop shared by every plugin.
//!
//! A reader thread owns stdin so heartbeats (`{"ping":n}` → `{"pong":n,"busyMs":..}`) are
//! answered even while a request is running, and so a newer search can cancel an older one.
//! Requests still run one at a time on the calling thread and every request gets exactly one
//! response line, in order; a cancelled search answers with an empty response.
//!
//! The host's first request is `{"init":{}}`. The transport runs the handler registered
//! with [`on_init`] (if any) and answers `{"ready":true}`, or `{"error":"..."}` when the
//! handler fails; [`progress`] lines may precede the answer.
use crate::{PluginActionRequest, SearchRequest, SearchResponse};
use std::{
    cell::RefCell,
    collections::VecDeque,
    io::{self, BufRead, Write},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// The request being handled on this thread.
struct Current {
    /// Non-zero for searches, which a newer search cancels.
    sequence: u64,
    deadline: Option<Instant>,
    /// Sequence number of the newest search received so far.
    latest: Arc<AtomicU64>,
}

type Output = Arc<Mutex<Box<dyn Write + Send>>>;
type InitHandler = Box<dyn FnOnce() -> Result<(), String> + Send>;

static INIT: Mutex<Option<InitHandler>> = Mutex::new(None);

thread_local! {
    static CURRENT: RefCell<Option<Current>> = const { RefCell::new(None) };
    /// Set while the init handler runs, so [`progress`] can report to the host.
    static PROGRESS: RefCell<Option<Output>> = const { RefCell::new(None) };
}

/// Registers work the plugin must finish before it answers searches, such as opening a
/// database. Call it before `serve`; the host waits for it (up to its startup timeout) and
/// shows the plugin as starting meanwhile. Return `Err` with a short reason when the plugin
/// cannot work; the host then stops it and shows the reason. Plugins without one are ready
/// as soon as they serve. Long indexing should rather run in the background after init.
pub fn on_init(handler: impl FnOnce() -> Result<(), String> + Send + 'static) {
    *INIT.lock().unwrap() = Some(Box::new(handler));
}

/// Reports what a running init handler is doing, e.g. "Scanning the Start menu". Has no
/// effect outside the handler.
pub fn progress(message: &str) {
    PROGRESS.with(|output| {
        if let Some(output) = output.borrow().as_ref() {
            let _ = write_line(
                output,
                &SearchResponse {
                    progress: Some(message.into()),
                    ..Default::default()
                },
            );
        }
    });
}

fn initialize(output: &Output) -> SearchResponse {
    let handler = INIT.lock().unwrap().take();
    PROGRESS.with(|current| *current.borrow_mut() = Some(output.clone()));
    let result = handler.map_or(Ok(()), |handler| handler());
    PROGRESS.with(|current| *current.borrow_mut() = None);
    match result {
        Ok(()) => SearchResponse {
            ready: Some(true),
            ..Default::default()
        },
        Err(error) => SearchResponse {
            error: Some(error),
            ..Default::default()
        },
    }
}

/// True when the host has sent a newer search than the one being handled, so its
/// result would be discarded. Long-running searches should check this and return early.
/// Always false for actions and outside a request.
pub fn cancelled() -> bool {
    CURRENT.with(|current| {
        current.borrow().as_ref().is_some_and(|current| {
            current.sequence != 0 && current.sequence < current.latest.load(Ordering::SeqCst)
        })
    })
}

/// When the host stops waiting for the current search, if it said so (`timeoutMs`).
/// Plugins with several internal steps use it to fit their work into the budget.
pub fn deadline() -> Option<Instant> {
    CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()
            .and_then(|current| current.deadline)
    })
}

enum Job {
    /// `sequence` is non-zero for searches, which may be cancelled by newer ones.
    Request {
        request: SearchRequest,
        sequence: u64,
        received: Instant,
    },
    End(io::Result<()>),
}

struct Queue {
    jobs: Mutex<VecDeque<Job>>,
    ready: Condvar,
    /// When the request being handled started; reported as `busyMs`.
    busy_since: Mutex<Option<Instant>>,
}

fn write_line(output: &Mutex<Box<dyn Write + Send>>, response: &SearchResponse) -> io::Result<()> {
    let mut output = output.lock().unwrap();
    serde_json::to_writer(&mut *output, response)?;
    writeln!(output)?;
    output.flush()
}

pub(crate) fn run(
    input: impl BufRead + Send + 'static,
    output: impl Write + Send + 'static,
    handle: impl Fn(SearchRequest) -> SearchResponse,
) -> io::Result<()> {
    let output: Output = Arc::new(Mutex::new(Box::new(output)));
    let queue = Arc::new(Queue {
        jobs: Mutex::new(VecDeque::new()),
        ready: Condvar::new(),
        busy_since: Mutex::new(None),
    });
    let latest = Arc::new(AtomicU64::new(0));
    let reader_queue = queue.clone();
    let reader_output = output.clone();
    let reader_latest = latest.clone();
    std::thread::spawn(move || {
        let end = read(input, &reader_queue, &reader_output, &reader_latest);
        reader_queue.jobs.lock().unwrap().push_back(Job::End(end));
        reader_queue.ready.notify_one();
    });
    loop {
        let job = {
            let mut jobs = queue.jobs.lock().unwrap();
            loop {
                if let Some(job) = jobs.pop_front() {
                    break job;
                }
                jobs = queue.ready.wait(jobs).unwrap();
            }
        };
        let (request, sequence, received) = match job {
            Job::Request {
                request,
                sequence,
                received,
            } => (request, sequence, received),
            Job::End(result) => return result,
        };
        let deadline = request
            .timeout_ms
            .map(|ms| received + Duration::from_millis(ms));
        CURRENT.with(|current| {
            *current.borrow_mut() = Some(Current {
                sequence,
                deadline,
                latest: latest.clone(),
            })
        });
        let response = if request.init.is_some() {
            initialize(&output)
        } else if cancelled() {
            SearchResponse::default()
        } else {
            *queue.busy_since.lock().unwrap() = Some(Instant::now());
            let response = handle(request);
            *queue.busy_since.lock().unwrap() = None;
            response
        };
        CURRENT.with(|current| *current.borrow_mut() = None);
        write_line(&output, &response)?;
    }
}

fn read(
    input: impl BufRead,
    queue: &Queue,
    output: &Mutex<Box<dyn Write + Send>>,
    latest: &AtomicU64,
) -> io::Result<()> {
    let mut searches = 0u64;
    for line in input.lines() {
        let request: SearchRequest = serde_json::from_str(&line?)?;
        if let Some(ping) = request.ping {
            let busy = queue
                .busy_since
                .lock()
                .unwrap()
                .map(|since| since.elapsed().as_millis() as u64)
                .unwrap_or(0);
            write_line(
                output,
                &SearchResponse {
                    pong: Some(ping),
                    busy_ms: Some(busy),
                    ..Default::default()
                },
            )?;
            continue;
        }
        let sequence = if request.action.is_none() && request.init.is_none() {
            searches += 1;
            latest.store(searches, Ordering::SeqCst);
            searches
        } else {
            0
        };
        queue.jobs.lock().unwrap().push_back(Job::Request {
            request,
            sequence,
            received: Instant::now(),
        });
        queue.ready.notify_one();
    }
    Ok(())
}

/// Dispatches a request to the search or action handler.
pub(crate) fn dispatch(
    request: SearchRequest,
    search: &impl Fn(&str) -> SearchResponse,
    action: &impl Fn(PluginActionRequest) -> SearchResponse,
) -> SearchResponse {
    match request.action {
        Some(command) => action(command),
        None => search(&request.query),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Read};

    /// Collects written bytes so a test can read them back.
    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn responses(sink: &Sink) -> Vec<SearchResponse> {
        sink.0
            .lock()
            .unwrap()
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect()
    }

    /// A pipe whose reader blocks until the test writes, like a real stdin.
    fn pipe() -> (
        std::sync::mpsc::Sender<Vec<u8>>,
        impl BufRead + Send + 'static,
    ) {
        struct Pipe(std::sync::mpsc::Receiver<Vec<u8>>, Vec<u8>);
        impl Read for Pipe {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if self.1.is_empty() {
                    match self.0.recv() {
                        Ok(bytes) => self.1 = bytes,
                        Err(_) => return Ok(0),
                    }
                }
                let count = buffer.len().min(self.1.len());
                buffer[..count].copy_from_slice(&self.1[..count]);
                self.1.drain(..count);
                Ok(count)
            }
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        (sender, BufReader::new(Pipe(receiver, vec![])))
    }

    #[test]
    fn init_reports_progress_then_ready_before_searches() {
        on_init(|| {
            progress("warming up");
            Ok(())
        });
        let sink = Sink::default();
        let input = b"{\"init\":{}}\n{\"query\":\"q\"}\n".to_vec();
        run(std::io::Cursor::new(input), sink.clone(), |request| {
            SearchResponse {
                error: Some(request.query),
                ..Default::default()
            }
        })
        .unwrap();
        let all = responses(&sink);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].progress.as_deref(), Some("warming up"));
        assert_eq!(all[1].ready, Some(true));
        assert_eq!(all[2].error.as_deref(), Some("q"));
        // The handler runs once; a later init without one is ready at once.
        let sink = Sink::default();
        run(
            std::io::Cursor::new(b"{\"init\":{}}\n".to_vec()),
            sink.clone(),
            |_| SearchResponse::default(),
        )
        .unwrap();
        assert_eq!(responses(&sink)[0].ready, Some(true));
    }

    #[test]
    fn ping_is_answered_while_a_search_runs_and_newer_search_cancels_older() {
        let (input, reader) = pipe();
        let sink = Sink::default();
        let output = sink.clone();
        let (started, wait_started) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            run(reader, output, move |request| {
                if request.query == "slow" {
                    started.send(()).unwrap();
                    let give_up = Instant::now() + Duration::from_secs(10);
                    while !cancelled() && Instant::now() < give_up {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    return SearchResponse {
                        error: Some(format!("cancelled={}", cancelled())),
                        ..Default::default()
                    };
                }
                SearchResponse {
                    error: Some(format!("{}:{:?}", request.query, deadline().is_some())),
                    ..Default::default()
                }
            })
        });
        input.send(b"{\"query\":\"slow\"}\n".to_vec()).unwrap();
        wait_started.recv().unwrap();
        input.send(b"{\"ping\":7}\n".to_vec()).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while responses(&sink).is_empty() {
            assert!(
                Instant::now() < until,
                "ping was not answered during a search"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let pong = &responses(&sink)[0];
        assert_eq!(pong.pong, Some(7));
        assert!(pong.busy_ms.is_some());
        input
            .send(b"{\"query\":\"fast\",\"timeoutMs\":5000}\n".to_vec())
            .unwrap();
        drop(input);
        worker.join().unwrap().unwrap();
        let all = responses(&sink);
        assert_eq!(all.len(), 3);
        assert_eq!(all[1].error.as_deref(), Some("cancelled=true"));
        assert_eq!(all[2].error.as_deref(), Some("fast:true"));
    }
}
