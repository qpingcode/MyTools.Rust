use mytools_plugin_api::{Icon, Resource, SearchResponse};
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

struct Plugin {
    child: Child,
    input: ChildStdin,
    responses: Receiver<SearchResponse>,
    directory: PathBuf,
}

impl Plugin {
    fn start() -> Self {
        let directory =
            std::env::temp_dir().join(format!("mytools-app-protocol-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("Visual Studio Code.lnk"), b"fixture").unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_mytools-applications"))
            .env("MYTOOLS_APPLICATION_DIRS", &directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (sender, responses) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let response = serde_json::from_str(&line.unwrap()).unwrap();
                if sender.send(response).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            responses,
            directory,
        }
    }

    fn search(&mut self, query: &str) -> SearchResponse {
        writeln!(self.input, "{}", serde_json::json!({ "query": query })).unwrap();
        self.input.flush().unwrap();
        self.responses
            .recv_timeout(Duration::from_secs(2))
            .expect("plugin must stay responsive")
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn persistent_plugin_returns_application_resources_and_filters_removed_shortcuts() {
    let mut plugin = Plugin::start();
    let deadline = Instant::now() + Duration::from_secs(5);
    let result = loop {
        let response = plugin.search("vsc");
        if let Some(result) = response.items.into_iter().next() {
            break result;
        }
        assert!(
            Instant::now() < deadline,
            "background index never became available"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(result.title, "Visual Studio Code");
    assert_eq!(
        result.description,
        mytools_plugin_api::paths::display(
            &plugin
                .directory
                .join("Visual Studio Code.lnk")
                .canonicalize()
                .unwrap()
        )
    );
    #[cfg(windows)]
    assert!(!result.description.starts_with(r"\\?\"));
    assert_eq!(
        result.resource,
        Resource::Application {
            path: plugin
                .directory
                .join("Visual Studio Code.lnk")
                .canonicalize()
                .unwrap(),
        }
    );
    assert!(result.actions.is_none());
    assert_eq!(
        result.icon,
        Some(Icon::Native {
            native: plugin
                .directory
                .join("Visual Studio Code.lnk")
                .canonicalize()
                .unwrap(),
        })
    );
    assert_eq!(plugin.search("code VISUAL").items.len(), 1);
    assert!(plugin.search("not-installed").items.is_empty());
    std::fs::remove_file(plugin.directory.join("Visual Studio Code.lnk")).unwrap();
    assert!(plugin.search("vsc").items.is_empty());
}
