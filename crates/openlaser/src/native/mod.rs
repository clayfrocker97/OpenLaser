// SPDX-License-Identifier: GPL-3.0-or-later

//! The native window owns the server lifetime and shuts it down when closed.

mod instance;
#[cfg(target_os = "macos")]
mod menu;
#[cfg(windows)]
mod runtime;
mod window;

use instance::Instance;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::WindowBuilder;

enum Update {
    #[cfg(target_os = "macos")]
    Quit,
}

pub fn run(arguments: &crate::Arguments, runtime: &tokio::runtime::Runtime) -> Result<(), String> {
    let (config, simulator) = runtime.block_on(crate::configure(arguments))?;
    drop(simulator);
    let Some(mut instance) = Instance::acquire(&config.data_dir)? else { return Ok(()) };
    #[cfg(windows)]
    runtime::ensure(&config.data_dir)?;
    let service = crate::service::ensure(arguments, &config)?;
    let origin = crate::desktop::browser_url(service.address());
    let url = format!("{origin}app");
    let mut events = EventLoopBuilder::<Update>::with_user_event().build();
    #[cfg(target_os = "macos")]
    let _menu = menu::install(events.create_proxy())?;
    let window = WindowBuilder::new()
        .with_title("OpenLaser")
        .with_inner_size(LogicalSize::new(1400.0, 960.0))
        .with_min_inner_size(LogicalSize::new(960.0, 640.0))
        .with_visible(false)
        .build(&events)
        .map_err(|e| e.to_string())?;
    let mut context = wry::WebContext::new(Some(config.data_dir.join("webview")));
    let webview = window::build(&window, &mut context, Rc::new(RefCell::new(origin)))?;
    webview.load_url(&url).map_err(|e| e.to_string())?;
    window.set_visible(true);
    let mut service = Some(service);
    let mut closed = Ok(());
    events.run_return(|event, _, flow| {
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(300));
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                *flow = ControlFlow::Exit;
            }
            #[cfg(target_os = "macos")]
            Event::UserEvent(Update::Quit) => *flow = ControlFlow::Exit,
            Event::NewEvents(_) if instance.activated() => {
                window.set_minimized(false);
                window.set_focus();
            }
            // Sent as the loop ends, including when macOS quits the app from
            // the Dock or at logout, which ends the process as soon as this
            // returns: shut the service down here, never after the loop.
            // This sends the normal admitted shutdown, waits for controller
            // cleanup and for the server to release its listener and lock.
            Event::LoopDestroyed => {
                window.set_visible(false);
                if let Some(service) = service.take() {
                    closed = service.close();
                }
            }
            _ => {}
        }
    });
    match service {
        Some(service) => {
            window.set_visible(false);
            service.close()
        }
        None => closed,
    }
}
