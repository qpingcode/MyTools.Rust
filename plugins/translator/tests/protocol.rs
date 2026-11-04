use mytools_plugin_api::{Resource, SearchResponse};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

struct Fixture {
    child: Child,
    directory: std::path::PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn fixture_cache(directory: &std::path::Path) -> std::path::PathBuf {
    directory.join("cache.json")
}

#[test]
fn action_waits_for_translation_then_one_search_reads_the_completed_result() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (ready, received) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let server = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        let mut length = 0;
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["messages"][1]["content"], "Hello");
        ready.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(3)).unwrap();
        let body =
            serde_json::json!({"choices":[{"finish_reason":"stop","message":{"content":"你好"}}]})
                .to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    let directory = std::env::temp_dir().join(format!(
        "mytools-translation-protocol-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let config = directory.join("config.json");
    std::fs::write(
        &config,
        serde_json::json!({"base_url":format!("http://{address}"),"model":"test"}).to_string(),
    )
    .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_mytools-translator"))
        .env("MYTOOLS_TRANSLATOR_CONFIG", config)
        // Never touch the real per-user cache.
        .env("MYTOOLS_TRANSLATOR_CACHE", fixture_cache(&directory))
        .env("DEEPSEEK_API_KEY", "test-key")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut fixture = Fixture { child, directory };
    let mut input = fixture.child.stdin.take().unwrap();
    let output = fixture.child.stdout.take().unwrap();
    let (sender, responses) = mpsc::channel();
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
    writeln!(input, "{}", serde_json::json!({"query":"Hello"})).unwrap();
    input.flush().unwrap();
    let first = responses.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(received.try_recv().is_err());
    let Resource::PluginItem { id } = &first.items[0].resource else {
        panic!()
    };
    writeln!(
        input,
        "{}",
        serde_json::json!({"action":{"command":"translate","item":id}})
    )
    .unwrap();
    input.flush().unwrap();
    received.recv_timeout(Duration::from_secs(3)).unwrap();
    // No early acknowledgment and no polling while the API is still running.
    assert!(matches!(
        responses.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    release.send(()).unwrap();
    server.join().unwrap();
    let completed = responses.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(completed.error.is_none(), "{:?}", completed.error);
    assert!(completed.effect.unwrap().refresh);
    let mut request = |value: serde_json::Value| {
        writeln!(input, "{value}").unwrap();
        input.flush().unwrap();
        responses.recv_timeout(Duration::from_secs(1)).unwrap()
    };
    let response = request(serde_json::json!({"query":"Hello"}));
    assert!(matches!(&response.items[0].resource, Resource::Text { text } if text == "你好"));
    assert_ne!(response.revision, first.revision);
    // Server has closed: repeated searches must still use the cached translation.
    for _ in 0..3 {
        let result = request(serde_json::json!({"query":"Hello"}));
        assert!(matches!(&result.items[0].resource, Resource::Text { text } if text == "你好"));
    }
    assert!(fixture_cache(&fixture.directory).is_file());
}
