use crate::i18n;
use anyhow::{Context, Result, anyhow, bail};
use mytools_plugin_api::{PluginActionRequest, PluginEffect, SearchRequest, SearchResponse};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::Duration,
};

pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(1);
const ACTION_TIMEOUT: Duration = Duration::from_secs(60);
type Reply = Receiver<Result<SearchResponse>>;

struct Request {
    wire: SearchRequest,
    reply: mpsc::Sender<Result<SearchResponse>>,
}

#[derive(Debug)]
pub struct SearchProcess {
    child: Mutex<Child>,
    requests: SyncSender<Request>,
    stopped: AtomicBool,
    action_busy: Mutex<bool>,
}

impl SearchProcess {
    pub fn start(path: &Path) -> Result<Self> {
        let mut command = Command::new(path);
        command
            .current_dir(path.parent().context(i18n::t("error.plugin_directory"))?)
            .env("MYTOOLS_LOCALE", mytools_plugin_api::i18n::language());
        Self::start_command(command)
    }

    fn start_command(mut command: Command) -> Result<Self> {
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
                    break;
                }
            }
        });
        Ok(Self {
            child: Mutex::new(child),
            requests,
            stopped: AtomicBool::new(false),
            action_busy: Mutex::new(false),
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
        if self.stopped.load(Ordering::Relaxed) {
            bail!(i18n::t("error.process_stopped"));
        }
        let (reply, receiver) = mpsc::channel();
        self.requests
            .try_send(Request { wire, reply })
            .map_err(|err| anyhow!(i18n::format("error.busy", &[("error", &err.to_string())])))?;
        Ok(receiver)
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

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for SearchProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(all(test, windows))]
impl SearchProcess {
    pub(crate) fn fixture() -> Self {
        let mut command = Command::new("powershell.exe");
        command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
            "$count=0; while ($null -ne ($line=[Console]::ReadLine())) { $q=($line|ConvertFrom-Json).query; $count++; @{items=@(@{title=$q;description=[string]$count;resource=@{type='text';text=$q}})}|ConvertTo-Json -Compress -Depth 5 }"]);
        let process = Self::start_command(command).unwrap();
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
        let mut command = Command::new("powershell.exe");
        command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
            "while ($null -ne ($line=[Console]::ReadLine())) { $r=$line|ConvertFrom-Json; if ($r.action) { Start-Sleep -Milliseconds 1200; '{\"items\":[],\"effect\":{\"refresh\":true}}' } else { '{\"items\":[]}' } }"]);
        let process = std::sync::Arc::new(SearchProcess::start_command(command).unwrap());
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
}
