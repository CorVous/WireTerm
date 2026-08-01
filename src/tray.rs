//! Windows notification-area integration for `WireTerm`'s single host process.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use eframe::egui;
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

const OPEN_MENU_ID: &str = "wireterm-open";
const QUIT_MENU_ID: &str = "wireterm-quit";
const ICON_SIZE: u32 = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayAction {
    Open,
    Quit,
}

pub struct TrayIntegration {
    _icon: TrayIcon,
    actions: Arc<Mutex<VecDeque<TrayAction>>>,
}

impl TrayIntegration {
    pub fn new(ctx: &egui::Context) -> Result<Self, String> {
        let open = MenuItem::with_id(OPEN_MENU_ID, "Open WireTerm", true, None);
        let quit = MenuItem::with_id(QUIT_MENU_ID, "Quit WireTerm", true, None);
        let menu = Menu::with_items(&[&open, &quit])
            .map_err(|error| format!("could not create the tray menu: {error}"))?;
        let icon = Icon::from_rgba(tray_icon_rgba(), ICON_SIZE, ICON_SIZE)
            .map_err(|error| format!("could not create the tray icon image: {error}"))?;
        let tray_icon = TrayIconBuilder::new()
            .with_tooltip("WireTerm")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|error| format!("could not register the tray icon: {error}"))?;

        let actions = Arc::new(Mutex::new(VecDeque::new()));
        let menu_actions = Arc::clone(&actions);
        let menu_ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id.0.as_str() {
                OPEN_MENU_ID => Some(TrayAction::Open),
                QUIT_MENU_ID => Some(TrayAction::Quit),
                _ => None,
            };
            if let Some(action) = action {
                if let Ok(mut queue) = menu_actions.lock() {
                    queue.push_back(action);
                }
                menu_ctx.request_repaint();
            }
        }));

        let icon_actions = Arc::clone(&actions);
        let icon_ctx = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if matches!(event, TrayIconEvent::DoubleClick { .. }) {
                if let Ok(mut queue) = icon_actions.lock() {
                    queue.push_back(TrayAction::Open);
                }
                icon_ctx.request_repaint();
            }
        }));

        Ok(Self {
            _icon: tray_icon,
            actions,
        })
    }

    pub fn drain_actions(&self) -> Vec<TrayAction> {
        self.actions
            .lock()
            .map_or_else(|_| Vec::new(), |mut actions| actions.drain(..).collect())
    }
}

fn tray_icon_rgba() -> Vec<u8> {
    let mut rgba = vec![0; (ICON_SIZE * ICON_SIZE * 4) as usize];
    for y in 2..30 {
        for x in 2..30 {
            let border = !(5..27).contains(&x) || !(5..27).contains(&y);
            let (red, green, blue, alpha) = if border {
                (205, 35, 35, 255)
            } else {
                (24, 26, 31, 255)
            };
            set_pixel(&mut rgba, x, y, red, green, blue, alpha);
        }
    }

    // A compact terminal prompt remains legible at Windows tray sizes.
    for offset in 0..7 {
        set_pixel(&mut rgba, 9 + offset, 10 + offset, 255, 255, 255, 255);
        set_pixel(&mut rgba, 15 - offset, 16 + offset, 255, 255, 255, 255);
    }
    for x in 16..24 {
        for y in 21..23 {
            set_pixel(&mut rgba, x, y, 255, 255, 255, 255);
        }
    }
    rgba
}

fn set_pixel(rgba: &mut [u8], x: u32, y: u32, red: u8, green: u8, blue: u8, alpha: u8) {
    let index = ((y * ICON_SIZE + x) * 4) as usize;
    rgba[index..index + 4].copy_from_slice(&[red, green, blue, alpha]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_icon_has_the_expected_size_and_visible_pixels() {
        let rgba = tray_icon_rgba();
        assert_eq!(rgba.len(), (ICON_SIZE * ICON_SIZE * 4) as usize);
        assert!(rgba.chunks_exact(4).any(|pixel| pixel[3] == 255));
        assert!(
            rgba.chunks_exact(4)
                .any(|pixel| pixel == [205, 35, 35, 255])
        );
        assert!(
            rgba.chunks_exact(4)
                .any(|pixel| pixel == [255, 255, 255, 255])
        );
    }
}
