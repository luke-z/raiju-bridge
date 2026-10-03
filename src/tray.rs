use anyhow::Result;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

pub enum Command {
    Show,
    Toggle,
    Quit,
}
pub struct Tray {
    icon: TrayIcon,
    show: MenuItem,
    toggle: MenuItem,
    quit: MenuItem,
}
impl Tray {
    pub fn new() -> Result<Self> {
        let menu = Menu::new();
        let show = MenuItem::new("Open Raiju Bridge", true, None);
        let toggle = MenuItem::new("Start bridge", true, None);
        let quit = MenuItem::new("Quit", true, None);
        menu.append_items(&[&show, &toggle, &PredefinedMenuItem::separator(), &quit])?;
        let icon = TrayIconBuilder::new()
            .with_icon(Icon::from_resource(1, Some((32, 32)))?)
            .with_menu(Box::new(menu))
            .with_tooltip("Raiju Bridge · Stopped")
            .with_menu_on_left_click(false)
            .with_guid(0x6b416c269e624e47a35d5e0c4d5d2371)
            .build()?;
        Ok(Self {
            icon,
            show,
            toggle,
            quit,
        })
    }
    pub fn update(&self, running: bool, stopping: bool, status: &str) {
        self.toggle.set_text(if stopping {
            "Stopping…"
        } else if running {
            "Stop"
        } else {
            "Start bridge"
        });
        self.toggle.set_enabled(!stopping);
        let short: String = status.chars().take(90).collect();
        let _ = self
            .icon
            .set_tooltip(Some(format!("Raiju Bridge · {short}")));
    }
    pub fn commands(&self) -> Vec<Command> {
        let mut commands = Vec::new();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == *self.show.id() {
                commands.push(Command::Show);
            } else if event.id == *self.toggle.id() {
                commands.push(Command::Toggle);
            } else if event.id == *self.quit.id() {
                commands.push(Command::Quit);
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                commands.push(Command::Show);
            }
        }
        commands
    }
}
