use crate::error::AppError;
use crate::import::source::connect;
use crate::import::{mcp, prompts, skills, ImportPreview, ImportRecord, ImportSource};
use crate::index::Index;
use crate::store::secrets::{secrets_path, Secrets};
use crate::store::Store;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub mcp_imported: usize,
    pub prompts_imported: usize,
    pub skills_imported: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub failed: Vec<String>,
    pub missing_skill_dirs: Vec<String>,
    pub ambiguous_active: Vec<String>,
    pub preview: ImportPreview,
}

/// cc-switch 的家目录：与 spec 3.1 一致，只认 `~/.cc-switch/cc-switch.db`。
pub fn detect_home(home: &Path) -> Option<PathBuf> {
    let candidate = home.join(".cc-switch");
    candidate
        .join("cc-switch.db")
        .is_file()
        .then_some(candidate)
}

pub fn preview(store: &Store, ccswitch_home: &Path) -> Result<ImportPreview, AppError> {
    let source = ImportSource::InstalledDb(ccswitch_home.join("cc-switch.db"));
    Ok(crate::import::preview(
        store,
        &source,
        &ccswitch_home.join("skills"),
    )?)
}

/// 一次导入 = 预览 + 三个导入器 + 落盘密钥 + 写导入记录。
/// 全程只读 cc-switch；任何一步失败都直接返回错误，不假装成功。
pub fn apply(
    store: &Store,
    index: &mut Index,
    secrets: &mut Secrets,
    ccswitch_home: &Path,
    policy: mcp::ConflictPolicy,
) -> Result<ImportSummary, AppError> {
    let db = ccswitch_home.join("cc-switch.db");
    let skills_root = ccswitch_home.join("skills");
    let source = ImportSource::InstalledDb(db.clone());
    let preview = crate::import::preview(store, &source, &skills_root)?;

    let opened = connect(&source)?;
    let mcp_report = mcp::run(store, secrets, index, opened.conn(), policy)?;
    let prompt_report = prompts::run(store, index, opened.conn())?;
    let skill_report = skills::run(store, index, opened.conn(), &skills_root)?;
    drop(opened);

    secrets.save(&secrets_path(store.root()))?;
    let mut failed = mcp_report.failed.clone();
    failed.extend(prompt_report.failed.clone());
    failed.extend(skill_report.failed.clone());
    crate::import::record_import(
        store,
        &ImportRecord {
            source: db.to_string_lossy().to_string(),
            finished_at: chrono::Utc::now().timestamp_millis(),
            mcp_imported: mcp_report.imported,
            prompts_imported: prompt_report.imported,
            skills_imported: skill_report.imported,
            failed: failed.clone(),
            missing_skill_dirs: skill_report.missing_dirs.clone(),
            ignored_columns: preview.ignored_columns.clone(),
        },
    )?;

    Ok(ImportSummary {
        mcp_imported: mcp_report.imported,
        prompts_imported: prompt_report.imported,
        skills_imported: skill_report.imported,
        unchanged: mcp_report.unchanged + prompt_report.unchanged + skill_report.unchanged,
        skipped: mcp_report.skipped,
        failed,
        missing_skill_dirs: skill_report.missing_dirs,
        ambiguous_active: prompt_report.ambiguous_active,
        preview,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::source::seed_fake_ccswitch;
    use crate::index::Index;
    use tempfile::tempdir;

    #[test]
    fn apply_imports_everything_and_writes_the_record() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        let mut secrets = Secrets::default();

        let ccswitch_home = dir.path().join("cc-switch");
        std::fs::create_dir_all(ccswitch_home.join("skills/demo")).unwrap();
        seed_fake_ccswitch(&ccswitch_home.join("cc-switch.db"));
        std::fs::write(
            ccswitch_home.join("skills/demo/SKILL.md"),
            "---\nname: demo\n---\n\n正文\n",
        )
        .unwrap();

        let summary = apply(
            &store,
            &mut index,
            &mut secrets,
            &ccswitch_home,
            mcp::ConflictPolicy::Skip,
        )
        .unwrap();
        assert_eq!(summary.mcp_imported, 1);
        assert_eq!(summary.prompts_imported, 1);
        assert_eq!(summary.skills_imported, 1);
        assert!(summary.failed.is_empty(), "{:?}", summary.failed);

        // 导入记录落在 state/import.json
        let record = std::fs::read_to_string(dir.path().join("store/state/import.json")).unwrap();
        assert!(record.contains("\"mcpImported\": 1"), "{record}");

        // 再跑一次：什么都不该变
        let second = apply(
            &store,
            &mut index,
            &mut secrets,
            &ccswitch_home,
            mcp::ConflictPolicy::Skip,
        )
        .unwrap();
        assert_eq!(second.mcp_imported, 0);
        assert_eq!(second.skills_imported, 0);
        assert!(second.unchanged >= 3, "{second:?}");
    }

    #[test]
    fn detect_home_only_accepts_a_real_database() {
        let dir = tempdir().unwrap();
        assert_eq!(detect_home(dir.path()), None);
        std::fs::create_dir_all(dir.path().join(".cc-switch")).unwrap();
        assert_eq!(detect_home(dir.path()), None, "空目录不算装了 cc-switch");
        std::fs::write(dir.path().join(".cc-switch/cc-switch.db"), b"x").unwrap();
        assert!(detect_home(dir.path()).is_some());
    }
}
