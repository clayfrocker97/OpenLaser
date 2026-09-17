// SPDX-License-Identifier: GPL-3.0-or-later

//! Standard macOS editing shortcuts and a quit action for the client window.

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tao::event_loop::EventLoopProxy;

pub fn install(proxy: EventLoopProxy<super::Update>) -> Result<Menu, String> {
    let quit =
        MenuItem::new("Quit OpenLaser", true, Some(Accelerator::new(Modifiers::META, Code::KeyQ)));
    let app = Submenu::with_items(
        "OpenLaser",
        true,
        &[&PredefinedMenuItem::hide(None), &PredefinedMenuItem::separator(), &quit],
    )
    .map_err(|e| e.to_string())?;
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ],
    )
    .map_err(|e| e.to_string())?;
    let menu = Menu::with_items(&[&app, &edit]).map_err(|e| e.to_string())?;
    menu.init_for_nsapp();
    let quit_id = quit.id().clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == quit_id {
            let _ = proxy.send_event(super::Update::Quit);
        }
    }));
    Ok(menu)
}
