//! 会话索引：只读缓存、全文搜索与增量扫描的落库接口。

use super::{Index, IndexError};
use crate::domain::session::{
    ParsedSession, SessionDetail, SessionMessage, SessionMessageKind, SessionRole, SessionSummary,
};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// 单条消息进索引前保留的最大 UTF-8 字节数；超出部分截断并留下可解释的标记。
/// 截断点会回退到字符边界。`Image` 行不走这个上限（解析器已限制 data URL 大小）。
pub const INDEX_CONTENT_CAP: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFilter {
    pub agent_id: Option<String>,
    pub offset: u32,
    pub limit: u32,
}

impl SessionFilter {
    pub fn all() -> Self {
        Self {
            agent_id: None,
            offset: 0,
            limit: u32::MAX,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSearchHit {
    pub session: SessionSummary,
    pub index: u32,
    pub seq: u32,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMatch {
    pub index: u32,
    pub seq: u32,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFileRow {
    pub agent_id: String,
    pub source_path: String,
    pub session_id: Option<String>,
    pub mtime: i64,
    pub size: i64,
    pub parse_error: Option<String>,
}

const SUMMARY_SELECT: &str = "SELECT s.agent_id,s.session_id,s.title,s.project_path,s.source_path,s.created_at,s.updated_at,s.message_count,f.parse_error, \
     (SELECT m.content FROM session_messages m \
      WHERE m.agent_id=s.agent_id AND m.session_id=s.session_id AND m.kind='text' \
      ORDER BY m.seq DESC LIMIT 1) AS preview \
     FROM sessions s LEFT JOIN session_files f ON f.agent_id=s.agent_id AND f.source_path=s.source_path";

impl Index {
    pub fn upsert_session(
        &mut self,
        agent_id: &str,
        parsed: &ParsedSession,
        source_path: &str,
        mtime: i64,
        size: i64,
    ) -> Result<(), IndexError> {
        let tx = self.conn.transaction()?;
        delete_session_rows(&tx, agent_id, &parsed.session_id)?;
        for (seq, message) in parsed.messages.iter().enumerate() {
            let seq = seq as i64;
            // 图片是 data URL：截断会得到无法解码的字符串，解析器已单独限制其大小。
            let content = if message.kind == SessionMessageKind::Image {
                message.content.clone()
            } else {
                cap_content(&message.content)
            };
            tx.execute(
                "INSERT INTO session_messages (agent_id,session_id,seq,role,kind,content,tool_name) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    agent_id,
                    parsed.session_id,
                    seq,
                    message.role.as_db(),
                    message.kind.as_db(),
                    content,
                    message.tool_name.as_deref()
                ],
            )?;
            if message.kind != SessionMessageKind::Image {
                tx.execute(
                    "INSERT INTO session_messages_fts (content,agent_id,session_id,seq) \
                     VALUES (?1,?2,?3,?4)",
                    params![content, agent_id, parsed.session_id, seq],
                )?;
            }
        }
        let project_path = parsed
            .project_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string());
        let message_count = i64::from(u32::try_from(parsed.messages.len()).unwrap_or(u32::MAX));
        tx.execute(
            "INSERT INTO sessions \
             (agent_id,session_id,title,project_path,source_path,created_at,updated_at,message_count) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8) \
             ON CONFLICT(agent_id,session_id) DO UPDATE SET \
             title=excluded.title,project_path=excluded.project_path,source_path=excluded.source_path,\
             created_at=excluded.created_at,updated_at=excluded.updated_at,message_count=excluded.message_count",
            params![
                agent_id,
                parsed.session_id,
                parsed.title,
                project_path,
                source_path,
                parsed.created_at,
                parsed.updated_at,
                message_count
            ],
        )?;
        tx.execute(
            "INSERT INTO session_files (agent_id,source_path,session_id,mtime,size,parse_error) \
             VALUES (?1,?2,?3,?4,?5,NULL) \
             ON CONFLICT(agent_id,source_path) DO UPDATE SET \
             session_id=excluded.session_id,mtime=excluded.mtime,size=excluded.size,parse_error=NULL",
            params![agent_id, source_path, parsed.session_id, mtime, size],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_session(&mut self, agent_id: &str, session_id: &str) -> Result<(), IndexError> {
        let tx = self.conn.transaction()?;
        delete_session_rows(&tx, agent_id, session_id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_sessions_by_source(
        &mut self,
        agent_id: &str,
        source_path: &str,
        keep: &[String],
    ) -> Result<usize, IndexError> {
        let tx = self.conn.transaction()?;
        let ids = {
            let mut stmt =
                tx.prepare("SELECT session_id FROM sessions WHERE agent_id=?1 AND source_path=?2")?;
            let rows = stmt.query_map(params![agent_id, source_path], |row| {
                row.get::<_, String>(0)
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut removed = 0;
        for id in ids {
            if !keep.iter().any(|kept| kept == &id) {
                delete_session_rows(&tx, agent_id, &id)?;
                removed += 1;
            }
        }
        tx.commit()?;
        Ok(removed)
    }

    pub fn list_sessions(&self, filter: &SessionFilter) -> Result<Vec<SessionSummary>, IndexError> {
        let sql = format!(
            "{SUMMARY_SELECT} WHERE (?1 IS NULL OR s.agent_id = ?1) \
             ORDER BY s.updated_at DESC, s.session_id ASC LIMIT ?2 OFFSET ?3"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![
                filter.agent_id.as_deref(),
                filter.limit as i64,
                filter.offset as i64
            ],
            row_to_summary,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn session_detail(
        &self,
        agent_id: &str,
        session_id: &str,
        offset: u32,
        limit: u32,
    ) -> Result<SessionDetail, IndexError> {
        let session = self
            .session_by_id(agent_id, session_id)?
            .ok_or_else(|| IndexError::NotFound(session_id.to_string()))?;
        let total: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM session_messages WHERE agent_id=?1 AND session_id=?2",
            params![agent_id, session_id],
            |row| row.get(0),
        )?;
        let mut stmt = self.conn.prepare(
            "SELECT seq,role,kind,content,tool_name FROM session_messages \
             WHERE agent_id=?1 AND session_id=?2 ORDER BY seq LIMIT ?3 OFFSET ?4",
        )?;
        let rows = stmt.query_map(
            params![agent_id, session_id, limit as i64, offset as i64],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )?;
        let mut messages = Vec::new();
        for row in rows {
            let (seq, role, kind, content, tool_name) = row?;
            messages.push(SessionMessage {
                seq: u32::try_from(seq).unwrap_or(u32::MAX),
                role: SessionRole::from_db(&role)?,
                kind: SessionMessageKind::from_db(&kind)?,
                content,
                tool_name,
            });
        }
        Ok(SessionDetail {
            session,
            messages,
            total: u32::try_from(total).unwrap_or(u32::MAX),
        })
    }

    pub fn search_sessions(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SessionSearchHit>, IndexError> {
        let Some(query) = normalized_query(query) else {
            return Ok(Vec::new());
        };
        let rows = if alphanumeric_count(query) <= 2 {
            self.like_hits(None, query, limit)?
        } else {
            self.fts_hits(None, query, limit)?
        };
        let mut hits = Vec::new();
        for (agent_id, session_id, seq, content) in rows {
            if let Some(session) = self.session_by_id(&agent_id, &session_id)? {
                hits.push(SessionSearchHit {
                    session,
                    index: u32::try_from(seq).unwrap_or(u32::MAX),
                    seq: u32::try_from(seq).unwrap_or(u32::MAX),
                    snippet: make_snippet(&content, query),
                });
            }
        }
        let mut seen: HashSet<(String, String)> = hits
            .iter()
            .map(|hit| (hit.session.agent_id.clone(), hit.session.session_id.clone()))
            .collect();
        for hit in self.metadata_hits(query, limit)? {
            if hits.len() >= limit as usize {
                break;
            }
            let key = (hit.session.agent_id.clone(), hit.session.session_id.clone());
            if seen.insert(key) {
                hits.push(hit);
            }
        }
        Ok(hits)
    }

    pub fn search_in_session(
        &self,
        agent_id: &str,
        session_id: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SessionMatch>, IndexError> {
        let Some(query) = normalized_query(query) else {
            return Ok(Vec::new());
        };
        let rows = if alphanumeric_count(query) <= 2 {
            self.like_hits(Some((agent_id, session_id)), query, limit)?
        } else {
            self.fts_hits(Some((agent_id, session_id)), query, limit)?
        };
        Ok(rows
            .into_iter()
            .map(|(_, _, seq, content)| SessionMatch {
                index: u32::try_from(seq).unwrap_or(u32::MAX),
                seq: u32::try_from(seq).unwrap_or(u32::MAX),
                snippet: make_snippet(&content, query),
            })
            .collect())
    }

    pub fn session_files_for_agent(
        &self,
        agent_id: &str,
    ) -> Result<Vec<SessionFileRow>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT agent_id,source_path,session_id,mtime,size,parse_error \
             FROM session_files WHERE agent_id=?1 ORDER BY source_path",
        )?;
        let rows = stmt.query_map([agent_id], row_to_file)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn session_file(
        &self,
        agent_id: &str,
        source_path: &str,
    ) -> Result<Option<SessionFileRow>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT agent_id,source_path,session_id,mtime,size,parse_error \
             FROM session_files WHERE agent_id=?1 AND source_path=?2",
        )?;
        let mut rows = stmt.query_map(params![agent_id, source_path], row_to_file)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn record_session_file(
        &mut self,
        agent_id: &str,
        source_path: &str,
        session_id: Option<&str>,
        mtime: i64,
        size: i64,
        parse_error: Option<&str>,
    ) -> Result<(), IndexError> {
        self.conn.execute(
            "INSERT INTO session_files (agent_id,source_path,session_id,mtime,size,parse_error) \
             VALUES (?1,?2,?3,?4,?5,?6) \
             ON CONFLICT(agent_id,source_path) DO UPDATE SET \
             session_id=excluded.session_id,mtime=excluded.mtime,size=excluded.size,\
             parse_error=excluded.parse_error",
            params![agent_id, source_path, session_id, mtime, size, parse_error],
        )?;
        Ok(())
    }

    pub fn delete_session_file(
        &mut self,
        agent_id: &str,
        source_path: &str,
    ) -> Result<(), IndexError> {
        self.conn.execute(
            "DELETE FROM session_files WHERE agent_id=?1 AND source_path=?2",
            params![agent_id, source_path],
        )?;
        Ok(())
    }

    pub fn session_by_id(
        &self,
        agent_id: &str,
        session_id: &str,
    ) -> Result<Option<SessionSummary>, IndexError> {
        let sql = format!("{SUMMARY_SELECT} WHERE s.agent_id=?1 AND s.session_id=?2");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query_map(params![agent_id, session_id], row_to_summary)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn session_count(&self, agent_id: &str) -> Result<usize, IndexError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE agent_id=?1",
            [agent_id],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    /// 标题、项目目录与会话 ID 也参与搜索（规格 3.2）；命中这些字段时
    /// 用 `index=0` 占位，前端只展示 snippet，不做会话内跳转。
    fn metadata_hits(&self, query: &str, limit: u32) -> Result<Vec<SessionSearchHit>, IndexError> {
        let pattern = like_pattern(query);
        let mut stmt = self.conn.prepare(
            "SELECT agent_id,session_id,title,project_path FROM sessions \
             WHERE title LIKE ?1 ESCAPE '\\' OR project_path LIKE ?1 ESCAPE '\\' \
             OR session_id LIKE ?1 ESCAPE '\\' ORDER BY updated_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        let needle = query.to_lowercase();
        let mut out = Vec::new();
        for row in rows {
            let (agent_id, session_id, title, project_path) = row?;
            let Some(session) = self.session_by_id(&agent_id, &session_id)? else {
                continue;
            };
            let snippet = if title.to_lowercase().contains(&needle) {
                format!("标题：{title}")
            } else if let Some(path) =
                project_path.filter(|path| path.to_lowercase().contains(&needle))
            {
                format!("目录：{path}")
            } else {
                format!("会话 ID：{session_id}")
            };
            out.push(SessionSearchHit {
                session,
                index: 0,
                seq: 0,
                snippet,
            });
        }
        Ok(out)
    }

    fn like_hits(
        &self,
        scope: Option<(&str, &str)>,
        query: &str,
        limit: u32,
    ) -> Result<Vec<(String, String, i64, String)>, IndexError> {
        let pattern = like_pattern(query);
        let mut out = Vec::new();
        match scope {
            Some((agent_id, session_id)) => {
                let mut stmt = self.conn.prepare(
                    "SELECT agent_id,session_id,seq,content FROM session_messages \
                     WHERE agent_id=?1 AND session_id=?2 AND kind<>'image' \
                     AND content LIKE ?3 ESCAPE '\\' ORDER BY seq LIMIT ?4",
                )?;
                let rows = stmt.query_map(
                    params![agent_id, session_id, pattern, limit as i64],
                    search_row,
                )?;
                out.extend(rows.collect::<Result<Vec<_>, _>>()?);
            }
            None => {
                let mut stmt = self.conn.prepare(
                    "SELECT agent_id,session_id,seq,content FROM (\
                     SELECT agent_id,session_id,seq,content,\
                     ROW_NUMBER() OVER (PARTITION BY agent_id,session_id ORDER BY seq) AS rn \
                     FROM session_messages WHERE kind<>'image' AND content LIKE ?1 ESCAPE '\\'\
                     ) WHERE rn=1 ORDER BY seq LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![pattern, limit as i64], search_row)?;
                out.extend(rows.collect::<Result<Vec<_>, _>>()?);
            }
        }
        Ok(out)
    }

    fn fts_hits(
        &self,
        scope: Option<(&str, &str)>,
        query: &str,
        limit: u32,
    ) -> Result<Vec<(String, String, i64, String)>, IndexError> {
        let match_expression = format!("\"{}\"", query.replace('"', "\"\""));
        let mut out = Vec::new();
        match scope {
            Some((agent_id, session_id)) => {
                let mut stmt = self.conn.prepare(
                    "SELECT agent_id,session_id,seq,content FROM session_messages_fts \
                     WHERE session_messages_fts MATCH ?1 AND agent_id=?2 AND session_id=?3 \
                     ORDER BY seq LIMIT ?4",
                )?;
                let rows = stmt.query_map(
                    params![match_expression, agent_id, session_id, limit as i64],
                    search_row,
                )?;
                out.extend(rows.collect::<Result<Vec<_>, _>>()?);
            }
            None => {
                let mut stmt = self.conn.prepare(
                    "SELECT agent_id,session_id,seq,content FROM (\
                     SELECT agent_id,session_id,seq,content,\
                     ROW_NUMBER() OVER (PARTITION BY agent_id,session_id ORDER BY seq) AS rn \
                     FROM session_messages_fts WHERE session_messages_fts MATCH ?1\
                     ) WHERE rn=1 ORDER BY seq LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![match_expression, limit as i64], search_row)?;
                out.extend(rows.collect::<Result<Vec<_>, _>>()?);
            }
        }
        Ok(out)
    }
}

fn delete_session_rows(
    conn: &Connection,
    agent_id: &str,
    session_id: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM session_messages_fts WHERE agent_id=?1 AND session_id=?2",
        params![agent_id, session_id],
    )?;
    conn.execute(
        "DELETE FROM session_messages WHERE agent_id=?1 AND session_id=?2",
        params![agent_id, session_id],
    )?;
    conn.execute(
        "DELETE FROM sessions WHERE agent_id=?1 AND session_id=?2",
        params![agent_id, session_id],
    )?;
    Ok(())
}

fn row_to_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSummary> {
    Ok(SessionSummary {
        agent_id: row.get(0)?,
        session_id: row.get(1)?,
        title: row.get(2)?,
        project_path: row.get(3)?,
        source_path: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        message_count: u32::try_from(row.get::<_, i64>(7)?).unwrap_or(u32::MAX),
        parse_error: row.get(8)?,
        preview: row
            .get::<_, Option<String>>(9)?
            .map(|text| make_preview(&text)),
    })
}

/// 摘要只用于列表行：压掉换行、限制长度，不把大段正文塞进 IPC。
fn make_preview(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = flat.chars();
    let short: String = chars.by_ref().take(160).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
}

fn row_to_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionFileRow> {
    Ok(SessionFileRow {
        agent_id: row.get(0)?,
        source_path: row.get(1)?,
        session_id: row.get(2)?,
        mtime: row.get(3)?,
        size: row.get(4)?,
        parse_error: row.get(5)?,
    })
}

fn search_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, String, i64, String)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn normalized_query(query: &str) -> Option<&str> {
    let query = query.trim();
    if query.is_empty() || alphanumeric_count(query) == 0 {
        return None;
    }
    Some(query)
}

fn alphanumeric_count(query: &str) -> usize {
    query.chars().filter(|c| c.is_alphanumeric()).count()
}

fn like_pattern(query: &str) -> String {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// 大小写折叠可能改变字符的字节长度（如 `İ` → `i̇`），折叠串里的偏移不能
/// 直接切原串；越界或落在字符中间时回退到原文的精确匹配。
fn snippet_position(content: &str, query: &str) -> Option<usize> {
    let lower = content.to_lowercase();
    let needle = query.to_lowercase();
    if let Some(byte) = lower.find(&needle) {
        if content.is_char_boundary(byte) {
            return Some(content[..byte].chars().count());
        }
    }
    content
        .find(query)
        .filter(|byte| content.is_char_boundary(*byte))
        .map(|byte| content[..byte].chars().count())
}

fn make_snippet(content: &str, query: &str) -> String {
    let total = content.chars().count();
    let char_pos = snippet_position(content, query).unwrap_or(0);
    let start = char_pos.saturating_sub(30);
    let end = (char_pos + query.chars().count() + 30).min(total);
    let mut snippet: String = content.chars().skip(start).take(end - start).collect();
    if start > 0 {
        snippet.insert(0, '…');
    }
    if end < total {
        snippet.push('…');
    }
    snippet
}

pub fn cap_content(content: &str) -> String {
    if content.len() <= INDEX_CONTENT_CAP {
        return content.to_string();
    }
    let chars = content.chars().count();
    // 上限按 UTF-8 字节算；回退到最近的字符边界，避免切坏多字节字符。
    let mut end = INDEX_CONTENT_CAP;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    let kept = &content[..end];
    format!("{kept}\n…[已截断，原文 {chars} 字符]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_content_counts_utf8_bytes_not_chars() {
        let text = "汉".repeat(3000);
        assert!(text.len() > INDEX_CONTENT_CAP, "测试输入必须超过上限");
        let capped = cap_content(&text);
        assert!(
            capped.len() <= INDEX_CONTENT_CAP + 64,
            "截断后 {} 字节",
            capped.len()
        );
        assert!(capped.contains("已截断"), "{capped}");
    }

    #[test]
    fn image_messages_keep_full_data_urls() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        let data_url = format!("data:image/png;base64,{}", "A".repeat(100 * 1024));
        let parsed = ParsedSession {
            session_id: "s1".into(),
            title: "图片会话".into(),
            project_path: None,
            created_at: None,
            updated_at: 1,
            messages: vec![crate::domain::session::ParsedMessage {
                role: SessionRole::User,
                kind: SessionMessageKind::Image,
                content: data_url.clone(),
                tool_name: None,
            }],
            warnings: Vec::new(),
        };
        index
            .upsert_session("claude", &parsed, "D:/s1.jsonl", 1, 1)
            .unwrap();

        let detail = index.session_detail("claude", "s1", 0, 10).unwrap();
        assert_eq!(
            detail.messages[0].content, data_url,
            "图片不能被索引上限截断"
        );
    }

    #[test]
    fn snippets_survive_unicode_case_folding() {
        let snippet = make_snippet("İ中", "中");
        assert!(snippet.contains('中'), "{snippet}");
    }

    #[test]
    fn search_matches_session_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        let parsed = ParsedSession {
            session_id: "session-xyz".into(),
            title: "只有标题能命中".into(),
            project_path: Some("D:/unique-project".into()),
            created_at: None,
            updated_at: 1,
            messages: vec![crate::domain::session::ParsedMessage {
                role: SessionRole::User,
                kind: SessionMessageKind::Text,
                content: "正文".into(),
                tool_name: None,
            }],
            warnings: Vec::new(),
        };
        index
            .upsert_session("claude", &parsed, "D:/s1.jsonl", 1, 1)
            .unwrap();

        for query in ["只有标题能命中", "unique-project", "session-xyz"] {
            let hits = index.search_sessions(query, 10).unwrap();
            assert_eq!(hits.len(), 1, "{query}：{hits:?}");
            assert!(
                hits[0].snippet.contains(query),
                "{query}：{}",
                hits[0].snippet
            );
        }
    }

    #[test]
    fn fts5_trigram_is_available() {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open(&dir.path().join("index.db")).unwrap();
        index
            .conn
            .execute(
                "INSERT INTO session_messages_fts(content,agent_id,session_id,seq) \
                 VALUES ('工具调用','claude','s1',0)",
                [],
            )
            .unwrap();
        let count: i64 = index
            .conn
            .query_row(
                "SELECT COUNT(*) FROM session_messages_fts \
                 WHERE session_messages_fts MATCH '\"工具调用\"'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
