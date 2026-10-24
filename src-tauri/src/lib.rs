pub mod adapters;
pub mod commands;
pub mod domain;
pub mod error;
pub mod import;
pub mod index;
pub mod net;
pub mod services;
pub mod sessions;
pub mod settings;
pub mod state;
pub mod store;
pub mod tray;

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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        // 启动后先扫一遍落盘（spec 5.4 的触发时机之一）。扫描失败不拦启动：
        // 把原因打到标准错误，面板上的「重新检查」还能手动触发。
        .setup(|app| {
            use tauri::Manager;
            tray::setup(app)?;
            let app_state = app.state::<state::AppState>();
            let Ok(mut index) = app_state.index.lock() else {
                eprintln!("跳过漂移检查：index 锁已被 panic 污染，请用面板上的「重新检查」");
                return Ok(());
            };
            match services::drift::rescan(&app_state.store, &mut index) {
                Ok(records) if !records.is_empty() => eprintln!(
                    "检测到 {} 处落盘漂移，请在面板里逐条选择「采纳磁盘」或「重刷真源」",
                    records.len()
                ),
                Ok(_) => {}
                Err(error) => eprintln!("启动时的漂移检查失败：{error}"),
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            commands::agents::activate_prompt,
            commands::agents::deactivate_prompt,
            commands::agents::list_agents,
            commands::agents::set_mcp_enabled,
            commands::agents::set_skill_enabled,
            commands::assets::list_assets,
            commands::assets::read_asset,
            commands::assets::write_asset,
            commands::assets::delete_asset,
            commands::assets::import_skill_dir,
            commands::drift::list_drifts,
            commands::drift::rescan_drift,
            commands::drift::resolve_drift,
            commands::hooks::foreign_hook_count,
            commands::hooks::hooks_gate_status,
            commands::hooks::set_hook_enabled,
            commands::import::import_apply,
            commands::import::import_preview,
            commands::import::revert_last,
            commands::rules::scan_rules,
            commands::rules::set_rule_enabled,
            commands::sessions::export_session_markdown,
            commands::sessions::get_session,
            commands::sessions::list_sessions,
            commands::sessions::scan_sessions,
            commands::sessions::search_in_session,
            commands::sessions::search_sessions,
            commands::sessions::session_providers,
            commands::sessions::session_resume,
            commands::skills::check_skill_updates,
            commands::skills::discover_available_skills,
            commands::skills::get_skill_repos,
            commands::skills::install_discovered_skill,
            commands::skills::install_skill,
            commands::skills::preview_skill_source,
            commands::skills::search_skills_sh,
            commands::skills::set_skill_repos,
            commands::skills::set_skills_enabled,
            commands::skills::uninstall_skills,
            commands::skills::update_skills,
            commands::subagents::scan_subagents,
            commands::subagents::set_subagent_enabled,
            commands::settings::get_settings,
            commands::settings::get_proxy_settings,
            commands::settings::scan_local_proxies,
            commands::settings::set_close_to_tray,
            commands::settings::set_enabled_agents,
            commands::settings::set_proxy_settings,
            commands::settings::set_store_root,
            commands::settings::set_theme,
            commands::settings::test_proxy_settings,
            commands::updater::check_app_update,
            commands::updater::install_app_update,
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
