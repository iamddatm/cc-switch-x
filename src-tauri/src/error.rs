use crate::domain::DomainError;
use crate::index::IndexError;
use crate::settings::SettingsError;
use crate::store::StoreError;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("存储错误：{0}")]
    Store(#[from] StoreError),
    #[error("索引错误：{0}")]
    Index(#[from] IndexError),
    #[error("资产错误：{0}")]
    Domain(#[from] DomainError),
    #[error("设置错误：{0}")]
    Settings(#[from] SettingsError),
    #[error("未找到：{0}")]
    NotFound(String),
    #[error("非法输入：{0}")]
    InvalidInput(String),
    #[error("暂不支持：{0}")]
    Unsupported(String),
}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
