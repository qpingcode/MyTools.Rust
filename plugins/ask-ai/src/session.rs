//! The current conversation: questions go to the model on a background thread, and each
//! change of the answer is pushed to the host while it is written.
use mytools_plugin_api::chat::{Chat, ChatMessage, ChatUpdate, Role};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// How often a growing answer is sent to the host at most.
const PUSH_INTERVAL: Duration = Duration::from_millis(50);

/// How an answer ended without an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    Finished,
    /// It reached the token limit.
    Length,
    Cancelled,
}

/// Writes answers; the real one asks DeepSeek, tests use a script.
pub trait Model: Send + Sync {
    /// Answers the last user message of `history`, calling `on_text` with each new piece
    /// (an empty one while the model only thinks); `false` from it stops the answer. An
    /// error is the text to show in its place.
    fn answer(
        &self,
        history: &[ChatMessage],
        on_text: &mut dyn FnMut(&str) -> bool,
    ) -> Result<Ending, String>;
}

/// Texts added to an answer, in the user's language.
pub struct Notes {
    pub length: String,
    pub stopped: String,
}

type Push = Arc<dyn Fn(ChatUpdate) + Send + Sync>;

pub struct Session {
    state: Arc<Mutex<State>>,
    model: Arc<dyn Model>,
    push: Push,
    notes: Arc<Notes>,
}

#[derive(Default)]
struct State {
    chat: Chat,
    conversations: u64,
    /// Set to stop the answer being written.
    cancel: Option<Arc<AtomicBool>>,
}

impl Session {
    pub fn new(model: Arc<dyn Model>, push: Push, notes: Notes) -> Self {
        let session = Self {
            state: Default::default(),
            model,
            push,
            notes: Arc::new(notes),
        };
        session.start("");
        session
    }

    pub fn chat(&self) -> Chat {
        self.state.lock().unwrap().chat.clone()
    }

    /// Starts a new conversation, asking `text` first unless it is empty.
    pub fn start(&self, text: &str) {
        self.stop();
        {
            let mut state = self.state.lock().unwrap();
            state.conversations += 1;
            state.chat = Chat {
                id: format!("{}-{}", std::process::id(), state.conversations),
                messages: Vec::new(),
            };
        }
        if !text.trim().is_empty() {
            let _ = self.send(text);
        }
    }

    /// Asks `text` in the current conversation; `Err` while an answer is being written.
    pub fn send(&self, text: &str) -> Result<(), ()> {
        let text = text.trim();
        let mut state = self.state.lock().unwrap();
        if text.is_empty() || state.chat.busy() {
            return Err(());
        }
        state.chat.messages.push(ChatMessage {
            role: Role::User,
            text: text.into(),
            ..Default::default()
        });
        let history = state.chat.messages.clone();
        state.chat.messages.push(ChatMessage {
            role: Role::Assistant,
            streaming: true,
            ..Default::default()
        });
        let cancel = Arc::new(AtomicBool::new(false));
        state.cancel = Some(cancel.clone());
        let answer = Answer {
            state: self.state.clone(),
            push: self.push.clone(),
            notes: self.notes.clone(),
            chat: state.chat.id.clone(),
            index: state.chat.messages.len() - 1,
            cancel,
        };
        let model = self.model.clone();
        std::thread::spawn(move || answer.write(&*model, &history));
        Ok(())
    }

    /// Stops the answer being written, keeping what it has so far.
    pub fn stop(&self) {
        let mut state = self.state.lock().unwrap();
        let Some(cancel) = state.cancel.take() else {
            return;
        };
        cancel.store(true, Ordering::SeqCst);
        let chat = state.chat.id.clone();
        let Some((index, message)) = state
            .chat
            .messages
            .iter_mut()
            .enumerate()
            .find(|(_, message)| message.streaming)
        else {
            return;
        };
        message.streaming = false;
        append(&mut message.text, &self.notes.stopped);
        (self.push)(ChatUpdate {
            chat,
            index,
            message: message.clone(),
        });
    }
}

/// Adds a note after the answer, closing a code block the answer left open.
fn append(text: &mut String, note: &str) {
    if text.matches("```").count() % 2 == 1 {
        text.push_str("\n```");
    }
    if !text.trim().is_empty() {
        text.push_str("\n\n");
    }
    text.push_str(note);
}

/// One answer being written into message `index` of conversation `chat`.
struct Answer {
    state: Arc<Mutex<State>>,
    push: Push,
    notes: Arc<Notes>,
    chat: String,
    index: usize,
    cancel: Arc<AtomicBool>,
}

impl Answer {
    fn write(self, model: &dyn Model, history: &[ChatMessage]) {
        let mut last_push = Instant::now();
        let result = model.answer(history, &mut |piece| {
            if self.cancel.load(Ordering::SeqCst) {
                return false;
            }
            if piece.is_empty() {
                return true;
            }
            self.update(
                |message| message.text.push_str(piece),
                |message| {
                    if last_push.elapsed() >= PUSH_INTERVAL {
                        last_push = Instant::now();
                        Some(message.clone())
                    } else {
                        None
                    }
                },
            )
        });
        if self.cancel.load(Ordering::SeqCst) {
            return; // `stop` finished the message.
        }
        self.update(
            |message| {
                message.streaming = false;
                match &result {
                    Ok(Ending::Length) => append(&mut message.text, &self.notes.length),
                    Ok(_) => {}
                    Err(error) => {
                        message.error = message.text.trim().is_empty();
                        append(&mut message.text, error);
                    }
                }
            },
            |message| Some(message.clone()),
        );
        let mut state = self.state.lock().unwrap();
        if state
            .cancel
            .as_ref()
            .is_some_and(|cancel| Arc::ptr_eq(cancel, &self.cancel))
        {
            state.cancel = None;
        }
    }

    /// Changes the message while it is still this answer's, then pushes what `send` picks;
    /// `false` once the conversation moved on.
    fn update(
        &self,
        change: impl FnOnce(&mut ChatMessage),
        send: impl FnOnce(&ChatMessage) -> Option<ChatMessage>,
    ) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.chat.id != self.chat {
            return false;
        }
        let Some(message) = state.chat.messages.get_mut(self.index) else {
            return false;
        };
        change(message);
        let update = send(message).map(|message| ChatUpdate {
            chat: self.chat.clone(),
            index: self.index,
            message,
        });
        drop(state);
        if let Some(update) = update {
            (self.push)(update);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// Answers with the given pieces, waiting for a signal before each one when asked.
    struct Script {
        pieces: Vec<&'static str>,
        gate: Option<Mutex<mpsc::Receiver<()>>>,
        ending: Result<Ending, String>,
    }

    impl Model for Script {
        fn answer(
            &self,
            history: &[ChatMessage],
            on_text: &mut dyn FnMut(&str) -> bool,
        ) -> Result<Ending, String> {
            assert_eq!(history.last().unwrap().role, Role::User);
            for piece in &self.pieces {
                if let Some(gate) = &self.gate {
                    gate.lock().unwrap().recv().unwrap();
                }
                if !on_text(piece) {
                    return Ok(Ending::Cancelled);
                }
            }
            self.ending.clone()
        }
    }

    fn session(script: Script) -> (Session, mpsc::Receiver<ChatUpdate>) {
        let (sender, updates) = mpsc::channel();
        let sender = Mutex::new(sender);
        let push: Push = Arc::new(move |update| {
            let _ = sender.lock().unwrap().send(update);
        });
        let notes = Notes {
            length: "[length]".into(),
            stopped: "[stopped]".into(),
        };
        (Session::new(Arc::new(script), push, notes), updates)
    }

    /// Waits for the update that finishes message `index`.
    fn finished(updates: &mpsc::Receiver<ChatUpdate>, index: usize) -> ChatMessage {
        loop {
            let update = updates.recv_timeout(Duration::from_secs(5)).unwrap();
            if update.index == index && !update.message.streaming {
                return update.message;
            }
        }
    }

    #[test]
    fn answers_stream_into_the_conversation() {
        let (session, updates) = session(Script {
            pieces: vec!["Hel", "", "lo"],
            gate: None,
            ending: Ok(Ending::Finished),
        });
        session.send(" hi ").unwrap();
        let chat = session.chat();
        assert_eq!(chat.messages[0].text, "hi");
        let answer = finished(&updates, 1);
        assert_eq!(answer.text, "Hello");
        assert_eq!(session.chat().messages[1], answer);
        assert!(!session.chat().busy());
        // The next question carries the whole conversation.
        session.send("again").unwrap();
        assert_eq!(finished(&updates, 3).text, "Hello");
    }

    #[test]
    fn stopping_keeps_the_partial_answer_and_frees_the_input() {
        let (gate, receiver) = mpsc::channel();
        let (session, updates) = session(Script {
            pieces: vec!["part", "never"],
            gate: Some(Mutex::new(receiver)),
            ending: Ok(Ending::Finished),
        });
        session.send("q").unwrap();
        assert!(session.send("other").is_err(), "busy while answering");
        gate.send(()).unwrap();
        while session.chat().messages[1].text.is_empty() {
            std::thread::sleep(Duration::from_millis(5));
        }
        session.stop();
        let stopped = finished(&updates, 1);
        assert_eq!(stopped.text, "part\n\n[stopped]");
        let _ = gate.send(());
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(session.chat().messages[1].text, "part\n\n[stopped]");
        assert!(!session.chat().busy());
    }

    #[test]
    fn failures_and_limits_are_noted_and_new_conversations_start_empty() {
        let (session, updates) = session(Script {
            pieces: vec![],
            gate: None,
            ending: Err("no key".into()),
        });
        let first = session.chat().id;
        session.send("q").unwrap();
        let failed = finished(&updates, 1);
        assert!(failed.error);
        assert_eq!(failed.text, "no key");
        session.start("");
        assert_ne!(session.chat().id, first);
        assert!(session.chat().messages.is_empty());

        let (session, updates) = session_with_length();
        session.start("long");
        assert_eq!(finished(&updates, 1).text, "a\n\n[length]");
        let mut open = "```rust\nfn".to_owned();
        append(&mut open, "[stopped]");
        assert_eq!(open, "```rust\nfn\n```\n\n[stopped]");
    }

    fn session_with_length() -> (Session, mpsc::Receiver<ChatUpdate>) {
        session(Script {
            pieces: vec!["a"],
            gate: None,
            ending: Ok(Ending::Length),
        })
    }
}
