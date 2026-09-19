use tauri::{
    Manager,
    menu::{AboutMetadata, MenuBuilder, MenuItemBuilder, SubmenuBuilder},
};

pub(super) const MENU_EVENT_NAME: &str = "koushi-desktop://menu";
const MENU_ID_ABOUT: &str = "about_koushi";
const MENU_ID_OPEN_USER_SETTINGS: &str = "open_user_settings";
const MENU_ID_SIGN_OUT: &str = "sign_out";
const MENU_ID_SHOW_HELP: &str = "show_help";
const MENU_ID_CHECK_FOR_UPDATES: &str = "check_for_updates";
const MENU_ID_TOGGLE_RIGHT_PANEL: &str = "toggle_right_panel";
const MENU_ID_ZOOM_IN: &str = "zoom_in";
const MENU_ID_ZOOM_OUT: &str = "zoom_out";
const MENU_ID_RESET_ZOOM: &str = "reset_zoom";
pub(super) const MENU_ID_TOGGLE_FULLSCREEN: &str = "toggle_fullscreen";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DesktopMenuItem {
    pub id: &'static str,
    pub label: &'static str,
    pub menu: &'static str,
    pub accelerator: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg(test)]
pub(crate) struct DesktopStandardMenuItem {
    pub id: &'static str,
    pub label: &'static str,
    pub menu: &'static str,
    pub accelerator: &'static str,
}

pub(crate) fn desktop_menu_items() -> Vec<DesktopMenuItem> {
    vec![
        DesktopMenuItem {
            id: MENU_ID_ABOUT,
            label: "About Koushi",
            menu: "app",
            accelerator: "",
        },
        DesktopMenuItem {
            id: MENU_ID_OPEN_USER_SETTINGS,
            label: "User Settings",
            menu: "app",
            accelerator: "CmdOrCtrl+,",
        },
        DesktopMenuItem {
            id: MENU_ID_SIGN_OUT,
            label: "Sign Out",
            menu: "app",
            accelerator: "",
        },
        DesktopMenuItem {
            id: MENU_ID_TOGGLE_RIGHT_PANEL,
            label: "Toggle Right Panel",
            menu: "view",
            accelerator: "CmdOrCtrl+.",
        },
        DesktopMenuItem {
            id: MENU_ID_SHOW_HELP,
            label: "Koushi Help",
            menu: "help",
            accelerator: "",
        },
        DesktopMenuItem {
            id: MENU_ID_CHECK_FOR_UPDATES,
            label: "Check for Updates…",
            menu: "help",
            accelerator: "",
        },
        DesktopMenuItem {
            id: MENU_ID_ZOOM_IN,
            label: "Zoom In",
            menu: "view",
            // AppKit receives the '+' character for NumpadAdd, including '+'
            // typed on the main US/JIS keyboard. Muda has no literal Plus token.
            accelerator: "CmdOrCtrl+NumpadAdd",
        },
        DesktopMenuItem {
            id: MENU_ID_ZOOM_OUT,
            label: "Zoom Out",
            menu: "view",
            accelerator: "CmdOrCtrl+-",
        },
        DesktopMenuItem {
            id: MENU_ID_RESET_ZOOM,
            label: "Actual Size",
            menu: "view",
            accelerator: "CmdOrCtrl+0",
        },
        #[cfg(target_os = "macos")]
        DesktopMenuItem {
            id: MENU_ID_TOGGLE_FULLSCREEN,
            label: "Toggle Fullscreen",
            menu: "view",
            accelerator: "Ctrl+Command+F",
        },
    ]
}

#[cfg(test)]
pub(crate) fn desktop_standard_menu_items() -> Vec<DesktopStandardMenuItem> {
    vec![
        DesktopStandardMenuItem {
            id: "close_window",
            label: "Close Window",
            menu: "file",
            accelerator: "CmdOrCtrl+W",
        },
        DesktopStandardMenuItem {
            id: "quit",
            label: "Quit",
            menu: "app",
            accelerator: "CmdOrCtrl+Q",
        },
    ]
}

pub(super) fn desktop_menu_action_id(menu_id: &str) -> Option<&'static str> {
    match menu_id {
        MENU_ID_OPEN_USER_SETTINGS => Some("openUserSettings"),
        MENU_ID_SIGN_OUT => Some("logout"),
        MENU_ID_TOGGLE_RIGHT_PANEL => Some("toggleRightPanel"),
        MENU_ID_SHOW_HELP => Some("showHelp"),
        MENU_ID_CHECK_FOR_UPDATES => Some("checkForUpdates"),
        MENU_ID_TOGGLE_FULLSCREEN => Some("toggleFullscreen"),
        MENU_ID_ZOOM_IN => Some("zoomIn"),
        MENU_ID_ZOOM_OUT => Some("zoomOut"),
        MENU_ID_RESET_ZOOM => Some("resetZoom"),
        _ => None,
    }
}

pub(super) fn build_desktop_menu<R: tauri::Runtime, M: Manager<R>>(
    manager: &M,
) -> tauri::Result<tauri::menu::Menu<R>> {
    let open_user_settings = menu_item(manager, MENU_ID_OPEN_USER_SETTINGS)?;
    let sign_out = menu_item(manager, MENU_ID_SIGN_OUT)?;
    let toggle_right_panel = menu_item(manager, MENU_ID_TOGGLE_RIGHT_PANEL)?;
    let show_help = menu_item(manager, MENU_ID_SHOW_HELP)?;
    let check_for_updates = menu_item(manager, MENU_ID_CHECK_FOR_UPDATES)?;
    let zoom_in = menu_item(manager, MENU_ID_ZOOM_IN)?;
    let zoom_out = menu_item(manager, MENU_ID_ZOOM_OUT)?;
    let reset_zoom = menu_item(manager, MENU_ID_RESET_ZOOM)?;

    #[cfg(target_os = "macos")]
    let toggle_fullscreen = menu_item(manager, MENU_ID_TOGGLE_FULLSCREEN)?;

    let about_metadata = AboutMetadata {
        name: manager
            .config()
            .product_name
            .clone()
            .or_else(|| Some("Koushi".to_owned())),
        version: Some(manager.package_info().version.to_string()),
        copyright: manager.config().bundle.copyright.clone(),
        license: Some("MIT OR Apache-2.0".to_owned()),
        website: Some("https://github.com/shinaoka/koushi-matrix".to_owned()),
        website_label: Some("Koushi on GitHub".to_owned()),
        icon: manager.app_handle().default_window_icon().cloned(),
        ..Default::default()
    };
    let app_menu = SubmenuBuilder::new(manager, "Koushi")
        .about_with_text("About Koushi", Some(about_metadata))
        .separator()
        .item(&open_user_settings)
        .item(&sign_out)
        .separator()
        .quit()
        .build()?;
    let file_menu = SubmenuBuilder::new(manager, "File")
        .close_window()
        .build()?;
    let edit_menu = SubmenuBuilder::new(manager, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let view_menu = {
        let builder = SubmenuBuilder::new(manager, "View")
            .item(&toggle_right_panel)
            .separator()
            .item(&zoom_in)
            .item(&zoom_out)
            .item(&reset_zoom)
            .separator();
        #[cfg(target_os = "macos")]
        let builder = builder.item(&toggle_fullscreen);
        builder.build()?
    };
    let help_menu = SubmenuBuilder::new(manager, "Help")
        .item(&show_help)
        .item(&check_for_updates)
        .build()?;

    MenuBuilder::new(manager)
        .item(&app_menu)
        .item(&file_menu)
        .item(&edit_menu)
        .item(&view_menu)
        .item(&help_menu)
        .build()
}

#[cfg(target_os = "macos")]
pub(super) fn toggle_main_window_fullscreen(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if let Ok(fullscreen) = window.is_fullscreen() {
            let _ = window.set_fullscreen(!fullscreen);
        }
    }
}

fn menu_item<R: tauri::Runtime, M: Manager<R>>(
    manager: &M,
    id: &str,
) -> tauri::Result<tauri::menu::MenuItem<R>> {
    let item = desktop_menu_items()
        .into_iter()
        .find(|item| item.id == id)
        .expect("desktop menu item id should be registered");
    let builder = MenuItemBuilder::with_id(item.id, item.label);
    if item.accelerator.is_empty() {
        builder.build(manager)
    } else {
        builder.accelerator(item.accelerator).build(manager)
    }
}

#[cfg(test)]
mod tests {
    use super::{desktop_menu_action_id, desktop_menu_items};

    #[test]
    fn native_zoom_keys_dispatch_to_the_shared_webview_zoom_owner() {
        let items = desktop_menu_items();
        for (id, action, accelerator) in [
            ("zoom_in", "zoomIn", "CmdOrCtrl+NumpadAdd"),
            ("zoom_out", "zoomOut", "CmdOrCtrl+-"),
            ("reset_zoom", "resetZoom", "CmdOrCtrl+0"),
        ] {
            let item = items.iter().find(|item| item.id == id).unwrap();
            assert_eq!(item.menu, "view");
            assert_eq!(item.accelerator, accelerator);
            assert_eq!(desktop_menu_action_id(item.id), Some(action));
        }
    }

    #[test]
    fn help_menu_dispatches_help_without_a_keyboard_shortcut() {
        let items = desktop_menu_items();
        let help = items.iter().find(|item| item.menu == "help").unwrap();
        assert_eq!(help.label, "Koushi Help");
        assert!(help.accelerator.is_empty());
        assert_eq!(desktop_menu_action_id(help.id), Some("showHelp"));
        assert_eq!(desktop_menu_action_id("show_keyboard_settings"), None);
        let check = items
            .iter()
            .find(|item| item.id == "check_for_updates")
            .unwrap();
        assert_eq!(check.label, "Check for Updates…");
        assert_eq!(desktop_menu_action_id(check.id), Some("checkForUpdates"));
    }
}
