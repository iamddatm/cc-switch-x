//! 会话发现、增量扫描与 provider 状态。

use crate::adapters::paths::expand_root;
use crate::domain::session::{ParsedSession, SessionProviderStatus};
use crate::error::AppError;
use crate::index::Index;
use crate::services::agents::load_state;
use crate::sessions::{discover_provider, providers, SessionProvider};
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionScanReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub failures: Vec<SessionFileFailure>,
    pub warnings: Vec<SessionFileWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFileFailure {
    pub agent_id: String,
    pub file: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFileWarning {
    pub agent_id: String,
    pub file: String,
    pub line: usize,
    pub message: String,
}

pub fn scan_all(
    store: &Store,
    index: &mut Index,
    home: &Path,
) -> Result<SessionScanReport, AppError> {
    let mut report = SessionScanReport::default();
    for provider in providers() {
        scan_provider(store, index, home, provider, &mut report)?;
    }
    Ok(report)
}

pub fn providers_status(
    index: &Index,
    store: &Store,
    home: &Path,
) -> Result<Vec<SessionProviderStatus>, AppError> {
    let mut out = Vec::new();
    for provider in providers() {
        let base_override = load_state(store, provider.agent_id)?.base_override;
        let expanded: Vec<PathBuf> = provider
            .roots
            .iter()
            .map(|root| expand_root(root, base_override.as_deref(), home))
            .collect();
        let roots = expanded
            .iter()
            .map(|root| root.to_string_lossy().into_owned())
            .collect();
        let root_exists = expanded.iter().any(|root| root.is_dir());
        let files = discover_provider(provider, home, base_override.as_deref());
        out.push(SessionProviderStatus {
            agent_id: provider.agent_id.to_string(),
            display_name: provider.display_name.to_string(),
            verified: provider.verified,
            parse_ready: provider.parse.is_some(),
            roots,
            root_exists,
            found_files: files.len(),
            session_count: index.session_count(provider.agent_id)?,
        });
    }
    Ok(out)
}

struct Candidate {
    parsed: ParsedSession,
    file: String,
    mtime: i64,
    size: i64,
    existed: bool,
}

fn scan_provider(
    store: &Store,
    index: &mut Index,
    home: &Path,
    provider: &SessionProvider,
    report: &mut SessionScanReport,
) -> Result<(), AppError> {
    let Some(parse) = provider.parse else {
        return Ok(());
    };
    let base_override = load_state(store, provider.agent_id)?.base_override;
    let files = discover_provider(provider, home, base_override.as_deref());
    let discovered: HashSet<String> = files
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    let stale: Vec<_> = index
        .session_files_for_agent(provider.agent_id)?
        .into_iter()
        .filter(|row| !discovered.contains(&row.source_path))
        .collect();

    let mut chosen: HashMap<String, Candidate> = HashMap::new();
    let mut parsed_sources: Vec<String> = Vec::new();

    for file in &files {
        let source = file.to_string_lossy().into_owned();
        let metadata = match std::fs::metadata(file) {
            Ok(metadata) => metadata,
            Err(error) => {
                report.failures.push(SessionFileFailure {
                    agent_id: provider.agent_id.to_string(),
                    file: source,
                    reason: error.to_string(),
                });
                continue;
            }
        };
        let size = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0);
        let previous_row = index.session_file(provider.agent_id, &source)?;
        if let Some(row) = &previous_row {
            if row.mtime == mtime && row.size == size && row.parse_error.is_none() {
                report.unchanged += 1;
                continue;
            }
        }

        let parsed_sessions = match parse(file) {
            Ok(sessions) => sessions,
            Err(error) => {
                let reason = error.to_string();
                index.record_session_file(
                    provider.agent_id,
                    &source,
                    previous_row
                        .as_ref()
                        .and_then(|row| row.session_id.as_deref()),
                    mtime,
                    size,
                    Some(&reason),
                )?;
                report.failures.push(SessionFileFailure {
                    agent_id: provider.agent_id.to_string(),
                    file: source,
                    reason,
                });
                continue;
            }
        };

        for session in &parsed_sessions {
            for warning in &session.warnings {
                report.warnings.push(SessionFileWarning {
                    agent_id: provider.agent_id.to_string(),
                    file: source.clone(),
                    line: warning.line,
                    message: warning.message.clone(),
                });
            }
        }
        index.record_session_file(
            provider.agent_id,
            &source,
            parsed_sessions
                .first()
                .map(|session| session.session_id.as_str()),
            mtime,
            size,
            None,
        )?;
        parsed_sources.push(source.clone());

        for parsed in parsed_sessions {
            let id = parsed.session_id.clone();
            let mut candidate = Candidate {
                parsed,
                file: source.clone(),
                mtime,
                size,
                existed: false,
            };
            if let Some(current) = chosen.get(&id) {
                if candidate_wins(&candidate.parsed, &current.parsed) {
                    record_duplicate(report, provider.agent_id, &current.file, &id);
                    chosen.insert(id, candidate);
                } else {
                    record_duplicate(report, provider.agent_id, &candidate.file, &id);
                }
                continue;
            }

            if let Some(previous) = index.session_by_id(provider.agent_id, &id)? {
                candidate.existed = true;
                if previous.source_path != source
                    && !candidate_wins_values(
                        candidate.parsed.updated_at,
                        candidate.parsed.messages.len(),
                        previous.updated_at,
                        previous.message_count as usize,
                    )
                {
                    record_duplicate(report, provider.agent_id, &candidate.file, &id);
                    continue;
                }
                if previous.source_path != source {
                    record_duplicate(report, provider.agent_id, &previous.source_path, &id);
                }
            }
            chosen.insert(id, candidate);
        }
    }

    let mut candidates: Vec<Candidate> = chosen.into_values().collect();
    candidates.sort_by(|left, right| {
        (&left.file, &left.parsed.session_id).cmp(&(&right.file, &right.parsed.session_id))
    });
    let mut winners_by_source: HashMap<String, Vec<String>> = HashMap::new();
    for candidate in candidates {
        if candidate.existed {
            report.updated += 1;
        } else {
            report.added += 1;
        }
        index.upsert_session(
            provider.agent_id,
            &candidate.parsed,
            &candidate.file,
            candidate.mtime,
            candidate.size,
        )?;
        winners_by_source
            .entry(candidate.file)
            .or_default()
            .push(candidate.parsed.session_id);
    }

    for source in parsed_sources {
        let keep = winners_by_source.remove(&source).unwrap_or_default();
        report.removed += index.delete_sessions_by_source(provider.agent_id, &source, &keep)?;
    }
    for row in stale {
        report.removed +=
            index.delete_sessions_by_source(provider.agent_id, &row.source_path, &[])?;
        index.delete_session_file(provider.agent_id, &row.source_path)?;
    }
    Ok(())
}

fn candidate_wins(candidate: &ParsedSession, current: &ParsedSession) -> bool {
    candidate_wins_values(
        candidate.updated_at,
        candidate.messages.len(),
        current.updated_at,
        current.messages.len(),
    )
}

fn candidate_wins_values(
    candidate_updated: i64,
    candidate_count: usize,
    current_updated: i64,
    current_count: usize,
) -> bool {
    candidate_updated > current_updated
        || (candidate_updated == current_updated && candidate_count > current_count)
}

fn record_duplicate(report: &mut SessionScanReport, agent_id: &str, file: &str, session_id: &str) {
    report.failures.push(SessionFileFailure {
        agent_id: agent_id.to_string(),
        file: file.to_string(),
        reason: format!("会话 ID 重复：{session_id}"),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::SessionFilter;
    use crate::services::agents::save_override;
    use std::path::PathBuf;

    struct Sandbox {
        _dir: tempfile::TempDir,
        store: Store,
        index: Index,
        home: PathBuf,
        session_file: PathBuf,
    }

    impl Sandbox {
        fn scan(&mut self) -> SessionScanReport {
            scan_all(&self.store, &mut self.index, &self.home).unwrap()
        }
    }

    fn claude_json_with_cwd(cwd: &str, role: &str, text: &str, timestamp: &str) -> String {
        serde_json::json!({
            "type": role,
            "sessionId": "s1",
            "cwd": cwd,
            "timestamp": timestamp,
            "message": {"role": role, "content": [{"type": "text", "text": text}]}
        })
        .to_string()
    }

    fn claude_json(role: &str, text: &str, timestamp: &str) -> String {
        claude_json_with_cwd("D:/repo", role, text, timestamp)
    }

    fn sandbox_with_lines(lines: &[String]) -> Sandbox {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        save_override(&store, "claude", Some(&home)).unwrap();
        let index = Index::open(&dir.path().join("index.db")).unwrap();
        let session_file = home.join(".claude/projects/p1/s1.jsonl");
        std::fs::create_dir_all(session_file.parent().unwrap()).unwrap();
        std::fs::write(&session_file, format!("{}\n", lines.join("\n"))).unwrap();
        Sandbox {
            _dir: dir,
            store,
            index,
            home,
            session_file,
        }
    }

    fn sandbox_with_claude_session() -> Sandbox {
        sandbox_with_lines(&[
            claude_json("user", "工具调用", "2026-10-20T10:00:00Z"),
            claude_json("assistant", "完成", "2026-10-20T10:01:00Z"),
        ])
    }

    #[test]
    fn scanning_is_idempotent_and_only_reparses_changed_files() {
        let mut fx = sandbox_with_claude_session();
        let first = fx.scan();
        assert_eq!((first.added, first.updated, first.unchanged), (1, 0, 0));
        let second = fx.scan();
        assert_eq!((second.added, second.updated, second.unchanged), (0, 0, 1));
    }

    #[test]
    fn a_deleted_session_file_disappears_from_the_index() {
        let mut fx = sandbox_with_claude_session();
        fx.scan();
        std::fs::remove_file(&fx.session_file).unwrap();
        let report = fx.scan();
        assert_eq!(report.removed, 1, "{report:?}");
        assert!(fx
            .index
            .list_sessions(&SessionFilter::all())
            .unwrap()
            .is_empty());
        assert_eq!(
            fx.index
                .session_detail("claude", "s1", 0, 10)
                .unwrap_err()
                .to_string(),
            "未找到：s1"
        );
    }

    #[test]
    fn a_broken_line_does_not_lose_the_rest_of_the_session() {
        let mut fx = sandbox_with_lines(&[
            claude_json("user", "保留我", "2026-10-20T10:00:00Z"),
            "not-json".to_string(),
            claude_json("assistant", "也保留", "2026-10-20T10:01:00Z"),
        ]);
        let report = fx.scan();
        assert!(
            report.warnings.iter().any(|warning| warning.line == 2),
            "{:?}",
            report.warnings
        );
        assert_eq!(
            fx.index
                .session_detail("claude", "s1", 0, 10)
                .unwrap()
                .messages
                .len(),
            2
        );
    }

    #[test]
    fn chinese_and_two_character_queries_are_safe() {
        let mut fx = sandbox_with_claude_session();
        fx.scan();
        assert_eq!(fx.index.search_sessions("工具调用", 10).unwrap().len(), 1);
        assert_eq!(fx.index.search_sessions("工具", 10).unwrap().len(), 1);
        assert!(fx.index.search_sessions("\" * %", 10).unwrap().is_empty());
    }

    #[test]
    fn duplicate_session_ids_keep_the_newer_and_name_the_other() {
        let mut fx = sandbox_with_lines(&[claude_json("user", "较旧", "2026-10-20T09:00:00Z")]);
        let newer = fx.home.join(".claude/projects/p2/s1.jsonl");
        std::fs::create_dir_all(newer.parent().unwrap()).unwrap();
        std::fs::write(
            &newer,
            format!("{}\n", claude_json("user", "较新", "2026-10-20T11:00:00Z")),
        )
        .unwrap();

        let report = fx.scan();
        assert!(
            report
                .failures
                .iter()
                .any(|failure| failure.reason.contains("会话 ID 重复")),
            "{report:?}"
        );
        let detail = fx.index.session_detail("claude", "s1", 0, 10).unwrap();
        assert!(
            detail.session.title.contains("较新"),
            "{}",
            detail.session.title
        );
        assert!(!detail
            .messages
            .iter()
            .any(|message| message.content.contains("较旧")));
    }

    #[test]
    fn oversized_content_is_truncated_before_it_enters_the_index() {
        let big = "A".repeat(20 * 1024);
        let mut fx = sandbox_with_lines(&[
            claude_json("user", &big, "2026-10-20T10:00:00Z"),
            claude_json("assistant", "完成", "2026-10-20T10:01:00Z"),
        ]);
        fx.scan();
        let detail = fx.index.session_detail("claude", "s1", 0, 10).unwrap();
        let huge = detail
            .messages
            .iter()
            .max_by_key(|message| message.content.len())
            .unwrap();
        assert!(
            huge.content.contains("[已截断，原文"),
            "{}",
            &huge.content[..80.min(huge.content.len())]
        );
        assert!(huge.content.len() <= 8 * 1024 + 64);
    }

    #[test]
    fn image_rows_are_not_indexed_in_the_fts() {
        let image = serde_json::json!({
            "type": "user",
            "sessionId": "s1",
            "cwd": "D:/repo",
            "timestamp": "2026-10-20T10:00:00Z",
            "message": {
                "role": "user",
                "content": [
                    {"type": "text", "text": "看图"},
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "QUJD"}}
                ]
            }
        })
        .to_string();
        let mut fx = sandbox_with_lines(&[
            image,
            claude_json("assistant", "完成", "2026-10-20T10:01:00Z"),
        ]);
        fx.scan();
        assert!(
            fx.index.search_sessions("QUJD", 10).unwrap().is_empty(),
            "图片 base64 不参与全文搜索"
        );
    }

    #[test]
    fn scanning_never_copies_session_content_into_the_store() {
        let mut fx = sandbox_with_lines(&[
            claude_json("user", "SESSION_SECRET_MARKER", "2026-10-20T10:00:00Z"),
            claude_json("assistant", "完成", "2026-10-20T10:01:00Z"),
        ]);
        fx.scan();
        assert!(!store_text_files_contain(
            &fx.store,
            "SESSION_SECRET_MARKER"
        ));
    }

    #[test]
    fn a_failed_file_keeps_its_previous_sessions_and_reports_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        save_override(&store, "pi", Some(&home)).unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        let session_file = home.join(".pi/agent/sessions/p/s1.jsonl");
        std::fs::create_dir_all(session_file.parent().unwrap()).unwrap();
        std::fs::write(
            &session_file,
            concat!(
                "{\"type\":\"session\",\"id\":\"p1\",\"version\":3,\"cwd\":\"D:/p\",\"timestamp\":\"2026-10-20T10:00:00Z\"}\n",
                "{\"id\":\"e1\",\"parentId\":null,\"type\":\"message\",\"timestamp\":\"2026-10-20T10:01:00Z\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"保留的问题\"}]}}\n"
            ),
        )
        .unwrap();
        let first = scan_all(&store, &mut index, &home).unwrap();
        assert_eq!(first.added, 1, "{first:?}");

        std::fs::write(
            &session_file,
            "{\"id\":\"e1\",\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"坏文件\"}]}}\n",
        )
        .unwrap();
        let report = scan_all(&store, &mut index, &home).unwrap();
        assert_eq!((report.added, report.updated, report.removed), (0, 0, 0));
        assert!(
            report
                .failures
                .iter()
                .any(|failure| failure.agent_id == "pi" && failure.reason.contains("session 头")),
            "{report:?}"
        );
        let detail = index.session_detail("pi", "p1", 0, 10).unwrap();
        assert!(detail
            .messages
            .iter()
            .any(|message| message.content == "保留的问题"));
        assert!(detail.session.parse_error.is_some());
    }

    #[test]
    fn providers_status_reports_roots_and_session_counts() {
        let mut fx = sandbox_with_claude_session();
        fx.scan();
        let statuses = providers_status(&fx.index, &fx.store, &fx.home).unwrap();
        let claude = statuses
            .iter()
            .find(|status| status.agent_id == "claude")
            .unwrap();
        assert!(claude.parse_ready);
        assert_eq!(claude.session_count, 1);
        assert!(claude
            .roots
            .iter()
            .any(|root| root.contains(".claude") && root.contains("projects")));
        let qoder = statuses
            .iter()
            .find(|status| status.agent_id == "qoder-cn")
            .unwrap();
        assert!(qoder.parse_ready);
    }

    fn store_text_files_contain(store: &Store, needle: &str) -> bool {
        fn walk(dir: &Path, needle: &str) -> bool {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return false;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if walk(&path, needle) {
                        return true;
                    }
                } else if path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("index.db"))
                {
                    continue;
                } else if std::fs::read(&path)
                    .is_ok_and(|bytes| String::from_utf8_lossy(&bytes).contains(needle))
                {
                    return true;
                }
            }
            false
        }
        walk(store.root(), needle)
    }
}
