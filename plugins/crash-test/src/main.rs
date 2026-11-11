//! A plugin that misbehaves on purpose, to exercise how MyTools handles failing plugins.
//! Every fault is set in `config.json` next to the executable (see `config.example.json`;
//! a missing or invalid file means the defaults):
//!
//! | Key | Default | Fault |
//! | --- | --- | --- |
//! | `enabled` | `true` | `false` turns every fault off |
//! | `startupCrash` | `true` | the first start ever crashes during init; the marker file `startup-crashed` beside the executable makes it happen once, delete it to repeat |
//! | `startupDelayMs` | `0` | init takes this long, reporting progress each second (past the host's startup timeout the plugin is paused) |
//! | `startupError` | `""` | non-empty: init fails with this reason |
//! | `runtimeCrash` | `true` | once ready, the process crashes every `crashIntervalSecs` |
//! | `crashIntervalSecs` | `30` | |
//! | `pongDelayMs` | `0` | heartbeat answers are held back this long (over 3 s the host restarts the plugin) |
//! | `pongDelayIntervalSecs` | `0` | at most one held-back answer per this many seconds; `0` holds back every one |
//! | `searchDelayMs` | `0` | every search takes this long (over 20 s the host restarts the plugin) |
//!
//! `MYTOOLS_CRASH_TEST=off` turns every fault off whatever the file says. Crashes are
//! panics, so they also show up in `logs/crash.log`.
//!
//! To control heartbeats, the plugin speaks the line protocol itself instead of using
//! `serve`: pings are answered on the reader thread, everything else in order on the main
//! thread.
use mytools_plugin_api::{SearchItem, SearchRequest, SearchResponse, i18n::Catalog, text};
use serde::Deserialize;
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
struct Config {
    enabled: bool,
    startup_crash: bool,
    startup_delay_ms: u64,
    startup_error: String,
    runtime_crash: bool,
    crash_interval_secs: u64,
    pong_delay_ms: u64,
    pong_delay_interval_secs: u64,
    search_delay_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            startup_crash: true,
            startup_delay_ms: 0,
            startup_error: String::new(),
            runtime_crash: true,
            crash_interval_secs: 30,
            pong_delay_ms: 0,
            pong_delay_interval_secs: 0,
            search_delay_ms: 0,
        }
    }
}

impl Config {
    fn load() -> Self {
        let config = std::fs::read(beside_exe("config.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self::with_switch(config, std::env::var("MYTOOLS_CRASH_TEST").ok().as_deref())
    }

    /// Applies the `MYTOOLS_CRASH_TEST` switch and `enabled`, so later code only looks at
    /// the individual faults.
    fn with_switch(mut config: Self, switch: Option<&str>) -> Self {
        let off = switch.is_some_and(|value| {
            matches!(value.trim().to_lowercase().as_str(), "off" | "0" | "false")
        });
        if off || !config.enabled {
            config = Self {
                enabled: false,
                startup_crash: false,
                runtime_crash: false,
                crash_interval_secs: config.crash_interval_secs,
                ..Self::quiet()
            };
        }
        config.crash_interval_secs = config.crash_interval_secs.max(1);
        config
    }

    /// No faults at all.
    fn quiet() -> Self {
        Self {
            enabled: false,
            startup_crash: false,
            runtime_crash: false,
            ..Self::default()
        }
    }
}

fn beside_exe(name: &str) -> PathBuf {
    std::env::current_exe()
        .unwrap_or_default()
        .with_file_name(name)
}

/// Panics (so the crash is logged) and ends the process, even off the main thread.
fn crash(reason: &str) -> ! {
    let reason = reason.to_owned();
    let _ = std::panic::catch_unwind(move || panic!("{reason}"));
    std::process::exit(101);
}

type Output = Arc<Mutex<io::Stdout>>;

fn write(output: &Output, response: &SearchResponse) {
    let mut output = output.lock().unwrap_or_else(|error| error.into_inner());
    let written = serde_json::to_writer(&mut *output, response)
        .map_err(io::Error::other)
        .and_then(|()| writeln!(output))
        .and_then(|()| output.flush());
    if written.is_err() {
        std::process::exit(0); // The host is gone.
    }
}

/// Decides which heartbeat answers are held back.
struct Heartbeat {
    delay: Duration,
    interval: Duration,
    last_delayed: Option<Instant>,
    started: Instant,
}

impl Heartbeat {
    fn new(config: &Config) -> Self {
        Self {
            delay: Duration::from_millis(config.pong_delay_ms),
            interval: Duration::from_secs(config.pong_delay_interval_secs),
            last_delayed: None,
            started: Instant::now(),
        }
    }

    /// How long to hold back the answer to a ping arriving now.
    fn hold(&mut self, now: Instant) -> Duration {
        if self.delay.is_zero() {
            return Duration::ZERO;
        }
        let since = now.duration_since(self.last_delayed.unwrap_or(self.started));
        if self.interval.is_zero() || since >= self.interval {
            self.last_delayed = Some(now);
            self.delay
        } else {
            Duration::ZERO
        }
    }
}

struct Plugin {
    config: Config,
    catalog: Catalog,
    output: Output,
    /// When the runtime crash is due, once ready.
    crash_at: Option<Instant>,
}

impl Plugin {
    fn initialize(&mut self) -> SearchResponse {
        let config = &self.config;
        if config.startup_crash {
            let marker = beside_exe("startup-crashed");
            if !marker.exists() {
                let _ = std::fs::write(
                    &marker,
                    "The crash test plugin crashed on its first start.\n",
                );
                crash(self.catalog.get("startup_crash"));
            }
        }
        let total = Duration::from_millis(config.startup_delay_ms);
        let started = Instant::now();
        while started.elapsed() < total {
            let progress = self.catalog.format(
                "slow_start",
                &[
                    ("elapsed", &started.elapsed().as_secs().to_string()),
                    ("total", &total.as_secs_f32().ceil().to_string()),
                ],
            );
            write(
                &self.output,
                &SearchResponse {
                    progress: Some(progress),
                    ..Default::default()
                },
            );
            std::thread::sleep(
                total
                    .saturating_sub(started.elapsed())
                    .min(Duration::from_secs(1)),
            );
        }
        if !config.startup_error.is_empty() {
            return SearchResponse {
                error: Some(config.startup_error.clone()),
                ..Default::default()
            };
        }
        if config.runtime_crash {
            let every = Duration::from_secs(config.crash_interval_secs);
            let reason = self.catalog.get("runtime_crash").to_owned();
            std::thread::spawn(move || {
                std::thread::sleep(every);
                crash(&reason);
            });
            self.crash_at = Some(Instant::now() + every);
        }
        SearchResponse {
            ready: Some(true),
            ..Default::default()
        }
    }

    fn search(&self, request: &SearchRequest) -> SearchResponse {
        std::thread::sleep(Duration::from_millis(self.config.search_delay_ms));
        SearchResponse {
            items: search(
                &request.query,
                request.scoped,
                &self.catalog,
                self.config.enabled,
                self.crash_at,
            ),
            ..Default::default()
        }
    }
}

/// Answers anything typed after its alias (`scoped`); in global search only queries that
/// name it, so other searches are not cluttered.
fn search(
    query: &str,
    scoped: bool,
    catalog: &Catalog,
    enabled: bool,
    crash_at: Option<Instant>,
) -> Vec<SearchItem> {
    let query = query.trim().to_lowercase();
    let names_it =
        query.is_empty() || "crash".contains(&query) || catalog.get("name").contains(&query);
    if !scoped && !names_it {
        return Vec::new();
    }
    if !enabled {
        return vec![text::hint(catalog.get("off"), catalog.get("off_detail"))];
    }
    let pid = std::process::id().to_string();
    let detail = match crash_at {
        Some(at) => {
            let left = at.saturating_duration_since(Instant::now()).as_secs();
            catalog.format("next", &[("seconds", &left.to_string()), ("pid", &pid)])
        }
        None => catalog.format("running", &[("pid", &pid)]),
    };
    let mut items = vec![text::hint(catalog.get("status"), detail)];
    if scoped && !query.is_empty() {
        items.extend(test_results(&query, catalog));
    }
    items
}

/// Sample results for a query typed after the alias: a number from 1 to 50 asks for that
/// many, anything else gets three that echo it. Enter copies each one's text.
fn test_results(query: &str, catalog: &Catalog) -> Vec<SearchItem> {
    let count = query
        .parse::<usize>()
        .ok()
        .filter(|count| (1..=50).contains(count))
        .unwrap_or(3);
    let total = count.to_string();
    (1..=count)
        .map(|index| {
            let index = index.to_string();
            let title = catalog.format("result", &[("index", &index), ("count", &total)]);
            let copied = format!("{title} · {query}");
            text::item(
                title,
                copied,
                catalog.format("result_detail", &[("query", query)]),
            )
        })
        .collect()
}

fn main() {
    mytools_plugin_api::crash::install();
    let catalog = Catalog::for_language(
        include_str!("../locales/en.json"),
        include_str!("../locales/zh-CN.json"),
    )
    .unwrap();
    let config = Config::load();
    let output: Output = Arc::new(Mutex::new(io::stdout()));
    let busy_since: Arc<Mutex<Option<Instant>>> = Arc::default();
    let (requests, received) = mpsc::channel::<SearchRequest>();

    let (pong_output, pong_busy) = (output.clone(), busy_since.clone());
    let mut heartbeat = Heartbeat::new(&config);
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let Ok(request) = serde_json::from_str::<SearchRequest>(&line) else {
                continue;
            };
            let Some(sequence) = request.ping else {
                if requests.send(request).is_err() {
                    break;
                }
                continue;
            };
            let busy_ms = pong_busy
                .lock()
                .unwrap()
                .map_or(0, |since| since.elapsed().as_millis() as u64);
            let pong = SearchResponse {
                pong: Some(sequence),
                busy_ms: Some(busy_ms),
                ..Default::default()
            };
            let hold = heartbeat.hold(Instant::now());
            let output = pong_output.clone();
            if hold.is_zero() {
                write(&output, &pong);
            } else {
                std::thread::spawn(move || {
                    std::thread::sleep(hold);
                    write(&output, &pong);
                });
            }
        }
        // stdin closed: MyTools has stopped the plugin.
        std::process::exit(0);
    });

    let mut plugin = Plugin {
        config,
        catalog,
        output,
        crash_at: None,
    };
    for request in received {
        *busy_since.lock().unwrap() = Some(Instant::now());
        let response = if request.init.is_some() {
            plugin.initialize()
        } else if request.action.is_some() {
            SearchResponse::default()
        } else {
            plugin.search(&request)
        };
        *busy_since.lock().unwrap() = None;
        write(&plugin.output, &response);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_only_its_own_queries() {
        let catalog = Catalog::from_json(include_str!("../locales/en.json"), "{}").unwrap();
        let later = Some(Instant::now() + Duration::from_secs(30));
        assert_eq!(search("", false, &catalog, true, later).len(), 1);
        assert_eq!(search("cra", false, &catalog, true, later).len(), 1);
        assert!(search("1+1", false, &catalog, true, later).is_empty());
        // After its alias, `crash 5` reaches the plugin as "5": the status and five results.
        assert_eq!(search("5", true, &catalog, true, later).len(), 6);
        let echoed = search("hello", true, &catalog, true, later);
        assert_eq!(echoed.len(), 4);
        assert_eq!(echoed[1].title, "Test result 1/3");
        assert_eq!(search("99", true, &catalog, true, later).len(), 4);
        assert_eq!(search("", true, &catalog, true, later).len(), 1);
        assert!(
            search("crash", false, &catalog, true, None)[0]
                .description
                .contains("no crash")
        );
        assert_eq!(
            search("crash", false, &catalog, false, None)[0].title,
            "Crash simulation is off"
        );
    }

    #[test]
    fn config_file_and_switch_choose_the_faults() {
        let example: Config = serde_json::from_str(include_str!("../config.example.json")).unwrap();
        assert_eq!(example, Config::default());
        let partial: Config =
            serde_json::from_str(r#"{"runtimeCrash":false,"pongDelayMs":5000}"#).unwrap();
        assert!(partial.startup_crash && !partial.runtime_crash);
        assert_eq!(partial.pong_delay_ms, 5000);
        let switched = Config::with_switch(partial, Some("OFF"));
        assert!(!switched.startup_crash && !switched.runtime_crash);
        assert_eq!(switched.pong_delay_ms, 0);
        let disabled: Config =
            serde_json::from_str(r#"{"enabled":false,"searchDelayMs":9}"#).unwrap();
        assert_eq!(Config::with_switch(disabled, None).search_delay_ms, 0);
        assert_eq!(
            Config::with_switch(Config::default(), Some("on")),
            Config::default()
        );
    }

    #[test]
    fn heartbeat_holds_back_at_most_one_answer_per_interval() {
        let config = Config {
            pong_delay_ms: 4000,
            pong_delay_interval_secs: 60,
            ..Config::default()
        };
        let mut heartbeat = Heartbeat::new(&config);
        let start = heartbeat.started;
        assert!(heartbeat.hold(start + Duration::from_secs(10)).is_zero());
        assert_eq!(
            heartbeat.hold(start + Duration::from_secs(60)),
            Duration::from_secs(4)
        );
        assert!(heartbeat.hold(start + Duration::from_secs(70)).is_zero());
        assert_eq!(
            heartbeat.hold(start + Duration::from_secs(120)),
            Duration::from_secs(4)
        );
        let mut every = Heartbeat::new(&Config {
            pong_delay_ms: 10,
            ..Config::default()
        });
        assert_eq!(every.hold(Instant::now()), Duration::from_millis(10));
        assert_eq!(every.hold(Instant::now()), Duration::from_millis(10));
        assert!(
            Heartbeat::new(&Config::default())
                .hold(Instant::now())
                .is_zero()
        );
    }
}
