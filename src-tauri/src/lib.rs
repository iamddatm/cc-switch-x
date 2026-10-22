pub mod commands;
pub mod domain;
pub mod error;
pub mod index;
pub mod services;
pub mod settings;
pub mod state;
pub mod store;

#[tauri::command]
fn ping() -> &'static str {
    "pong"
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 设置文件损坏时不静默覆盖：先告警再用默认值启动，用户可立即看到原因。
    let settings = match settings::load() {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("读取本机设置失败，本次以默认设置启动：{e}");
            settings::AppSettings::default()
        }
    };
    let state = state::AppState::init(settings).expect("初始化应用状态失败");
    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            ping,
            commands::assets::list_assets,
            commands::assets::read_asset,
            commands::assets::write_asset,
            commands::assets::delete_asset,
            commands::settings::get_settings,
            commands::settings::set_theme,
            commands::settings::set_store_root,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    #[test]
    fn ping_returns_pong() {
        assert_eq!(super::ping(), "pong");
    }
}
