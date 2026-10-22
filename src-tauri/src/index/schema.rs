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
"#;
