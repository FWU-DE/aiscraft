//! The macOS app menu in German: About, Hide, Hide Others, Show All and Quit.
//!
//! winit installs an English menu of its own whose Quit (⌘Q) ends the process at once. This one
//! sends the root window a close request instead, so the app quits the same way as with the close
//! button (`craft_quit::quit()` in `AisCraftApp::logic`). Call it once, from the eframe creator:
//! winit builds its default menu before that and would replace one installed earlier.

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

const NAME: &str = "AIS.ChatCraft";

/// Replace winit's default menu. On error winit's menu stays and the app still runs.
pub fn install_app_menu(ctx: &egui::Context) {
    if let Err(e) = build(ctx) {
        log::warn!("macOS-Menü nicht installiert: {e}");
    }
}

fn build(ctx: &egui::Context) -> Result<(), Box<dyn std::error::Error>> {
    let quit = MenuItem::new(format!("{NAME} beenden"), true, Some(Accelerator::new(Modifiers::META, Code::KeyQ)));
    let quit_id = quit.id().clone();
    let app_menu = Submenu::new(NAME, true);
    app_menu.append_items(&[
        &PredefinedMenuItem::about(Some(&format!("Über {NAME}")), None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::services(Some("Dienste")),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::hide(Some(&format!("{NAME} ausblenden"))),
        &PredefinedMenuItem::hide_others(Some("Andere ausblenden")),
        &PredefinedMenuItem::show_all(Some("Alle einblenden")),
        &PredefinedMenuItem::separator(),
        &quit,
    ])?;
    let menu = Menu::new();
    menu.append(&app_menu)?;
    menu.init_for_nsapp();
    let ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if e.id == quit_id {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ctx.request_repaint();
        }
    }));
    // The menu must outlive this function; AppKit holds only the native items.
    std::mem::forget(menu);
    Ok(())
}
