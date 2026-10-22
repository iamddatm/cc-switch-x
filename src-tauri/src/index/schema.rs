pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS assets (
    asset_type TEXT NOT NULL,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    hash TEXT NOT NULL,
    PRIMARY KEY (asset_type, id)
);
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS agent_state (
    agent_id TEXT NOT NULL,
    asset_type TEXT NOT NULL,
    asset_id TEXT NOT NULL,
    state TEXT NOT NULL,
    last_error TEXT,
    applied_at INTEGER NOT NULL,
    PRIMARY KEY (agent_id, asset_type, asset_id)
);
CREATE TABLE IF NOT EXISTS runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at INTEGER NOT NULL,
    dry_run INTEGER NOT NULL,
    plan_json TEXT NOT NULL,
    result TEXT NOT NULL,
    backup_dir TEXT NOT NULL,
    reverted INTEGER NOT NULL DEFAULT 0
);
-- 漂移记录：同一个 (资产 × agent) 同时最多留一条「未解决」的（resolution IS NULL），
-- 重新检测时替换它；解决掉的记录留着，作为「什么时候、按哪种方式处置过」的历史。
CREATE TABLE IF NOT EXISTS drift (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_type TEXT NOT NULL,
    asset_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    detected_at INTEGER NOT NULL,
    kind TEXT NOT NULL,
    resolution TEXT
);
CREATE INDEX IF NOT EXISTS drift_by_target ON drift (agent_id, asset_type, asset_id);
-- 会话只读索引：正文只进 index.db（派生缓存），不回写真源目录。
CREATE TABLE IF NOT EXISTS sessions (
    agent_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    title TEXT NOT NULL,
    project_path TEXT,
    source_path TEXT NOT NULL,
    created_at INTEGER,
    updated_at INTEGER NOT NULL,
    message_count INTEGER NOT NULL,
    PRIMARY KEY (agent_id, session_id)
);
CREATE TABLE IF NOT EXISTS session_messages (
    agent_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    role TEXT NOT NULL,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    tool_name TEXT,
    PRIMARY KEY (agent_id, session_id, seq)
);
CREATE TABLE IF NOT EXISTS session_files (
    agent_id TEXT NOT NULL,
    source_path TEXT NOT NULL,
    session_id TEXT,
    mtime INTEGER NOT NULL,
    size INTEGER NOT NULL,
    parse_error TEXT,
    PRIMARY KEY (agent_id, source_path)
);
CREATE VIRTUAL TABLE IF NOT EXISTS session_messages_fts USING fts5(
    content,
    agent_id UNINDEXED,
    session_id UNINDEXED,
    seq UNINDEXED,
    tokenize='trigram'
);
"#;
