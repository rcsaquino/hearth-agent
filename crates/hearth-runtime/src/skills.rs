//! The Markdown skill library, the `skill` tool, and the prompt index.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hearth_model::ToolSpec;
use hearth_tools::{Tool, ToolError, ToolOutput};
use serde_json::{Value, json};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The prompt index token cap from section 9.6.
pub const INDEX_MAX_TOKENS: i64 = 300;
/// The prompt index active-skill cap from section 9.6.
pub const INDEX_MAX_ACTIVE: usize = 50;

/// Where a skill file lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillScope {
    User,
    Learned,
}

impl SkillScope {
    /// The stored word for this scope.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Learned => "learned",
        }
    }
}

/// The lifecycle state of a skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillStatus {
    Draft,
    Active,
    Quarantined,
    Archived,
}

impl SkillStatus {
    /// The stored word for this status.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Quarantined => "quarantined",
            Self::Archived => "archived",
        }
    }

    /// Parse a stored status.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "draft" => Some(Self::Draft),
            "active" => Some(Self::Active),
            "quarantined" => Some(Self::Quarantined),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }
}

/// One parsed skill.
#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub version: u64,
    pub status: SkillStatus,
    pub created: String,
    pub updated: String,
    pub last_used: Option<String>,
    pub uses: u64,
    pub successes: u64,
    pub failures: u64,
    pub evidence: Vec<String>,
    pub scope: SkillScope,
    pub path: PathBuf,
    pub body: String,
}

impl Skill {
    /// The success rate used by the index cap and the CLI.
    pub fn success_rate(&self) -> f64 {
        if self.uses == 0 {
            0.0
        } else {
            self.successes as f64 / self.uses as f64
        }
    }

    fn index_line(&self) -> String {
        format!("- {}: {}", self.name, self.description)
    }
}

/// The values needed to save or revise a learned skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSave {
    pub name: String,
    pub description: String,
    pub body: String,
    pub evidence: Vec<String>,
}

/// Counts and top skills for `hearth skills stats`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillStats {
    pub total: usize,
    pub active: usize,
    pub draft: usize,
    pub quarantined: usize,
    pub archived: usize,
    pub user: usize,
    pub learned: usize,
    pub top: Vec<(String, u64)>,
}

/// A skill library failure.
#[derive(Debug, Error)]
pub enum SkillError {
    #[error("frontmatter: {0}")]
    Frontmatter(String),
    #[error("invalid skill: {0}")]
    Invalid(String),
    #[error("skill is read-only: {0}")]
    ReadOnly(String),
    #[error("skill not found: {0}")]
    NotFound(String),
    #[error("skill file operation failed: {0}")]
    Io(#[from] std::io::Error),
}

/// The skill library rooted at one Hearth home.
#[derive(Debug, Clone)]
pub struct SkillLibrary {
    home: PathBuf,
}

impl SkillLibrary {
    /// Build a library rooted at `home`.
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }

    /// The user skill directory.
    pub fn skills_root(&self) -> PathBuf {
        self.home.join("skills")
    }

    /// The learned skill directory.
    pub fn learned_root(&self) -> PathBuf {
        self.skills_root().join("learned")
    }

    /// Parse every user and learned skill. Malformed frontmatter is an error.
    pub fn load_all(&self) -> Result<Vec<Skill>, SkillError> {
        let mut skills = Vec::new();
        let user = self.skills_root();
        if user.is_dir() {
            for entry in fs::read_dir(&user)? {
                let entry = entry?;
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if name == "learned" || name.starts_with('.') {
                    continue;
                }
                let file = if path.is_dir() {
                    path.join("SKILL.md")
                } else if path.extension().is_some_and(|extension| extension == "md") {
                    path
                } else {
                    continue;
                };
                if !file.is_file() {
                    continue;
                }
                skills.push(parse_file(&file, SkillScope::User, false)?);
            }
        }
        let learned = self.learned_root();
        if learned.is_dir() {
            for entry in fs::read_dir(&learned)? {
                let entry = entry?;
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if name == "CHANGELOG.md" || name.starts_with('.') || !name.ends_with(".md") {
                    continue;
                }
                if !path.is_file() {
                    continue;
                }
                skills.push(parse_file(&path, SkillScope::Learned, true)?);
            }
        }
        skills.sort_by(|left, right| {
            scope_rank(left.scope)
                .cmp(&scope_rank(right.scope))
                .then(left.name.cmp(&right.name))
        });
        Ok(skills)
    }

    /// The prompt skill index: active skills, capped at 300 tokens and 50 lines.
    /// Learned skills that no longer fit are archived. User skills are read-only.
    pub fn index(&self) -> Result<String, SkillError> {
        let skills = self.load_all()?;
        let mut users: Vec<Skill> = skills
            .iter()
            .filter(|skill| skill.scope == SkillScope::User && skill.status == SkillStatus::Active)
            .cloned()
            .collect();
        let mut learned: Vec<Skill> = skills
            .into_iter()
            .filter(|skill| {
                skill.scope == SkillScope::Learned && skill.status == SkillStatus::Active
            })
            .collect();
        users.sort_by(|left, right| left.name.cmp(&right.name));
        learned.sort_by(|left, right| {
            let rate = right
                .success_rate()
                .partial_cmp(&left.success_rate())
                .unwrap_or(std::cmp::Ordering::Equal);
            rate.then_with(|| right.last_used.cmp(&left.last_used))
                .then_with(|| left.name.cmp(&right.name))
        });

        let mut lines: Vec<String> = Vec::new();
        let mut active = 0usize;
        for skill in &users {
            if active >= INDEX_MAX_ACTIVE {
                break;
            }
            let line = skill.index_line();
            let mut candidate = lines.clone();
            candidate.push(line);
            if hearth_model::estimate_tokens(&candidate.join("\n")) > INDEX_MAX_TOKENS {
                break;
            }
            lines = candidate;
            active += 1;
        }
        for mut skill in learned {
            if active >= INDEX_MAX_ACTIVE {
                self.archive_for_index(&mut skill)?;
                continue;
            }
            let line = skill.index_line();
            let mut candidate = lines.clone();
            candidate.push(line);
            if hearth_model::estimate_tokens(&candidate.join("\n")) > INDEX_MAX_TOKENS {
                self.archive_for_index(&mut skill)?;
                continue;
            }
            lines = candidate;
            active += 1;
        }
        Ok(lines.join("\n"))
    }

    /// Find one skill by name. User skills are searched before learned skills.
    pub fn find(&self, name: &str) -> Result<Option<Skill>, SkillError> {
        Ok(self
            .load_all()?
            .into_iter()
            .find(|skill| skill.name == name))
    }

    /// The body of one skill.
    pub fn load_body(&self, name: &str) -> Result<Option<String>, SkillError> {
        Ok(self.find(name)?.map(|skill| skill.body))
    }

    /// Record one use of a learned skill. User skills are read-only.
    pub fn record_use(&self, name: &str) -> Result<Option<Skill>, SkillError> {
        let Some(mut skill) = self.find_learned(name)? else {
            return Ok(None);
        };
        skill.uses = skill.uses.saturating_add(1);
        skill.last_used = Some(now());
        write_skill(&skill)?;
        Ok(Some(skill))
    }

    /// Save a new learned skill or a new version of an existing one.
    pub fn save_learned(&self, save: SkillSave) -> Result<Skill, SkillError> {
        let name = save.name.clone();
        validate_name(&name)?;
        validate_body(&save.body)?;
        if save.description.contains('\n') {
            return Err(SkillError::Invalid(
                "the description must be one line".to_owned(),
            ));
        }
        if self
            .load_all()?
            .into_iter()
            .any(|skill| skill.scope == SkillScope::User && skill.name == name)
        {
            return Err(SkillError::ReadOnly(name));
        }

        let now = now();
        let path = self.learned_root().join(format!("{name}.md"));
        let existing = self.find_learned(&name)?;
        let mut skill = if let Some(existing) = existing {
            let history_dir = self.learned_root().join(".history").join(&name);
            fs::create_dir_all(&history_dir)?;
            fs::copy(
                &existing.path,
                history_dir.join(format!("v{}.md", existing.version)),
            )?;
            Skill {
                name: name.clone(),
                description: save.description,
                version: existing.version.saturating_add(1),
                status: existing.status,
                created: existing.created,
                updated: now,
                last_used: existing.last_used,
                uses: existing.uses,
                successes: existing.successes,
                failures: existing.failures,
                evidence: if save.evidence.is_empty() {
                    existing.evidence
                } else {
                    save.evidence
                },
                scope: SkillScope::Learned,
                path: path.clone(),
                body: save.body,
            }
        } else {
            Skill {
                name: name.clone(),
                description: save.description,
                version: 1,
                status: SkillStatus::Draft,
                created: now.clone(),
                updated: now,
                last_used: None,
                uses: 0,
                successes: 0,
                failures: 0,
                evidence: save.evidence,
                scope: SkillScope::Learned,
                path: path.clone(),
                body: save.body,
            }
        };
        skill.path = path;
        write_skill(&skill)?;
        self.append_changelog(&skill, "saved")?;
        Ok(skill)
    }

    /// Archive one learned skill.
    pub fn archive(&self, name: &str) -> Result<Skill, SkillError> {
        let Some(mut skill) = self.find(name)? else {
            return Err(SkillError::NotFound(name.to_owned()));
        };
        if skill.scope == SkillScope::User {
            return Err(SkillError::ReadOnly(name.to_owned()));
        }
        skill.status = SkillStatus::Archived;
        skill.updated = now();
        write_skill(&skill)?;
        self.append_changelog(&skill, "archived")?;
        Ok(skill)
    }

    /// Preserved versions of one learned skill, oldest first.
    pub fn history(&self, name: &str) -> Result<Vec<u64>, SkillError> {
        let mut versions = Vec::new();
        if let Some(current) = self.find_learned(name)? {
            versions.push(current.version);
        }
        let directory = self.learned_root().join(".history").join(name);
        if directory.is_dir() {
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let file = entry.file_name().to_string_lossy().to_string();
                if let Some(version) = file
                    .strip_prefix('v')
                    .and_then(|value| value.strip_suffix(".md"))
                    .and_then(|value| value.parse::<u64>().ok())
                {
                    versions.push(version);
                }
            }
        }
        versions.sort_unstable();
        versions.dedup();
        Ok(versions)
    }

    /// The full text of one version of a learned skill.
    pub fn version_body(&self, name: &str, version: u64) -> Result<Option<String>, SkillError> {
        if let Some(current) = self.find_learned(name)?
            && current.version == version
        {
            return Ok(Some(fs::read_to_string(&current.path)?));
        }
        let path = self
            .learned_root()
            .join(".history")
            .join(name)
            .join(format!("v{version}.md"));
        if path.is_file() {
            Ok(Some(fs::read_to_string(path)?))
        } else {
            Ok(None)
        }
    }

    /// Summary counts for the CLI.
    pub fn stats(&self) -> Result<SkillStats, SkillError> {
        let skills = self.load_all()?;
        let mut stats = SkillStats {
            total: skills.len(),
            ..SkillStats::default()
        };
        for skill in &skills {
            match skill.status {
                SkillStatus::Active => stats.active += 1,
                SkillStatus::Draft => stats.draft += 1,
                SkillStatus::Quarantined => stats.quarantined += 1,
                SkillStatus::Archived => stats.archived += 1,
            }
            match skill.scope {
                SkillScope::User => stats.user += 1,
                SkillScope::Learned => stats.learned += 1,
            }
        }
        let mut top: Vec<(String, u64)> = skills
            .into_iter()
            .map(|skill| (skill.name, skill.uses))
            .collect();
        top.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        top.truncate(5);
        stats.top = top;
        Ok(stats)
    }

    /// Re-read the user-owned skills for an index cap check.
    fn find_learned(&self, name: &str) -> Result<Option<Skill>, SkillError> {
        Ok(self
            .load_all()?
            .into_iter()
            .find(|skill| skill.name == name && skill.scope == SkillScope::Learned))
    }

    fn archive_for_index(&self, skill: &mut Skill) -> Result<(), SkillError> {
        skill.status = SkillStatus::Archived;
        skill.updated = now();
        write_skill(skill)?;
        self.append_changelog(skill, "archived by index cap")
    }

    fn append_changelog(&self, skill: &Skill, action: &str) -> Result<(), SkillError> {
        let path = self.learned_root().join("CHANGELOG.md");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut options = fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        writeln!(
            file,
            "- {} {} v{} {} {}",
            now(),
            skill.name,
            skill.version,
            skill.status.as_str(),
            action
        )?;
        Ok(())
    }
}

/// The model-facing skill tool.
pub struct SkillTool {
    library: Arc<SkillLibrary>,
}

impl SkillTool {
    /// Build the tool over one library.
    pub fn new(library: Arc<SkillLibrary>) -> Self {
        Self { library }
    }
}

impl Tool for SkillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "skill".to_owned(),
            description: "Load, save, or archive a Markdown skill. User-made skills are read-only; saves and archives apply to skills/learned/."
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["load", "save", "archive"]},
                    "name": {"type": "string"},
                    "description": {"type": "string"},
                    "body": {"type": "string"},
                    "evidence": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["action", "name"]
            }),
        }
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "action is required"))?;
        let name = arguments
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "name is required"))?;
        match action {
            "load" => {
                let skill = self
                    .library
                    .find(name)
                    .map_err(skill_tool_error)?
                    .ok_or_else(|| {
                        ToolError::invalid("not_found", format!("skill not found: {name}"))
                    })?;
                let skill = if skill.scope == SkillScope::Learned {
                    self.library
                        .record_use(name)
                        .map_err(skill_tool_error)?
                        .unwrap_or(skill)
                } else {
                    skill
                };
                Ok(ToolOutput {
                    content: format!(
                        "[{} v{} {}] {}\n---\n{}",
                        skill.name,
                        skill.version,
                        skill.status.as_str(),
                        skill.description,
                        skill.body
                    ),
                    is_error: false,
                })
            }
            "save" => {
                let description = arguments
                    .get("description")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ToolError::invalid("invalid_arguments", "description is required for save")
                    })?;
                let body = arguments
                    .get("body")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ToolError::invalid("invalid_arguments", "body is required for save")
                    })?;
                let evidence = arguments
                    .get("evidence")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                let skill = self
                    .library
                    .save_learned(SkillSave {
                        name: name.to_owned(),
                        description: description.to_owned(),
                        body: body.to_owned(),
                        evidence,
                    })
                    .map_err(skill_tool_error)?;
                Ok(ToolOutput {
                    content: format!(
                        "saved {} v{} ({})",
                        skill.name,
                        skill.version,
                        skill.status.as_str()
                    ),
                    is_error: false,
                })
            }
            "archive" => {
                let skill = self.library.archive(name).map_err(skill_tool_error)?;
                Ok(ToolOutput {
                    content: format!("archived {} v{}", skill.name, skill.version),
                    is_error: false,
                })
            }
            other => Err(ToolError::invalid(
                "unknown_action",
                format!("unknown skill action: {other}"),
            )),
        }
    }
}

fn skill_tool_error(error: SkillError) -> ToolError {
    ToolError::invalid("skill", error.to_string())
}

fn parse_file(
    path: &Path,
    scope: SkillScope,
    require_frontmatter: bool,
) -> Result<Skill, SkillError> {
    let text = fs::read_to_string(path)?;
    parse_skill(&text, path, scope, require_frontmatter)
}

fn parse_skill(
    text: &str,
    path: &Path,
    scope: SkillScope,
    require_frontmatter: bool,
) -> Result<Skill, SkillError> {
    let (front, body) = split_frontmatter(text)?;
    if front.is_none() && require_frontmatter {
        return Err(SkillError::Frontmatter(format!(
            "{} has no frontmatter",
            path.display()
        )));
    }
    let fields = match front {
        Some(front) => parse_fields(&front)?,
        None => BTreeMap::new(),
    };
    let default_name = default_name(path);
    let name = fields
        .get("name")
        .filter(|value| !value.is_empty())
        .cloned()
        .unwrap_or(default_name);
    let description = fields
        .get("description")
        .filter(|value| !value.is_empty())
        .cloned()
        .or_else(|| first_description(&body))
        .unwrap_or_default();
    let version = number(&fields, "version", 1)?;
    let status = match fields.get("status").map(String::as_str) {
        Some(value) if !value.is_empty() => SkillStatus::parse(value)
            .ok_or_else(|| SkillError::Frontmatter(format!("unknown skill status: {value}")))?,
        _ if scope == SkillScope::User => SkillStatus::Active,
        _ => SkillStatus::Draft,
    };
    Ok(Skill {
        name,
        description,
        version,
        status,
        created: fields.get("created").cloned().unwrap_or_default(),
        updated: fields.get("updated").cloned().unwrap_or_default(),
        last_used: fields
            .get("last_used")
            .filter(|value| !value.is_empty())
            .cloned(),
        uses: number(&fields, "uses", 0)?,
        successes: number(&fields, "successes", 0)?,
        failures: number(&fields, "failures", 0)?,
        evidence: fields
            .get("evidence")
            .map(|value| parse_list(value))
            .unwrap_or_default(),
        scope,
        path: path.to_path_buf(),
        body,
    })
}

fn split_frontmatter(text: &str) -> Result<(Option<String>, String), SkillError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return Ok((None, String::new()));
    };
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return Ok((None, text.to_owned()));
    }
    let mut front = String::new();
    let mut offset = first.len();
    let mut closed = false;
    for line in lines {
        let content = line.trim_end_matches(['\r', '\n']);
        offset += line.len();
        if content == "---" {
            closed = true;
            break;
        }
        front.push_str(line);
    }
    if !closed {
        return Err(SkillError::Frontmatter(
            "frontmatter is not closed".to_owned(),
        ));
    }
    Ok((
        Some(front),
        text[offset.min(text.len())..]
            .trim_start_matches(['\r', '\n'])
            .to_owned(),
    ))
}

fn parse_fields(front: &str) -> Result<BTreeMap<String, String>, SkillError> {
    let mut fields = BTreeMap::new();
    for line in front.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(SkillError::Frontmatter(format!(
                "invalid frontmatter line: {line}"
            )));
        };
        fields.insert(key.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    Ok(fields)
}

fn number(fields: &BTreeMap<String, String>, key: &str, default: u64) -> Result<u64, SkillError> {
    match fields.get(key).filter(|value| !value.is_empty()) {
        Some(value) => value
            .parse()
            .map_err(|_| SkillError::Frontmatter(format!("{key} is not a number: {value}"))),
        None => Ok(default),
    }
}

fn parse_list(value: &str) -> Vec<String> {
    let value = value.trim();
    let inner = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(value);
    inner
        .split(',')
        .map(|item| item.trim().trim_matches(['"', '\'']).to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

fn default_name(path: &Path) -> String {
    if path.file_name().is_some_and(|name| name == "SKILL.md")
        && let Some(parent) = path.parent().and_then(Path::file_name)
    {
        return parent.to_string_lossy().to_string();
    }
    path.file_stem()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "skill".to_owned())
}

fn first_description(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("---"))
        .map(str::to_owned)
}

fn validate_name(name: &str) -> Result<(), SkillError> {
    if name.is_empty()
        || !name.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
        || name.starts_with('.')
        || name.starts_with('-')
    {
        return Err(SkillError::Invalid(format!("invalid skill name: {name}")));
    }
    Ok(())
}

fn scope_rank(scope: SkillScope) -> u8 {
    match scope {
        SkillScope::User => 0,
        SkillScope::Learned => 1,
    }
}

fn validate_body(body: &str) -> Result<(), SkillError> {
    let lower = body.to_ascii_lowercase();
    if !lower.contains("when to use") || !lower.contains("steps") || !lower.contains("pitfalls") {
        return Err(SkillError::Invalid(
            "the body needs When to use, Steps, and Pitfalls sections".to_owned(),
        ));
    }
    Ok(())
}

fn render_skill(skill: &Skill) -> String {
    let last_used = skill.last_used.clone().unwrap_or_default();
    let evidence = skill.evidence.join(", ");
    format!(
        "---\nname: {}\ndescription: {}\nversion: {}\nstatus: {}\ncreated: {}\nupdated: {}\nlast_used: {}\nuses: {}\nsuccesses: {}\nfailures: {}\nevidence: [{}]\n---\n\n{}\n",
        skill.name,
        skill.description,
        skill.version,
        skill.status.as_str(),
        skill.created,
        skill.updated,
        last_used,
        skill.uses,
        skill.successes,
        skill.failures,
        evidence,
        skill.body
    )
}

fn write_skill(skill: &Skill) -> Result<(), SkillError> {
    if let Some(parent) = skill.path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = render_skill(skill);
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&skill.path)?;
    file.write_all(text.as_bytes())?;
    file.flush()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&skill.path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".to_owned())
}

/// The read-only CLI actions for `hearth skills`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillsCommand {
    List,
    Show {
        name: String,
    },
    History {
        name: String,
    },
    Diff {
        name: String,
        from: Option<u64>,
        to: Option<u64>,
    },
    Stats,
}

/// Run one `hearth skills` command.
pub fn run_cli(home: &Path, command: SkillsCommand) -> Result<(), SkillError> {
    let library = SkillLibrary::new(home);
    match command {
        SkillsCommand::List => {
            let skills = library.load_all()?;
            if skills.is_empty() {
                println!("no skills");
                return Ok(());
            }
            println!("name\tstatus\tversion\tuses\tsuccess\twhere");
            for skill in skills {
                println!(
                    "{}\t{}\tv{}\t{}\t{:.2}\t{}",
                    skill.name,
                    skill.status.as_str(),
                    skill.version,
                    skill.uses,
                    skill.success_rate(),
                    skill.scope.as_str()
                );
            }
        }
        SkillsCommand::Show { name } => {
            let skill = library
                .find(&name)?
                .ok_or_else(|| SkillError::NotFound(name.clone()))?;
            println!("{}", skill.body);
        }
        SkillsCommand::History { name } => {
            let versions = library.history(&name)?;
            if versions.is_empty() {
                return Err(SkillError::NotFound(name));
            }
            for version in versions {
                println!("v{version}");
            }
        }
        SkillsCommand::Diff { name, from, to } => {
            let versions = library.history(&name)?;
            if versions.is_empty() {
                return Err(SkillError::NotFound(name));
            }
            let from = from.unwrap_or_else(|| {
                versions
                    .get(versions.len().saturating_sub(2))
                    .copied()
                    .unwrap_or(1)
            });
            let to = to.unwrap_or_else(|| versions.last().copied().unwrap_or(from));
            let old = library
                .version_body(&name, from)?
                .ok_or_else(|| SkillError::NotFound(format!("{name} v{from}")))?;
            let new = library
                .version_body(&name, to)?
                .ok_or_else(|| SkillError::NotFound(format!("{name} v{to}")))?;
            println!("--- {name} v{from}");
            println!("+++ {name} v{to}");
            let old_lines: Vec<&str> = old.lines().collect();
            let new_lines: Vec<&str> = new.lines().collect();
            for line in &old_lines {
                if !new_lines.contains(line) {
                    println!("- {line}");
                }
            }
            for line in &new_lines {
                if !old_lines.contains(line) {
                    println!("+ {line}");
                }
            }
        }
        SkillsCommand::Stats => {
            let stats = library.stats()?;
            println!(
                "total {} active {} draft {} quarantined {} archived {} user {} learned {}",
                stats.total,
                stats.active,
                stats.draft,
                stats.quarantined,
                stats.archived,
                stats.user,
                stats.learned
            );
            for (name, uses) in stats.top {
                println!("{name}\t{uses}");
            }
        }
    }
    Ok(())
}
