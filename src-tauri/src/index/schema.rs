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
"#;
