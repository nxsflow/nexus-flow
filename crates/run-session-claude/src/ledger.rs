//! The session ledger (§8.3, §9): one directory per session key, the only state this adapter has. `spawn.json` is created
//! with `O_EXCL` before the spawn, `child.json` and every other record is written by temp file and rename, and facts are
//! complete lines only. The read position is not here: the host owns the `offset` file, because it advances only after
//! the facts are in its inbox, and hands it to `drain_facts` (from 6j6v.m8we).

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::sample::SampleRecord;

const FACTS: &str = "facts.jsonl";
const CHILD: &str = "child.json";
const SPAWN: &str = "spawn.json";
const SAMPLE: &str = "sample.json";
const SPEC: &str = "spec.json";
const FINDINGS: &str = "findings.log";
const SIDECAR_LOG: &str = "sidecar.log";

/// Who the session's process is, in the form the child can write itself. Either writer's: the two agree within the
/// guard's slack, or the process is not this session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildRecord {
    pub pid: u32,
    pub start_time: String,
    /// Written by the child itself, with its signal handlers installed. Node's default for SIGUSR2 terminates the
    /// process, so this is what a nudge asks first (§9). The adapter's own record says `false`.
    #[serde(default)]
    pub handlers: bool,
}

/// Written before the spawn.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnRecord {
    pub spec_hash: String,
    /// When, in unix milliseconds: how long a spawn with no child record has stood.
    pub at_ms: i64,
}

/// What is complete in the facts file from an offset on.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Lines {
    /// Each line's own offset and its bytes, the newline left out.
    pub lines: Vec<(u64, Vec<u8>)>,
    /// The offset after the last complete line.
    pub next: u64,
    /// The offset lies past the end of the file: this file is not the one the offset was read from.
    pub past_the_end: bool,
}

#[derive(Clone, Debug)]
pub struct SessionLedger {
    dir: PathBuf,
}

/// Every file of the ledger is the owner's alone: it holds the session's spec and what it said.
#[cfg(unix)]
fn owner_only() -> OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = OpenOptions::new();
    options.mode(0o600);
    options
}

/// No mode bits to set. Never reached by a start, which refuses there first (`SPAWNS_HERE`).
#[cfg(not(unix))]
fn owner_only() -> OpenOptions {
    OpenOptions::new()
}

/// Writes through a temp file beside it, so a reader sees the old file or the new one, whole.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temp = path.with_extension("tmp");
    let mut file = owner_only()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temp)?;
    file.write_all(bytes)?;
    fs::rename(&temp, path)
}

fn json_error(error: serde_json::Error) -> std::io::Error {
    std::io::Error::other(error)
}

impl SessionLedger {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn create(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)
    }

    /// Whether this call created `spawn.json`. Only the one that did may spawn.
    pub fn claim_spawn(&self, record: &SpawnRecord) -> std::io::Result<bool> {
        let opened = owner_only()
            .write(true)
            .create_new(true)
            .open(self.dir.join(SPAWN));
        match opened {
            Ok(mut file) => {
                file.write_all(&serde_json::to_vec(record).map_err(json_error)?)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// A spawn that failed never ran: the next attempt at this key may claim it.
    pub fn release_spawn(&self) -> std::io::Result<()> {
        fs::remove_file(self.dir.join(SPAWN))
    }

    pub fn spawn_record(&self) -> Option<SpawnRecord> {
        let path = self.dir.join(SPAWN);
        match fs::read(path) {
            // A crash between the create and the write leaves an empty file: still a claim.
            Ok(bytes) => Some(serde_json::from_slice(&bytes).unwrap_or_default()),
            Err(_) => None,
        }
    }

    pub fn write_child(&self, record: &ChildRecord) -> std::io::Result<()> {
        write_atomically(
            &self.dir.join(CHILD),
            &serde_json::to_vec(record).map_err(json_error)?,
        )
    }

    /// The adapter's record, right after the spawn: only if the child has not written its own, which always stands. By
    /// a hard link, the one way to create a name in one step without replacing what is there.
    pub fn write_child_unless_there(&self, record: &ChildRecord) -> std::io::Result<()> {
        let temp = self.dir.join("child.adapter.tmp");
        fs::write(&temp, serde_json::to_vec(record).map_err(json_error)?)?;
        let linked = fs::hard_link(&temp, self.dir.join(CHILD));
        fs::remove_file(&temp)?;
        match linked {
            Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => Err(error),
            _ => Ok(()),
        }
    }

    pub fn child(&self) -> Option<ChildRecord> {
        serde_json::from_slice(&fs::read(self.dir.join(CHILD)).ok()?).ok()
    }

    pub fn spec_path(&self) -> PathBuf {
        self.dir.join(SPEC)
    }

    pub fn write_spec(&self, spec: &serde_json::Value) -> std::io::Result<()> {
        write_atomically(
            &self.spec_path(),
            &serde_json::to_vec(spec).map_err(json_error)?,
        )
    }

    /// Where the sidecar's stdout and stderr go: no pipe, so nothing here can block it or die with this process.
    pub fn sidecar_log(&self) -> std::io::Result<File> {
        owner_only()
            .append(true)
            .create(true)
            .open(self.dir.join(SIDECAR_LOG))
    }

    pub fn sample(&self) -> SampleRecord {
        fs::read(self.dir.join(SAMPLE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn write_sample(&self, record: &SampleRecord) -> std::io::Result<()> {
        write_atomically(
            &self.dir.join(SAMPLE),
            &serde_json::to_vec(record).map_err(json_error)?,
        )
    }

    /// One fact the adapter made, into the same file with the same key scheme (round 3, CQ #17). One `write` under
    /// `O_APPEND`, as the sidecar's. A torn last line gets its newline first, or this fact would be swallowed by it.
    pub fn append_fact(&self, fact: &serde_json::Value) -> std::io::Result<()> {
        let mut line = if self.ends_torn() {
            b"\n".to_vec()
        } else {
            Vec::new()
        };
        line.extend(serde_json::to_vec(fact).map_err(json_error)?);
        line.push(b'\n');
        let mut file = owner_only()
            .append(true)
            .create(true)
            .open(self.dir.join(FACTS))?;
        file.write_all(&line)
    }

    fn ends_torn(&self) -> bool {
        let Ok(mut file) = File::open(self.dir.join(FACTS)) else {
            return false;
        };
        if file.seek(SeekFrom::End(-1)).is_err() {
            return false;
        }
        let mut last = [0_u8; 1];
        file.read_exact(&mut last).is_ok() && last[0] != b'\n'
    }

    /// The complete lines from `from` on. A torn last line waits for its newline.
    pub fn lines_from(&self, from: u64) -> std::io::Result<Lines> {
        let mut file = match File::open(self.dir.join(FACTS)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Lines {
                    next: from,
                    past_the_end: from > 0,
                    ..Lines::default()
                })
            }
            Err(error) => return Err(error),
        };
        if from > file.metadata()?.len() {
            return Ok(Lines {
                next: from,
                past_the_end: true,
                ..Lines::default()
            });
        }
        file.seek(SeekFrom::Start(from))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let mut lines = Vec::new();
        let mut offset = from;
        let mut rest = bytes.as_slice();
        while let Some(end) = rest.iter().position(|byte| *byte == b'\n') {
            lines.push((offset, rest[..end].to_vec()));
            offset += end as u64 + 1;
            rest = &rest[end + 1..];
        }
        Ok(Lines {
            lines,
            next: offset,
            past_the_end: false,
        })
    }

    /// What a person should hear about this ledger: a line a reader could not use. Its text never comes along, because
    /// the text may be anything the SDK said.
    pub fn add_finding(&self, finding: &str) {
        eprintln!("session ledger {}: {finding}", self.dir.display());
        let opened = owner_only()
            .append(true)
            .create(true)
            .open(self.dir.join(FINDINGS));
        if let Ok(mut file) = opened {
            // A finding that cannot be written still went to the log line above.
            let _ = writeln!(file, "{finding}");
        }
    }

    pub fn findings(&self) -> Vec<String> {
        fs::read_to_string(self.dir.join(FINDINGS))
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}
