use crate::error::AppError;
use crate::index::Index;
use crate::settings::AppSettings;
use crate::store::secrets::{secrets_path, Secrets};
use crate::store::Store;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct AppState {
    pub store: Store,
    pub index: Mutex<Index>,
    pub settings: Mutex<AppSettings>,
    pub secrets: Mutex<Secrets>,
    /// 真实家目录：适配器落点里的 `~` 展开到它（被 `local/agents` 的覆盖优先）。
    pub home: PathBuf,
}

impl AppState {
    /// 启动即以文件真源重建索引：index.db 只是缓存，可以随时删除。
    pub fn init(settings: AppSettings) -> Result<Self, AppError> {
        let home =
            dirs::home_dir().ok_or_else(|| AppError::InvalidInput("找不到用户主目录".into()))?;
        let store = Store::init(settings.store_root.clone())?;
        let mut index = Index::open(&settings.store_root.join("index.db"))?;
        index.rebuild(&store)?;
        let secrets = Secrets::load(&secrets_path(store.root()))?;
        Ok(Self {
            store,
            index: Mutex::new(index),
            settings: Mutex::new(settings),
            secrets: Mutex::new(secrets),
            home,
        })
    }
}
