//! The chat store and the declarations as a file, so what was posted survives an abort of the host and a crash
//! between a post and its `done` can be seen.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use nexus_run_contracts::MessageId;
use nexus_run_engine::{ChannelDecl, DeclaredTree, OnComplete, StepDecl, TurnKind, TurnPart};
use serde::{Deserialize, Serialize};

use crate::ports::{
    DeclarationPort, Message, MessagePort, PersonaDecl, PortError, PortResult, PromptBundle,
    PromptFor, Refs, SidecarPort,
};

/// One line of the port's file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "line", rename_all = "snake_case")]
pub enum PortLine {
    Open {
        thread: String,
        parent: Option<String>,
        channel: String,
        opener: String,
    },
    Post {
        id: String,
        thread: String,
        sender: String,
        body: String,
        refs: Refs,
        key: String,
    },
}

/// The chat store as a file, so what was posted survives an abort of the host and a crash between a post and
/// its `done` can be seen.
pub struct FilePorts {
    path: PathBuf,
    /// Posts refused while set: the chat store is locked.
    pub refuse_posts: std::sync::atomic::AtomicBool,
    /// A post whose body holds this text panics, on the host's thread.
    pub panic_on_body: std::sync::Mutex<Option<String>>,
    /// Every post takes this long, in real milliseconds.
    pub post_delay_ms: std::sync::atomic::AtomicU64,
    /// Every prompt purpose and turn the host asked for.
    pub asked: std::sync::Mutex<Vec<String>>,
}

impl FilePorts {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            refuse_posts: std::sync::atomic::AtomicBool::new(false),
            panic_on_body: std::sync::Mutex::new(None),
            post_delay_ms: std::sync::atomic::AtomicU64::new(0),
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn lines(&self) -> Vec<PortLine> {
        read_port_lines(&self.path)
    }

    fn append(&self, line: &PortLine) -> PortResult<()> {
        let text = serde_json::to_string(line).map_err(port_error)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(port_error)?;
        file.write_all(format!("{text}\n").as_bytes())
            .map_err(port_error)
    }

    fn note(&self, what: String) {
        self.asked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(what);
    }
}

fn port_error(error: impl std::fmt::Display) -> PortError {
    PortError {
        what: error.to_string(),
    }
}

pub fn read_port_lines(path: &Path) -> Vec<PortLine> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

impl MessagePort for FilePorts {
    fn post(
        &self,
        thread: &str,
        sender: &str,
        body: &str,
        refs: &Refs,
        key: &str,
    ) -> PortResult<MessageId> {
        if self.refuse_posts.load(Ordering::SeqCst) {
            return Err(port_error("the chat store is locked"));
        }
        let panic_on = self
            .panic_on_body
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if panic_on.is_some_and(|text| body.contains(&text)) {
            panic!("a scripted panic in the chat port");
        }
        let delay = self.post_delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay));
        }
        let id = format!("m-{}", ulid::Ulid::new());
        self.append(&PortLine::Post {
            id: id.clone(),
            thread: thread.to_owned(),
            sender: sender.to_owned(),
            body: body.to_owned(),
            refs: refs.clone(),
            key: key.to_owned(),
        })?;
        Ok(id)
    }

    fn find(&self, thread: &str, key: &str) -> PortResult<Option<MessageId>> {
        Ok(self.lines().into_iter().find_map(|line| match line {
            PortLine::Post {
                id,
                thread: in_thread,
                key: with_key,
                ..
            } if in_thread == thread && with_key == key => Some(id),
            _ => None,
        }))
    }

    fn read(&self, message: &str) -> PortResult<Option<Message>> {
        Ok(self.lines().into_iter().find_map(|line| match line {
            PortLine::Post {
                id,
                sender,
                body,
                refs,
                ..
            } if id == message => Some(Message { sender, body, refs }),
            _ => None,
        }))
    }

    fn open_thread(
        &self,
        thread_id: &str,
        parent: Option<&str>,
        channel: &str,
        opener: &str,
    ) -> PortResult<()> {
        self.append(&PortLine::Open {
            thread: thread_id.to_owned(),
            parent: parent.map(str::to_owned),
            channel: channel.to_owned(),
            opener: opener.to_owned(),
        })
    }
}

/// `coding.yaml` as data: build -> verify -> judge -> finish.
pub fn coding_decl() -> ChannelDecl {
    let step = |id: &str, target: &str, next: Option<&str>| StepDecl {
        id: id.to_owned(),
        target: target.to_owned(),
        next: next.map(str::to_owned),
        ..StepDecl::default()
    };
    let back = |mut step: StepDecl| {
        step.on_needs_rework = Some("build".to_owned());
        step.max_passes = Some(3);
        step
    };
    let mut build = step("build", "coder", Some("verify"));
    build.resume = true;
    let mut verify = back(step("verify", "verifier", Some("judge")));
    verify.input = Some(Vec::new());
    let mut judge = back(step("judge", "review", Some("finish")));
    judge.task = Some("Judge the change on the branch.".to_owned());
    let mut finish = back(step("finish", "finisher", None));
    finish.input = Some(vec!["verify".to_owned(), "judge".to_owned()]);
    ChannelDecl {
        name: "coding".to_owned(),
        steps: vec![build, verify, judge, finish],
        working_tree: DeclaredTree::Exclusive,
        ..ChannelDecl::default()
    }
}

pub fn review_decl() -> ChannelDecl {
    ChannelDecl {
        name: "review".to_owned(),
        members: vec![
            "code-quality".to_owned(),
            "test-quality".to_owned(),
            "integrity".to_owned(),
        ],
        on_complete: OnComplete::Summarize,
        summary_prompt: Some("Consolidate the three verdicts.".to_owned()),
        ..ChannelDecl::default()
    }
}

impl DeclarationPort for FilePorts {
    fn channel(&self, name: &str) -> PortResult<ChannelDecl> {
        match name {
            "coding" => Ok(coding_decl()),
            "review" => Ok(review_decl()),
            other => Err(port_error(format!("no channel {other}"))),
        }
    }

    fn persona(&self, handle: &str) -> PortResult<PersonaDecl> {
        Ok(PersonaDecl {
            handle: handle.to_owned(),
            model: Some("a-model".to_owned()),
            ..PersonaDecl::default()
        })
    }

    fn prompt(&self, handle: &str, purpose: PromptFor) -> PortResult<PromptBundle> {
        self.note(format!("prompt {handle} {purpose:?}"));
        Ok(PromptBundle {
            system_prompt: format!("Standing context of {handle}."),
            use_claude_code_preset: true,
            granted_tools: vec!["Bash".to_owned()],
            declaration_hash: "sha256:decl".to_owned(),
        })
    }

    /// Reads each part by id and frames it, as the v1 side does.
    fn turn(&self, handle: &str, parts: &[TurnPart]) -> PortResult<String> {
        let mut text = Vec::new();
        for part in parts {
            let id = part.message.as_deref().unwrap_or_default();
            let label = match &part.kind {
                TurnKind::Root => "commission".to_owned(),
                TurnKind::Reply { step, member } => format!("reply of {member} on {step}"),
                TurnKind::Answer => "answer".to_owned(),
                other => format!("{other:?}"),
            };
            self.note(format!("turn {handle} {label} {id}"));
            let body = self
                .read(id)?
                .map(|message| message.body)
                .unwrap_or_default();
            text.push(format!("<{label}>{body}</{label}>"));
        }
        Ok(text.join("\n"))
    }
}

impl SidecarPort for FilePorts {
    fn bundle_path(&self) -> PortResult<PathBuf> {
        Ok(PathBuf::from("/nonexistent/sidecar.mjs"))
    }
}
