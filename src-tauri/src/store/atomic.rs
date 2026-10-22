use std::fs;
use std::io::Write;
use std::path::Path;

/// 原子覆盖写：先写同目录临时文件并落盘，再改名替换目标。
/// 用 keep() 主动交出句柄后再 rename：Windows 下带打开句柄的文件改名会失败，
/// 而 NamedTempFile::persist 仍持有句柄，因此不走 persist。
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    let (tmp_file, tmp_path) = tmp.keep().map_err(|e| e.error)?;
    drop(tmp_file);
    fs::rename(tmp_path, path)
}
