use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEntry {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub added_at: String,
    pub last_opened_at: String,
    pub auto_sync: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRegistry {
    pub version: u32,
    pub projects: Vec<ProjectEntry>,
}

impl Default for ProjectRegistry {
    fn default() -> Self {
        Self {
            version: 1,
            projects: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectSkillDirKey {
    Claude,
    Qoder,
    Agents,
}

impl ProjectSkillDirKey {
    pub const ALL: [Self; 3] = [Self::Claude, Self::Qoder, Self::Agents];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Qoder => "qoder",
            Self::Agents => "agents",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectSkillDirSpec {
    pub key: ProjectSkillDirKey,
    pub relative_path: &'static str,
    pub agent_ids: &'static [&'static str],
    /// 是否已在临时项目里做过真机读取验证；false 时 UI 不允许把它选为同步目标。
    pub verified: bool,
}

/// 项目级落点的 `verified` 只能由真机证据升档（与 `domain/agent.rs` 同一套档位规则）：
/// - **A 真机读取**：在这个仓库自己的服务器上把探针 skill 放进项目目录，agent 确实报告了它。
/// - 未升档的目录在矩阵里显示「待实测」，`selectable_dir_keys` 拒绝把它作为同步目标。
pub const PROJECT_SKILL_DIRS: [ProjectSkillDirSpec; 3] = [
    ProjectSkillDirSpec {
        key: ProjectSkillDirKey::Claude,
        relative_path: ".claude/skills",
        agent_ids: &["claude"],
        // 2026-10-26 真机项目实测：放入 `.claude/skills/zz-ccswitchx-probe` 后
        // `claude -p` 报出该 skill 名；移走目录后同一问句回答 NONE（正/负对照）。
        verified: true,
    },
    ProjectSkillDirSpec {
        key: ProjectSkillDirKey::Qoder,
        relative_path: ".qoder/skills",
        agent_ids: &["qoder", "qoder-cn"],
        // 2026-10-26 真机项目实测：`qoderclicn skills list` 在项目目录下报出
        // <project>\.qoder\skills\zz-ccswitchx-probe\SKILL.md（路径即证据）。
        // 注意：`qoder`（国际版）的用户级 Skill 能力仍未实测，启用它时该目录会被能力闸再次挡下。
        verified: true,
    },
    ProjectSkillDirSpec {
        key: ProjectSkillDirKey::Agents,
        relative_path: ".agents/skills",
        agent_ids: &["codex", "gemini", "pi"],
        // 本机无法取证：codex CLI 无 skills 概念、gemini 未安装、pi 因缺少凭据报 Connection error。
        // 需要在这三者任一能跑通的项目里读到 `.agents/skills` 后再升档。
        verified: false,
    },
];

pub fn dir_spec(key: ProjectSkillDirKey) -> &'static ProjectSkillDirSpec {
    PROJECT_SKILL_DIRS
        .iter()
        .find(|spec| spec.key == key)
        .expect("PROJECT_SKILL_DIRS 必须覆盖所有 ProjectSkillDirKey")
}

/// 项目台账格式版本。v1 是「对等副本」模型（agent 目录互为副本），v2 起
/// 项目真源是唯一可编辑副本、agent 目录只是渲染产物。
///
/// **v3 只把真源与台账从项目仓库搬进客户端 store，格式一个字段都没动**
/// （台账里没有任何项目根相对路径或绝对路径），所以版本号保持不变，布局另用
/// `layout.json` 这条独立的轴表达。把这里升到 3 会让 `ledger_needs_migration`
/// 把所有 v2 台账误判成待迁移，连带打断 v1→v2 迁移器的幂等闸。
pub const PROJECT_LEDGER_VERSION: u32 = 2;
/// 能读懂的最低版本；低于它的文件按损坏处理，高于它的必须要求升级。
pub const PROJECT_LEDGER_MIN_READABLE: u32 = 1;

/// 项目数据布局版本：真源与台账住在 `<storeRoot>/local/projects/<project-id>/`。
/// 与 `PROJECT_LEDGER_VERSION` 是两条独立的轴，交叉才能表达「两边都有」的半状态。
pub const PROJECT_LAYOUT_VERSION: u32 = 3;
/// 高于它的 `layout.json` 必须报错，不能按本程序的语义去删用户的目录。
pub const PROJECT_LAYOUT_MIN_READABLE: u32 = 3;

/// store 根下存放项目数据的子树。`local/` 已在 store 的 .gitignore 必需条目里，
/// 且 `backup::collect_files` 是白名单，所以这一份真源**不参与任何跨设备同步**。
pub const PROJECT_DATA_RELATIVE: &str = "local/projects";
/// v3 的布局戳记文件名：存在即表示这个项目已经用上新布局。
pub const LAYOUT_FILE: &str = "layout.json";

/// 旧布局在项目仓库里的位置，只用于「检测 + 搬迁」。
/// v3 之后我们不再往这里写任何东西，项目里除了 agent 渲染目录不属于我们的文件。
pub const LEGACY_DIR_RELATIVE: &str = ".ccswitchx";

/// 一个项目的全部落点。构造一次往下传，避免 `project_id` 与 `project_root`
/// 成对散落到几十个调用点上。纯 `PathBuf`，不持有 `Store` 引用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectPaths {
    pub id: String,
    /// `<project>`：渲染目标与扫描仍在这里，v3 没有改。
    pub root: PathBuf,
    data_dir: PathBuf,
    canonical_root: PathBuf,
    ledger: PathBuf,
    layout: PathBuf,
    legacy_dir: PathBuf,
}

impl ProjectPaths {
    /// 唯一的构造入口。除了 ID 安全与防逃逸，还钉住两条搬迁安全不变量：
    /// 真源必须在 store 的 `local/projects` 之下；旧目录与 store 必须互不包含
    /// （否则「删掉 `<project>/.ccswitchx`」有可能删到我们自己的数据）。
    pub fn resolve(store_root: &Path, id: &str, project_root: &Path) -> Result<Self, AppError> {
        if !crate::store::is_safe_id(id) {
            return Err(AppError::InvalidInput(format!(
                "项目 ID 不能用作目录名：{id}"
            )));
        }
        let data_root = store_root.join(PROJECT_DATA_RELATIVE);
        let data_dir = data_root.join(id);
        let legacy_dir = project_root.join(LEGACY_DIR_RELATIVE);
        if legacy_dir == store_root
            || legacy_dir.starts_with(store_root)
            || store_root.starts_with(&legacy_dir)
        {
            return Err(AppError::InvalidInput(format!(
                "项目里的 {} 与 CC Switch X 的数据目录重叠，拒绝操作：{}",
                LEGACY_DIR_RELATIVE,
                store_root.display()
            )));
        }
        Ok(Self {
            id: id.to_string(),
            root: project_root.to_path_buf(),
            canonical_root: data_dir.join("skills"),
            ledger: data_dir.join("skills.json"),
            layout: data_dir.join(LAYOUT_FILE),
            legacy_dir,
            data_dir,
        })
    }

    /// store 根下所有项目数据的公共目录（搬迁、清理、备份都从这里算相对路径）。
    pub fn store_data_root(store_root: &Path) -> PathBuf {
        store_root.join(PROJECT_DATA_RELATIVE)
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 项目真源的父目录。v3 起在 store 里，不再是 `<project>/.ccswitchx/skills`。
    pub fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    pub fn ledger_path(&self) -> &Path {
        &self.ledger
    }

    pub fn layout_path(&self) -> &Path {
        &self.layout
    }

    /// 旧布局目录，只给搬迁检测用。
    pub fn legacy_dir(&self) -> &Path {
        &self.legacy_dir
    }

    /// 某个 skill 的真源目录。ID 直接参与目录名，先过 ID 安全，
    /// 再确认结果仍在真源根内，防 `..` 之类的构造逃逸。
    pub fn canonical_skill_dir(&self, id: &str) -> Result<PathBuf, AppError> {
        if !crate::store::is_safe_id(id) {
            return Err(AppError::InvalidInput(format!(
                "Skill ID 不能用作目录名：{id}"
            )));
        }
        let path = self.canonical_root.join(id);
        if !path.starts_with(&self.canonical_root) {
            return Err(AppError::InvalidInput(format!(
                "路径越出真源根：{}",
                path.display()
            )));
        }
        Ok(path)
    }

    /// 某个 agent 的项目级渲染目录（仍在项目仓库里）。
    /// 「目录 key + skill id」的组合校验留在 `project_sync::skill_dir`：
    /// 那是渲染路径的唯一写入口，也要保持它原有的 `AppError::Project` 错误档位。
    pub fn render_dir(&self, key: ProjectSkillDirKey) -> PathBuf {
        self.root.join(dir_spec(key).relative_path)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProjectLayoutStatus {
    /// v3：项目里没有我们的文件，数据全在 store 里。
    #[default]
    Native,
    /// 只有项目里的旧布局，还没搬。
    Legacy,
    /// 两边都有数据（上次搬到一半，或用户手工放过）：必须人工裁决，不能自动覆盖。
    Hybrid { collisions: Vec<String> },
    /// 已迁完但旧目录还留着，且内容与 store 逐项一致：只差清理。
    RedundantLegacy { entries: Vec<String> },
    /// 旧目录里有不属于我们的条目：搬迁可以做，但绝不删那个目录。
    Foreign { entries: Vec<String> },
    /// 布局戳记来自更新版本：拒绝一切读写，避免按旧语义删用户文件。
    Unsupported { version: u32 },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDirState {
    pub present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_hash: Option<String>,
    /// 空清单不序列化：台账随仓库提交，多余的 `"files": {}` 只会制造 diff 噪音。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub files: BTreeMap<String, String>,
    /// 目录内文件的最大 mtime（毫秒）。只在冲突时用于推荐“最新一侧”，不写进账本。
    #[serde(skip, default)]
    pub mtime: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSkillSource {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subpath: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSkillOp {
    pub dir: String,
    pub action: String,
    pub at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSkillEntry {
    /// 渲染目标（物理目录 key）。v2 起语义从「对等副本」变为「真源的部署目标」。
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ProjectSkillSource>,
    /// 项目真源（客户端 `local/projects/<id>/skills/<id>/`）的上次已知状态。
    /// `present: false` 表示真源尚未落地 —— 未迁移或未被「收进真源」。
    #[serde(default)]
    pub canonical: ProjectDirState,
    /// 上次成功渲染进各目标的字节基线。
    #[serde(default)]
    pub dirs: BTreeMap<String, ProjectDirState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_op: Option<ProjectSkillOp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectLedger {
    pub version: u32,
    pub updated_at: String,
    #[serde(default)]
    pub skills: BTreeMap<String, ProjectSkillEntry>,
}

impl Default for ProjectLedger {
    fn default() -> Self {
        Self {
            version: 1,
            updated_at: String::new(),
            skills: BTreeMap::new(),
        }
    }
}

/// 去掉 Windows `canonicalize` 加上的 `\\?\` 逐字前缀。
///
/// 前缀只是给 WinAPI 用的（解超长路径与跳过规范化），Rust 标准库在需要时会自己补回，
/// 因此真源与 UI 一律保存/显示去掉前缀的可读路径；`explorer.exe` 也不认逐字形式。
/// 含非法 Unicode 的路径原样返回，不做有损转换。
pub fn strip_verbatim(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(rest) = path.to_str().and_then(|text| text.strip_prefix(r"\\?\UNC\")) {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = path.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

/// 校验项目根：拒绝 home 本身、storeRoot 内部、以及用户级 agent 配置根内部。
///
/// 注意：home 的子目录是允许的（绝大多数项目都在 home 下），只拒绝 home 本身。
/// 返回值是去掉逐字前缀的规范化路径，可直接落进注册表。
///
/// 这条闸还是 v3 搬迁安全的**地基**：正因为项目根不可能位于 storeRoot 内，
/// 「删掉 `<project>/.ccswitchx`」才不可能碰到客户端数据。
/// `ProjectPaths::resolve` 与 `project_relocate::apply` 各自还会再断言一次，不依赖调用方守规矩。
pub fn validate_project_root(
    path: &Path,
    home: &Path,
    store_root: &Path,
    user_level_roots: &[PathBuf],
) -> Result<PathBuf, AppError> {
    if !path.is_dir() {
        return Err(AppError::InvalidInput("项目目录不存在".into()));
    }
    let canonical = path.canonicalize()?;
    let home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let store_root = store_root
        .canonicalize()
        .unwrap_or_else(|_| store_root.to_path_buf());

    if canonical == home {
        return Err(AppError::InvalidInput(
            "不能把用户主目录本身添加为项目".into(),
        ));
    }
    if canonical == store_root || canonical.starts_with(&store_root) {
        return Err(AppError::InvalidInput(
            "不能把 CC Switch X 的数据目录添加为项目".into(),
        ));
    }
    for root in user_level_roots {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        if canonical == root || canonical.starts_with(&root) {
            return Err(AppError::InvalidInput(format!(
                "不能把用户级 agent 配置目录添加为项目：{}",
                root.display()
            )));
        }
    }
    Ok(strip_verbatim(&canonical))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真源目录名直接参与文件系统路径，非法 ID 必须在拼接前挡下。
    #[test]
    fn canonical_skill_dir_rejects_unsafe_ids() {
        let paths = ProjectPaths::resolve(Path::new("D:/store"), "p1", Path::new("D:/repos/app"))
            .unwrap();
        assert_eq!(
            paths.canonical_skill_dir("code-review").unwrap(),
            paths.canonical_root().join("code-review")
        );
        for bad in ["..", "../escape", "a b", "", &"x".repeat(129)] {
            assert!(
                paths.canonical_skill_dir(bad).is_err(),
                "非法 ID 必须被拒绝：{bad:?}"
            );
        }
    }

    /// v3 的三条落点：真源与台账在 store 里，渲染产物仍在项目里。
    #[test]
    fn project_paths_split_store_and_project_sides() {
        let store = Path::new("D:/store");
        let project = Path::new("D:/repos/app");
        let paths = ProjectPaths::resolve(store, "app-1a2b3c4d5e6f", project).unwrap();
        assert_eq!(
            paths.canonical_root(),
            store.join("local/projects/app-1a2b3c4d5e6f/skills")
        );
        assert_eq!(
            paths.ledger_path(),
            store
                .join("local/projects/app-1a2b3c4d5e6f")
                .join("skills.json")
        );
        assert_eq!(paths.data_dir(), paths.layout_path().parent().unwrap());
        assert_eq!(paths.legacy_dir(), project.join(".ccswitchx"));
        // 渲染目标没搬家。
        assert_eq!(
            paths.render_dir(ProjectSkillDirKey::Claude),
            project.join(".claude/skills")
        );
        assert_eq!(
            paths.render_dir(ProjectSkillDirKey::Agents),
            project.join(".agents/skills")
        );
    }

    /// 旧目录与客户端数据目录重叠时绝不能构造出 paths：后面的「删掉 .ccswitchx」会删到 store。
    #[test]
    fn resolve_refuses_overlapping_store_and_legacy_dir() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        // 项目根就是 store 的父目录：.ccswitchx 与 store 互不包含，允许。
        assert!(ProjectPaths::resolve(&store, "p1", dir.path()).is_ok());
        // store 就放在项目的 .ccswitchx 里：删旧目录等于删 store，必须拒绝。
        assert!(ProjectPaths::resolve(&dir.path().join(".ccswitchx"), "p1", dir.path()).is_err());
        // 项目根在 store 内部（validate_project_root 已经拦下，这里再钉一道）。
        assert!(ProjectPaths::resolve(&store, "p1", &store.join("proj")).is_err());
    }

    #[test]
    fn project_skill_dirs_are_stable() {
        let specs = &PROJECT_SKILL_DIRS;
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].key.as_str(), "claude");
        assert_eq!(specs[0].relative_path, ".claude/skills");
        assert_eq!(specs[1].key.as_str(), "qoder");
        assert_eq!(specs[1].relative_path, ".qoder/skills");
        assert_eq!(specs[2].key.as_str(), "agents");
        assert_eq!(specs[2].relative_path, ".agents/skills");
    }

    #[test]
    fn validate_rejects_home_store_root_and_user_level_roots() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let store = dir.path().join("store");
        let claude = home.join(".claude");
        let project = home.join("work/app");
        for path in [&home, &store, &claude] {
            std::fs::create_dir_all(path).unwrap();
        }
        std::fs::create_dir_all(&project).unwrap();

        assert!(validate_project_root(&home, &home, &store, &[claude.clone()]).is_err());
        assert!(validate_project_root(&store, &home, &store, &[claude.clone()]).is_err());
        assert!(validate_project_root(&claude, &home, &store, &[claude.clone()]).is_err());
        let ok = validate_project_root(&project, &home, &store, &[claude]).unwrap();
        assert_eq!(
            ok,
            strip_verbatim(&project.canonicalize().unwrap()),
            "home 下的普通项目必须允许"
        );
        assert!(
            !ok.to_string_lossy().starts_with(r"\\?\"),
            "注册表要存可读路径：{ok:?}"
        );
    }

    #[test]
    fn validate_strips_windows_verbatim_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let store = dir.path().join("store");
        let project = home.join("work/app");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::create_dir_all(&project).unwrap();

        let plain = validate_project_root(&project, &home, &store, &[]).unwrap();
        if cfg!(windows) {
            let verbatim = PathBuf::from(format!(r"\\?\{}", plain.display()));
            assert_eq!(
                validate_project_root(&verbatim, &home, &store, &[]).unwrap(),
                plain,
                "同一项目的逐字与非逐字写法必须得到同一个 ID 输入"
            );
        } else {
            assert_eq!(plain, project.canonicalize().unwrap());
        }
    }

    #[test]
    fn strip_verbatim_only_touches_windows_prefixes() {
        assert_eq!(strip_verbatim(Path::new("/tmp/app")), PathBuf::from("/tmp/app"));
        if cfg!(windows) {
            assert_eq!(
                strip_verbatim(Path::new(r"\\?\D:\repos\ai\PddWorker")),
                PathBuf::from(r"D:\repos\ai\PddWorker")
            );
            assert_eq!(
                strip_verbatim(Path::new(r"\\?\UNC\fileserver\team")),
                PathBuf::from(r"\\fileserver\team")
            );
            assert_eq!(
                strip_verbatim(Path::new(r"D:\repos\ai\PddWorker")),
                PathBuf::from(r"D:\repos\ai\PddWorker")
            );
        }
    }
}
