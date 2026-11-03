use mytools_plugin_api::{ActionId, SearchResponse};
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct Fixture {
    child: Child,
    input: ChildStdin,
    responses: Receiver<SearchResponse>,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        Self::start(false)
    }
    fn start(monitor: bool) -> Self {
        let root =
            std::env::temp_dir().join(format!("mytools-clip-protocol-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let id = "a".repeat(64);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        std::fs::write(root.join("history.json"), serde_json::to_vec(&serde_json::json!([
            {"id":id,"timestamp":timestamp,"bytes":26,"content":{"kind":"text","text":"Clipboard protocol fixture","html":null}}
        ])).unwrap()).unwrap();
        std::fs::write(root.join(format!("{id}.txt")), "Clipboard protocol fixture").unwrap();
        if monitor {
            let image = image::RgbaImage::from_pixel(32, 24, image::Rgba([40, 120, 220, 255]));
            image
                .save(root.join(format!("{}.png", "b".repeat(64))))
                .unwrap();
            image
                .save(root.join(format!("{}.preview.png", "b".repeat(64))))
                .unwrap();
            let file = root.join("clipboard-roundtrip.txt");
            std::fs::write(&file, "file clipboard fixture").unwrap();
            let mut entries: Vec<serde_json::Value> =
                serde_json::from_slice(&std::fs::read(root.join("history.json")).unwrap()).unwrap();
            entries.push(serde_json::json!({"id":"b".repeat(64),"timestamp":timestamp,"bytes":3072,"content":{"kind":"image","width":32,"height":24}}));
            entries.push(serde_json::json!({"id":"c".repeat(64),"timestamp":timestamp,"bytes":80,"content":{"kind":"files","paths":[file]}}));
            std::fs::write(
                root.join("history.json"),
                serde_json::to_vec(&entries).unwrap(),
            )
            .unwrap();
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_mytools-clipboard"));
        command.env("MYTOOLS_CLIPBOARD_DIR", &root);
        if !monitor {
            command.env("MYTOOLS_CLIPBOARD_NO_MONITOR", "1");
        }
        let mut child = command
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
            root,
        }
    }
    fn request(&mut self, value: serde_json::Value) -> SearchResponse {
        writeln!(self.input, "{value}").unwrap();
        self.input.flush().unwrap();
        self.responses.recv_timeout(Duration::from_secs(3)).unwrap()
    }
}

#[test]
#[ignore = "uses the system clipboard; run explicitly with a temporary clipboard fixture"]
fn native_clipboard_round_trips_text_images_and_files() {
    struct Restore {
        text: String,
        html: Option<String>,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                if let Some(html) = &self.html {
                    let _ = clipboard.set_html(html, Some(&self.text));
                } else {
                    let _ = clipboard.set_text(&self.text);
                }
            }
        }
    }
    let mut clipboard = arboard::Clipboard::new().unwrap();
    let _restore = Restore {
        text: clipboard
            .get_text()
            .expect("run only when clipboard holds a disposable text fixture"),
        html: clipboard.get().html().ok(),
    };
    let mut fixture = Fixture::start(true);
    std::thread::sleep(Duration::from_millis(200));
    for seed in ['a', 'b', 'c'] {
        let effect = fixture.request(
            serde_json::json!({"action":{"command":"copy","item":seed.to_string().repeat(64)}}),
        );
        assert!(effect.error.is_none(), "{:?}", effect.error);
        assert!(!effect.effect.unwrap().paste);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let response = fixture.request(serde_json::json!({"query":""}));
            let captured = response.items.iter().any(|item| {
                let mytools_plugin_api::Resource::PluginItem { id } = &item.resource else {
                    return false;
                };
                if ['a', 'b', 'c']
                    .iter()
                    .any(|seed| id == &seed.to_string().repeat(64))
                {
                    return false;
                }
                let Some(preview) = &item.preview else {
                    return false;
                };
                match seed {
                    'a' => preview.text == "Clipboard protocol fixture",
                    'b' => {
                        preview.image.as_ref().is_some_and(|image| image.exists())
                            && item.title.contains("32")
                    }
                    'c' => preview.text.contains("clipboard-roundtrip.txt"),
                    _ => false,
                }
            });
            if captured {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "listener did not persist content kind {seed}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn searches_previews_and_executes_commands_in_one_resident_session() {
    let mut fixture = Fixture::new();
    let response = fixture.request(serde_json::json!({"query":"protocol"}));
    assert!(response.error.is_none());
    assert_eq!(response.items.len(), 1);
    assert_eq!(response.items[0].title, "Clipboard protocol fixture");
    let preview = response.items[0].preview.as_ref().unwrap();
    assert_eq!(preview.text, "Clipboard protocol fixture");
    assert!(preview.text_file.as_ref().unwrap().is_absolute());
    let actions = response.items[0].actions.as_ref().unwrap();
    assert_eq!(actions.len(), 5);
    assert!(actions.iter().all(|action| action.id == ActionId::Plugin));
    assert_eq!(actions[0].command.as_deref(), Some("paste"));
    let invalid =
        fixture.request(serde_json::json!({"action":{"command":"unknown","item":"a".repeat(64)}}));
    assert!(invalid.error.is_some());
    let deleted =
        fixture.request(serde_json::json!({"action":{"command":"delete","item":"a".repeat(64)}}));
    assert!(deleted.effect.unwrap().refresh);
    let empty = fixture.request(serde_json::json!({"query":""}));
    assert_ne!(empty.revision, response.revision);
    assert!(empty.items[0].actions.as_ref().unwrap().is_empty());
    assert!(
        !fixture
            .root
            .join(format!("{}.txt", "a".repeat(64)))
            .exists()
    );
    assert!(
        fixture
            .request(serde_json::json!({"action":{"command":"clear","item":""}}))
            .effect
            .unwrap()
            .refresh
    );
}
