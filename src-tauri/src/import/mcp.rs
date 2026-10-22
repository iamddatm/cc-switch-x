use crate::domain::enablement::AgentEnablement;
use crate::domain::mcp::{McpServer, Transport};
use crate::domain::secret::{reference, SecretRef};
use crate::domain::{is_known_agent, AssetSummary, AssetType};
use crate::import::source::ImportError;
use crate::index::{sha256_hex, Index};
use crate::store::atomic::write_atomic;
use crate::store::secrets::{secrets_path, Secrets};
use crate::store::Store;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::BTreeMap;

/// `mcp_servers` 的一行。`enabled` 是按实际列名读出来的 (app, 是否启用)，
/// cc-switch 加了新 agent 列时我们只忽略并报告，不猜它的含义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpRow {
    pub id: String,
    pub name: String,
    pub server_config: String,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub docs: Option<String>,
    pub tags: Option<String>,
    pub enabled: Vec<(String, bool)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPolicy {
    Skip,
    Overwrite,
    KeepBoth,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportCount {
    pub imported: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub failed: Vec<String>,
}

/// 把 cc-switch 的 `server_config` 转成 canonical，并把明文密钥摘出来。
/// env 与 headers 的值**一律**外移（spec 4.5），URL 只摘名字像凭据的查询参数。
pub fn convert(
    server_config: &str,
    row: &McpRow,
) -> Result<(McpServer, Vec<(SecretRef, String)>), ImportError> {
    let config: Value = serde_json::from_str(server_config)?;
    let transport = match config
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("stdio")
    {
        "stdio" => Transport::Stdio,
        "http" => Transport::Http,
        "sse" => Transport::Sse,
        "ws" => Transport::Ws,
        other => return Err(ImportError::UnsupportedTransport(other.to_string())),
    };

    let scope = format!("mcp.{}", row.id);
    let mut moved: Vec<(SecretRef, String)> = Vec::new();
    let env = take_map(config.get("env"), &scope, &mut moved);
    let headers = take_map(config.get("headers"), &scope, &mut moved);
    let url = config
        .get("url")
        .and_then(|value| value.as_str())
        .map(|raw| move_url_credentials(raw, &scope, &mut moved));

    let args = config
        .get("args")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| item.to_string())
                })
                .collect()
        })
        .unwrap_or_default();

    let server = McpServer {
        id: row.id.clone(),
        name: row.name.clone(),
        transport,
        command: config
            .get("command")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        args,
        env,
        url,
        headers,
        description: row.description.clone().unwrap_or_default(),
        homepage: row.homepage.clone().unwrap_or_default(),
        docs: row.docs.clone().unwrap_or_default(),
        tags: parse_tags(row.tags.as_deref()),
    };
    Ok((server, moved))
}

fn take_map(
    value: Option<&Value>,
    scope: &str,
    moved: &mut Vec<(SecretRef, String)>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(entries) = value.and_then(|value| value.as_object()) else {
        return out;
    };
    for (key, raw) in entries {
        let plaintext = raw
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| raw.to_string());
        out.insert(key.clone(), reference(scope, key));
        moved.push((
            SecretRef {
                scope: scope.to_string(),
                key: key.clone(),
            },
            plaintext,
        ));
    }
    out
}

fn parse_tags(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|text| serde_json::from_str::<Vec<String>>(text).ok())
        .unwrap_or_default()
}

/// 名字像凭据的查询参数才摘走，`?v=2` 这类普通参数留在原处。
fn looks_credential(name: &str) -> bool {
    let lowered = name.to_lowercase();
    ["key", "token", "secret", "password", "auth"]
        .iter()
        .any(|marker| lowered.contains(marker))
}

fn move_url_credentials(raw: &str, scope: &str, moved: &mut Vec<(SecretRef, String)>) -> String {
    let Some((base, query)) = raw.split_once('?') else {
        return raw.to_string();
    };
    let pairs = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((name, value))
                if looks_credential(name)
                    && !value.is_empty()
                    && !value.starts_with("${secret:") =>
            {
                moved.push((
                    SecretRef {
                        scope: scope.to_string(),
                        key: name.to_string(),
                    },
                    value.to_string(),
                ));
                format!("{name}={}", reference(scope, name))
            }
            _ => pair.to_string(),
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{pairs}")
}

/// 按实际列名读 `mcp_servers`；`enabled_*` 列不存在就跳过，不写死列清单。
pub fn read_rows(conn: &Connection) -> Result<Vec<McpRow>, ImportError> {
    let mut stmt = conn.prepare("PRAGMA table_info(mcp_servers)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .flatten()
        .collect::<Vec<_>>();
    let enabled_columns: Vec<String> = columns
        .iter()
        .filter(|name| name.starts_with("enabled_"))
        .cloned()
        .collect();
    if enabled_columns.is_empty() {
        return Err(ImportError::MissingTable(
            "mcp_servers 的 enabled_* 列".into(),
        ));
    }

    let selected = [
        "id",
        "name",
        "server_config",
        "description",
        "homepage",
        "docs",
        "tags",
    ];
    let sql = format!(
        "SELECT {} FROM mcp_servers ORDER BY id",
        selected
            .iter()
            .map(|name| name.to_string())
            .chain(enabled_columns.clone())
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let mut row_values = Vec::new();
        for (offset, column) in enabled_columns.iter().enumerate() {
            let on: i64 = row
                .get::<_, Option<i64>>(selected.len() + offset)?
                .unwrap_or(0);
            row_values.push((column["enabled_".len()..].to_string(), on != 0));
        }
        Ok(McpRow {
            id: row.get(0)?,
            name: row.get(1)?,
            server_config: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            description: row.get(3)?,
            homepage: row.get(4)?,
            docs: row.get(5)?,
            tags: row.get(6)?,
            enabled: row_values,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn run(
    store: &Store,
    secrets: &mut Secrets,
    index: &mut Index,
    conn: &Connection,
    policy: ConflictPolicy,
) -> Result<ImportCount, ImportError> {
    let mut count = ImportCount::default();
    let mut matrices: BTreeMap<String, AgentEnablement> = BTreeMap::new();

    for row in read_rows(conn)? {
        let converted = match convert(&row.server_config, &row) {
            Ok(converted) => converted,
            Err(_) => {
                count.failed.push(row.id);
                continue;
            }
        };
        let (server, moved) = converted;
        if server.validate().is_err() {
            count.failed.push(row.id);
            continue;
        }
        let canonical = serde_json::to_vec_pretty(&server)?;

        let existing = store.read(AssetType::Mcp, &row.id).ok();
        let target_id = if existing.as_deref() == Some(canonical.as_slice()) {
            count.unchanged += 1;
            row.id.clone()
        } else if existing.is_some() {
            match policy {
                ConflictPolicy::Skip => {
                    count.skipped += 1;
                    continue;
                }
                ConflictPolicy::Overwrite => row.id.clone(),
                ConflictPolicy::KeepBoth => unique_id(store, &row.id)?,
            }
        } else {
            row.id.clone()
        };

        if existing.as_deref() != Some(canonical.as_slice()) || target_id != row.id {
            if store.read(AssetType::Mcp, &target_id).is_ok() {
                store.backup(AssetType::Mcp, &target_id)?;
            }
            store.write(AssetType::Mcp, &target_id, &canonical)?;
            count.imported += 1;
        }
        for (secret_ref, value) in &moved {
            secrets.insert(secret_ref, value);
        }
        index.upsert(&AssetSummary {
            asset_type: AssetType::Mcp,
            id: target_id.clone(),
            name: server.name.clone(),
            updated_at: chrono::Utc::now().timestamp_millis(),
            hash: sha256_hex(&canonical),
        })?;

        for (app, on) in &row.enabled {
            if !*on || !is_known_agent(app) {
                continue;
            }
            matrices
                .entry(app.clone())
                .or_insert_with(|| AgentEnablement::default())
                .enable(AssetType::Mcp, &target_id);
        }
    }

    for (app, matrix) in &mut matrices {
        let merged = merge_matrix(store, app, matrix)?;
        write_atomic(
            &store.agent_config_path(app),
            &serde_json::to_vec_pretty(&merged)?,
        )?;
    }
    if !count.failed.is_empty() || count.imported > 0 || !matrices.is_empty() {
        secrets.save(&secrets_path(store.root()))?;
    }
    Ok(count)
}

/// 矩阵文件是真源，导入只能往里加，不能把别的类型（skills/prompts）整份覆盖掉。
fn merge_matrix(
    store: &Store,
    app: &str,
    additions: &AgentEnablement,
) -> Result<AgentEnablement, ImportError> {
    let path = store.agent_config_path(app);
    let mut current = if path.exists() {
        serde_json::from_slice(&std::fs::read(&path)?)?
    } else {
        AgentEnablement::default()
    };
    for id in additions.enabled_for(AssetType::Mcp) {
        current.enable(AssetType::Mcp, &id);
    }
    Ok(current)
}

fn unique_id(store: &Store, base: &str) -> Result<String, ImportError> {
    let mut suffix = 2;
    loop {
        let candidate = format!("{base}-{suffix}");
        if store.read(AssetType::Mcp, &candidate).is_err() {
            return Ok(candidate);
        }
        suffix += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::mcp::Transport;
    use crate::import::source::fake_ccswitch_sql;
    use crate::index::Index;
    use crate::store::Store;
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn row(id: &str, name: &str) -> McpRow {
        McpRow {
            id: id.to_string(),
            name: name.to_string(),
            server_config: String::new(),
            description: None,
            homepage: None,
            docs: None,
            tags: None,
            enabled: Vec::new(),
        }
    }

    fn convert_stdio(config: &str) -> (McpServer, Vec<(SecretRef, String)>) {
        convert(config, &row("pw", "Playwright")).unwrap()
    }

    #[test]
    fn every_env_value_moves_into_secrets_not_just_the_secret_looking_ones() {
        let (server, moved) = convert_stdio(
            r#"{"type":"stdio","command":"npx","args":[],"env":{"API_TOKEN":"sk-live-123","LOG":"info"}}"#,
        );
        let text = serde_json::to_string(&server).unwrap();
        assert!(!text.contains("sk-live-123"), "canonical 里不得留明文");
        assert!(text.contains("${secret:mcp.pw.API_TOKEN}"));
        // spec 4.5 是「把明文 env 值抽到这里」，一律外移；不做「像不像密钥」的猜测
        assert!(text.contains("${secret:mcp.pw.LOG}"));
        assert_eq!(moved.len(), 2);
        assert!(moved
            .iter()
            .any(|(secret_ref, value)| secret_ref.key == "API_TOKEN" && value == "sk-live-123"));
    }

    #[test]
    fn url_query_credentials_are_extracted_but_ordinary_params_stay() {
        let (server, moved) = convert_stdio(
            r#"{"type":"http","url":"https://api.tavily.com/mcp?tavilyApiKey=tvly-abc&v=2"}"#,
        );
        assert_eq!(
            server.url.unwrap(),
            "https://api.tavily.com/mcp?tavilyApiKey=${secret:mcp.pw.tavilyApiKey}&v=2"
        );
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].1, "tvly-abc");
    }

    #[test]
    fn transport_type_maps_to_canonical_and_unknown_types_are_rejected() {
        assert_eq!(
            convert_stdio(r#"{"type":"sse","url":"https://x/sse"}"#)
                .0
                .transport,
            Transport::Sse
        );
        assert!(matches!(
            convert(
                r#"{"type":"grpc","url":"https://x"}"#,
                &row("pw", "Playwright")
            ),
            Err(ImportError::UnsupportedTransport(_))
        ));
    }

    #[test]
    fn canonical_id_comes_from_the_row_and_docs_is_not_dropped() {
        let mut source = row("pw", "Playwright");
        source.docs = Some("https://docs.example.com".into());
        source.tags = Some(r#"["search","web"]"#.into());
        let (server, _) =
            convert(r#"{"type":"stdio","command":"npx","args":[]}"#, &source).unwrap();
        assert_eq!(server.id, "pw");
        assert_eq!(server.name, "Playwright");
        assert_eq!(server.docs, "https://docs.example.com");
        assert_eq!(server.tags, vec!["search".to_string(), "web".to_string()]);
    }

    #[test]
    fn enabled_columns_become_the_agent_matrix() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        let mut secrets = Secrets::default();
        let conn = seeded_conn(&[(
            "pw",
            "Playwright",
            r#"{"type":"stdio","command":"npx","args":[],"env":{}}"#,
            1,
            0,
        )]);

        run(
            &store,
            &mut secrets,
            &mut index,
            &conn,
            ConflictPolicy::Skip,
        )
        .unwrap();

        let enablement = read_enablement(&store, "claude");
        assert_eq!(
            enablement.enabled_for(AssetType::Mcp),
            vec!["pw".to_string()]
        );
        assert!(
            !store.agent_config_path("codex").exists(),
            "enabled_codex=0 就不该被写进矩阵"
        );
        assert!(
            !dir.path().join("store/agents/quantum.json").exists(),
            "未知 app 不得建 agent 文件"
        );
    }

    #[test]
    fn importing_twice_changes_nothing() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        let mut secrets = Secrets::default();
        let conn = seeded_conn(&[(
            "pw",
            "Playwright",
            r#"{"type":"stdio","command":"npx","args":[],"env":{"T":"v"}}"#,
            1,
            0,
        )]);

        let first = run(
            &store,
            &mut secrets,
            &mut index,
            &conn,
            ConflictPolicy::Skip,
        )
        .unwrap();
        let second = run(
            &store,
            &mut secrets,
            &mut index,
            &conn,
            ConflictPolicy::Skip,
        )
        .unwrap();
        assert_eq!(first.imported, 1);
        assert_eq!(second.imported, 0, "再导入只补新增/变更");
        assert_eq!(second.unchanged, 1);
        assert_eq!(
            store.list_ids(AssetType::Mcp).unwrap(),
            vec!["pw".to_string()]
        );
        assert_eq!(
            read_enablement(&store, "claude").enabled_for(AssetType::Mcp),
            vec!["pw".to_string()],
            "矩阵不能被导入两次撑出重复项"
        );

        // 密钥链路收尾：明文进 secrets.local.json，真源文件里只有引用
        let stored = std::fs::read_to_string(secrets_path(store.root())).unwrap();
        assert!(stored.contains("\"T\": \"v\""), "{stored}");
        let canonical = String::from_utf8(store.read(AssetType::Mcp, "pw").unwrap()).unwrap();
        assert!(canonical.contains("${secret:mcp.pw.T}"), "{canonical}");
        assert!(!canonical.contains("\"v\""), "明文不得留在真源文件里");
    }

    #[test]
    fn a_bad_row_fails_only_itself() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        let mut secrets = Secrets::default();
        let conn = seeded_conn(&[
            (
                "good",
                "Good",
                r#"{"type":"stdio","command":"run","args":[]}"#,
                1,
                0,
            ),
            ("bad", "Bad", r#"{"type":"grpc","url":"https://x"}"#, 1, 0),
            ("empty", "Empty", r#"{"type":"stdio","args":[]}"#, 1, 0),
        ]);

        let count = run(
            &store,
            &mut secrets,
            &mut index,
            &conn,
            ConflictPolicy::Skip,
        )
        .unwrap();
        assert_eq!(count.imported, 1);
        assert_eq!(count.failed, vec!["bad".to_string(), "empty".to_string()]);
        assert_eq!(
            store.list_ids(AssetType::Mcp).unwrap(),
            vec!["good".to_string()]
        );
        assert_eq!(
            read_enablement(&store, "claude").enabled_for(AssetType::Mcp),
            vec!["good".to_string()],
            "失败的行不能进矩阵"
        );
    }

    fn seeded_conn(rows: &[(&str, &str, &str, i64, i64)]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(fake_ccswitch_sql()).unwrap();
        conn.execute("DELETE FROM mcp_servers", []).unwrap();
        for (id, name, config, claude, codex) in rows {
            conn.execute(
                "INSERT INTO mcp_servers (id,name,server_config,enabled_claude,enabled_codex,enabled_quantum)
                 VALUES (?1,?2,?3,?4,?5,1)",
                rusqlite::params![id, name, config, claude, codex],
            )
            .unwrap();
        }
        conn
    }

    fn read_enablement(store: &Store, agent_id: &str) -> AgentEnablement {
        let bytes = std::fs::read(store.agent_config_path(agent_id)).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
}
