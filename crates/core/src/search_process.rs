use crate::i18n;
use anyhow::{Context, Result, anyhow, bail};
use mytools_plugin_api::{PluginActionRequest, PluginEffect, SearchRequest, SearchResponse};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(1);
const ACTION_TIMEOUT: Duration = Duration::from_secs(60);
/// A plugin that crashes repeatedly is restarted at most this often per window.
const RESTART_LIMIT: usize = 5;
const RESTART_WINDOW: Duration = Duration::from_secs(60);
/// Heartbeats are separate `ping` messages answered by the plugin transport even while
/// a request runs, so they measure whether the process responds, not how slow a search is.
const PING_INTERVAL: Duration = Duration::from_secs(10);
const PONG_TIMEOUT: Duration = Duration::from_secs(3);
/// A process that has not answered anything yet may still be initialising.
const STARTUP_GRACE: Duration = Duration::from_secs(30);
/// A search still running this long (the longest alias timeout plus slack) is stuck.
const SEARCH_HUNG: Duration = Duration::from_secs(20);
/// Searches abandoned by the host stay queued until the plugin answers or skips them.
const MAX_PENDING: usize = 8;
type Reply = Receiver<Result<SearchResponse>>;
type Factory = Box<dyn Fn() -> Command + Send + Sync>;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Search,
    Action,
}

struct Pending {
    reply: mpsc::Sender<Result<SearchResponse>>,
    kind: Kind,
}

#[derive(Default)]
struct Heartbeat {
    sequence: u64,
    /// When the unanswered ping was sent.
    outstanding: Option<Instant>,
    last_sent: Option<Instant>,
}

/// State shared by the requesting threads, the reader thread and the watchdog.
struct Shared {
    stdin: Mutex<ChildStdin>,
    /// Responses arrive in request order; `pong` lines are not requests.
    pending: Mutex<VecDeque<Pending>>,
    /// Cleared when the pipe breaks, a response is invalid or the plugin is stuck.
    healthy: AtomicBool,
    /// Set by the first line from the plugin: it has finished starting.
    responsive: AtomicBool,
    started: Instant,
    heartbeat: Mutex<Heartbeat>,
}

impl Shared {
    fn write(&self, wire: &SearchRequest) -> Result<()> {
        let mut stdin = self.stdin.lock().unwrap();
        let written = (|| -> std::io::Result<()> {
            serde_json::to_writer(&mut *stdin, wire)?;
            writeln!(stdin)?;
            stdin.flush()
        })();
        if written.is_err() {
            self.healthy.store(false, Ordering::Relaxed);
        }
        Ok(written?)
    }

    /// Sends a ping when one is due; marks the plugin unhealthy when a ping went unanswered.
    fn heartbeat(&self) {
        let now = Instant::now();
        let mut heartbeat = self.heartbeat.lock().unwrap();
        if let Some(sent) = heartbeat.outstanding {
            let starting = !self.responsive.load(Ordering::Relaxed)
                && now.duration_since(self.started) < STARTUP_GRACE;
            if now.duration_since(sent) > PONG_TIMEOUT && !starting {
                self.healthy.store(false, Ordering::Relaxed);
            }
            return;
        }
        if heartbeat
            .last_sent
            .is_some_and(|last| now.duration_since(last) < PING_INTERVAL)
        {
            return;
        }
        heartbeat.sequence += 1;
        heartbeat.outstanding = Some(now);
        heartbeat.last_sent = Some(now);
        let ping = SearchRequest {
            query: String::new(),
            action: None,
            timeout_ms: None,
            ping: Some(heartbeat.sequence),
        };
        drop(heartbeat);
        let _ = self.write(&ping);
    }

    /// Handles a `pong`: clears the outstanding ping and flags a request that is stuck.
    fn pong(&self, sequence: u64, busy_ms: u64) {
        let mut heartbeat = self.heartbeat.lock().unwrap();
        if heartbeat.sequence == sequence {
            heartbeat.outstanding = None;
        }
        drop(heartbeat);
        let front = self
            .pending
            .lock()
            .unwrap()
            .front()
            .map(|pending| pending.kind);
        let limit = match front {
            Some(Kind::Action) => ACTION_TIMEOUT,
            _ => SEARCH_HUNG,
        };
        if Duration::from_millis(busy_ms) > limit {
            self.healthy.store(false, Ordering::Relaxed);
        }
    }

    fn fail_pending(&self, error: &anyhow::Error) {
        for pending in self.pending.lock().unwrap().drain(..) {
            let _ = pending.reply.send(Err(anyhow!("{error:#}")));
        }
    }
}

/// One spawned plugin process and its ordered request transport.
struct Running {
    child: Child,
    shared: Arc<Shared>,
}

impl Running {
    fn is_alive(&mut self) -> bool {
        self.shared.healthy.load(Ordering::Relaxed) && matches!(self.child.try_wait(), Ok(None))
    }

    fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A resident search plugin. It restarts itself when the process exits, stops answering
/// heartbeats, stays stuck on a request or returns an invalid response, so references
/// held by actions stay valid.
pub struct SearchProcess {
    factory: Factory,
    running: Mutex<Option<Running>>,
    shutdown: AtomicBool,
    action_busy: Mutex<bool>,
    restarts: Mutex<VecDeque<Instant>>,
}

impl std::fmt::Debug for SearchProcess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SearchProcess")
            .field("shutdown", &self.shutdown.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl SearchProcess {
    pub fn start(path: &Path) -> Result<Self> {
        let directory = path
            .parent()
            .context(i18n::t("error.plugin_directory"))?
            .to_path_buf();
        let program = path.to_path_buf();
        Self::start_factory(Box::new(move || {
            let mut command = Command::new(&program);
            command
                .current_dir(&directory)
                .env("MYTOOLS_LOCALE", mytools_plugin_api::i18n::language())
                .env(
                    mytools_plugin_api::plugin_settings::ENV,
                    crate::plugins::settings_path(),
                )
                .env(
                    mytools_plugin_api::crash::ENV,
                    mytools_plugin_api::crash::directory(),
                );
            command
        }))
    }

    fn start_factory(factory: Factory) -> Result<Self> {
        let running = spawn(factory())?;
        Ok(Self {
            factory,
            running: Mutex::new(Some(running)),
            shutdown: AtomicBool::new(false),
            action_busy: Mutex::new(false),
            restarts: Mutex::new(VecDeque::new()),
        })
    }

    /// Starts a search; `timeout` tells the plugin how long the host will wait for it.
    /// A search the host stops waiting for is not killed: the plugin skips or finishes it.
    pub fn begin(&self, query: &str, timeout: Duration) -> Result<Reply> {
        // Do not queue a search behind a long-running action on the same ordered
        // transport. The action's completion may request a fresh search.
        let busy = self.action_busy.lock().unwrap();
        if *busy {
            bail!(i18n::t("error.action_in_progress"));
        }
        self.request(
            SearchRequest {
                query: query.into(),
                action: None,
                timeout_ms: Some(timeout.as_millis() as u64),
                ping: None,
            },
            Kind::Search,
        )
    }

    fn request(&self, wire: SearchRequest, kind: Kind) -> Result<Reply> {
        let mut running = self.running.lock().unwrap();
        let shared = self.ensure_running(&mut running)?;
        drop(running);
        let (reply, receiver) = mpsc::channel();
        // Queue and write under one lock so responses match requests in order.
        let mut pending = shared.pending.lock().unwrap();
        if pending.len() >= MAX_PENDING {
            bail!(i18n::format("error.busy", &[("error", "queue full")]));
        }
        pending.push_back(Pending { reply, kind });
        if let Err(error) = shared.write(&wire) {
            pending.pop_back();
            return Err(error);
        }
        Ok(receiver)
    }

    /// Returns the live transport, restarting a dead process within the restart budget.
    fn ensure_running(&self, running: &mut Option<Running>) -> Result<Arc<Shared>> {
        if self.shutdown.load(Ordering::Relaxed) {
            bail!(i18n::t("error.process_stopped"));
        }
        if let Some(current) = running.as_mut()
            && current.is_alive()
        {
            return Ok(current.shared.clone());
        }
        if let Some(dead) = running.take() {
            dead.kill();
        }
        if !self.allow_restart() {
            bail!(i18n::t("error.process_stopped"));
        }
        let fresh = spawn((self.factory)())?;
        let shared = fresh.shared.clone();
        *running = Some(fresh);
        Ok(shared)
    }

    fn allow_restart(&self) -> bool {
        let mut restarts = self.restarts.lock().unwrap();
        let now = Instant::now();
        while restarts
            .front()
            .is_some_and(|time| now.duration_since(*time) > RESTART_WINDOW)
        {
            restarts.pop_front();
        }
        if restarts.len() >= RESTART_LIMIT {
            return false;
        }
        restarts.push_back(now);
        true
    }

    pub fn execute(&self, command: &str, item: Option<&str>) -> Result<PluginEffect> {
        let mut busy = self.action_busy.lock().unwrap();
        if *busy {
            bail!(i18n::t("error.action_in_progress"));
        }
        *busy = true;
        let reply = self.request(
            SearchRequest {
                query: String::new(),
                action: Some(PluginActionRequest {
                    command: command.into(),
                    item: item.map(str::to_owned),
                }),
                timeout_ms: None,
                ping: None,
            },
            Kind::Action,
        );
        drop(busy);
        let result = self.wait_action(reply);
        *self.action_busy.lock().unwrap() = false;
        result
    }

    fn wait_action(&self, reply: Result<Reply>) -> Result<PluginEffect> {
        let response = match reply?.recv_timeout(ACTION_TIMEOUT) {
            Ok(response) => response?,
            Err(_) => {
                self.stop();
                bail!(i18n::t("error.timeout"));
            }
        };
        if let Some(error) = response.error {
            bail!("{error}");
        }
        response
            .effect
            .ok_or_else(|| anyhow!(i18n::t("error.response_invalid")))
    }

    /// Kills a misbehaving process; the next request or the watchdog starts a new one.
    pub fn stop(&self) {
        if let Some(running) = self.running.lock().unwrap().take() {
            running.kill();
        }
    }

    /// Permanently stops the plugin, e.g. when MyTools exits.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.stop();
    }

    /// One watchdog tick: restarts a dead, unresponsive or stuck process and sends a
    /// heartbeat when one is due.
    pub(crate) fn check(&self) {
        if self.shutdown.load(Ordering::Relaxed) {
            return;
        }
        let mut running = self.running.lock().unwrap();
        let alive = running.as_mut().is_some_and(Running::is_alive);
        if !alive {
            // Errors (restart budget exhausted, spawn failure) are retried next tick.
            let _ = self.ensure_running(&mut running);
            return;
        }
        let shared = running.as_ref().map(|running| running.shared.clone());
        drop(running);
        if let Some(shared) = shared {
            shared.heartbeat();
        }
    }
}

/// Supervises resident plugins until every process is dropped.
pub(crate) fn watch(processes: Vec<std::sync::Weak<SearchProcess>>) {
    if processes.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let live: Vec<_> = processes.iter().filter_map(|weak| weak.upgrade()).collect();
            if live.is_empty()
                || live
                    .iter()
                    .all(|process| process.shutdown.load(Ordering::Relaxed))
            {
                break;
            }
            for process in live {
                process.check();
            }
        }
    });
}

fn spawn(mut command: Command) -> Result<Running> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW: headless plugin.
    }
    let mut child = command.spawn().context(i18n::t("error.process_start"))?;
    let stdin = child.stdin.take().context(i18n::t("error.stdin"))?;
    let stdout = BufReader::new(child.stdout.take().context(i18n::t("error.stdout"))?);
    let shared = Arc::new(Shared {
        stdin: Mutex::new(stdin),
        pending: Mutex::new(VecDeque::new()),
        healthy: AtomicBool::new(true),
        responsive: AtomicBool::new(false),
        started: Instant::now(),
        heartbeat: Mutex::new(Heartbeat::default()),
    });
    let reader = shared.clone();
    std::thread::spawn(move || {
        let error = read_responses(stdout, &reader);
        reader.healthy.store(false, Ordering::Relaxed);
        reader.fail_pending(&error);
    });
    Ok(Running { child, shared })
}

/// Routes plugin output until the pipe closes or a line is invalid; returns why it stopped.
fn read_responses(mut stdout: impl BufRead, shared: &Shared) -> anyhow::Error {
    loop {
        let mut line = Vec::new();
        let read = match Read::take(&mut stdout, 1_048_577).read_until(b'\n', &mut line) {
            Ok(read) => read,
            Err(error) => return error.into(),
        };
        if read == 0 {
            return anyhow!(i18n::t("error.process_exited"));
        }
        if read > 1_048_576 || !line.ends_with(b"\n") {
            return anyhow!(i18n::t("error.response_limit"));
        }
        let mut response: SearchResponse = match serde_json::from_slice(&line) {
            Ok(response) => response,
            Err(error) => return anyhow!(error).context(i18n::t("error.response_invalid")),
        };
        shared.responsive.store(true, Ordering::Relaxed);
        if let Some(sequence) = response.pong {
            shared.pong(sequence, response.busy_ms.unwrap_or(0));
            continue;
        }
        response.items.truncate(50);
        // The receiver may be gone when the host stopped waiting; that is fine.
        let next = shared.pending.lock().unwrap().pop_front();
        if let Some(pending) = next {
            let _ = pending.reply.send(Ok(response));
        }
    }
}

impl Drop for SearchProcess {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(all(test, windows))]
impl SearchProcess {
    fn powershell(script: &'static str) -> Factory {
        Box::new(move || {
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ]);
            command
        })
    }

    pub(crate) fn fixture() -> Self {
        let process = Self::start_factory(Self::powershell(
            "$count=0; while ($null -ne ($line=[Console]::ReadLine())) { $r=$line|ConvertFrom-Json; if ($r.ping) { continue }; $q=$r.query; if ($q -eq 'slow') { Start-Sleep -Milliseconds 1500 }; $count++; @{items=@(@{title=$q;description=[string]$count;resource=@{type='text';text=$q}})}|ConvertTo-Json -Compress -Depth 5 }",
        ))
        .unwrap();
        process
            .begin("warmup", SEARCH_TIMEOUT)
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        process
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn running_action_is_awaited_without_queueing_short_timeout_searches() {
        let process = std::sync::Arc::new(SearchProcess::start_factory(SearchProcess::powershell(
            "while ($null -ne ($line=[Console]::ReadLine())) { $r=$line|ConvertFrom-Json; if ($r.action) { Start-Sleep -Milliseconds 1200; '{\"items\":[],\"effect\":{\"refresh\":true}}' } else { '{\"items\":[]}' } }",
        )).unwrap());
        process
            .begin("warmup", SEARCH_TIMEOUT)
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let worker = process.clone();
        let action = std::thread::spawn(move || worker.execute("translate", Some("test")));
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !*process.action_busy.lock().unwrap() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(process.begin("changed input", SEARCH_TIMEOUT).is_err());
        assert!(action.join().unwrap().unwrap().refresh);
        assert!(
            process
                .begin("completed", SEARCH_TIMEOUT)
                .unwrap()
                .recv_timeout(SEARCH_TIMEOUT)
                .unwrap()
                .is_ok()
        );
    }

    #[test]
    fn stopped_or_crashed_process_restarts_on_next_request() {
        let process = SearchProcess::fixture();
        process.stop();
        let response = process
            .begin("after stop", SEARCH_TIMEOUT)
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        // A fresh process starts counting from one again.
        assert_eq!(response.items[0].description, "1");
        if let Some(running) = process.running.lock().unwrap().as_mut() {
            let _ = running.child.kill();
            let _ = running.child.wait();
        }
        process.check();
        let response = process
            .begin("after crash", SEARCH_TIMEOUT)
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert_eq!(response.items[0].title, "after crash");
        process.shutdown();
        assert!(process.begin("after shutdown", SEARCH_TIMEOUT).is_err());
    }

    #[test]
    fn abandoned_search_does_not_shift_later_responses() {
        let process = SearchProcess::fixture();
        let slow = process.begin("slow", SEARCH_TIMEOUT).unwrap();
        assert!(slow.recv_timeout(Duration::from_millis(100)).is_err());
        drop(slow); // The host stopped waiting; the process must not be killed.
        let next = process
            .begin("next", SEARCH_TIMEOUT)
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert_eq!(next.items[0].title, "next");
        // Same process: the counter continues instead of restarting at one.
        assert_eq!(next.items[0].description, "3");
    }

    #[test]
    fn unanswered_heartbeat_or_stuck_request_restarts_plugin() {
        let process = SearchProcess::fixture();
        process.check(); // Sends the first ping, which this fixture ignores.
        let shared = process
            .running
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .shared
            .clone();
        let sent = shared.heartbeat.lock().unwrap().outstanding.unwrap();
        shared.heartbeat.lock().unwrap().outstanding = Some(sent - PONG_TIMEOUT * 2);
        process.check(); // Ping overdue: unhealthy.
        assert!(!shared.healthy.load(Ordering::Relaxed));
        process.check(); // Restarted.
        let restarted = process
            .begin("after restart", SEARCH_TIMEOUT)
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert_eq!(restarted.items[0].description, "1");

        let shared = process
            .running
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .shared
            .clone();
        shared.pong(0, 1000);
        assert!(shared.healthy.load(Ordering::Relaxed));
        shared.pong(0, SEARCH_HUNG.as_millis() as u64 + 1);
        assert!(!shared.healthy.load(Ordering::Relaxed));
    }

    #[test]
    fn restart_budget_limits_crash_loops() {
        let process = SearchProcess::fixture();
        for _ in 0..RESTART_LIMIT {
            assert!(process.allow_restart());
        }
        assert!(!process.allow_restart());
    }
}
