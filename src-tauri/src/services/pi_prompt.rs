//! pi 的三类原生提示词资源：
//!
//! - 提示库：`AGENTS.md`，仍走 canonical prompt 的整文件接管；
//! - 系统提示：`APPEND_SYSTEM.md`（在 pi 自带系统提示后追加）与 `SYSTEM.md`（整份替换）；
//! - 斜杠命令模板：`prompts/<slug>.md`。
//!
//! 后两类是 pi 自己的文件，没有 canonical 副本，按 pi 的规则直管：
//! 写入前留底、原子写、带 revision 的外部改动检测。revision 由读取方持有，
//! 保存时对不上就拒绝，绝不用旧视图覆盖别人刚写的内容。

use crate::adapters::paths::expand_root;
use crate::domain::agent::AgentDescriptor;
use crate::error::AppError;
use crate::index::sha256_hex;
use crate::store::atomic::write_atomic;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MISSING_REVISION: &str = "missing";
const BACKUP_DIR: &str = "pi-prompts";
const BACKUP_KEEP: usize = 50;

/// 两个固定文件：存在即生效，没有启用开关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiPromptFileKind {
    /// 追加到 pi 自带系统提示之后；推荐用它做长期约定。
    SystemAppend,
    /// 整份替换 pi 自带的系统提示。
    SystemOverride,
}

impl PiPromptFileKind {
    fn filename(self) -> &'static str {
        match self {
            Self::SystemAppend => "APPEND_SYSTEM.md",
            Self::SystemOverride => "SYSTEM.md",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PiPromptFileSnapshot {
    pub exists: bool,
    pub revision: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PiPromptTemplate {
    pub slug: String,
    pub content: String,
    pub revision: String,
}

pub fn agent_dir(agent: &AgentDescriptor, home: &Path, base_override: Option<&Path>) -> PathBuf {
    expand_root(&agent.config_root, base_override, home)
}

pub fn read_file(
    agent: &AgentDescriptor,
    kind: PiPromptFileKind,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PiPromptFileSnapshot, AppError> {
    read_snapshot(&agent_dir(agent, home, base_override).join(kind.filename()))
}

pub fn replace_file(
    store: &Store,
    agent: &AgentDescriptor,
    kind: PiPromptFileKind,
    expected_revision: &str,
    content: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PiPromptFileSnapshot, AppError> {
    if content.trim().is_empty() {
        return Err(AppError::InvalidInput(
            "系统提示不能为空；不需要时请删除文件".into(),
        ));
    }
    validate_size(content)?;
    let root = agent_dir(agent, home, base_override);
    let path = root.join(kind.filename());
    ensure_revision(&path, expected_revision)?;
    backup_existing(store, &path)?;
    write_atomic(&path, content.as_bytes())?;
    read_snapshot(&path)
}

pub fn delete_file(
    store: &Store,
    agent: &AgentDescriptor,
    kind: PiPromptFileKind,
    expected_revision: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<bool, AppError> {
    let path = agent_dir(agent, home, base_override).join(kind.filename());
    ensure_revision(&path, expected_revision)?;
    backup_existing(store, &path)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub fn list_templates(
    agent: &AgentDescriptor,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<Vec<PiPromptTemplate>, AppError> {
    let dir = agent_dir(agent, home, base_override).join("prompts");
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };

    let mut templates = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }
        let Some(slug) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        if validate_slug(slug).is_err() {
            continue;
        }
        let snapshot = read_snapshot(&path)?;
        templates.push(PiPromptTemplate {
            slug: slug.to_string(),
            content: snapshot.content,
            revision: snapshot.revision,
        });
    }
    templates.sort_by(|left, right| left.slug.cmp(&right.slug));
    Ok(templates)
}

pub fn save_template(
    store: &Store,
    agent: &AgentDescriptor,
    slug: &str,
    original_slug: Option<&str>,
    expected_revision: &str,
    content: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PiPromptTemplate, AppError> {
    validate_slug(slug)?;
    if let Some(original_slug) = original_slug {
        validate_slug(original_slug)?;
    }
    validate_size(content)?;

    let dir = agent_dir(agent, home, base_override).join("prompts");
    let path = dir.join(format!("{slug}.md"));

    if let Some(original_slug) = original_slug.filter(|value| *value != slug) {
        // 改名 + 改内容是一步：先确认旧文件没被外部改过、新名字没被占用，
        // 再 rename；写入失败时把 rename 退回去，绝不留下两个半成品。
        let original_path = dir.join(format!("{original_slug}.md"));
        ensure_revision(&original_path, expected_revision)?;
        ensure_revision(&path, MISSING_REVISION)?;
        backup_existing(store, &original_path)?;
        fs::rename(&original_path, &path)?;
        if let Err(write_error) = write_atomic(&path, content.as_bytes()) {
            if let Err(rollback_error) = fs::rename(&path, &original_path) {
                return Err(AppError::InvalidInput(format!(
                    "模板保存失败（{write_error}）；rename 回滚也失败：{rollback_error}"
                )));
            }
            return Err(write_error.into());
        }
    } else {
        ensure_revision(&path, expected_revision)?;
        backup_existing(store, &path)?;
        write_atomic(&path, content.as_bytes())?;
    }

    Ok(PiPromptTemplate {
        slug: slug.to_string(),
        content: content.to_string(),
        revision: sha256_hex(content.as_bytes()),
    })
}

pub fn delete_template(
    store: &Store,
    agent: &AgentDescriptor,
    slug: &str,
    expected_revision: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<bool, AppError> {
    validate_slug(slug)?;
    let path = agent_dir(agent, home, base_override)
        .join("prompts")
        .join(format!("{slug}.md"));
    ensure_revision(&path, expected_revision)?;
    backup_existing(store, &path)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn read_snapshot(path: &Path) -> Result<PiPromptFileSnapshot, AppError> {
    match fs::File::open(path) {
        Ok(file) => {
            let bytes = read_limited(file, path)?;
            let content = String::from_utf8(bytes.clone()).map_err(|_| {
                AppError::InvalidInput(format!("{} 不是 UTF-8，无法编辑", path.display()))
            })?;
            Ok(PiPromptFileSnapshot {
                exists: true,
                revision: sha256_hex(&bytes),
                content,
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(PiPromptFileSnapshot {
            exists: false,
            revision: MISSING_REVISION.to_string(),
            content: String::new(),
        }),
        Err(error) => Err(error.into()),
    }
}

fn read_limited(file: fs::File, path: &Path) -> Result<Vec<u8>, AppError> {
    let metadata = file.metadata()?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err(AppError::InvalidInput(format!(
            "{} 超过 1 MiB，无法编辑",
            path.display()
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(AppError::InvalidInput(format!(
            "{} 超过 1 MiB，无法编辑",
            path.display()
        )));
    }
    Ok(bytes)
}

fn ensure_revision(path: &Path, expected: &str) -> Result<(), AppError> {
    let actual = match fs::File::open(path) {
        Ok(file) => sha256_hex(&read_limited(file, path)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => MISSING_REVISION.to_string(),
        Err(error) => return Err(error.into()),
    };
    if actual == expected {
        Ok(())
    } else {
        Err(AppError::InvalidInput(format!(
            "{} 在外部被改过，请刷新后再保存",
            path.display()
        )))
    }
}

fn validate_size(content: &str) -> Result<(), AppError> {
    if content.len() as u64 > MAX_FILE_BYTES {
        return Err(AppError::InvalidInput("内容超过 1 MiB".into()));
    }
    Ok(())
}

/// 模板名就是文件名，沿用项目的 ID 规则：只允许 `[A-Za-z0-9._-]`，并挡住 Windows 保留名。
fn validate_slug(slug: &str) -> Result<(), AppError> {
    let unsafe_name = !crate::store::is_safe_id(slug)
        || slug.starts_with('.')
        || slug.ends_with('.')
        || windows_reserved(slug);
    if unsafe_name {
        return Err(AppError::InvalidInput(
            "模板名只能包含字母、数字、点、下划线或连字符，且不能是 Windows 保留名".into(),
        ));
    }
    Ok(())
}

fn windows_reserved(slug: &str) -> bool {
    let base = slug
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        base.as_str(),
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "com5"
            | "com6"
            | "com7"
            | "com8"
            | "com9"
            | "lpt1"
            | "lpt2"
            | "lpt3"
            | "lpt4"
            | "lpt5"
            | "lpt6"
            | "lpt7"
            | "lpt8"
            | "lpt9"
    )
}

/// 覆盖或删除前把现有字节留一份到 `backups/pi-prompts/`；失败就让整次写入失败。
fn backup_existing(store: &Store, path: &Path) -> Result<(), AppError> {
    if !path.is_file() {
        return Ok(());
    }
    let dir = store.root().join("backups").join(BACKUP_DIR);
    fs::create_dir_all(&dir)?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("prompt.md");
    let stamp = chrono::Utc::now().timestamp_millis();
    let mut target = dir.join(format!("{stamp}_{name}"));
    let mut suffix = 1;
    while target.exists() {
        suffix += 1;
        target = dir.join(format!("{stamp}_{suffix}_{name}"));
    }
    fs::copy(path, &target)?;
    prune_backups(&dir);
    Ok(())
}

fn prune_backups(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut names = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    names.sort();
    let excess = names.len().saturating_sub(BACKUP_KEEP);
    for name in names.into_iter().take(excess) {
        let _ = fs::remove_file(dir.join(name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::builtin_agents;
    use tempfile::tempdir;

    fn pi() -> AgentDescriptor {
        builtin_agents()
            .into_iter()
            .find(|agent| agent.id == "pi")
            .expect("注册表里没有 pi")
    }

    #[test]
    fn template_names_stay_single_portable_ids() {
        for valid in ["review-pr", "release.v2"] {
            validate_slug(valid).expect(valid);
        }
        for invalid in [
            "",
            ".hidden",
            "with space",
            "a/b",
            r"a\b",
            "CON",
            "lpt1.txt",
            "评审",
        ] {
            assert!(validate_slug(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn system_file_round_trips_and_rejects_a_stale_revision() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        fs::create_dir_all(home.join(".pi/agent")).unwrap();
        let agent = pi();

        let missing = read_file(&agent, PiPromptFileKind::SystemAppend, &home, None).unwrap();
        assert!(!missing.exists);
        assert_eq!(missing.revision, MISSING_REVISION);

        let saved = replace_file(
            &store,
            &agent,
            PiPromptFileKind::SystemAppend,
            MISSING_REVISION,
            "长期约定\n",
            &home,
            None,
        )
        .unwrap();
        assert!(saved.exists);
        assert_eq!(saved.content, "长期约定\n");

        let stale = replace_file(
            &store,
            &agent,
            PiPromptFileKind::SystemAppend,
            MISSING_REVISION,
            "覆盖\n",
            &home,
            None,
        )
        .unwrap_err();
        assert!(stale.to_string().contains("在外部被改过"));
        assert_eq!(
            fs::read_to_string(home.join(".pi/agent/APPEND_SYSTEM.md")).unwrap(),
            "长期约定\n"
        );
    }

    #[test]
    fn templates_list_rename_and_delete_with_revision_checks() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        fs::create_dir_all(home.join(".pi/agent")).unwrap();
        let agent = pi();

        let created = save_template(
            &store,
            &agent,
            "draft",
            None,
            MISSING_REVISION,
            "/draft",
            &home,
            None,
        )
        .unwrap();
        assert_eq!(
            list_templates(&agent, &home, None).unwrap(),
            vec![created.clone()]
        );

        let renamed = save_template(
            &store,
            &agent,
            "review",
            Some("draft"),
            &created.revision,
            "/review",
            &home,
            None,
        )
        .unwrap();
        assert_eq!(renamed.slug, "review");
        assert!(!home.join(".pi/agent/prompts/draft.md").exists());
        assert!(home.join(".pi/agent/prompts/review.md").is_file());

        let error =
            delete_template(&store, &agent, "review", MISSING_REVISION, &home, None).unwrap_err();
        assert!(error.to_string().contains("在外部被改过"));

        assert!(delete_template(&store, &agent, "review", &renamed.revision, &home, None).unwrap());
        assert!(list_templates(&agent, &home, None).unwrap().is_empty());
    }
}
