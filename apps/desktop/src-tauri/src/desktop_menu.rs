use tauri::{
    Manager,
    menu::{AboutMetadata, MenuBuilder, MenuItemBuilder, SubmenuBuilder},
};

pub(super) const MENU_EVENT_NAME: &str = "koushi-desktop://menu";
const MENU_ID_ABOUT: &str = "about_koushi";
const MENU_ID_OPEN_USER_SETTINGS: &str = "open_user_settings";
const MENU_ID_SIGN_OUT: &str = "sign_out";
const MENU_ID_SHOW_KEYBOARD_SETTINGS: &str = "show_keyboard_settings";
const MENU_ID_TOGGLE_RIGHT_PANEL: &str = "toggle_right_panel";
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
            id: MENU_ID_SHOW_KEYBOARD_SETTINGS,
            label: "Keyboard Shortcuts",
            menu: "help",
            accelerator: "CmdOrCtrl+/",
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
        MENU_ID_SHOW_KEYBOARD_SETTINGS => Some("showKeyboardSettings"),
        MENU_ID_TOGGLE_FULLSCREEN => Some("toggleFullscreen"),
        _ => None,
    }
}

pub(super) fn build_desktop_menu<R: tauri::Runtime, M: Manager<R>>(
    manager: &M,
) -> tauri::Result<tauri::menu::Menu<R>> {
    let open_user_settings = menu_item(manager, MENU_ID_OPEN_USER_SETTINGS)?;
    let sign_out = menu_item(manager, MENU_ID_SIGN_OUT)?;
    let toggle_right_panel = menu_item(manager, MENU_ID_TOGGLE_RIGHT_PANEL)?;
    let show_keyboard_settings = menu_item(manager, MENU_ID_SHOW_KEYBOARD_SETTINGS)?;

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
        let builder = SubmenuBuilder::new(manager, "View").item(&toggle_right_panel);
        #[cfg(target_os = "macos")]
        let builder = builder.item(&toggle_fullscreen);
        builder.build()?
    };
    let help_menu = SubmenuBuilder::new(manager, "Help")
        .item(&show_keyboard_settings)
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
