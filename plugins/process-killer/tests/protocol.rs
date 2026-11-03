use mytools_plugin_api::{Resource, SearchResponse};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn queries_real_ports_and_kills_only_the_selected_fixture_process() {
    let binary = env!("CARGO_BIN_EXE_mytools-process-killer");
    // A second plugin waiting on stdin is a disposable process owned by this test.
    let mut victim = ChildGuard(
        Command::new(binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut plugin = ChildGuard(
        Command::new(binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut input = plugin.0.stdin.take().unwrap();
    let output = plugin.0.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            if sender
                .send(serde_json::from_str::<SearchResponse>(&line.unwrap()).unwrap())
                .is_err()
            {
                break;
            }
        }
    });
    let mut request = |value: serde_json::Value| {
        writeln!(input, "{value}").unwrap();
        input.flush().unwrap();
        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("must fit the host's search timeout")
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let result = request(serde_json::json!({"query":format!(":{port}")}));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(result.items.iter().any(|item| matches!(&item.resource, Resource::PluginItem { id } if id.starts_with(&format!("{}:", std::process::id())))));
    let results = request(serde_json::json!({"query":"mytools-process-killer"}));
    let id = results
        .items
        .iter()
        .find_map(|item| match &item.resource {
            Resource::PluginItem { id } if id.starts_with(&format!("{}:", victim.0.id())) => {
                Some(id.clone())
            }
            _ => None,
        })
        .expect("victim must be found by process name");
    let forged = request(serde_json::json!({"action":{"command":"kill","item":format!("{id}0")}}));
    assert!(forged.error.is_some());
    assert!(victim.0.try_wait().unwrap().is_none());
    let killed = request(serde_json::json!({"action":{"command":"kill","item":id}}));
    assert!(killed.error.is_none(), "{:?}", killed.error);
    assert!(killed.effect.unwrap().refresh);
    let deadline = Instant::now() + Duration::from_secs(3);
    while victim.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
}
