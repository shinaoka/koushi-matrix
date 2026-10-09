use std::collections::HashMap;

use tauri::{
    Manager,
    menu::{AboutMetadata, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder},
};

pub(super) const MENU_EVENT_NAME: &str = "koushi-desktop://menu";
const MENU_ID_ABOUT: &str = "about_koushi";
const MENU_ID_OPEN_ACCOUNT_SETTINGS: &str = "open_account_settings";
const MENU_ID_OPEN_APP_SETTINGS: &str = "open_app_settings";
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
    /// Catalog message id for the user-facing label, per docs/architecture/i18n.md.
    /// `label` stays as the English fallback used before the webview resolves it.
    pub label_key: &'static str,
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
            label_key: "menu.aboutKoushi",
            label: "About Koushi",
            menu: "app",
            accelerator: "",
        },
        DesktopMenuItem {
            id: MENU_ID_OPEN_ACCOUNT_SETTINGS,
            label_key: "menu.accountSettings",
            label: "Account Settings…",
            menu: "app",
            accelerator: "",
        },
        DesktopMenuItem {
            id: MENU_ID_OPEN_APP_SETTINGS,
            label_key: "menu.appSettings",
            label: "App Settings…",
            menu: "app",
            accelerator: "CmdOrCtrl+,",
        },
        DesktopMenuItem {
            id: MENU_ID_SIGN_OUT,
            label_key: "menu.signOut",
            label: "Sign Out",
            menu: "app",
            accelerator: "",
        },
        DesktopMenuItem {
            id: MENU_ID_TOGGLE_RIGHT_PANEL,
            label_key: "menu.toggleRightPanel",
            label: "Toggle Right Panel",
            menu: "view",
            accelerator: "CmdOrCtrl+.",
        },
        DesktopMenuItem {
            id: MENU_ID_SHOW_HELP,
            label_key: "menu.koushiHelp",
            label: "Koushi Help",
            menu: "help",
            accelerator: "",
        },
        DesktopMenuItem {
            id: MENU_ID_CHECK_FOR_UPDATES,
            label_key: "menu.checkForUpdates",
            label: "Check for Updates…",
            menu: "app",
            accelerator: "",
        },
        DesktopMenuItem {
            id: MENU_ID_ZOOM_IN,
            label_key: "menu.zoomIn",
            label: "Zoom In",
            menu: "view",
            // AppKit receives the '+' character for NumpadAdd, including '+'
            // typed on the main US/JIS keyboard. Muda has no literal Plus token.
            accelerator: "CmdOrCtrl+NumpadAdd",
        },
        DesktopMenuItem {
            id: MENU_ID_ZOOM_OUT,
            label_key: "menu.zoomOut",
            label: "Zoom Out",
            menu: "view",
            accelerator: "CmdOrCtrl+-",
        },
        DesktopMenuItem {
            id: MENU_ID_RESET_ZOOM,
            label_key: "menu.actualSize",
            label: "Actual Size",
            menu: "view",
            accelerator: "CmdOrCtrl+0",
        },
        #[cfg(target_os = "macos")]
        DesktopMenuItem {
            id: MENU_ID_TOGGLE_FULLSCREEN,
            label_key: "menu.toggleFullscreen",
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
        MENU_ID_OPEN_ACCOUNT_SETTINGS => Some("openAccountSettings"),
        MENU_ID_OPEN_APP_SETTINGS => Some("openAppSettings"),
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

/// Catalog message ids and English fallbacks for the menu labels that are not
/// built from `DesktopMenuItem`: the submenu titles Koushi authors, and the
/// predefined items muda would otherwise title with its own English text.
const MENU_PLAIN_LABELS: [(&str, &str); 12] = [
    ("menu.file", "File"),
    ("menu.edit", "Edit"),
    ("menu.view", "View"),
    ("menu.help", "Help"),
    ("menu.undo", "Undo"),
    ("menu.redo", "Redo"),
    ("menu.cut", "Cut"),
    ("menu.copy", "Copy"),
    ("menu.paste", "Paste"),
    ("menu.selectAll", "Select All"),
    ("menu.closeWindow", "Close Window"),
    ("menu.quit", "Quit Koushi"),
];

/// Localized labels resolved by the webview, keyed by the catalog message ids
/// this module owns. Missing keys fall back to the built-in English literals.
pub(super) type MenuLabels = HashMap<String, String>;

fn localized<'a>(labels: &'a MenuLabels, key: &str) -> &'a str {
    let fallback = MENU_PLAIN_LABELS
        .iter()
        .find(|(label_key, _)| *label_key == key)
        .map(|(_, fallback)| *fallback)
        .or_else(|| {
            desktop_menu_items()
                .iter()
                .find(|item| item.label_key == key)
                .map(|item| item.label)
        })
        .expect("menu label key should be registered");
    labels.get(key).map(String::as_str).unwrap_or(fallback)
}

fn native_menu_label_ids() -> Vec<String> {
    let mut ids: Vec<String> = desktop_menu_items()
        .iter()
        .map(|item| item.label_key)
        .chain(MENU_PLAIN_LABELS.iter().map(|(key, _)| *key))
        .map(str::to_owned)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// The catalog message ids the native menu is built from. The webview resolves
/// them and pushes the localized strings back through `set_native_menu_labels`.
#[tauri::command]
pub(crate) fn native_menu_label_keys() -> Vec<String> {
    native_menu_label_ids()
}

#[tauri::command]
pub(crate) fn set_native_menu_labels(
    app: tauri::AppHandle,
    labels: MenuLabels,
) -> Result<(), String> {
    // The menu is built before the webview knows the catalog locale, so the
    // resolved labels are pushed back and the menu is rebuilt and re-applied.
    let menu = build_desktop_menu(&app, &labels).map_err(|error| error.to_string())?;
    app.set_menu(menu).map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn build_desktop_menu<R: tauri::Runtime, M: Manager<R>>(
    manager: &M,
    labels: &MenuLabels,
) -> tauri::Result<tauri::menu::Menu<R>> {
    let open_account_settings = menu_item(manager, MENU_ID_OPEN_ACCOUNT_SETTINGS, labels)?;
    let open_app_settings = menu_item(manager, MENU_ID_OPEN_APP_SETTINGS, labels)?;
    let sign_out = menu_item(manager, MENU_ID_SIGN_OUT, labels)?;
    let toggle_right_panel = menu_item(manager, MENU_ID_TOGGLE_RIGHT_PANEL, labels)?;
    let show_help = menu_item(manager, MENU_ID_SHOW_HELP, labels)?;
    let check_for_updates = menu_item(manager, MENU_ID_CHECK_FOR_UPDATES, labels)?;
    let zoom_in = menu_item(manager, MENU_ID_ZOOM_IN, labels)?;
    let zoom_out = menu_item(manager, MENU_ID_ZOOM_OUT, labels)?;
    let reset_zoom = menu_item(manager, MENU_ID_RESET_ZOOM, labels)?;

    #[cfg(target_os = "macos")]
    let toggle_fullscreen = menu_item(manager, MENU_ID_TOGGLE_FULLSCREEN, labels)?;

    // muda titles predefined items in English unless it is given text, so the
    // whole menu bar resolves through the catalog.
    let undo = PredefinedMenuItem::undo(manager, Some(localized(labels, "menu.undo")))?;
    let redo = PredefinedMenuItem::redo(manager, Some(localized(labels, "menu.redo")))?;
    let cut = PredefinedMenuItem::cut(manager, Some(localized(labels, "menu.cut")))?;
    let copy = PredefinedMenuItem::copy(manager, Some(localized(labels, "menu.copy")))?;
    let paste = PredefinedMenuItem::paste(manager, Some(localized(labels, "menu.paste")))?;
    let select_all =
        PredefinedMenuItem::select_all(manager, Some(localized(labels, "menu.selectAll")))?;
    let close_window =
        PredefinedMenuItem::close_window(manager, Some(localized(labels, "menu.closeWindow")))?;
    let quit = PredefinedMenuItem::quit(manager, Some(localized(labels, "menu.quit")))?;

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
        .about_with_text(localized(labels, "menu.aboutKoushi"), Some(about_metadata))
        .item(&check_for_updates)
        .separator()
        .item(&open_account_settings)
        .item(&open_app_settings)
        .item(&sign_out)
        .separator()
        .item(&quit)
        .build()?;
    let file_menu = SubmenuBuilder::new(manager, localized(labels, "menu.file"))
        .item(&close_window)
        .build()?;
    let edit_menu = SubmenuBuilder::new(manager, localized(labels, "menu.edit"))
        .item(&undo)
        .item(&redo)
        .separator()
        .item(&cut)
        .item(&copy)
        .item(&paste)
        .item(&select_all)
        .build()?;
    let view_menu = {
        let builder = SubmenuBuilder::new(manager, localized(labels, "menu.view"))
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
    let help_menu = SubmenuBuilder::new(manager, localized(labels, "menu.help"))
        .item(&show_help)
        .build()?;

    MenuBuilder::new(manager)
        .item(&app_menu)
        .item(&file_menu)
        .item(&edit_menu)
        .item(&view_menu)
        .item(&help_menu)
        .build()
}

/// Opt out before AppKit finishes launching: Koushi already owns one localized
/// fullscreen toggle. Apple's injected item otherwise duplicates it and keeps
/// an inaccurate Enter title after a programmatic fullscreen transition.
/// https://developer.apple.com/library/archive/releasenotes/AppKit/RN-AppKitOlderNotes/index.html
#[cfg(target_os = "macos")]
pub(super) fn configure_fullscreen_menu() {
    objc2_foundation::NSUserDefaults::standardUserDefaults().setBool_forKey(
        false,
        &objc2_foundation::NSString::from_str("NSFullScreenMenuItemEverywhere"),
    );
}

#[cfg(target_os = "macos")]
pub(super) fn toggle_main_window_fullscreen(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main")
        && let Ok(fullscreen) = window.is_fullscreen()
    {
        let _ = window.set_fullscreen(!fullscreen);
    }
}

fn menu_item<R: tauri::Runtime, M: Manager<R>>(
    manager: &M,
    id: &str,
    labels: &MenuLabels,
) -> tauri::Result<tauri::menu::MenuItem<R>> {
    let item = desktop_menu_items()
        .into_iter()
        .find(|item| item.id == id)
        .expect("desktop menu item id should be registered");
    let builder = MenuItemBuilder::with_id(item.id, localized(labels, item.label_key));
    if item.accelerator.is_empty() {
        builder.build(manager)
    } else {
        builder.accelerator(item.accelerator).build(manager)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MenuLabels, desktop_menu_action_id, desktop_menu_items, localized, native_menu_label_keys,
    };

    #[cfg(target_os = "macos")]
    #[test]
    fn fullscreen_menu_opt_out_preserves_one_authored_toggle() {
        super::configure_fullscreen_menu();
        assert!(
            !objc2_foundation::NSUserDefaults::standardUserDefaults().boolForKey(
                &objc2_foundation::NSString::from_str("NSFullScreenMenuItemEverywhere"),
            )
        );
        let items = desktop_menu_items();
        let fullscreen: Vec<_> = items
            .iter()
            .filter(|item| item.id == "toggle_fullscreen")
            .collect();
        assert_eq!(fullscreen.len(), 1);
        assert_eq!(fullscreen[0].accelerator, "Ctrl+Command+F");
        assert_eq!(fullscreen[0].label_key, "menu.toggleFullscreen");
    }

    #[test]
    fn account_and_app_settings_are_separate_menu_items_with_distinct_targets() {
        let items = desktop_menu_items();
        let account = items
            .iter()
            .find(|item| item.id == "open_account_settings")
            .expect("Account Settings menu item should exist");
        let app = items
            .iter()
            .find(|item| item.id == "open_app_settings")
            .expect("App Settings menu item should exist");

        assert_eq!(account.menu, "app");
        assert_eq!(account.label_key, "menu.accountSettings");
        assert_eq!(account.label, "Account Settings…");
        assert!(account.accelerator.is_empty());
        assert_eq!(
            desktop_menu_action_id(account.id),
            Some("openAccountSettings")
        );

        assert_eq!(app.menu, "app");
        assert_eq!(app.label_key, "menu.appSettings");
        assert_eq!(app.label, "App Settings…");
        assert_eq!(app.accelerator, "CmdOrCtrl+,");
        assert_eq!(desktop_menu_action_id(app.id), Some("openAppSettings"));
    }

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
        assert_eq!(check.menu, "app");
        assert_eq!(desktop_menu_action_id(check.id), Some("checkForUpdates"));
    }

    #[test]
    fn every_koushi_authored_label_resolves_through_the_message_catalog() {
        let ids = native_menu_label_keys();
        for key in [
            "menu.aboutKoushi",
            "menu.accountSettings",
            "menu.appSettings",
            "menu.signOut",
            "menu.toggleRightPanel",
            "menu.koushiHelp",
            "menu.checkForUpdates",
            "menu.zoomIn",
            "menu.zoomOut",
            "menu.actualSize",
            "menu.file",
            "menu.edit",
            "menu.view",
            "menu.help",
            "menu.undo",
            "menu.redo",
            "menu.cut",
            "menu.copy",
            "menu.paste",
            "menu.selectAll",
            "menu.closeWindow",
            "menu.quit",
        ] {
            assert!(
                ids.contains(&key.to_owned()),
                "{key} is not offered to the webview"
            );
        }

        // English stays the fallback for any key the webview has not resolved.
        let japanese = MenuLabels::from([("menu.view".to_owned(), "表示".to_owned())]);
        assert_eq!(localized(&japanese, "menu.view"), "表示");
        assert_eq!(localized(&japanese, "menu.actualSize"), "Actual Size");
        assert_eq!(localized(&japanese, "menu.copy"), "Copy");
        assert_eq!(localized(&Default::default(), "menu.view"), "View");
    }
}
