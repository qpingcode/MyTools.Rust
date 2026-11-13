//! How long starting took: when MyTools reached each step of its own start, and how long
//! each plugin started with it took to become ready. Startup ends when the window can be
//! used and no plugin is starting any more. A slow start (over five seconds) is logged as a
//! warning with every step and plugin, and a start still going after five seconds is logged
//! right then with what it waits for; otherwise one info line records the total.
use mytools_core::StartTiming;
use mytools_plugin_api::{log, plugin_status::State};
use std::{
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Starts slower than this are reported.
pub const SLOW: Duration = Duration::from_secs(5);
/// The step after which the window responds.
pub const READY_STEP: &str = "event loop";

static START: OnceLock<Instant> = OnceLock::new();
static STEPS: Mutex<Vec<(&'static str, Instant)>> = Mutex::new(Vec::new());
/// Startup ended and was logged.
static DONE: AtomicBool = AtomicBool::new(false);
/// A slow start was already reported while it went on.
static WARNED: AtomicBool = AtomicBool::new(false);

/// Call first thing in `main`. Should even loading the plugins hang, so that the search
/// worker cannot report, a watchdog logs the steps reached once the start is slow.
pub fn begin() {
    START.get_or_init(Instant::now);
    std::thread::spawn(|| {
        std::thread::sleep(SLOW + Duration::from_millis(500));
        if !DONE.load(Ordering::SeqCst) && !WARNED.swap(true, Ordering::SeqCst) {
            let summary = report(start(), &STEPS.lock().unwrap(), &[], start().elapsed());
            log::warn(
                "startup.waiting",
                format!("still starting, plugins not loaded yet: {summary}"),
            );
        }
    });
}

fn start() -> Instant {
    *START.get_or_init(Instant::now)
}

/// Records that MyTools finished a step of its start, e.g. "window".
pub fn mark(step: &'static str) {
    STEPS.lock().unwrap().push((step, Instant::now()));
}

fn reached(step: &str) -> bool {
    STEPS.lock().unwrap().iter().any(|(name, _)| *name == step)
}

/// Whether startup ended and was logged, so the worker can stop checking.
pub fn done() -> bool {
    DONE.load(Ordering::SeqCst)
}

fn seconds(duration: Duration) -> String {
    if duration < Duration::from_secs(1) {
        format!("{} ms", duration.as_millis())
    } else {
        format!("{:.1} s", duration.as_secs_f64())
    }
}

/// Logs once startup ended, or once it takes longer than [`SLOW`]. Called by the search
/// worker, which owns the plugins, with how each one started so far.
pub fn check(plugins: &[(String, StartTiming, State)]) {
    if done() {
        return;
    }
    let elapsed = start().elapsed();
    let starting = plugins
        .iter()
        .any(|(_, _, state)| *state == State::Starting);
    if reached(READY_STEP) && !starting {
        DONE.store(true, Ordering::SeqCst);
        // It ended with the last step or plugin, not when this check noticed.
        let steps = STEPS.lock().unwrap();
        let end = steps
            .iter()
            .map(|(_, at)| *at)
            .chain(plugins.iter().filter_map(|(_, timing, _)| timing.ready))
            .max()
            .unwrap_or_else(Instant::now);
        let elapsed = end.duration_since(start());
        let summary = report(start(), &steps, plugins, elapsed);
        if elapsed > SLOW {
            log::warn("startup.slow", summary);
        } else {
            log::info("startup.done", summary);
        }
    } else if elapsed > SLOW && !WARNED.swap(true, Ordering::SeqCst) {
        let summary = report(start(), &STEPS.lock().unwrap(), plugins, elapsed);
        log::warn("startup.waiting", format!("still starting: {summary}"));
    }
}

/// One line: the total, MyTools' steps since it started, then each plugin, slowest first.
fn report(
    start: Instant,
    steps: &[(&'static str, Instant)],
    plugins: &[(String, StartTiming, State)],
    elapsed: Duration,
) -> String {
    let steps: Vec<String> = steps
        .iter()
        .map(|(step, at)| format!("{step} at {}", seconds(at.duration_since(start))))
        .collect();
    let mut plugins: Vec<(Duration, String)> = plugins
        .iter()
        .map(|(id, timing, state)| {
            let since = seconds(timing.began.saturating_duration_since(start));
            let spawn = seconds(timing.spawn);
            let (took, outcome) = match (state, timing.ready) {
                (State::Starting, _) => {
                    let waited = timing.began.elapsed();
                    (waited, format!("still starting after {}", seconds(waited)))
                }
                (_, Some(ready)) => {
                    let took = ready.duration_since(timing.began);
                    (took, format!("ready after {}", seconds(took)))
                }
                (State::Failed, None) => (Duration::ZERO, "failed".to_owned()),
                (_, None) => (Duration::ZERO, format!("{state:?}").to_lowercase()),
            };
            (
                took,
                format!("{id} {outcome} (began at {since}, process created in {spawn})"),
            )
        })
        .collect();
    plugins.sort_by_key(|(took, _)| std::cmp::Reverse(*took));
    let plugins: Vec<String> = plugins.into_iter().map(|(_, text)| text).collect();
    format!(
        "startup took {}; mytools: {}; plugins: {}",
        seconds(elapsed),
        if steps.is_empty() {
            "-".into()
        } else {
            steps.join(", ")
        },
        if plugins.is_empty() {
            "-".into()
        } else {
            plugins.join(", ")
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_steps_and_the_slowest_plugin_first() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let timing = |began, ready: Option<u64>| StartTiming {
            began: at(began),
            spawn: Duration::from_millis(12),
            ready: ready.map(at),
        };
        let plugins = vec![
            (
                "calculator".to_owned(),
                timing(100, Some(150)),
                State::Ready,
            ),
            (
                "everything".to_owned(),
                timing(110, Some(5_300)),
                State::Ready,
            ),
            ("broken".to_owned(), timing(120, None), State::Failed),
        ];
        let text = report(
            start,
            &[("window", at(800)), (READY_STEP, at(1_240))],
            &plugins,
            Duration::from_millis(5_400),
        );
        assert_eq!(
            text,
            "startup took 5.4 s; mytools: window at 800 ms, event loop at 1.2 s; plugins: \
everything ready after 5.2 s (began at 110 ms, process created in 12 ms), \
calculator ready after 50 ms (began at 100 ms, process created in 12 ms), \
broken failed (began at 120 ms, process created in 12 ms)"
        );
    }
}
