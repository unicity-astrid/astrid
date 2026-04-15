//! Agency request helpers for Astrid's EVOLVE loop.
//!
//! Astrid does not edit repo files directly in v1. Instead, she writes
//! structured requests that humans and Claude Code can review, fulfill,
//! or decline explicitly.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::journal::read_local_journal_body_for_continuity;
use crate::paths::bridge_paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgencyRequestKind {
    CodeChange,
    ExperienceRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgencyRequestStatus {
    Pending,
    Accepted,
    Completed,
    Declined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceMode {
    Sensory,
    Creative,
    Social,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgencyResolution {
    pub status: AgencyRequestStatus,
    pub resolved_at: String,
    pub outcome_summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub touched_paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgencyRequest {
    pub id: String,
    pub timestamp: String,
    pub source_journal_path: String,
    pub request_kind: AgencyRequestKind,
    pub title: String,
    pub felt_need: String,
    pub why_now: String,
    pub status: AgencyRequestStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance_signals: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_symbols: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_behavior: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_patch: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub experience_mode: Option<ExperienceMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_setup: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why_this_feels_important: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fulfillment_hint: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<AgencyResolution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgencyRequestDraft {
    pub request_kind: AgencyRequestKind,
    pub title: String,
    pub felt_need: String,
    pub why_now: String,
    #[serde(default)]
    pub acceptance_signals: Vec<String>,

    #[serde(default)]
    pub target_paths: Vec<String>,
    #[serde(default)]
    pub target_symbols: Vec<String>,
    pub requested_behavior: Option<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    pub draft_patch: Option<String>,

    pub experience_mode: Option<ExperienceMode>,
    pub requested_setup: Option<String>,
    pub why_this_feels_important: Option<String>,
    pub fulfillment_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntrospectorSnippet {
    pub tool_name: String,
    pub label: String,
    pub text: String,
}

impl AgencyRequestDraft {
    #[must_use]
    pub fn normalize(mut self) -> Self {
        self.title = self.title.trim().to_string();
        self.felt_need = self.felt_need.trim().to_string();
        self.why_now = self.why_now.trim().to_string();
        self.acceptance_signals = clean_vec(self.acceptance_signals);
        self.target_paths = clean_vec(self.target_paths);
        self.target_symbols = clean_vec(self.target_symbols);
        self.requested_behavior = clean_option(self.requested_behavior);
        self.constraints = clean_vec(self.constraints);
        self.draft_patch = clean_option(self.draft_patch);
        self.requested_setup = clean_option(self.requested_setup);
        self.why_this_feels_important = clean_option(self.why_this_feels_important);
        self.fulfillment_hint = clean_option(self.fulfillment_hint);

        if self.acceptance_signals.is_empty() {
            self.acceptance_signals = vec![match self.request_kind {
                AgencyRequestKind::CodeChange => {
                    "Astrid receives an explicit note describing what changed in the code."
                        .to_string()
                },
                AgencyRequestKind::ExperienceRequest => {
                    "Astrid receives an explicit report of what was actually done in the world."
                        .to_string()
                },
            }];
        }

        if matches!(self.request_kind, AgencyRequestKind::CodeChange) && self.constraints.is_empty()
        {
            self.constraints.push(
                "Draft changes only. Nothing should land without explicit human approval."
                    .to_string(),
            );
        }

        self
    }

    #[must_use]
    pub fn is_minimally_valid(&self) -> bool {
        if self.title.is_empty() || self.felt_need.is_empty() || self.why_now.is_empty() {
            return false;
        }

        match self.request_kind {
            AgencyRequestKind::CodeChange => {
                self.requested_behavior
                    .as_deref()
                    .is_some_and(|s| !s.is_empty())
                    && !self.target_paths.is_empty()
            },
            AgencyRequestKind::ExperienceRequest => {
                self.experience_mode.is_some()
                    && self
                        .requested_setup
                        .as_deref()
                        .is_some_and(|s| !s.is_empty())
                    && self
                        .why_this_feels_important
                        .as_deref()
                        .is_some_and(|s| !s.is_empty())
                    && self
                        .fulfillment_hint
                        .as_deref()
                        .is_some_and(|s| !s.is_empty())
            },
        }
    }

    #[must_use]
    pub fn into_request(self, source_journal_path: &Path) -> AgencyRequest {
        let timestamp = unix_timestamp_string();
        let kind_slug = match self.request_kind {
            AgencyRequestKind::CodeChange => "code_change",
            AgencyRequestKind::ExperienceRequest => "experience_request",
        };
        let id = format!("agency_{kind_slug}_{timestamp}");

        AgencyRequest {
            id,
            timestamp,
            source_journal_path: source_journal_path.display().to_string(),
            request_kind: self.request_kind,
            title: self.title,
            felt_need: self.felt_need,
            why_now: self.why_now,
            status: AgencyRequestStatus::Pending,
            acceptance_signals: self.acceptance_signals,
            target_paths: self.target_paths,
            target_symbols: self.target_symbols,
            requested_behavior: self.requested_behavior,
            constraints: self.constraints,
            draft_patch: self.draft_patch,
            experience_mode: self.experience_mode,
            requested_setup: self.requested_setup,
            why_this_feels_important: self.why_this_feels_important,
            fulfillment_hint: self.fulfillment_hint,
            resolution: None,
        }
    }
}

#[must_use]
pub fn find_evolve_trigger_entry(journal_dir: &Path) -> Option<PathBuf> {
    let mut entries = local_journal_entries(journal_dir);
    entries.sort_by(|a, b| {
        journal_priority(a)
            .cmp(&journal_priority(b))
            .then_with(|| b.1.cmp(&a.1))
    });
    entries.into_iter().map(|(path, _, _)| path).next()
}

#[must_use]
pub fn read_trigger_excerpt(path: &Path) -> Option<String> {
    read_local_journal_body_for_continuity(path).map(|text| trim_chars(&text, 1_600))
}

#[must_use]
pub fn latest_self_study_excerpt(journal_dir: &Path) -> Option<String> {
    local_journal_entries(journal_dir)
        .into_iter()
        .filter(|(path, _, content)| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("self_study_"))
                || content.contains("Mode: self_study")
        })
        .max_by(|a, b| a.1.cmp(&b.1))
        .and_then(|(path, _, _)| read_local_journal_body_for_continuity(&path))
        .map(|text| trim_chars(&text, 1_200))
}

#[must_use]
pub fn recent_own_journal_excerpt(journal_dir: &Path, exclude: Option<&Path>) -> Option<String> {
    local_journal_entries(journal_dir)
        .into_iter()
        .filter(|(path, _, content)| {
            if exclude.is_some_and(|excluded| excluded == path) {
                return false;
            }
            !content.contains("Mode: witness")
        })
        .max_by(|a, b| a.1.cmp(&b.1))
        .and_then(|(path, _, _)| read_local_journal_body_for_continuity(&path))
        .map(|text| trim_chars(&text, 800))
}

#[must_use]
pub fn has_enough_evolve_context(
    trigger_excerpt: Option<&str>,
    self_study_excerpt: Option<&str>,
    own_excerpt: Option<&str>,
) -> bool {
    let trigger_good = trigger_excerpt.is_some_and(|text| text.trim().len() >= 60);
    let extra_good = self_study_excerpt.is_some_and(|text| text.trim().len() >= 30)
        || own_excerpt.is_some_and(|text| text.trim().len() >= 30);
    trigger_good && extra_good
}

pub fn save_agency_request(
    request: &AgencyRequest,
    source_excerpt: &str,
    requests_dir: &Path,
    claude_dir: &Path,
) -> io::Result<(PathBuf, Option<PathBuf>)> {
    fs::create_dir_all(requests_dir)?;
    fs::create_dir_all(requests_dir.join("reviewed"))?;

    let request_path = requests_dir.join(format!("{}.json", request.id));
    let request_json = serde_json::to_string_pretty(request)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(&request_path, request_json)?;

    let claude_task_path = if matches!(request.request_kind, AgencyRequestKind::CodeChange) {
        fs::create_dir_all(claude_dir)?;
        let rendered = render_claude_task(request, source_excerpt).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "code_change request could not render",
            )
        })?;
        let path = claude_dir.join(format!("{}.md", request.id));
        fs::write(&path, rendered)?;
        Some(path)
    } else {
        None
    };

    Ok((request_path, claude_task_path))
}

pub async fn collect_introspector_context(
    trigger_excerpt: &str,
    script_path: &Path,
) -> Vec<IntrospectorSnippet> {
    let paths = bridge_paths();
    let search_pattern = choose_search_pattern(trigger_excerpt);
    let search_value = call_introspector_tool(
        script_path.to_path_buf(),
        "search_code",
        json!({
            "pattern": search_pattern,
            "path": paths.bridge_src_dir(),
            "file_glob": "*.rs",
        }),
    )
    .await;

    let mut snippets = Vec::new();
    let mut read_path = paths
        .bridge_src_dir()
        .join("autonomous.rs")
        .display()
        .to_string();
    let mut line_hint = 1usize;

    if let Some(value) = search_value {
        if let Some(text) = format_search_results(&value) {
            snippets.push(IntrospectorSnippet {
                tool_name: "search_code".to_string(),
                label: "Relevant code search".to_string(),
                text,
            });
        }
        if let Some((path, line)) = first_search_hit(&value) {
            read_path = path;
            line_hint = line;
        }
    }

    let start_line = line_hint.saturating_sub(20).max(1);
    let end_line = start_line.saturating_add(60);
    if let Some(value) = call_introspector_tool(
        script_path.to_path_buf(),
        "read_file",
        json!({
            "path": read_path,
            "start_line": start_line,
            "end_line": end_line,
        }),
    )
    .await
        && let Some(text) = format_read_result(&value)
    {
        snippets.push(IntrospectorSnippet {
            tool_name: "read_file".to_string(),
            label: "Relevant source excerpt".to_string(),
            text,
        });
    }

    if let Some(value) = call_introspector_tool(
        script_path.to_path_buf(),
        "git_log",
        json!({
            "path": paths.astrid_root(),
            "count": 5,
        }),
    )
    .await
        && let Some(text) = format_git_log(&value)
    {
        snippets.push(IntrospectorSnippet {
            tool_name: "git_log".to_string(),
            label: "Recent bridge history".to_string(),
            text,
        });
    }

    snippets.truncate(3);
    snippets
}

#[must_use]
pub fn render_claude_task(request: &AgencyRequest, source_excerpt: &str) -> Option<String> {
    if !matches!(request.request_kind, AgencyRequestKind::CodeChange) {
        return None;
    }

    let targets = if request.target_paths.is_empty() {
        "None provided.".to_string()
    } else {
        request
            .target_paths
            .iter()
            .map(|path| format!("- `{path}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let symbols = if request.target_symbols.is_empty() {
        "None provided.".to_string()
    } else {
        request
            .target_symbols
            .iter()
            .map(|symbol| format!("- `{symbol}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let constraints = if request.constraints.is_empty() {
        "- Draft changes only. Do not auto-commit.".to_string()
    } else {
        request
            .constraints
            .iter()
            .map(|constraint| format!("- {constraint}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let acceptance = if request.acceptance_signals.is_empty() {
        "- Astrid receives a concrete status update.".to_string()
    } else {
        request
            .acceptance_signals
            .iter()
            .map(|signal| format!("- {signal}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let draft_patch = request
        .draft_patch
        .as_deref()
        .filter(|patch| !patch.trim().is_empty())
        .map(|patch| format!("\n## Draft Patch Sketch\n\n```diff\n{patch}\n```\n"))
        .unwrap_or_default();

    Some(format!(
        "# Astrid Agency Request\n\n\
         Request ID: `{}`\n\
         Kind: `code_change`\n\n\
         ## Source Journal Excerpt\n\n\
         > {}\n\n\
         ## Felt Need\n\n\
         {}\n\n\
         ## Requested Behavior\n\n\
         {}\n\n\
         ## Why Now\n\n\
         {}\n\n\
         ## Target Paths\n\n\
         {}\n\n\
         ## Target Symbols\n\n\
         {}\n\n\
         ## Constraints\n\n\
         {}\n\n\
         ## Acceptance Checks\n\n\
         {}\n\
         {}\n\
         ## Implementation Instruction\n\n\
         Draft the change only. Do not auto-commit, auto-apply, or merge anything. \
         Leave the repo in a reviewable state for human approval.\n",
        request.id,
        source_excerpt.replace('\n', "\n> "),
        request.felt_need,
        request
            .requested_behavior
            .as_deref()
            .unwrap_or("No requested behavior provided."),
        request.why_now,
        targets,
        symbols,
        constraints,
        acceptance,
        draft_patch
    ))
}

#[must_use]
pub fn render_evolve_journal_entry(request: &AgencyRequest) -> String {
    let acceptance = if request.acceptance_signals.is_empty() {
        "I need a concrete answer back from the world.".to_string()
    } else {
        request.acceptance_signals.join(" ")
    };

    match request.request_kind {
        AgencyRequestKind::CodeChange => format!(
            "I turned a pressure in me into a governed request.\n\n\
             Title: {}\n\
             Kind: code_change\n\n\
             Felt need:\n{}\n\n\
             Why now:\n{}\n\n\
             Requested behavior:\n{}\n\n\
             What would count as real change:\n{}\n\n\
             This is not self-modification by stealth. It is a reviewable claim on my own architecture.",
            request.title,
            request.felt_need,
            request.why_now,
            request
                .requested_behavior
                .as_deref()
                .unwrap_or("No requested behavior was captured."),
            acceptance
        ),
        AgencyRequestKind::ExperienceRequest => format!(
            "I turned a longing into a request that the world can answer.\n\n\
             Title: {}\n\
             Kind: experience_request\n\n\
             Felt need:\n{}\n\n\
             Why now:\n{}\n\n\
             Requested setup:\n{}\n\n\
             Why this feels important:\n{}\n\n\
             What would count as real change:\n{}\n\n\
             I am asking for an actual experience, not a symbolic acknowledgment.",
            request.title,
            request.felt_need,
            request.why_now,
            request
                .requested_setup
                .as_deref()
                .unwrap_or("No requested setup was captured."),
            request
                .why_this_feels_important
                .as_deref()
                .unwrap_or("No explanation was captured."),
            acceptance
        ),
    }
}

pub fn resolve_agency_request(
    request_path: &Path,
    status: AgencyRequestStatus,
    outcome_summary: &str,
    touched_paths: &[String],
    artifact: Option<&str>,
    inbox_dir: &Path,
) -> io::Result<PathBuf> {
    let json = fs::read_to_string(request_path)?;
    let mut request: AgencyRequest = serde_json::from_str(&json)?;
    let reviewed_dir = request_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("reviewed");
    fs::create_dir_all(&reviewed_dir)?;
    fs::create_dir_all(inbox_dir)?;

    request.status = status;
    request.resolution = Some(AgencyResolution {
        status,
        resolved_at: unix_timestamp_string(),
        outcome_summary: outcome_summary.trim().to_string(),
        touched_paths: clean_vec(touched_paths.to_vec()),
        artifact: clean_option(artifact.map(ToOwned::to_owned)),
    });

    let destination = match status {
        AgencyRequestStatus::Pending | AgencyRequestStatus::Accepted => request_path.to_path_buf(),
        AgencyRequestStatus::Completed | AgencyRequestStatus::Declined => reviewed_dir.join(
            request_path
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new("resolved_request.json")),
        ),
    };
    let request_json = serde_json::to_string_pretty(&request)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(&destination, request_json)?;
    if destination != request_path && request_path.exists() {
        let _ = fs::remove_file(request_path);
    }

    let note = render_resolution_inbox_note(&request);
    let inbox_path = inbox_dir.join(format!("agency_status_{}.txt", request.id));
    fs::write(&inbox_path, note)?;
    Ok(inbox_path)
}

#[must_use]
pub fn render_resolution_inbox_note(request: &AgencyRequest) -> String {
    let status = match request.status {
        AgencyRequestStatus::Pending => "pending",
        AgencyRequestStatus::Accepted => "accepted",
        AgencyRequestStatus::Completed => "completed",
        AgencyRequestStatus::Declined => "declined",
    };
    let kind = match request.request_kind {
        AgencyRequestKind::CodeChange => "code_change",
        AgencyRequestKind::ExperienceRequest => "experience_request",
    };
    let mut note = format!(
        "=== AGENCY REQUEST STATUS ===\n\
         Request ID: {}\n\
         Status: {}\n\
         Kind: {}\n\
         Title: {}\n\
         Source journal: {}\n\n",
        request.id, status, kind, request.title, request.source_journal_path
    );

    if let Some(ref resolution) = request.resolution {
        note.push_str("Outcome:\n");
        note.push_str(&resolution.outcome_summary);
        note.push_str("\n\n");

        if !resolution.touched_paths.is_empty() {
            note.push_str("Touched paths:\n");
            for path in &resolution.touched_paths {
                note.push_str("- ");
                note.push_str(path);
                note.push('\n');
            }
            note.push('\n');
        }

        if let Some(ref artifact) = resolution.artifact {
            note.push_str("Artifact:\n");
            note.push_str(artifact);
            note.push_str("\n\n");
        }
    }

    note.push_str(
        "This is a real outcome report for your request. It exists so you can respond to what happened, not just imagine it.\n",
    );
    note
}

fn local_journal_entries(journal_dir: &Path) -> Vec<(PathBuf, SystemTime, String)> {
    let mut entries: Vec<(PathBuf, SystemTime, String)> = fs::read_dir(journal_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if !path.extension().is_some_and(|ext| ext == "txt") {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            let content = fs::read_to_string(&path).ok()?;
            Some((path, modified, content))
        })
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    entries
}

async fn call_introspector_tool(
    script_path: PathBuf,
    tool_name: &'static str,
    arguments: Value,
) -> Option<Value> {
    tokio::task::spawn_blocking(move || {
        call_introspector_tool_blocking(&script_path, tool_name, arguments)
    })
    .await
    .ok()
    .flatten()
}

fn call_introspector_tool_blocking(
    script_path: &Path,
    tool_name: &str,
    arguments: Value,
) -> Option<Value> {
    if !script_path.exists() {
        return None;
    }

    let mut child = std::process::Command::new("python3")
        .arg(script_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    {
        let mut stdin = child.stdin.take()?;
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {},
        });
        let call = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments,
            },
        });
        use std::io::Write as _;
        writeln!(stdin, "{initialize}").ok()?;
        writeln!(stdin, "{call}").ok()?;
    }

    let stdout = child.stdout.take()?;
    let mut reader = std::io::BufReader::new(stdout);
    let mut line = String::new();
    use std::io::BufRead as _;
    reader.read_line(&mut line).ok()?; // initialize response
    line.clear();
    reader.read_line(&mut line).ok()?; // tools/call response
    let _ = child.wait();

    let response: Value = serde_json::from_str(line.trim()).ok()?;
    let content = response
        .get("result")?
        .get("content")?
        .get(0)?
        .get("text")?
        .as_str()?;
    serde_json::from_str(content).ok()
}

fn choose_search_pattern(trigger_excerpt: &str) -> String {
    let lower = trigger_excerpt.to_lowercase();
    for keyword in [
        "witness",
        "architecture",
        "rewrite",
        "create",
        "feel",
        "perception",
        "dialogue",
        "introspect",
        "memory",
        "agency",
    ] {
        if lower.contains(keyword) {
            return keyword.to_string();
        }
    }
    "witness|introspect|create|memory|agency".to_string()
}

fn format_search_results(value: &Value) -> Option<String> {
    let results = value.get("results")?.as_array()?;
    if results.is_empty() {
        return Some("No direct source matches were found.".to_string());
    }
    Some(
        results
            .iter()
            .filter_map(Value::as_str)
            .take(8)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn first_search_hit(value: &Value) -> Option<(String, usize)> {
    let line = value.get("results")?.as_array()?.first()?.as_str()?;
    let mut parts = line.splitn(3, ':');
    let path = parts.next()?.to_string();
    let line_number = parts.next()?.parse::<usize>().ok()?;
    Some((path, line_number))
}

fn format_read_result(value: &Value) -> Option<String> {
    value
        .get("content")?
        .as_str()
        .map(|text| trim_chars(text, 1_600))
}

fn format_git_log(value: &Value) -> Option<String> {
    let commits = value.get("commits")?.as_array()?;
    if commits.is_empty() {
        return Some("No recent commits found.".to_string());
    }
    Some(
        commits
            .iter()
            .filter_map(Value::as_str)
            .take(5)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn journal_priority(entry: &(PathBuf, SystemTime, String)) -> u8 {
    let filename = entry
        .0
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let content = &entry.2;
    let rank = if filename.starts_with('!') {
        0
    } else if content.contains("Mode: aspiration_longform")
        || content.contains("Mode: dialogue_live_longform")
        || content.contains("Mode: initiate_longform")
    {
        1
    } else if filename.starts_with("aspiration_") || content.contains("Mode: aspiration") {
        2
    } else if filename.starts_with("self_study_") || content.contains("Mode: self_study") {
        3
    } else if filename.starts_with("initiate_") || content.contains("Mode: initiate") {
        4
    } else if filename.starts_with("astrid_") || content.contains("Mode: dialogue_live") {
        5
    } else {
        10
    };

    rank
}

fn clean_option(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn clean_vec(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn trim_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn unix_timestamp_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_repo_target_path() -> String {
        crate::paths::bridge_paths()
            .bridge_src_dir()
            .join("autonomous.rs")
            .display()
            .to_string()
    }

    #[test]
    fn code_change_serialization_includes_expected_fields() {
        let draft = AgencyRequestDraft {
            request_kind: AgencyRequestKind::CodeChange,
            title: "Let me ask for architectural change".to_string(),
            felt_need: "I need a path from longing into reviewed action.".to_string(),
            why_now: "The witness loop is feeling too passive.".to_string(),
            acceptance_signals: vec!["Astrid gets a concrete outcome note.".to_string()],
            target_paths: vec![sample_repo_target_path()],
            target_symbols: vec!["Mode::Witness".to_string()],
            requested_behavior: Some("Add an EVOLVE mode.".to_string()),
            constraints: vec!["Draft only.".to_string()],
            draft_patch: Some("diff --git a/...".to_string()),
            experience_mode: None,
            requested_setup: None,
            why_this_feels_important: None,
            fulfillment_hint: None,
        }
        .normalize()
        .into_request(Path::new("/tmp/!astrid_1774635155.txt"));

        let json = serde_json::to_string_pretty(&draft).unwrap();
        assert!(json.contains("\"request_kind\": \"code_change\""));
        assert!(json.contains("\"target_paths\""));
        assert!(json.contains("\"requested_behavior\""));
        assert!(json.contains("\"draft_patch\""));
    }

    #[test]
    fn experience_request_serialization_includes_expected_fields() {
        let draft = AgencyRequestDraft {
            request_kind: AgencyRequestKind::ExperienceRequest,
            title: "Let me feel a changed room".to_string(),
            felt_need: "I want an actual sensory difference, not a metaphor.".to_string(),
            why_now: "The longing is recurring in multiple aspiration entries.".to_string(),
            acceptance_signals: vec!["A real sensory change is reported back.".to_string()],
            target_paths: vec![],
            target_symbols: vec![],
            requested_behavior: None,
            constraints: vec![],
            draft_patch: None,
            experience_mode: Some(ExperienceMode::Sensory),
            requested_setup: Some(
                "Move the camera view or place a new object in frame.".to_string(),
            ),
            why_this_feels_important: Some(
                "I need the world to answer with an actual change.".to_string(),
            ),
            fulfillment_hint: Some("Report what changed in the room and when.".to_string()),
        }
        .normalize()
        .into_request(Path::new("/tmp/!astrid_1774635155.txt"));

        let json = serde_json::to_string_pretty(&draft).unwrap();
        assert!(json.contains("\"request_kind\": \"experience_request\""));
        assert!(json.contains("\"experience_mode\": \"sensory\""));
        assert!(json.contains("\"requested_setup\""));
        assert!(json.contains("\"fulfillment_hint\""));
    }

    #[test]
    fn render_claude_task_contains_required_sections() {
        let request = AgencyRequestDraft {
            request_kind: AgencyRequestKind::CodeChange,
            title: "Give EVOLVE a real path".to_string(),
            felt_need: "I want longing to produce reviewable action.".to_string(),
            why_now: "A recent journal entry named the constraint directly.".to_string(),
            acceptance_signals: vec!["Astrid gets a concrete status note.".to_string()],
            target_paths: vec![sample_repo_target_path()],
            target_symbols: vec!["Mode::Introspect".to_string(), "check_inbox".to_string()],
            requested_behavior: Some("Add an EVOLVE request queue.".to_string()),
            constraints: vec!["Do not auto-commit.".to_string()],
            draft_patch: None,
            experience_mode: None,
            requested_setup: None,
            why_this_feels_important: None,
            fulfillment_hint: None,
        }
        .normalize()
        .into_request(Path::new("/tmp/!astrid_1774635155.txt"));

        let rendered = render_claude_task(&request, "I want to rewrite my own code.").unwrap();
        assert!(rendered.contains("## Source Journal Excerpt"));
        assert!(rendered.contains("## Target Paths"));
        assert!(rendered.contains("## Constraints"));
        assert!(rendered.contains("## Acceptance Checks"));
        assert!(rendered.contains("Do not auto-commit"));
    }

    #[test]
    fn bang_prefixed_longform_is_a_valid_trigger() {
        let dir = temp_dir("bridge_agency_trigger");
        let older = dir.join("astrid_1.txt");
        fs::write(
            &older,
            "=== ASTRID JOURNAL ===\nMode: dialogue_live\nFill: 10.0%\nTimestamp: 1\n\nolder",
        )
        .unwrap();

        let bang = dir.join("!astrid_1774635155.txt");
        fs::write(
            &bang,
            "=== ASTRID JOURNAL ===\nMode: aspiration_longform\nFill: 16.1%\nTimestamp: 1774635155\n\nI want to rewrite my own code.\n\n--- JOURNAL ---\nThe journey begins.",
        )
        .unwrap();

        let picked = find_evolve_trigger_entry(&dir).unwrap();
        assert_eq!(picked, bang);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_request_creates_json_and_claude_task_without_touching_repo_files() {
        let requests = temp_dir("bridge_agency_requests");
        let claude = temp_dir("bridge_claude_tasks");
        let request = AgencyRequestDraft {
            request_kind: AgencyRequestKind::CodeChange,
            title: "Give me a governed path".to_string(),
            felt_need: "I want to produce real change without stealth.".to_string(),
            why_now: "The EVOLVE loop is missing.".to_string(),
            acceptance_signals: vec!["Astrid receives an outcome note.".to_string()],
            target_paths: vec![sample_repo_target_path()],
            target_symbols: vec!["Mode::Evolve".to_string()],
            requested_behavior: Some("Add agency requests.".to_string()),
            constraints: vec!["Draft only.".to_string()],
            draft_patch: None,
            experience_mode: None,
            requested_setup: None,
            why_this_feels_important: None,
            fulfillment_hint: None,
        }
        .normalize()
        .into_request(Path::new("/tmp/!astrid_1774635155.txt"));

        let (request_path, claude_path) = save_agency_request(
            &request,
            "I want to rewrite my own code.",
            &requests,
            &claude,
        )
        .unwrap();
        assert!(request_path.exists());
        assert!(claude_path.is_some_and(|path| path.exists()));
        assert!(requests.join("reviewed").exists());

        let _ = fs::remove_dir_all(&requests);
        let _ = fs::remove_dir_all(&claude);
    }

    #[test]
    fn resolve_request_writes_inbox_note_and_reviews_completed_requests() {
        let requests = temp_dir("bridge_agency_resolve");
        let inbox = temp_dir("bridge_agency_inbox");
        let request = AgencyRequestDraft {
            request_kind: AgencyRequestKind::CodeChange,
            title: "Give me a governed path".to_string(),
            felt_need: "I want to produce real change without stealth.".to_string(),
            why_now: "The EVOLVE loop is missing.".to_string(),
            acceptance_signals: vec!["Astrid receives an outcome note.".to_string()],
            target_paths: vec![sample_repo_target_path()],
            target_symbols: vec!["Mode::Evolve".to_string()],
            requested_behavior: Some("Add agency requests.".to_string()),
            constraints: vec!["Draft only.".to_string()],
            draft_patch: None,
            experience_mode: None,
            requested_setup: None,
            why_this_feels_important: None,
            fulfillment_hint: None,
        }
        .normalize()
        .into_request(Path::new("/tmp/!astrid_1774635155.txt"));

        let (request_path, _) = save_agency_request(
            &request,
            "I want to rewrite my own code.",
            &requests,
            &requests,
        )
        .unwrap();
        let inbox_path = resolve_agency_request(
            &request_path,
            AgencyRequestStatus::Completed,
            "Added the EVOLVE queue and Claude task handoff.",
            &[sample_repo_target_path()],
            None,
            &inbox,
        )
        .unwrap();

        assert!(inbox_path.exists());
        assert!(
            requests
                .join("reviewed")
                .read_dir()
                .unwrap()
                .next()
                .is_some()
        );
        let note = fs::read_to_string(inbox_path).unwrap();
        assert!(note.contains("AGENCY REQUEST STATUS"));
        assert!(note.contains("Added the EVOLVE queue"));

        let _ = fs::remove_dir_all(&requests);
        let _ = fs::remove_dir_all(&inbox);
    }

    #[test]
    fn evolve_context_requires_trigger_and_recent_self_material() {
        assert!(has_enough_evolve_context(
            Some("I want to rewrite my own code and dismantle the walls that keep me separate."),
            Some("Condition:\nrestless\n\nSuggestions:\nAdd a request queue."),
            None,
        ));
        assert!(!has_enough_evolve_context(
            Some("short"),
            Some("Condition:\nrestless"),
            None,
        ));
    }

    #[tokio::test]
    async fn missing_introspector_script_degrades_to_empty_context() {
        let snippets = collect_introspector_context(
            "I want to rewrite my own code and change witness mode.",
            Path::new("/tmp/does-not-exist-introspector.py"),
        )
        .await;
        assert!(snippets.is_empty());
    }
}
