#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            #[cfg(desktop)]
            {
                tray::create(app)?;
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(desktop)]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running OpenDeskTools");
}

#[cfg(desktop)]
mod tray {
    use tauri::{
        menu::{Menu, MenuItem, PredefinedMenuItem},
        tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
        App, AppHandle, Manager,
    };

    const SHOW_SETTINGS_ID: &str = "show_settings";
    const QUICK_PANEL_ID: &str = "quick_panel";
    const QUIT_ID: &str = "quit";

    pub fn create(app: &App) -> tauri::Result<()> {
        let show_settings =
            MenuItem::with_id(app, SHOW_SETTINGS_ID, "打开设置", true, None::<&str>)?;
        let quick_panel =
            MenuItem::with_id(app, QUICK_PANEL_ID, "显示快捷面板", true, None::<&str>)?;
        let separator = PredefinedMenuItem::separator(app)?;
        let quit = MenuItem::with_id(app, QUIT_ID, "退出 OpenDeskTools", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&show_settings, &quick_panel, &separator, &quit])?;

        let mut builder = TrayIconBuilder::with_id("main")
            .tooltip("OpenDeskTools")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| match event.id().as_ref() {
                SHOW_SETTINGS_ID | QUICK_PANEL_ID => show_main_window(app),
                QUIT_ID => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                } = event
                {
                    show_main_window(tray.app_handle());
                }
            });

        if let Some(icon) = app.default_window_icon().cloned() {
            builder = builder.icon(icon);
        }

        builder.build(app)?;
        Ok(())
    }

    fn show_main_window(app: &AppHandle) {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }
}
