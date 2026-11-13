//! The current conversation: questions go to the model on a background thread, and each
//! change of the answer is pushed to the host while it is written. Finished conversations
//! are kept in the history.
use crate::history::History;
use mytools_plugin_api::chat::{
    Chat, ChatMessage, ChatSource, ChatStep, ChatUpdate, Role, StepState,
};
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

/// Something that happened while answering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Text(String),
    Reasoning(String),
    /// Step `index` started or changed, e.g. a search finished.
    Step(usize, ChatStep),
    Source(ChatSource),
}

/// Writes answers; the real one asks DeepSeek, tests use a script.
pub trait Model: Send + Sync {
    /// Answers the last user message of `history` with `model` (the configured one when
    /// `None`), reporting what happens through `on_event`; `false` from it stops the answer.
    /// An error is the text to show in its place.
    fn answer(
        &self,
        history: &[ChatMessage],
        model: Option<&str>,
        on_event: &mut dyn FnMut(Event) -> bool,
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
    history: Arc<History>,
}

#[derive(Default)]
struct State {
    chat: Chat,
    conversations: u64,
    /// Set to stop the answer being written.
    cancel: Option<Arc<AtomicBool>>,
    /// The conversation started on a selection that can be replaced with an answer.
    replaceable: bool,
    /// The model picked with Ctrl+K; `None` uses the configured one.
    model: Option<String>,
}

impl Session {
    pub fn new(model: Arc<dyn Model>, push: Push, notes: Notes, history: History) -> Self {
        let session = Self {
            state: Default::default(),
            model,
            push,
            notes: Arc::new(notes),
            history: Arc::new(history),
        };
        session.state.lock().unwrap().model = session.history.model();
        session.start("");
        session
    }

    pub fn chat(&self) -> Chat {
        self.state.lock().unwrap().chat.clone()
    }

    pub fn replaceable(&self) -> bool {
        self.state.lock().unwrap().replaceable
    }

    pub fn model(&self) -> Option<String> {
        self.state.lock().unwrap().model.clone()
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    /// Answers the next questions with `model`; `None` goes back to the configured one.
    pub fn use_model(&self, model: Option<String>) {
        self.history.set_model(model.as_deref());
        self.state.lock().unwrap().model = model;
    }

    fn reset(&self, chat: Chat) {
        self.stop();
        let mut state = self.state.lock().unwrap();
        state.chat = chat;
        state.replaceable = false;
    }

    fn new_id(&self) -> String {
        let mut state = self.state.lock().unwrap();
        state.conversations += 1;
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|time| time.as_millis())
            .unwrap_or_default();
        format!("{millis}-{}", state.conversations)
    }

    /// Starts a new conversation, asking `text` first unless it is empty.
    pub fn start(&self, text: &str) {
        let id = self.new_id();
        self.reset(Chat {
            id,
            ..Default::default()
        });
        if !text.trim().is_empty() {
            let _ = self.send(text);
        }
    }

    /// Starts a new conversation about `selection`, which goes with the first question;
    /// `replaceable` when the answer may be pasted over it.
    pub fn start_on(&self, selection: &str, replaceable: bool) {
        self.start("");
        let mut state = self.state.lock().unwrap();
        state.chat.attachment = Some(selection.trim().to_owned()).filter(|text| !text.is_empty());
        state.replaceable = replaceable && state.chat.attachment.is_some();
    }

    /// Drops the text waiting to go with the next question.
    pub fn detach(&self) {
        self.state.lock().unwrap().chat.attachment = None;
    }

    /// Shows a conversation from the history; `false` when it is gone.
    pub fn open(&self, id: &str) -> bool {
        let Some(chat) = self.history.load(id) else {
            return false;
        };
        self.reset(chat);
        true
    }

    /// Asks `text` in the current conversation; `Err` while an answer is being written or
    /// when there is nothing to ask.
    pub fn send(&self, text: &str) -> Result<(), ()> {
        let text = text.trim();
        let mut state = self.state.lock().unwrap();
        if state.chat.busy() || (text.is_empty() && state.chat.attachment.is_none()) {
            return Err(());
        }
        let attachment = state.chat.attachment.take();
        state.chat.messages.push(ChatMessage {
            role: Role::User,
            text: text.into(),
            attachment,
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
            history: self.history.clone(),
            chat: state.chat.id.clone(),
            index: state.chat.messages.len() - 1,
            cancel,
        };
        let model = self.model.clone();
        let chosen = state.model.clone();
        std::thread::spawn(move || answer.write(&*model, &history, chosen.as_deref()));
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
        finish_steps(message);
        append(&mut message.text, &self.notes.stopped);
        let update = ChatUpdate {
            chat,
            index,
            message: message.clone(),
        };
        self.history.save(&state.chat);
        drop(state);
        (self.push)(update);
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

/// Steps still running when the answer ends did not finish.
fn finish_steps(message: &mut ChatMessage) {
    for step in &mut message.steps {
        if step.state == StepState::Running {
            step.state = StepState::Failed;
        }
    }
}

/// One answer being written into message `index` of conversation `chat`.
struct Answer {
    state: Arc<Mutex<State>>,
    push: Push,
    notes: Arc<Notes>,
    history: Arc<History>,
    chat: String,
    index: usize,
    cancel: Arc<AtomicBool>,
}

impl Answer {
    fn write(self, model: &dyn Model, history: &[ChatMessage], chosen: Option<&str>) {
        let mut last_push = Instant::now();
        let result = model.answer(history, chosen, &mut |event| {
            if self.cancel.load(Ordering::SeqCst) {
                return false;
            }
            // Steps and sources show at once; text arrives often, so it is batched.
            let urgent = matches!(event, Event::Step(..) | Event::Source(_));
            self.update(
                |message| match event {
                    Event::Text(text) => message.text.push_str(&text),
                    Event::Reasoning(text) => message.reasoning.push_str(&text),
                    Event::Step(index, step) => {
                        if index < message.steps.len() {
                            message.steps[index] = step;
                        } else {
                            message.steps.push(step);
                        }
                    }
                    Event::Source(source) => message.sources.push(source),
                },
                |message| {
                    (urgent || last_push.elapsed() >= PUSH_INTERVAL).then(|| {
                        last_push = Instant::now();
                        message.clone()
                    })
                },
            )
        });
        if self.cancel.load(Ordering::SeqCst) {
            return; // `stop` finished the message.
        }
        self.update(
            |message| {
                message.streaming = false;
                finish_steps(message);
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
        if state.chat.id == self.chat {
            self.history.save(&state.chat);
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

    /// Answers with the given events, waiting for a signal before each one when asked.
    struct Script {
        events: Vec<Event>,
        gate: Option<Mutex<mpsc::Receiver<()>>>,
        ending: Result<Ending, String>,
        /// The last question and model seen.
        seen: Mutex<Option<(ChatMessage, Option<String>)>>,
    }

    impl Script {
        fn new(events: Vec<Event>, ending: Result<Ending, String>) -> Self {
            Self {
                events,
                gate: None,
                ending,
                seen: Mutex::new(None),
            }
        }
    }

    impl Model for Script {
        fn answer(
            &self,
            history: &[ChatMessage],
            model: Option<&str>,
            on_event: &mut dyn FnMut(Event) -> bool,
        ) -> Result<Ending, String> {
            let question = history.last().unwrap().clone();
            assert_eq!(question.role, Role::User);
            *self.seen.lock().unwrap() = Some((question, model.map(str::to_owned)));
            for event in &self.events {
                if let Some(gate) = &self.gate {
                    gate.lock().unwrap().recv().unwrap();
                }
                if !on_event(event.clone()) {
                    return Ok(Ending::Cancelled);
                }
            }
            self.ending.clone()
        }
    }

    fn text(piece: &str) -> Event {
        Event::Text(piece.into())
    }

    fn session(script: Arc<Script>) -> (Session, mpsc::Receiver<ChatUpdate>, History) {
        let (sender, updates) = mpsc::channel();
        let sender = Mutex::new(sender);
        let push: Push = Arc::new(move |update| {
            let _ = sender.lock().unwrap().send(update);
        });
        let notes = Notes {
            length: "[length]".into(),
            stopped: "[stopped]".into(),
        };
        let history = History::temporary();
        (
            Session::new(script, push, notes, history.clone()),
            updates,
            history,
        )
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
    fn answers_stream_into_the_conversation_with_steps_and_sources() {
        let step = |state| ChatStep {
            text: "search".into(),
            state,
        };
        let script = Arc::new(Script::new(
            vec![
                Event::Reasoning("hmm".into()),
                Event::Step(0, step(StepState::Running)),
                Event::Step(0, step(StepState::Done)),
                Event::Source(ChatSource {
                    title: "A".into(),
                    url: "https://a".into(),
                }),
                text("Hel"),
                text("lo [1]"),
            ],
            Ok(Ending::Finished),
        ));
        let (session, updates, history) = session(script);
        session.send(" hi ").unwrap();
        assert_eq!(session.chat().messages[0].text, "hi");
        let answer = finished(&updates, 1);
        assert_eq!(answer.text, "Hello [1]");
        assert_eq!(answer.reasoning, "hmm");
        assert_eq!(answer.steps, [step(StepState::Done)]);
        assert_eq!(answer.sources.len(), 1);
        // The history is written after the final update; wait for it.
        let id = session.chat().id;
        let until = Instant::now() + Duration::from_secs(5);
        while history.recent(5).first().map(|(id, _)| id.clone()) != Some(id.clone()) {
            assert!(Instant::now() < until, "not saved");
            std::thread::sleep(Duration::from_millis(5));
        }
        session.send("again").unwrap();
        assert_eq!(finished(&updates, 3).text, "Hello [1]");
        std::thread::sleep(Duration::from_millis(50));
        session.start("");
        assert!(session.chat().messages.is_empty());
        assert!(session.open(&id));
        assert_eq!(session.chat().messages.len(), 4);
    }

    #[test]
    fn stopping_keeps_the_partial_answer_and_frees_the_input() {
        let (gate, receiver) = mpsc::channel();
        let mut script = Script::new(vec![text("part"), text("never")], Ok(Ending::Finished));
        script.gate = Some(Mutex::new(receiver));
        let (session, updates, _) = session(Arc::new(script));
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
    fn a_selection_goes_with_the_first_question_only() {
        let script = Arc::new(Script::new(vec![text("ok")], Ok(Ending::Finished)));
        let (session, updates, _) = session(script.clone());
        session.start_on("  Hello world ", true);
        assert_eq!(session.chat().attachment.as_deref(), Some("Hello world"));
        assert!(session.replaceable());
        // The selection alone is a question too.
        session.send("").unwrap();
        finished(&updates, 1);
        let (question, _) = script.seen.lock().unwrap().clone().unwrap();
        assert_eq!(question.attachment.as_deref(), Some("Hello world"));
        assert_eq!(session.chat().attachment, None);
        std::thread::sleep(Duration::from_millis(50));
        assert!(session.send("").is_err(), "nothing left to ask");
        session.start_on(" ", true);
        assert!(!session.replaceable());
        session.start_on("x", false);
        session.detach();
        assert_eq!(session.chat().attachment, None);
    }

    #[test]
    fn the_picked_model_is_used_and_remembered() {
        let script = Arc::new(Script::new(vec![], Ok(Ending::Finished)));
        let (session, updates, history) = session(script.clone());
        session.use_model(Some("other".into()));
        session.send("q").unwrap();
        finished(&updates, 1);
        let (_, model) = script.seen.lock().unwrap().clone().unwrap();
        assert_eq!(model.as_deref(), Some("other"));
        assert_eq!(history.model().as_deref(), Some("other"));
    }

    #[test]
    fn failures_and_limits_are_noted() {
        let running = ChatStep {
            text: "s".into(),
            state: StepState::Running,
        };
        let (failing, updates, _) = session(Arc::new(Script::new(
            vec![Event::Step(0, running)],
            Err("no key".into()),
        )));
        failing.send("q").unwrap();
        let failed = finished(&updates, 1);
        assert!(failed.error);
        assert_eq!(failed.text, "no key");
        assert_eq!(failed.steps[0].state, StepState::Failed);

        let (session, updates, _) =
            session(Arc::new(Script::new(vec![text("a")], Ok(Ending::Length))));
        session.start("long");
        assert_eq!(finished(&updates, 1).text, "a\n\n[length]");
        let mut open = "```rust\nfn".to_owned();
        append(&mut open, "[stopped]");
        assert_eq!(open, "```rust\nfn\n```\n\n[stopped]");
    }
}
