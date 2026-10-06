//! `nxs personas migrate` (nxf 6j6v.h9ee, epic 6j6v.phcx) — rewrites a declaration folder from the
//! older form into the new one: every `<handle>.yaml` into `<handle>/SKILL.md`, the list in
//! `channels.yaml` into one `channels/<name>.yaml` per channel, and removes the old files.
//!
//! **It rewrites TEXT, not values.** The declarations it exists for carry their authors' notes as
//! YAML comments — measured on the real team in `agents`: 28 of 29 files, 59 lines of design notes
//! in `channels.yaml` alone — and a parse-and-serialize round trip would drop every one of them
//! without a word. So the file is cut into its top-level keys, each key keeping the comments above
//! it, and the pieces are moved: `handle` becomes `name`, `job_description` becomes `description`,
//! `tools` becomes `allowed-tools` (its value as written, a YAML list being one of the accepted
//! spellings), `system_prompt` becomes the body, and every nexus-flow field moves under `nxs:`.
//!
//! **And then it PROVES the rewrite.** Every new file is read back by the loader every seam uses
//! and must yield exactly the declaration the old one did — `address_book` and the three inert
//! fields aside, which the migration drops on purpose and reports per file. A rewrite that would
//! change anything else stops the run before a single file is written, naming the file.
//!
//! **It refuses to guess.** A key the mapping does not know aborts the run before anything is
//! written, naming the file and the key. Lieber ein Abbruch mit Nennung der Datei als eine stille
//! Umschrift — the brief's words.
//!
//! **Idempotent**: once nothing of the older form is left, a second run finds nothing to do.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_yaml::Value;

use crate::channel::{ChannelDecl, CHANNELS_DIR};
use crate::error::{NxfError, Result};
use crate::role::RoleDecl;
use crate::skill::{INERT_FIELDS, NXS_FIELDS, RETIRED_FIELD, SKILL_FILE};

/// The list file of the older channel form.
const CHANNELS_LIST: &str = "channels.yaml";

/// Where the comments heading `channels.yaml` go — they describe the whole set and belong to no
/// one channel. The loader reads only `*.yaml` under `channels/`, so this file is never a
/// declaration.
const CHANNELS_README: &str = "README.md";

/// What a migration did — or, without `apply`, would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationReport {
    /// The declaration folder migrated.
    pub folder: PathBuf,
    /// Whether the files were written. `false` is the plan: everything checked, nothing touched.
    pub applied: bool,
    pub personas: Vec<MigratedFile>,
    pub channels: Vec<MigratedFile>,
    /// Where the comments heading `channels.yaml` went, when it had any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_moved_to: Option<PathBuf>,
}

impl MigrationReport {
    /// Whether the folder holds nothing of the older form — the state a second run finds.
    pub fn nothing_to_do(&self) -> bool {
        self.personas.is_empty() && self.channels.is_empty()
    }

    /// How many `address_book` declarations the run drops.
    pub fn dropped_address_books(&self) -> usize {
        self.personas
            .iter()
            .filter(|p| p.dropped.iter().any(|d| d.starts_with(RETIRED_FIELD)))
            .count()
    }
}

/// One declaration moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigratedFile {
    pub name: String,
    pub from: PathBuf,
    pub to: PathBuf,
    /// The keys left out, as a reader would name them: `address_book (3 entries)`, `session`, …
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dropped: Vec<String>,
}

/// Migrate the declaration folder `dir`. Without `apply` this is the plan: every file is read,
/// rewritten in memory and proved, and nothing is written.
pub fn migrate_declarations(dir: &Path, apply: bool) -> Result<MigrationReport> {
    let mut report = MigrationReport {
        folder: dir.to_path_buf(),
        applied: apply,
        personas: Vec::new(),
        channels: Vec::new(),
        header_moved_to: None,
    };
    if !dir.is_dir() {
        return Ok(report);
    }

    // Everything is rewritten and proved in memory first; only a run that got through all of it
    // writes anything.
    let mut writes: Vec<(PathBuf, String)> = Vec::new();
    let mut removals: Vec<PathBuf> = Vec::new();

    let mut persona_files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| NxfError::io(format!("reading {}: {e}", dir.display())))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().is_some_and(|ext| ext == "yaml")
                && !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n == CHANNELS_LIST || n == "workflow.yaml")
        })
        .collect();
    persona_files.sort();
    for path in persona_files {
        let content = std::fs::read_to_string(&path)
            .map_err(|e| NxfError::io(format!("reading {}: {e}", path.display())))?;
        let rewrite = rewrite_persona(&path, &content)?;
        let to = dir.join(&rewrite.name).join(SKILL_FILE);
        writes.push((to.clone(), rewrite.text));
        removals.push(path.clone());
        report.personas.push(MigratedFile {
            name: rewrite.name,
            from: path,
            to,
            dropped: rewrite.dropped,
        });
    }

    let list = dir.join(CHANNELS_LIST);
    if list.is_file() {
        let content = std::fs::read_to_string(&list)
            .map_err(|e| NxfError::io(format!("reading {}: {e}", list.display())))?;
        let rewrite = rewrite_channels(&list, &content)?;
        let folder = dir.join(CHANNELS_DIR);
        if let Some(header) = rewrite.header {
            let to = folder.join(CHANNELS_README);
            writes.push((to.clone(), header));
            report.header_moved_to = Some(to);
        }
        for (name, text) in rewrite.channels {
            let to = folder.join(format!("{name}.yaml"));
            writes.push((to.clone(), text));
            report.channels.push(MigratedFile {
                name,
                from: list.clone(),
                to,
                dropped: Vec::new(),
            });
        }
        removals.push(list);
    }

    for (i, (to, _)) in writes.iter().enumerate() {
        if to.exists() || writes[..i].iter().any(|(t, _)| t == to) {
            return Err(NxfError::validation(format!(
                "{} exists already — the migration writes nothing over a file; move it away or \
                 delete the declaration it duplicates, then run again",
                to.display()
            )));
        }
    }

    if apply {
        for (to, text) in &writes {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| NxfError::io(format!("creating {}: {e}", parent.display())))?;
            }
            let staged = to.with_extension("migrating");
            std::fs::write(&staged, text)
                .map_err(|e| NxfError::io(format!("writing {}: {e}", staged.display())))?;
            std::fs::rename(&staged, to)
                .map_err(|e| NxfError::io(format!("writing {}: {e}", to.display())))?;
        }
        for path in &removals {
            std::fs::remove_file(path)
                .map_err(|e| NxfError::io(format!("removing {}: {e}", path.display())))?;
        }
    }
    Ok(report)
}

struct PersonaRewrite {
    name: String,
    text: String,
    dropped: Vec<String>,
}

/// One top-level key of a YAML mapping as written: the comment and blank lines above it, then its
/// own line and every line that continues it.
#[derive(Debug)]
struct Chunk {
    key: String,
    leading: Vec<String>,
    body: Vec<String>,
}

/// Cut a top-level YAML mapping into its keys. Returns the lines heading the file (the comments
/// above the first key, up to the last blank line before it), the chunks, and the lines trailing
/// the last one.
fn top_level_chunks(path: &Path, content: &str) -> Result<(Vec<String>, Vec<Chunk>, Vec<String>)> {
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        let indented = line.len() != trimmed.len();
        if line.trim().is_empty() || (!indented && trimmed.starts_with('#')) {
            pending.push(line.to_string());
        } else if indented {
            let Some(chunk) = chunks.last_mut() else {
                return Err(unrewritable(path, "it opens with an indented line"));
            };
            chunk.body.append(&mut pending);
            chunk.body.push(line.to_string());
        } else {
            let Some(key) = top_level_key(line) else {
                return Err(unrewritable(
                    path,
                    &format!("its line {line:?} is not a top-level key"),
                ));
            };
            chunks.push(Chunk {
                key,
                leading: std::mem::take(&mut pending),
                body: vec![line.to_string()],
            });
        }
    }
    let mut header = Vec::new();
    if let Some(first) = chunks.first_mut() {
        if let Some(last_blank) = first.leading.iter().rposition(|l| l.trim().is_empty()) {
            header = first.leading.drain(..=last_blank).collect();
        }
    }
    Ok((header, chunks, pending))
}

fn top_level_key(line: &str) -> Option<String> {
    let (key, rest) = line.split_once(':')?;
    let valid = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    (valid && (rest.is_empty() || rest.starts_with(' '))).then(|| key.to_string())
}

fn unrewritable(path: &Path, why: &str) -> NxfError {
    NxfError::validation(format!(
        "{}: cannot rewrite this file — {why}. Nothing was written; rewrite it by hand into \
         <name>/SKILL.md",
        path.display()
    ))
}

/// Replace the key of a chunk's first line, keeping its value as written.
fn rename_key(chunk: &Chunk, to: &str) -> Vec<String> {
    let mut lines = chunk.leading.clone();
    let first = &chunk.body[0];
    lines.push(format!("{to}{}", &first[chunk.key.len()..]));
    lines.extend(chunk.body[1..].iter().cloned());
    lines
}

fn indented(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|l| match l.trim().is_empty() {
            true => l,
            false => format!("  {l}"),
        })
        .collect()
}

fn rewrite_persona(path: &Path, content: &str) -> Result<PersonaRewrite> {
    let original: RoleDecl = serde_yaml::from_str(content)
        .map_err(|e| NxfError::validation(format!("parsing {}: {e}", path.display())))?;
    let raw: Value = serde_yaml::from_str(content)
        .map_err(|e| NxfError::validation(format!("parsing {}: {e}", path.display())))?;
    let (header, chunks, trailer) = top_level_chunks(path, content)?;

    let (mut top, mut nxs, mut closing) = (Vec::new(), Vec::new(), Vec::new());
    let mut dropped = Vec::new();
    let mut body = None;
    let (mut name, mut description, mut tools) = (Vec::new(), Vec::new(), Vec::new());
    for chunk in &chunks {
        match chunk.key.as_str() {
            "handle" => name = rename_key(chunk, "name"),
            "job_description" => description = rename_key(chunk, "description"),
            "tools" => tools = rename_key(chunk, "allowed-tools"),
            "system_prompt" => {
                closing.extend(chunk.leading.iter().cloned());
                body = Some(original.system_prompt.clone());
            }
            "license" | "compatibility" | "metadata" => top.extend(rename_key(chunk, &chunk.key)),
            "job_title" => nxs.extend(indented(rename_key(chunk, "title"))),
            RETIRED_FIELD => dropped.push(match raw.get(RETIRED_FIELD) {
                Some(Value::Sequence(book)) if book.is_empty() => format!("{RETIRED_FIELD} ([])"),
                Some(Value::Sequence(book)) => format!(
                    "{RETIRED_FIELD} ({} {})",
                    book.len(),
                    if book.len() == 1 { "entry" } else { "entries" }
                ),
                _ => RETIRED_FIELD.to_string(),
            }),
            key if INERT_FIELDS.contains(&key) => dropped.push(key.to_string()),
            key if NXS_FIELDS.contains(&key) && key != "title" => {
                nxs.extend(indented(rename_key(chunk, key)))
            }
            key => {
                return Err(NxfError::validation(format!(
                    "{}: the key `{key}` has no place in the skill form — the migration does not \
                     guess, so nothing was written. Remove it or rename it to a field `nxc guide \
                     personas` lists, then run again",
                    path.display()
                )))
            }
        }
    }
    let body = body.ok_or_else(|| unrewritable(path, "it declares no `system_prompt`"))?;

    let mut front = header;
    front.extend(name);
    front.extend(description);
    front.extend(tools);
    front.extend(top);
    if !nxs.is_empty() {
        front.push("nxs:".to_string());
        front.extend(nxs);
    }
    front.extend(closing);
    front.extend(trailer.into_iter().filter(|l| !l.trim().is_empty()));
    // The loader strips ONE blank line after the closing `---`, so a prompt that itself opens with
    // one keeps it.
    let separator = if body.starts_with('\n') { "\n" } else { "" };
    let text = format!("---\n{}\n---\n{separator}{body}", front.join("\n"));

    // The proof: read back by the loader every seam uses, the same declaration.
    let target = path
        .parent()
        .unwrap_or(Path::new(""))
        .join(&original.handle)
        .join(SKILL_FILE);
    let migrated = crate::skill::parse_skill(&target, &text).map_err(|e| {
        NxfError::validation(format!(
            "{}: the rewrite does not read back ({e}) — nothing was written",
            path.display()
        ))
    })?;
    let mut expected = original.clone();
    expected.session = Default::default();
    expected.sub_agents = false;
    expected.reports_to = None;
    let old_file = crate::skill::yaml_record_from_str(path, content, &original)?;
    let same_extras = migrated.file.requires == old_file.requires
        && migrated.file.license == old_file.license
        && migrated.file.compatibility == old_file.compatibility
        && migrated.file.metadata == old_file.metadata;
    if migrated.decl != expected || !same_extras {
        return Err(NxfError::validation(format!(
            "{}: the rewrite would change the declaration, so nothing was written — rewrite this \
             file by hand into {}",
            path.display(),
            target.display()
        )));
    }
    Ok(PersonaRewrite {
        name: original.handle,
        text,
        dropped,
    })
}

struct ChannelsRewrite {
    header: Option<String>,
    channels: Vec<(String, String)>,
}

/// Cut the list in `channels.yaml` into one text per channel — each item dedented into a map, with
/// the comments above it — and prove each against the channel the list declared.
fn rewrite_channels(path: &Path, content: &str) -> Result<ChannelsRewrite> {
    let declared: Vec<Value> = serde_yaml::from_str::<Option<Vec<Value>>>(content)
        .map_err(|e| NxfError::validation(format!("parsing {}: {e}", path.display())))?
        .unwrap_or_default();

    let mut items: Vec<(Vec<String>, Vec<String>)> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let mut header: Vec<String> = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        let indented = line.len() != trimmed.len();
        if line.trim().is_empty() || (!indented && trimmed.starts_with('#')) {
            pending.push(line.to_string());
        } else if let Some(rest) = line.strip_prefix("- ").or((line == "-").then_some("")) {
            let mut leading = std::mem::take(&mut pending);
            if items.is_empty() {
                if let Some(last_blank) = leading.iter().rposition(|l| l.trim().is_empty()) {
                    header = leading.drain(..=last_blank).collect();
                }
            }
            items.push((leading, vec![rest.to_string()]));
        } else if let Some(stripped) = line.strip_prefix("  ") {
            let Some((_, body)) = items.last_mut() else {
                return Err(unrewritable(path, "it opens with an indented line"));
            };
            body.append(&mut pending);
            body.push(stripped.to_string());
        } else {
            return Err(unrewritable(
                path,
                &format!("its line {line:?} is not part of a list of channels"),
            ));
        }
    }

    if items.len() != declared.len() {
        return Err(unrewritable(
            path,
            "its channels are not one block per `- ` item",
        ));
    }
    let mut channels = Vec::new();
    for ((leading, body), value) in items.into_iter().zip(declared) {
        let leading: Vec<String> = leading
            .into_iter()
            .skip_while(|l| l.trim().is_empty())
            .collect();
        let text = format!(
            "{}\n",
            leading
                .into_iter()
                .chain(body)
                .collect::<Vec<_>>()
                .join("\n")
        );
        let reread: Value = serde_yaml::from_str(&text).map_err(|e| {
            NxfError::validation(format!(
                "{}: a channel does not read back on its own ({e}) — nothing was written",
                path.display()
            ))
        })?;
        let channel: ChannelDecl = serde_yaml::from_value(value.clone())
            .map_err(|e| NxfError::validation(format!("parsing {}: {e}", path.display())))?;
        if reread != value {
            return Err(NxfError::validation(format!(
                "{}: the rewrite of channel {:?} would change it, so nothing was written",
                path.display(),
                channel.name
            )));
        }
        if channel.name.is_empty()
            || channel.name.contains(['/', '\\'])
            || channel.name.starts_with('.')
        {
            return Err(NxfError::validation(format!(
                "{}: channel {:?} cannot name a file — nothing was written",
                path.display(),
                channel.name
            )));
        }
        channels.push((channel.name, text));
    }
    let header = header.iter().any(|l| !l.trim().is_empty()).then(|| {
        let text: Vec<String> = header
            .iter()
            .map(|l| {
                let l = l.trim_start();
                l.strip_prefix("# ")
                    .or_else(|| l.strip_prefix('#'))
                    .unwrap_or(l)
                    .to_string()
            })
            .collect();
        format!(
            "# Channels\n\nThe notes that headed `channels.yaml`, moved here by `nxs personas \
                 migrate` — each channel is a file of its own now.\n\n{}\n",
            text.join("\n").trim()
        )
    });
    Ok(ChannelsRewrite { header, channels })
}
