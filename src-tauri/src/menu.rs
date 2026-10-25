//! The app menu: the Mac's standard menus, plus Otomo's own items under
//! "About Otomo", where Mac apps keep them. Each item only tells the UI,
//! which owns the settings window and the update flow.

use tauri::menu::{Menu, MenuItem, MenuItemKind, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Runtime};

use crate::i18n::tr;

const CHECK_UPDATES: &str = "check_updates";
const SETTINGS: &str = "settings";

fn label(id: &str) -> String {
    match id {
        CHECK_UPDATES => tr!("アップデートを確認…", "Check for Updates…"),
        _ => tr!("設定…", "Settings…"),
    }
}

/// About Otomo / Check for Updates… / — / Settings… ⌘, / — / Services …
pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = Menu::default(app)?;
    if let Some(MenuItemKind::Submenu(app_menu)) = menu.items()?.into_iter().next() {
        let check = MenuItem::with_id(app, CHECK_UPDATES, label(CHECK_UPDATES), true, None::<&str>)?;
        let settings = MenuItem::with_id(app, SETTINGS, label(SETTINGS), true, Some("CmdOrCtrl+,"))?;
        app_menu.insert(&check, 1)?;
        app_menu.insert(&PredefinedMenuItem::separator(app)?, 2)?;
        app_menu.insert(&settings, 3)?;
    }
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        let event_name = match event.id().as_ref() {
            CHECK_UPDATES => "otomo://check-updates",
            SETTINGS => "otomo://open-settings",
            _ => return,
        };
        if let Err(e) = app.emit(event_name, ()) {
            log::error!("{event_name}: {e}");
        }
    });
    Ok(())
}

/// Re-word Otomo's items for the language the UI just switched to.
pub fn relabel<R: Runtime>(app: &AppHandle<R>) {
    let Some(menu) = app.menu() else { return };
    let Ok(items) = menu.items() else { return };
    let Some(MenuItemKind::Submenu(app_menu)) = items.into_iter().next() else {
        return;
    };
    for id in [CHECK_UPDATES, SETTINGS] {
        if let Some(MenuItemKind::MenuItem(item)) = app_menu.get(id) {
            if let Err(e) = item.set_text(label(id)) {
                log::error!("relabel menu {id}: {e}");
            }
        }
    }
}
