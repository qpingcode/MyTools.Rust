use crate::i18n;
use anyhow::{Context, Result, anyhow, bail};
use mytools_plugin_api::{PluginActionRequest, PluginEffect, SearchRequest, SearchResponse};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(1);
const ACTION_TIMEOUT: Duration = Duration::from_secs(60);
/// A plugin that crashes repeatedly is restarted at most this often per window.
const RESTART_LIMIT: usize = 5;
const RESTART_WINDOW: Duration = Duration::from_secs(60);
/// Idle plugins are probed so a hung process is replaced before the user needs it.
const HEARTBEAT_IDLE: Duration = Duration::from_secs(45);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);
type Reply = Receiver<Result<SearchResponse>>;
type Factory = Box<dyn Fn() -> Command + Send + Sync>;

struct Request {
    wire: SearchRequest,
    reply: mpsc::Sender<Result<SearchResponse>>,
}

/// One spawned plugin process and its ordered request transport.
struct Running {
    child: Child,
    requests: SyncSender<Request>,
    /// Cleared by the transport thread when the pipe breaks or a response is invalid.
    healthy: Arc<AtomicBool>,
}

impl Running {
    fn is_alive(&mut self) -> bool {
        self.healthy.load(Ordering::Relaxed) && matches!(self.child.try_wait(), Ok(None))
    }

    fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A resident search plugin. It restarts itself when the process exits, stops responding
/// or returns an invalid response, so references held by actions stay valid.
pub struct SearchProcess {
    factory: Factory,
    running: Mutex<Option<Running>>,
    shutdown: AtomicBool,
    action_busy: Mutex<bool>,
    restarts: Mutex<VecDeque<Instant>>,
    last_activity: Mutex<Instant>,
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
                .env("MYTOOLS_LOCALE", mytools_plugin_api::i18n::language());
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
            last_activity: Mutex::new(Instant::now()),
        })
    }

    pub fn begin(&self, query: &str) -> Result<Reply> {
        // Do not queue a 1-second search behind a long-running action on the same
        // ordered transport. The action's completion may request a fresh search.
        let busy = self.action_busy.lock().unwrap();
        if *busy {
            bail!(i18n::t("error.action_in_progress"));
        }
        self.request(SearchRequest {
            query: query.into(),
            action: None,
        })
    }

    fn request(&self, wire: SearchRequest) -> Result<Reply> {
        let mut running = self.running.lock().unwrap();
        let requests = self.ensure_running(&mut running)?;
        *self.last_activity.lock().unwrap() = Instant::now();
        let (reply, receiver) = mpsc::channel();
        requests
            .try_send(Request { wire, reply })
            .map_err(|err| anyhow!(i18n::format("error.busy", &[("error", &err.to_string())])))?;
        Ok(receiver)
    }

    /// Returns the live transport, restarting a dead process within the restart budget.
    fn ensure_running(&self, running: &mut Option<Running>) -> Result<SyncSender<Request>> {
        if self.shutdown.load(Ordering::Relaxed) {
            bail!(i18n::t("error.process_stopped"));
        }
        if let Some(current) = running.as_mut()
            && current.is_alive()
        {
            return Ok(current.requests.clone());
        }
        if let Some(dead) = running.take() {
            dead.kill();
        }
        if !self.allow_restart() {
            bail!(i18n::t("error.process_stopped"));
        }
        let fresh = spawn((self.factory)())?;
        let requests = fresh.requests.clone();
        *running = Some(fresh);
        Ok(requests)
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

    pub fn execute(&self, command: &str, item: &str) -> Result<PluginEffect> {
        let mut busy = self.action_busy.lock().unwrap();
        if *busy {
            bail!(i18n::t("error.action_in_progress"));
        }
        *busy = true;
        let reply = self.request(SearchRequest {
            query: String::new(),
            action: Some(PluginActionRequest {
                command: command.into(),
                item: item.into(),
            }),
        });
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

    /// Restarts a crashed process, and starts a heartbeat probe if it has been idle.
    /// Returns a pending probe; the caller waits for it with `finish_probe`.
    pub(crate) fn check(&self) -> Option<Reply> {
        if self.shutdown.load(Ordering::Relaxed) || *self.action_busy.lock().unwrap() {
            return None;
        }
        {
            let mut running = self.running.lock().unwrap();
            let alive = running.as_mut().is_some_and(Running::is_alive);
            if !alive {
                // Errors (restart budget exhausted, spawn failure) are retried next tick.
                let _ = self.ensure_running(&mut running);
                return None;
            }
        }
        if self.last_activity.lock().unwrap().elapsed() < HEARTBEAT_IDLE {
            return None;
        }
        self.request(SearchRequest {
            query: String::new(),
            action: None,
        })
        .ok()
    }

    pub(crate) fn finish_probe(&self, probe: Reply, deadline: Instant) {
        let answered = probe
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .is_ok_and(|response| response.is_ok());
        if !answered && !self.shutdown.load(Ordering::Relaxed) {
            self.stop();
            let mut running = self.running.lock().unwrap();
            let _ = self.ensure_running(&mut running);
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
            std::thread::sleep(Duration::from_secs(3));
            let live: Vec<_> = processes.iter().filter_map(|weak| weak.upgrade()).collect();
            if live.is_empty()
                || live
                    .iter()
                    .all(|process| process.shutdown.load(Ordering::Relaxed))
            {
                break;
            }
            let probes: Vec<_> = live
                .iter()
                .filter_map(|process| process.check().map(|probe| (process, probe)))
                .collect();
            let deadline = Instant::now() + HEARTBEAT_TIMEOUT;
            for (process, probe) in probes {
                process.finish_probe(probe, deadline);
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
    let mut stdin = child.stdin.take().context(i18n::t("error.stdin"))?;
    let mut stdout = BufReader::new(child.stdout.take().context(i18n::t("error.stdout"))?);
    let (requests, receiver) = mpsc::sync_channel::<Request>(1);
    let healthy = Arc::new(AtomicBool::new(true));
    let transport_health = healthy.clone();
    std::thread::spawn(move || {
        for request in receiver {
            let response = (|| -> Result<SearchResponse> {
                serde_json::to_writer(&mut stdin, &request.wire)?;
                writeln!(stdin)?;
                stdin.flush()?;
                let mut line = Vec::new();
                let read = Read::take(&mut stdout, 1_048_577).read_until(b'\n', &mut line)?;
                if read == 0 {
                    bail!(i18n::t("error.process_exited"));
                }
                if read > 1_048_576 || !line.ends_with(b"\n") {
                    bail!(i18n::t("error.response_limit"));
                }
                let mut response: SearchResponse =
                    serde_json::from_slice(&line).context(i18n::t("error.response_invalid"))?;
                response.items.truncate(50);
                Ok(response)
            })();
            let failed = response.is_err();
            let _ = request.reply.send(response);
            if failed {
                transport_health.store(false, Ordering::Relaxed);
                break;
            }
        }
    });
    Ok(Running {
        child,
        requests,
        healthy,
    })
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
            "$count=0; while ($null -ne ($line=[Console]::ReadLine())) { $q=($line|ConvertFrom-Json).query; $count++; @{items=@(@{title=$q;description=[string]$count;resource=@{type='text';text=$q}})}|ConvertTo-Json -Compress -Depth 5 }",
        ))
        .unwrap();
        process
            .begin("warmup")
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
            .begin("warmup")
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let worker = process.clone();
        let action = std::thread::spawn(move || worker.execute("translate", "test"));
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !*process.action_busy.lock().unwrap() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(process.begin("changed input").is_err());
        assert!(action.join().unwrap().unwrap().refresh);
        assert!(
            process
                .begin("completed")
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
            .begin("after stop")
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
        assert!(process.check().is_none());
        let response = process
            .begin("after crash")
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert_eq!(response.items[0].title, "after crash");
        process.shutdown();
        assert!(process.begin("after shutdown").is_err());
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
