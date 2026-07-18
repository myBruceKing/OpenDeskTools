mod commands;
mod infrastructure;

use infrastructure::application::ApplicationRuntime;
use infrastructure::windowing::configure_main_window;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let runtime = ApplicationRuntime::initialize(app.handle())?;
            app.manage(runtime);
            if let Some(window) = app.get_webview_window("main") {
                configure_main_window(&window)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::overview::get_overview_view_model,
            commands::theme::get_theme_preferences,
            commands::theme::update_theme_preferences
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
