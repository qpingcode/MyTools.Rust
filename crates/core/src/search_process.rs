use crate::i18n;
use crate::status::{Board, Notice};
use anyhow::{Context, Result, anyhow, bail};
use mytools_plugin_api::{
    PluginActionRequest, PluginEffect, SearchRequest, SearchResponse,
    log::{self, Level, Record},
    plugin_status::State,
};
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
/// How long a plugin may take to answer `init` unless configured otherwise.
pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
pub const MIN_STARTUP_TIMEOUT: Duration = Duration::from_secs(1);
pub const MAX_STARTUP_TIMEOUT: Duration = Duration::from_secs(120);
/// A search still running this long (the longest alias timeout plus slack) is stuck.
const SEARCH_HUNG: Duration = Duration::from_secs(20);
/// Searches abandoned by the host stay queued until the plugin answers or skips them.
const MAX_PENDING: usize = 8;
type Reply = Receiver<Result<SearchResponse>>;
type Factory = Box<dyn Fn() -> Command + Send + Sync>;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Init,
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
    /// Why `healthy` was first cleared, for the log.
    unhealthy: Mutex<Option<String>>,
    /// Set when the plugin answers `init`: it takes searches and heartbeats from now on.
    ready: AtomicBool,
    /// The reason the plugin gave for failing `init`.
    init_error: Mutex<Option<String>>,
    started: Instant,
    heartbeat: Mutex<Heartbeat>,
    status: Status,
}

/// Where one process reports its state.
#[derive(Clone)]
struct Status {
    board: Arc<Board>,
    id: String,
}

impl Status {
    fn set(&self, state: State, detail: Option<String>) {
        self.board.set(&self.id, state, detail);
    }

    fn log(&self, level: Level, event: &str, message: impl Into<String>) {
        log::write_from(&self.id, Record::new(level, event, message));
    }
}

impl Shared {
    /// Marks the process for restart, remembering the first reason.
    fn unhealthy(&self, reason: impl FnOnce() -> String) {
        if self.healthy.swap(false, Ordering::Relaxed) {
            *self.unhealthy.lock().unwrap() = Some(reason());
        }
    }

    fn write(&self, wire: &SearchRequest) -> Result<()> {
        let mut stdin = self.stdin.lock().unwrap();
        let written = (|| -> std::io::Result<()> {
            serde_json::to_writer(&mut *stdin, wire)?;
            writeln!(stdin)?;
            stdin.flush()
        })();
        if let Err(error) = &written {
            self.unhealthy(|| format!("could not send a request to the plugin: {error}"));
        }
        Ok(written?)
    }

    /// Sends a ping when one is due; marks the plugin unhealthy when a ping went unanswered.
    fn heartbeat(&self) {
        let now = Instant::now();
        let mut heartbeat = self.heartbeat.lock().unwrap();
        if let Some(sent) = heartbeat.outstanding {
            if now.duration_since(sent) > PONG_TIMEOUT {
                self.unhealthy(|| {
                    format!("no heartbeat answer within {}s", PONG_TIMEOUT.as_secs())
                });
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
            init: None,
            scoped: false,
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
        let (limit, work) = match front {
            Some(Kind::Action) => (ACTION_TIMEOUT, "an action"),
            _ => (SEARCH_HUNG, "a search"),
        };
        if Duration::from_millis(busy_ms) > limit {
            self.unhealthy(|| format!("{work} ran longer than {}s", limit.as_secs()));
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

/// Who a process is and how long it may take to start.
pub struct ProcessOptions {
    pub id: String,
    pub name: String,
    pub startup_timeout: Duration,
    pub board: Arc<Board>,
}

/// A resident search plugin. A new process first answers `init`; one that fails or times
/// out there is stopped and stays failed until [`SearchProcess::restart`]. Once ready, it
/// restarts itself when the process exits, stops answering heartbeats, stays stuck on a
/// request or returns an invalid response, so references held by actions stay valid.
/// A lazy plugin starts on the first request ([`SearchProcess::activate`]).
pub struct SearchProcess {
    factory: Factory,
    running: Mutex<Option<Running>>,
    shutdown: AtomicBool,
    action_busy: Mutex<bool>,
    restarts: Mutex<VecDeque<Instant>>,
    name: String,
    status: Status,
    startup_timeout: Duration,
    /// A process has been started at least once; the watchdog only revives those.
    started_once: AtomicBool,
    /// Stopped after a failed start or a crash loop, until restarted by the user.
    suspended: AtomicBool,
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
    /// An eager plugin starts now; a lazy one waits for [`Self::activate`].
    pub fn new(path: &Path, options: ProcessOptions, eager: bool) -> Result<Self> {
        let directory = path
            .parent()
            .context(i18n::t("error.plugin_directory"))?
            .to_path_buf();
        let program = path.to_path_buf();
        let factory: Factory = Box::new(move || {
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
        });
        Ok(Self::with_factory(factory, options, eager))
    }

    fn with_factory(factory: Factory, options: ProcessOptions, eager: bool) -> Self {
        let process = Self {
            factory,
            running: Mutex::new(None),
            shutdown: AtomicBool::new(false),
            action_busy: Mutex::new(false),
            restarts: Mutex::new(VecDeque::new()),
            name: options.name,
            status: Status {
                board: options.board,
                id: options.id,
            },
            startup_timeout: options.startup_timeout,
            started_once: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
        };
        if eager {
            let mut running = process.running.lock().unwrap();
            let _ = process.ensure_running(&mut running);
        }
        process
    }

    /// The state shown to the user, with its reason or progress.
    pub fn state(&self) -> (State, Option<String>) {
        self.status
            .board
            .get(&self.status.id)
            .unwrap_or((State::Idle, None))
    }

    /// Starts a lazy plugin that has not been started (or was stopped); no effect on a
    /// running or failed one.
    pub fn activate(&self) {
        let mut running = self.running.lock().unwrap();
        if running.is_none() && !self.suspended.load(Ordering::Relaxed) {
            let _ = self.ensure_running(&mut running);
        }
    }

    /// Starts the plugin afresh at the user's request, clearing a failure.
    pub fn restart(&self) {
        if self.shutdown.load(Ordering::Relaxed) {
            return;
        }
        self.status.log(
            Level::Info,
            "plugin.restart_requested",
            "restarting on request",
        );
        let mut running = self.running.lock().unwrap();
        if let Some(current) = running.take() {
            current.kill();
        }
        self.suspended.store(false, Ordering::Relaxed);
        self.restarts.lock().unwrap().clear();
        self.started_once.store(false, Ordering::Relaxed);
        let _ = self.ensure_running(&mut running);
    }

    /// Starts a search; `timeout` tells the plugin how long the host will wait for it, and
    /// `scoped` that the user chose this plugin by its alias. A search the host stops
    /// waiting for is not killed: the plugin skips or finishes it.
    pub fn begin(&self, query: &str, timeout: Duration, scoped: bool) -> Result<Reply> {
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
                init: None,
                scoped,
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

    /// Returns the live transport, starting the process or restarting a dead one within
    /// the restart budget. A request sent while the process starts waits behind `init`.
    fn ensure_running(&self, running: &mut Option<Running>) -> Result<Arc<Shared>> {
        if self.shutdown.load(Ordering::Relaxed) {
            bail!(i18n::t("error.process_stopped"));
        }
        if self.suspended.load(Ordering::Relaxed) {
            bail!(
                self.state()
                    .1
                    .unwrap_or_else(|| i18n::t("error.process_stopped"))
            );
        }
        if let Some(current) = running.as_mut()
            && current.is_alive()
        {
            return Ok(current.shared.clone());
        }
        if let Some(dead) = running.take() {
            dead.kill();
        }
        // The first start is not a restart.
        if self.started_once.swap(true, Ordering::Relaxed) && !self.allow_restart() {
            let reason = i18n::t("plugin.crash_loop");
            self.fail(
                running,
                reason.clone(),
                format!(
                    "crashed more than {RESTART_LIMIT} times in {}s; stopped",
                    RESTART_WINDOW.as_secs()
                ),
            );
            bail!(reason);
        }
        let fresh = match spawn((self.factory)(), self.status.clone()) {
            Ok(fresh) => fresh,
            Err(error) => {
                self.fail(
                    running,
                    format!("{error:#}"),
                    format!("could not start the process: {error:#}"),
                );
                return Err(error);
            }
        };
        let shared = fresh.shared.clone();
        *running = Some(fresh);
        Ok(shared)
    }

    /// Stops the process and keeps it stopped until the user restarts it. `reason` is shown
    /// to the user; `log` says the same for the (English) log.
    fn fail(&self, running: &mut Option<Running>, reason: String, log: String) {
        if let Some(current) = running.take() {
            current.kill();
        }
        self.suspended.store(true, Ordering::Relaxed);
        self.status.log(Level::Error, "plugin.failed", log);
        self.status.set(State::Failed, Some(reason.clone()));
        self.status.board.notify(Notice {
            title: i18n::format("plugin.failed_title", &[("name", &self.name)]),
            message: reason,
        });
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
                init: None,
                scoped: false,
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
                self.status.log(
                    Level::Warn,
                    "plugin.action_timeout",
                    format!(
                        "an action did not finish within {}s; stopping the process",
                        ACTION_TIMEOUT.as_secs()
                    ),
                );
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

    /// One watchdog tick: fails a start that errs, exits or takes too long; restarts a
    /// dead, unresponsive or stuck ready process; sends a heartbeat when one is due.
    /// Lazy plugins that were never started and failed ones are left alone.
    pub(crate) fn check(&self) {
        if self.shutdown.load(Ordering::Relaxed) || self.suspended.load(Ordering::Relaxed) {
            return;
        }
        let mut running = self.running.lock().unwrap();
        let Some(current) = running.as_mut() else {
            // Stopped by `stop`; a lazy plugin that never ran waits for a request.
            if self.started_once.load(Ordering::Relaxed) {
                let _ = self.ensure_running(&mut running);
            }
            return;
        };
        if !current.shared.ready.load(Ordering::Relaxed) {
            if let Some((reason, log)) = self.start_failure(current) {
                self.fail(&mut running, reason, log);
            }
            return;
        }
        if !current.is_alive() {
            let reason = exit_reason(current);
            self.status.log(
                Level::Warn,
                "plugin.crashed",
                format!("{reason}; restarting"),
            );
            // A crash loop marks the plugin failed; other errors are retried next tick.
            let _ = self.ensure_running(&mut running);
            return;
        }
        let shared = running.as_ref().map(|running| running.shared.clone());
        drop(running);
        if let Some(shared) = shared {
            shared.heartbeat();
        }
    }

    /// Why a process that has not answered `init` should be given up, if it should: the
    /// reason shown to the user and the line for the log.
    fn start_failure(&self, current: &mut Running) -> Option<(String, String)> {
        if let Some(error) = current.shared.init_error.lock().unwrap().take() {
            return Some((
                i18n::format("plugin.init_failed", &[("error", &error)]),
                format!("initialization failed: {error}"),
            ));
        }
        if !matches!(current.child.try_wait(), Ok(None))
            || !current.shared.healthy.load(Ordering::Relaxed)
        {
            return Some((
                i18n::t("plugin.exited_starting"),
                format!("exited while starting: {}", exit_reason(current)),
            ));
        }
        let waited = current.shared.started.elapsed();
        let seconds = self.startup_timeout.as_secs();
        (waited > self.startup_timeout).then(|| {
            (
                i18n::format("plugin.start_timeout", &[("seconds", &seconds.to_string())]),
                format!("did not start within {seconds}s; stopped until restarted"),
            )
        })
    }
}

/// Why a process stopped working: its exit code once it has exited, else what made it
/// unhealthy.
fn exit_reason(current: &mut Running) -> String {
    let reason = current.shared.unhealthy.lock().unwrap().clone();
    match current.child.try_wait() {
        Ok(Some(status)) => match status.code() {
            Some(code) => format!("the process exited with code {code}"),
            None => "the process was ended by the system".into(),
        },
        _ => reason.unwrap_or_else(|| "the process stopped responding".into()),
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

fn spawn(mut command: Command, status: Status) -> Result<Running> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW: headless plugin.
    }
    let mut child = command.spawn().context(i18n::t("error.process_start"))?;
    let stdin = child.stdin.take().context(i18n::t("error.stdin"))?;
    let stdout = BufReader::new(child.stdout.take().context(i18n::t("error.stdout"))?);
    // What the plugin prints to stderr (its log records and anything else) goes to the log.
    if let Some(stderr) = child.stderr.take() {
        let source = status.id.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                if !line.trim().is_empty() {
                    log::write_from(&source, Record::from_stderr(&line));
                }
            }
        });
    }
    status.log(
        Level::Info,
        "plugin.start",
        format!("started the process (PID {})", child.id()),
    );
    let shared = Arc::new(Shared {
        stdin: Mutex::new(stdin),
        pending: Mutex::new(VecDeque::new()),
        healthy: AtomicBool::new(true),
        unhealthy: Mutex::new(None),
        ready: AtomicBool::new(false),
        init_error: Mutex::new(None),
        started: Instant::now(),
        heartbeat: Mutex::new(Heartbeat::default()),
        status,
    });
    shared.status.set(State::Starting, None);
    // Nobody waits for the init reply: the reader marks the plugin ready instead.
    let (reply, _) = mpsc::channel();
    shared.pending.lock().unwrap().push_back(Pending {
        reply,
        kind: Kind::Init,
    });
    // A broken pipe shows up as unhealthy; the watchdog reports it.
    let _ = shared.write(&SearchRequest::init());
    let reader = shared.clone();
    std::thread::spawn(move || {
        let error = read_responses(stdout, &reader);
        reader.unhealthy(|| format!("{error:#}"));
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
        if let Some(sequence) = response.pong {
            shared.pong(sequence, response.busy_ms.unwrap_or(0));
            continue;
        }
        if let Some(progress) = response.progress {
            if !shared.ready.load(Ordering::Relaxed) {
                shared.status.set(State::Starting, Some(progress));
            }
            continue;
        }
        response.items.truncate(50);
        // The receiver may be gone when the host stopped waiting; that is fine.
        let next = shared.pending.lock().unwrap().pop_front();
        match next {
            // Any answer without an error counts, so plugins that predate `init` (and
            // answer it like an empty search) still start.
            Some(pending) if pending.kind == Kind::Init => match response.error {
                Some(error) => *shared.init_error.lock().unwrap() = Some(error),
                None => {
                    shared.ready.store(true, Ordering::Relaxed);
                    shared.status.log(
                        Level::Info,
                        "plugin.ready",
                        format!("ready after {} ms", shared.started.elapsed().as_millis()),
                    );
                    shared.status.set(State::Ready, None);
                }
            },
            Some(pending) => {
                let _ = pending.reply.send(Ok(response));
            }
            None => {}
        }
    }
}

impl Drop for SearchProcess {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(all(test, windows))]
impl ProcessOptions {
    /// A fixture reporting to its own board, registered so its state is tracked.
    fn test() -> Self {
        let board = Arc::new(Board::new(None));
        board.register("fixture", State::Idle, None, None);
        Self {
            id: "fixture".into(),
            name: "Fixture".into(),
            startup_timeout: STARTUP_TIMEOUT,
            board,
        }
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

    /// An eager process reporting to a private board.
    pub(crate) fn start_factory(factory: Factory) -> Result<Self> {
        Ok(Self::with_factory(factory, ProcessOptions::test(), true))
    }

    pub(crate) fn fixture() -> Self {
        let process = Self::start_factory(Self::powershell(
            "$count=0; while ($null -ne ($line=[Console]::ReadLine())) { $r=$line|ConvertFrom-Json; if ($r.ping) { continue }; if ($r.init) { '{\"ready\":true}'; continue }; $q=$r.query; if ($q -eq 'slow') { Start-Sleep -Milliseconds 1500 }; $count++; @{items=@(@{title=$q;description=[string]$count;resource=@{type='text';text=$q}})}|ConvertTo-Json -Compress -Depth 5 }",
        ))
        .unwrap();
        process
            .begin("warmup", SEARCH_TIMEOUT, false)
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
            .begin("warmup", SEARCH_TIMEOUT, false)
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
        assert!(
            process
                .begin("changed input", SEARCH_TIMEOUT, false)
                .is_err()
        );
        assert!(action.join().unwrap().unwrap().refresh);
        assert!(
            process
                .begin("completed", SEARCH_TIMEOUT, false)
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
            .begin("after stop", SEARCH_TIMEOUT, false)
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
            .begin("after crash", SEARCH_TIMEOUT, false)
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert_eq!(response.items[0].title, "after crash");
        process.shutdown();
        assert!(
            process
                .begin("after shutdown", SEARCH_TIMEOUT, false)
                .is_err()
        );
    }

    #[test]
    fn abandoned_search_does_not_shift_later_responses() {
        let process = SearchProcess::fixture();
        let slow = process.begin("slow", SEARCH_TIMEOUT, false).unwrap();
        assert!(slow.recv_timeout(Duration::from_millis(100)).is_err());
        drop(slow); // The host stopped waiting; the process must not be killed.
        let next = process
            .begin("next", SEARCH_TIMEOUT, false)
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
            .begin("after restart", SEARCH_TIMEOUT, false)
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

#[cfg(all(test, unix))]
mod startup_tests {
    use super::*;

    /// A plugin written in sh: `init` gets `on_init`, every search echoes `title`.
    fn shell(on_init: &'static str) -> Factory {
        Box::new(move || {
            let mut command = Command::new("sh");
            command.args([
                "-c",
                &format!(
                    r#"while IFS= read -r line; do case "$line" in *'"init"'*) {on_init};; *'"ping"'*) ;; *) echo '{{"items":[{{"title":"hit","resource":{{"type":"text","text":"x"}}}}]}}';; esac; done"#
                ),
            ]);
            command
        })
    }

    fn process(
        on_init: &'static str,
        eager: bool,
        timeout: Duration,
    ) -> (SearchProcess, Arc<Board>) {
        let board = Arc::new(Board::new(None));
        board.register("fixture", State::Idle, None, None);
        let options = ProcessOptions {
            id: "fixture".into(),
            name: "Fixture".into(),
            startup_timeout: timeout,
            board: board.clone(),
        };
        (
            SearchProcess::with_factory(shell(on_init), options, eager),
            board,
        )
    }

    fn wait_for(process: &SearchProcess, state: State) -> Option<String> {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            process.check();
            let (current, detail) = process.state();
            if current == state {
                return detail;
            }
            assert!(
                Instant::now() < until,
                "still {current:?}, wanted {state:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn eager_plugin_reports_progress_then_ready_and_searches() {
        let (process, _) = process(
            r#"echo '{"progress":"warming"}'; sleep 0.3; echo '{"ready":true}'"#,
            true,
            Duration::from_secs(5),
        );
        let until = Instant::now() + Duration::from_secs(5);
        while process.state() != (State::Starting, Some("warming".into())) {
            assert!(Instant::now() < until, "no progress: {:?}", process.state());
            std::thread::sleep(Duration::from_millis(10));
        }
        wait_for(&process, State::Ready);
        let reply = process.begin("q", SEARCH_TIMEOUT, false).unwrap();
        let response = reply.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!(response.items[0].title, "hit");
    }

    #[test]
    fn lazy_plugin_waits_for_activation_and_watchdog_leaves_it_alone() {
        let (process, _) = process(r#"echo '{"ready":true}'"#, false, Duration::from_secs(5));
        process.check();
        assert_eq!(process.state().0, State::Idle);
        assert!(process.running.lock().unwrap().is_none());
        process.activate();
        wait_for(&process, State::Ready);
    }

    #[test]
    fn failed_init_stops_the_plugin_until_restarted() {
        let (process, board) = process(
            r#"echo '{"error":"no database"}'"#,
            true,
            Duration::from_secs(5),
        );
        let reason = wait_for(&process, State::Failed).unwrap();
        assert!(reason.contains("no database"), "{reason}");
        assert_eq!(board.take_notices().len(), 1);
        process.check();
        process.activate();
        assert!(process.running.lock().unwrap().is_none());
        assert!(process.begin("q", SEARCH_TIMEOUT, false).is_err());
        process.restart();
        assert_eq!(process.state().0, State::Starting);
        wait_for(&process, State::Failed);
    }

    #[test]
    fn start_that_times_out_or_exits_fails_with_a_notice() {
        let (slow, board) = process("sleep 5", true, Duration::from_millis(200));
        let reason = wait_for(&slow, State::Failed).unwrap();
        assert_eq!(
            reason,
            i18n::format("plugin.start_timeout", &[("seconds", "0")])
        );
        assert_eq!(board.take_notices()[0].message, reason);
        let (exits, _) = process("exit 3", true, Duration::from_secs(5));
        assert_eq!(
            wait_for(&exits, State::Failed),
            Some(i18n::t("plugin.exited_starting"))
        );
    }

    #[test]
    fn plugin_that_predates_init_is_ready_after_any_answer() {
        let (process, _) = process(r#"echo '{"items":[]}'"#, true, Duration::from_secs(5));
        wait_for(&process, State::Ready);
    }
}
