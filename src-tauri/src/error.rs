use crate::domain::DomainError;
use crate::index::IndexError;
use crate::settings::SettingsError;
use crate::store::StoreError;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("存储错误：{0}")]
    Store(#[from] StoreError),
    #[error("索引错误：{0}")]
    Index(#[from] IndexError),
    #[error("资产错误：{0}")]
    Domain(#[from] DomainError),
    #[error("设置错误：{0}")]
    Settings(#[from] SettingsError),
    #[error("密钥保管错误：{0}")]
    Secrets(#[from] crate::store::secrets::SecretsError),
    #[error("导入错误：{0}")]
    Import(#[from] crate::import::ImportError),
    #[error("元数据错误：{0}")]
    Meta(#[from] crate::domain::skill_meta::SkillMetaError),
    #[error("网络请求失败：{0}")]
    Network(String),
    // 消息由命令层带上「检查/下载/安装」上下文，这里不再重复加前缀。
    #[error("{0}")]
    Update(String),
    #[error("压缩包处理失败：{0}")]
    Archive(String),
    #[error("备份失败：{0}")]
    Backup(String),
    #[error("Git 操作失败：{0}")]
    Git(String),
    #[error("开机自启设置失败：{0}")]
    Autostart(String),
    #[error("来源无效：{0}")]
    Source(String),
    #[error("会话解析失败：{0}")]
    Session(#[from] crate::domain::session::SessionError),
    #[error("JSON 处理失败：{0}")]
    Json(#[from] serde_json::Error),
    #[error("未找到：{0}")]
    NotFound(String),
    #[error("非法输入：{0}")]
    InvalidInput(String),
    #[error("暂不支持：{0}")]
    Unsupported(String),
    #[error("未检测到：{0}")]
    NotDetected(String),
    #[error("有未解决的漂移：{0}")]
    DriftUnresolved(String),
}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
