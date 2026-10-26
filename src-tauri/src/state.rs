use crate::error::AppError;
use crate::index::Index;
use crate::net::ProxyConfig;
use crate::services::git_sync::GitEvent;
use crate::services::settings::proxy_secret_ref;
use crate::settings::AppSettings;
use crate::store::secrets::{secrets_path, Secrets};
use crate::store::Store;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub struct AppState {
    pub store: Store,
    pub index: Mutex<Index>,
    pub settings: Mutex<AppSettings>,
    pub secrets: Mutex<Secrets>,
    /// 真实家目录：适配器落点里的 `~` 展开到它（被 `local/agents` 的覆盖优先）。
    pub home: PathBuf,
    /// 网络/批量 Skill 任务互斥；重复触发直接报错，避免两个更新同时改真源。
    skill_busy: Arc<AtomicBool>,
    /// 真源写操作（读锁）与快照恢复/拉取（写锁）互斥。
    pub maintenance: Arc<RwLock<()>>,
    /// Git 命令串行；git 自己也会抢 index.lock，应用侧先排好队减少失败。
    pub git_ops: Arc<Mutex<()>>,
    /// 云备份上传 / 下载 / 状态探测串行，避免两个任务同时替换远端对象。
    pub cloud_ops: Arc<Mutex<()>>,
    git_tx: Sender<GitEvent>,
    git_rx: Mutex<Option<Receiver<GitEvent>>>,
}

/// 持有期间占用 Skill 任务槽；Drop 时释放，panic 也不会留下永久锁。
pub struct SkillJobGuard {
    busy: Arc<AtomicBool>,
}

impl Drop for SkillJobGuard {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::SeqCst);
    }
}

impl AppState {
    /// 网络任务启动前取一份代理快照；密码只从 secrets 读，不进设置文件。
    pub fn proxy_config(&self) -> Result<ProxyConfig, AppError> {
        let settings = self
            .settings
            .lock()
            .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
        let secrets = self
            .secrets
            .lock()
            .map_err(|_| AppError::InvalidInput("secrets 锁失败".into()))?;
        let password = secrets.get(&proxy_secret_ref()).map(str::to_string);
        Ok(ProxyConfig::new(
            settings.proxy.url.clone(),
            settings.proxy.username.clone(),
            password,
        ))
    }

    /// 托盘尚未建好时也按「关闭到托盘」处理，避免窗口被误关。
    pub fn close_to_tray(&self) -> bool {
        self.settings
            .lock()
            .map(|settings| settings.close_to_tray)
            .unwrap_or(true)
    }

    pub fn begin_skill_job(&self) -> Result<SkillJobGuard, AppError> {
        self.skill_busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| AppError::InvalidInput("已有 Skill 检查/更新任务在进行".into()))?;
        Ok(SkillJobGuard {
            busy: self.skill_busy.clone(),
        })
    }

    /// 普通写命令持读锁：多个写操作可并行，但会被恢复/拉取整体挡住。
    pub fn enter_write(&self) -> Result<RwLockReadGuard<'_, ()>, AppError> {
        self.maintenance
            .read()
            .map_err(|_| AppError::InvalidInput("maintenance 锁失败".into()))
    }

    /// 快照、恢复、rebase/reset 等需要一致视图的操作持写锁。
    pub fn enter_maintenance(&self) -> Result<RwLockWriteGuard<'_, ()>, AppError> {
        self.maintenance
            .write()
            .map_err(|_| AppError::InvalidInput("maintenance 锁失败".into()))
    }

    /// 写成功后通知 Git worker；是否真正提交由 worker 读当前设置决定。
    pub fn notify_git(&self, message: impl Into<String>) {
        let _ = self.git_tx.send(GitEvent::Changed(message.into()));
    }

    pub fn take_git_rx(&self) -> Option<Receiver<GitEvent>> {
        self.git_rx.lock().ok().and_then(|mut slot| slot.take())
    }
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
        // 上次异常退出可能留下预览暂存；超过一天的直接清掉。
        if let Err(error) = store.cleanup_staging(std::time::Duration::from_secs(24 * 3600)) {
            eprintln!("清理 Skill 暂存目录失败：{error}");
        }
        let (git_tx, git_rx) = mpsc::channel();
        Ok(Self {
            store,
            index: Mutex::new(index),
            settings: Mutex::new(settings),
            secrets: Mutex::new(secrets),
            home,
            skill_busy: Arc::new(AtomicBool::new(false)),
            maintenance: Arc::new(RwLock::new(())),
            git_ops: Arc::new(Mutex::new(())),
            cloud_ops: Arc::new(Mutex::new(())),
            git_tx,
            git_rx: Mutex::new(Some(git_rx)),
        })
    }
}
