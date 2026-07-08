//! Configuración de usuario: tema de colores y atajos de teclado.
//!
//! Se lee de `<config_dir>/xietiao/config.toml`. Si no existe, se usan los
//! valores por defecto (y la app sigue funcionando igual que siempre).

use std::collections::HashMap;
use std::fs;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::Color;
use serde::Deserialize;

use crate::model::Store;

/// Acción lógica que dispara una tecla. Permite reasignar teclas sin tocar
/// el código: la tecla se traduce a una de estas acciones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Help,
    Undo,
    NextPanel,
    PrevPanel,
    Down,
    Up,
    Left,
    Right,
    Add,
    Rename,
    Delete,
    Activate,
    AssignDate,
    SetDate,
    CyclePriority,
    CycleRecurrence,
    Subtasks,
    MoveTodo,
    LinkPomodoro,
    Search,
    ToggleNotesScope,
    MoveItemUp,
    MoveItemDown,
    AgendaToday,
    WeekAgenda,
    Pending,
    Stats,
    Trash,
    Menu,
    ResetClock,
    SwitchPomo,
    PrevMonth,
    NextMonth,
    CalendarToday,
    TodoistSync,
}

impl Action {
    /// Nombre usado en `config.toml` (sección `[keys]`).
    fn from_name(name: &str) -> Option<Action> {
        use Action::*;
        Some(match name {
            "quit" => Quit,
            "help" => Help,
            "undo" => Undo,
            "next_panel" => NextPanel,
            "prev_panel" => PrevPanel,
            "down" => Down,
            "up" => Up,
            "left" => Left,
            "right" => Right,
            "add" => Add,
            "rename" => Rename,
            "delete" => Delete,
            "activate" => Activate,
            "assign_date" => AssignDate,
            "set_date" => SetDate,
            "cycle_priority" => CyclePriority,
            "cycle_recurrence" => CycleRecurrence,
            "subtasks" => Subtasks,
            "move_todo" => MoveTodo,
            "link_pomodoro" => LinkPomodoro,
            "search" => Search,
            "toggle_notes_scope" => ToggleNotesScope,
            "move_item_up" => MoveItemUp,
            "move_item_down" => MoveItemDown,
            "agenda_today" => AgendaToday,
            "week_agenda" => WeekAgenda,
            "pending" => Pending,
            "stats" => Stats,
            "trash" => Trash,
            "menu" => Menu,
            "reset_clock" => ResetClock,
            "switch_pomo" => SwitchPomo,
            "prev_month" => PrevMonth,
            "next_month" => NextMonth,
            "calendar_today" => CalendarToday,
            "todoist_sync" => TodoistSync,
            _ => return None,
        })
    }
}

/// Asignaciones de teclas por defecto: (cadena de tecla, acción).
/// Varias teclas pueden apuntar a la misma acción.
fn default_bindings() -> Vec<(&'static str, Action)> {
    use Action::*;
    vec![
        ("q", Quit),
        ("?", Help),
        ("u", Undo),
        ("Tab", NextPanel),
        ("BackTab", PrevPanel),
        ("j", Down),
        ("Down", Down),
        ("k", Up),
        ("Up", Up),
        ("h", Left),
        ("Left", Left),
        ("l", Right),
        ("Right", Right),
        ("a", Add),
        ("n", Add),
        ("e", Rename),
        ("d", Delete),
        ("Enter", Activate),
        ("Space", Activate),
        ("f", AssignDate),
        ("D", SetDate),
        ("p", CyclePriority),
        ("R", CycleRecurrence),
        ("s", Subtasks),
        ("m", MoveTodo),
        ("v", LinkPomodoro),
        ("/", Search),
        ("g", ToggleNotesScope),
        ("J", MoveItemDown),
        ("K", MoveItemUp),
        ("t", AgendaToday),
        ("w", WeekAgenda),
        ("P", Pending),
        ("S", Stats),
        ("x", Trash),
        ("o", Menu),
        ("r", ResetClock),
        ("b", SwitchPomo),
        ("[", PrevMonth),
        ("]", NextMonth),
        ("T", CalendarToday),
        ("y", TodoistSync),
    ]
}

/// Normaliza una tecla a la cadena usada en la configuración (p. ej. "q",
/// "Tab", "Space", "ctrl+c", "J").
pub fn key_to_string(code: KeyCode, mods: KeyModifiers) -> String {
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let base = match code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::BackTab => "BackTab".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        other => format!("{other:?}"),
    };
    if ctrl {
        format!("ctrl+{base}")
    } else {
        base
    }
}

/// Tema de colores de la interfaz.
#[derive(Debug, Clone)]
pub struct Theme {
    pub focus: Color,
    pub idle: Color,
    pub accent: Color,
    pub done: Color,
    pub overdue: Color,
    pub today: Color,
    pub due: Color,
    pub tag: Color,
    pub recurrence: Color,
    pub priority_high: Color,
    pub priority_medium: Color,
    pub priority_low: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            focus: Color::Cyan,
            idle: Color::DarkGray,
            accent: Color::Cyan,
            done: Color::Green,
            overdue: Color::Red,
            today: Color::Rgb(255, 165, 0),
            due: Color::Yellow,
            tag: Color::Blue,
            recurrence: Color::Magenta,
            priority_high: Color::Red,
            priority_medium: Color::Yellow,
            priority_low: Color::Blue,
        }
    }
}

/// Convierte una cadena en un color: nombre ("cyan"), hex ("#ff8800") o
/// "r,g,b" ("255,165,0"). Devuelve None si no se reconoce.
fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::Rgb(r, g, b));
        }
        return None;
    }
    if s.contains(',') {
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() == 3 {
            let r = parts[0].trim().parse().ok()?;
            let g = parts[1].trim().parse().ok()?;
            let b = parts[2].trim().parse().ok()?;
            return Some(Color::Rgb(r, g, b));
        }
        return None;
    }
    Some(match s.to_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "white" => Color::White,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        _ => return None,
    })
}

/// Estructura cruda leída del TOML.
#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    #[serde(default)]
    theme: HashMap<String, String>,
    #[serde(default)]
    keys: HashMap<String, String>,
}

/// Configuración resuelta de la aplicación.
#[derive(Debug, Clone)]
pub struct Config {
    pub theme: Theme,
    /// Cadena de tecla → acción.
    pub keymap: HashMap<String, Action>,
}

impl Default for Config {
    fn default() -> Self {
        let keymap = default_bindings()
            .into_iter()
            .map(|(k, a)| (k.to_string(), a))
            .collect();
        Self {
            theme: Theme::default(),
            keymap,
        }
    }
}

impl Config {
    fn path() -> std::path::PathBuf {
        Store::config_dir().join("config.toml")
    }

    /// Carga la configuración del disco. Si no existe o falla, usa los valores
    /// por defecto (nunca aborta la app por un fallo de config).
    pub fn load() -> Self {
        let mut config = Config::default();
        let Ok(contents) = fs::read_to_string(Self::path()) else {
            return config;
        };
        let Ok(raw) = toml::from_str::<RawConfig>(&contents) else {
            return config;
        };

        // Tema.
        let t = &mut config.theme;
        for (key, value) in &raw.theme {
            let Some(color) = parse_color(value) else {
                continue;
            };
            match key.as_str() {
                "focus" => t.focus = color,
                "idle" => t.idle = color,
                "accent" => t.accent = color,
                "done" => t.done = color,
                "overdue" => t.overdue = color,
                "today" => t.today = color,
                "due" => t.due = color,
                "tag" => t.tag = color,
                "recurrence" => t.recurrence = color,
                "priority_high" => t.priority_high = color,
                "priority_medium" => t.priority_medium = color,
                "priority_low" => t.priority_low = color,
                _ => {}
            }
        }

        // Teclas: cada acción puede recibir una o varias teclas separadas por
        // comas. Reasignar una acción quita sus teclas por defecto.
        for (action_name, keys) in &raw.keys {
            let Some(action) = Action::from_name(action_name) else {
                continue;
            };
            config.keymap.retain(|_, a| *a != action);
            for k in keys.split(',') {
                let k = k.trim();
                if !k.is_empty() {
                    config.keymap.insert(k.to_string(), action);
                }
            }
        }

        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn parse_color_named_hex_rgb() {
        assert_eq!(parse_color("cyan"), Some(Color::Cyan));
        assert_eq!(parse_color("#ff8800"), Some(Color::Rgb(255, 136, 0)));
        assert_eq!(parse_color("255,165,0"), Some(Color::Rgb(255, 165, 0)));
        assert_eq!(parse_color("noexiste"), None);
    }

    #[test]
    fn key_to_string_normalizes() {
        assert_eq!(key_to_string(KeyCode::Char('q'), KeyModifiers::NONE), "q");
        assert_eq!(key_to_string(KeyCode::Char(' '), KeyModifiers::NONE), "Space");
        assert_eq!(key_to_string(KeyCode::Char('c'), KeyModifiers::CONTROL), "ctrl+c");
        assert_eq!(key_to_string(KeyCode::Tab, KeyModifiers::NONE), "Tab");
    }

    #[test]
    fn default_keymap_has_core_actions() {
        let cfg = Config::default();
        assert_eq!(cfg.keymap.get("q"), Some(&Action::Quit));
        assert_eq!(cfg.keymap.get("Tab"), Some(&Action::NextPanel));
        assert_eq!(cfg.keymap.get("u"), Some(&Action::Undo));
    }
}
