use crate::error::AppError;
use crate::index::Index;
use crate::settings::AppSettings;
use crate::store::Store;
use std::sync::Mutex;

pub struct AppState {
    pub store: Store,
    pub index: Mutex<Index>,
    pub settings: Mutex<AppSettings>,
}

impl AppState {
    /// 启动即以文件真源重建索引：index.db 只是缓存，可以随时删除。
    pub fn init(settings: AppSettings) -> Result<Self, AppError> {
        let store = Store::init(settings.store_root.clone())?;
        let mut index = Index::open(&settings.store_root.join("index.db"))?;
        index.rebuild(&store)?;
        Ok(Self {
            store,
            index: Mutex::new(index),
            settings: Mutex::new(settings),
        })
    }
}
