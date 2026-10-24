//! ZIP 安全解压、Skill 目录扫描与目录哈希。
//!
//! 哈希同时用于本地真源和远端归档，因此必须完全确定、忽略 `.meta.json`
//! 这类点开头文件；这样从 cc-switch 导入的副本与上游相同内容会得到同一个值。

use crate::domain::skill::parse_metadata;
use crate::error::AppError;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct ArchiveLimits {
    pub max_entries: usize,
    pub max_total_bytes: u64,
    /// 每个条目至少按这个大小计费，防止大量空文件绕过总量限制。
    pub min_entry_bytes: u64,
    pub max_depth: usize,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_entries: 30_000,
            max_total_bytes: 512 * 1024 * 1024,
            min_entry_bytes: 4 * 1024,
            max_depth: 32,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExtractReport {
    pub entries: usize,
    pub bytes: u64,
}

struct Entry {
    path: PathBuf,
    is_dir: bool,
    is_symlink: bool,
    charge: u64,
}

/// 解压 ZIP；自动剥掉 GitHub 归档唯一的顶层目录，并拒绝任何越界路径。
/// 符号链接条目直接跳过：不落盘链接可避免越界指向，也不依赖 Windows 建链权限。
pub fn extract_archive(
    zip_path: &Path,
    dest: &Path,
    limits: &ArchiveLimits,
) -> Result<ExtractReport, AppError> {
    let file = fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| AppError::Archive(format!("无法读取 ZIP：{error}")))?;
    if archive.is_empty() {
        return Err(AppError::Archive("ZIP 是空的".into()));
    }
    if archive.len() > limits.max_entries {
        return Err(AppError::Archive(format!(
            "ZIP 条目数 {} 超过上限 {}",
            archive.len(),
            limits.max_entries
        )));
    }

    let mut entries = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|error| AppError::Archive(format!("读取 ZIP 第 {index} 项失败：{error}")))?;
        if file.name().contains('\\') {
            return Err(AppError::Archive(format!(
                "ZIP 条目使用反斜杠路径，拒绝解压：{}",
                file.name()
            )));
        }
        let path = file.enclosed_name().ok_or_else(|| {
            AppError::Archive(format!("ZIP 条目路径越界，拒绝解压：{}", file.name()))
        })?;
        let is_symlink = file
            .unix_mode()
            .map(|mode| mode & 0o170000 == 0o120000)
            .unwrap_or(false);
        let is_dir = file.is_dir();
        let charge = file.size().max(limits.min_entry_bytes);
        entries.push(Entry {
            path,
            is_dir,
            is_symlink,
            charge,
        });
    }

    let strip = common_root(&entries);
    fs::create_dir_all(dest)?;
    let mut report = ExtractReport::default();
    let mut total = 0u64;

    for (index, entry) in entries.iter().enumerate() {
        let relative = match strip.as_ref() {
            Some(root) => match entry.path.strip_prefix(root) {
                Ok(path) => path.to_path_buf(),
                Err(_) => {
                    return Err(AppError::Archive(format!(
                        "ZIP 条目不在统一根目录下：{}",
                        entry.path.display()
                    )))
                }
            },
            None => entry.path.clone(),
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        validate_relative(&relative)?;
        if entry.is_symlink {
            continue;
        }

        report.entries += 1;
        total = total.saturating_add(entry.charge);
        if total > limits.max_total_bytes {
            return Err(AppError::Archive(format!(
                "ZIP 解压总量超过上限 {} 字节",
                limits.max_total_bytes
            )));
        }

        let target = dest.join(&relative);
        if entry.is_dir {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut source = archive
            .by_index(index)
            .map_err(|error| AppError::Archive(format!("读取 ZIP 第 {index} 项失败：{error}")))?;
        let mut output = fs::File::create(&target)?;
        let remaining = limits.max_total_bytes.saturating_sub(total);
        let mut limited = (&mut source).take(remaining.saturating_add(1));
        let copied = std::io::copy(&mut limited, &mut output)?;
        if copied > remaining {
            let _ = fs::remove_file(&target);
            return Err(AppError::Archive(format!(
                "ZIP 解压总量超过上限 {} 字节",
                limits.max_total_bytes
            )));
        }
        report.bytes = report.bytes.saturating_add(copied);
    }
    Ok(report)
}

/// 所有条目都在同一个顶层目录下时才剥掉首层（GitHub 归档的形状）。
/// 归档可能显式包含顶层目录条目（只有一个路径段），这不能否定剥离。
fn common_root(entries: &[Entry]) -> Option<PathBuf> {
    let mut root: Option<std::ffi::OsString> = None;
    let mut nested = false;
    for entry in entries {
        let mut components = entry.path.components();
        let first = components.next()?;
        if let Some(existing) = &root {
            if existing != first.as_os_str() {
                return None;
            }
        } else {
            root = Some(first.as_os_str().to_os_string());
        }
        if components.next().is_some() {
            nested = true;
        }
    }
    if !nested {
        return None;
    }
    root.map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

fn validate_relative(path: &Path) -> Result<(), AppError> {
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => {
                return Err(AppError::Archive(format!(
                    "ZIP 条目路径越界，拒绝解压：{}",
                    path.display()
                )))
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedSkill {
    /// 相对仓库/压缩包根的路径，根目录用 `.`。
    pub relative_path: String,
    pub name: String,
    pub description: Option<String>,
}

/// 找到所有含 `SKILL.md` 的目录；命中后不再向下递归，避免把子技能算成两个。
pub fn scan_skills(root: &Path, max: usize) -> Result<Vec<ScannedSkill>, AppError> {
    let mut out = Vec::new();
    scan_dir(root, root, 0, &mut out, max)?;
    out.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(out)
}

fn scan_dir(
    dir: &Path,
    root: &Path,
    depth: usize,
    out: &mut Vec<ScannedSkill>,
    max: usize,
) -> Result<(), AppError> {
    if out.len() >= max {
        return Err(AppError::Source(format!(
            "候选 Skill 超过 {max} 个，请缩小来源范围"
        )));
    }
    let skill_md = dir.join("SKILL.md");
    if skill_md.is_file() {
        let text = fs::read_to_string(&skill_md).unwrap_or_default();
        let meta = parse_metadata(&text);
        let relative_path = if dir == root {
            ".".to_string()
        } else {
            relative_string(dir.strip_prefix(root).unwrap_or(dir))
        };
        let fallback = dir
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("skill");
        out.push(ScannedSkill {
            relative_path,
            name: meta.name.unwrap_or_else(|| fallback.to_string()),
            description: meta.description,
        });
        return Ok(());
    }
    if depth >= 8 {
        return Ok(());
    }

    let mut children = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let name = child.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let file_type = child.file_type()?;
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        scan_dir(&child.path(), root, depth + 1, out, max)?;
    }
    Ok(())
}

fn relative_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// 对目录内容做确定性哈希：跳过点开头条目与符号链接，按相对路径排序。
pub fn compute_dir_hash(dir: &Path) -> Result<String, AppError> {
    if !dir.is_dir() {
        return Err(AppError::Source(format!(
            "目录不存在，无法计算哈希：{}",
            dir.display()
        )));
    }
    let mut files = Vec::new();
    collect_files(dir, dir, 0, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hasher = Sha256::new();
    for (relative, path) in files {
        hasher.update(relative.as_bytes());
        hasher.update(b"\0");
        let bytes = fs::read(&path)?;
        hasher.update(&bytes);
        hasher.update(b"\0");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_files(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), AppError> {
    if depth >= 32 {
        return Err(AppError::Source(format!(
            "目录层级过深，拒绝计算哈希：{}",
            dir.display()
        )));
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_files(root, &entry.path(), depth + 1, out)?;
        } else if file_type.is_file() {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap_or(&path);
            out.push((relative_string(relative), path));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = fs::File::create(path).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, data) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn hash_ignores_dot_entries_and_changes_with_content() {
        let dir = tempdir().unwrap();
        let skill = dir.path().join("demo");
        fs::create_dir_all(skill.join("references")).unwrap();
        fs::write(skill.join("SKILL.md"), "v1\n").unwrap();
        fs::write(skill.join("references/note.md"), "note\n").unwrap();
        fs::write(skill.join(".meta.json"), "{}").unwrap();
        fs::create_dir_all(skill.join(".hidden")).unwrap();
        fs::write(skill.join(".hidden/x"), "ignored").unwrap();

        let first = compute_dir_hash(&skill).unwrap();
        fs::write(skill.join(".meta.json"), r#"{"changed":true}"#).unwrap();
        fs::write(skill.join(".hidden/x"), "changed").unwrap();
        assert_eq!(first, compute_dir_hash(&skill).unwrap());

        fs::write(skill.join("references/note.md"), "changed\n").unwrap();
        assert_ne!(first, compute_dir_hash(&skill).unwrap());
    }

    #[test]
    fn hash_is_stable_across_file_creation_order() {
        let one = tempdir().unwrap();
        let two = tempdir().unwrap();
        for (root, order) in [(&one, ["a", "b"]), (&two, ["b", "a"])] {
            for name in order {
                fs::write(root.path().join(name), name).unwrap();
            }
        }
        assert_eq!(
            compute_dir_hash(one.path()).unwrap(),
            compute_dir_hash(two.path()).unwrap()
        );
    }

    #[test]
    fn extraction_rejects_zip_slip_and_absolute_paths() {
        let dir = tempdir().unwrap();
        let zip_path = dir.path().join("evil.zip");
        write_zip(&zip_path, &[("../evil.txt", b"x")]);
        let dest = dir.path().join("dest");
        assert!(extract_archive(&zip_path, &dest, &ArchiveLimits::default()).is_err());
        assert!(!dir.path().join("evil.txt").exists());

        let zip_path = dir.path().join("absolute.zip");
        write_zip(&zip_path, &[("/abs.txt", b"x")]);
        assert!(extract_archive(&zip_path, &dest, &ArchiveLimits::default()).is_err());
    }

    #[test]
    fn extraction_skips_symlink_entries_without_following_them() {
        let dir = tempdir().unwrap();
        let zip_path = dir.path().join("repo.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer
            .add_symlink("repo-main/.opencode/skills", "../skills/", options)
            .unwrap();
        writer
            .add_symlink("repo-main/AGENTS.md", "CLAUDE.md", options)
            .unwrap();
        writer
            .add_symlink("repo-main/escape", "../../outside", options)
            .unwrap();
        writer.start_file("repo-main/CLAUDE.md", options).unwrap();
        writer.write_all(b"# claude\n").unwrap();
        writer
            .start_file("repo-main/skills/demo/SKILL.md", options)
            .unwrap();
        writer.write_all(b"# demo\n").unwrap();
        writer.finish().unwrap();

        let dest = dir.path().join("dest");
        let report = extract_archive(&zip_path, &dest, &ArchiveLimits::default()).unwrap();
        assert_eq!(report.entries, 2);
        assert!(dest.join("CLAUDE.md").is_file());
        assert!(dest.join("skills/demo/SKILL.md").is_file());
        assert!(!dest.join("AGENTS.md").exists());
        assert!(!dest.join(".opencode").exists());
        assert!(!dest.join("escape").exists());
        assert!(!dir.path().join("outside").exists());
    }

    #[test]
    fn extraction_strips_the_github_root_folder() {
        let dir = tempdir().unwrap();
        let zip_path = dir.path().join("repo.zip");
        write_zip(
            &zip_path,
            &[
                ("repo-main/skills/demo/SKILL.md", b"# demo\n"),
                ("repo-main/skills/demo/references/a.md", b"a"),
                ("repo-main/README.md", b"readme"),
            ],
        );
        let dest = dir.path().join("dest");
        extract_archive(&zip_path, &dest, &ArchiveLimits::default()).unwrap();
        assert!(dest.join("skills/demo/SKILL.md").is_file());
        assert!(dest.join("README.md").is_file());
        assert!(!dest.join("repo-main").exists());
    }

    #[test]
    fn extraction_strips_root_even_when_the_archive_has_a_root_directory_entry() {
        let dir = tempdir().unwrap();
        let zip_path = dir.path().join("repo.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.add_directory("repo-main/", options).unwrap();
        writer
            .start_file("repo-main/skills/demo/SKILL.md", options)
            .unwrap();
        std::io::Write::write_all(&mut writer, b"# demo\n").unwrap();
        writer.finish().unwrap();

        let dest = dir.path().join("dest");
        extract_archive(&zip_path, &dest, &ArchiveLimits::default()).unwrap();
        assert!(dest.join("skills/demo/SKILL.md").is_file());
        assert!(!dest.join("repo-main").exists());
    }

    #[test]
    fn extraction_keeps_multiple_top_level_entries() {
        let dir = tempdir().unwrap();
        let zip_path = dir.path().join("skills.zip");
        write_zip(
            &zip_path,
            &[
                ("skill-a/SKILL.md", b"# a\n"),
                ("skill-b/SKILL.md", b"# b\n"),
            ],
        );
        let dest = dir.path().join("dest");
        extract_archive(&zip_path, &dest, &ArchiveLimits::default()).unwrap();
        assert!(dest.join("skill-a/SKILL.md").is_file());
        assert!(dest.join("skill-b/SKILL.md").is_file());
    }

    #[test]
    fn extraction_enforces_entry_and_size_limits() {
        let dir = tempdir().unwrap();
        let zip_path = dir.path().join("big.zip");
        write_zip(&zip_path, &[("a/SKILL.md", b"0123456789")]);
        let dest = dir.path().join("dest");

        let limits = ArchiveLimits {
            max_entries: 0,
            ..ArchiveLimits::default()
        };
        assert!(extract_archive(&zip_path, &dest, &limits).is_err());

        let limits = ArchiveLimits {
            max_total_bytes: 4 * 1024,
            ..ArchiveLimits::default()
        };
        assert!(extract_archive(&zip_path, &dest, &limits).is_err());
    }

    #[test]
    fn scanning_stops_at_the_first_skill_and_skips_hidden_and_node_modules() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("repo");
        fs::create_dir_all(root.join("skills/demo/references")).unwrap();
        fs::write(
            root.join("skills/demo/SKILL.md"),
            "---\nname: Demo\ndescription: 演示\n---\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("skills/demo/nested")).unwrap();
        fs::write(root.join("skills/demo/nested/SKILL.md"), "# nested\n").unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/SKILL.md"), "# hidden\n").unwrap();
        fs::create_dir_all(root.join("node_modules/x")).unwrap();
        fs::write(root.join("node_modules/x/SKILL.md"), "# dep\n").unwrap();

        let found = scan_skills(&root, 50).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].relative_path, "skills/demo");
        assert_eq!(found[0].name, "Demo");
        assert_eq!(found[0].description.as_deref(), Some("演示"));
    }

    #[test]
    fn scanning_recognizes_a_skill_at_the_root() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("SKILL.md"), "---\nname: Root\n---\n").unwrap();
        let found = scan_skills(dir.path(), 10).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].relative_path, ".");
        assert_eq!(found[0].name, "Root");
    }
}
