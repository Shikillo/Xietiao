//! Estado de la aplicación y lógica de interacción.

use std::sync::mpsc;
use std::time::Duration;

use chrono::{Datelike, Local, NaiveDate};

use crate::config::{key_to_string, Action, Config, Theme};
use crate::model::{
    PomodoroSession, Project, Recurrence, Store, Subtask, Todo, TrashItem, TrashKind,
};
use crate::todoist;

/// Vista superpuesta a pantalla (completa o parcial) sobre el dashboard.
#[derive(Debug, Clone)]
pub enum Overlay {
    /// Sin overlay: dashboard normal.
    None,
    /// Ayuda con los atajos.
    Help,
    /// Agenda de un día concreto, con selección para saltar a una tarea.
    Agenda { date: NaiveDate, sel: usize },
    /// Selector de proyecto destino para mover el to-do actual.
    MoveTodo { sel: usize },
    /// Editor de subtareas del to-do actual.
    Subtasks { sel: usize },
    /// Lista de todos los pendientes cruzando proyectos.
    Pending { sel: usize },
    /// Agenda semanal a partir de `anchor` (lunes de la semana mostrada).
    WeekAgenda { anchor: NaiveDate },
    /// Estadísticas y racha de hábitos.
    Stats,
    /// Papelera: restaurar o purgar elementos borrados.
    Trash { sel: usize },
    /// Menú de acciones (export/import, etc.).
    Menu { sel: usize },
}

impl Overlay {
    pub fn is_none(&self) -> bool {
        matches!(self, Overlay::None)
    }
}

/// Panel que tiene el foco actualmente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Projects,
    Todos,
    Calendar,
    Notes,
    Timer,
}

impl Focus {
    /// Orden de tabulación entre paneles.
    pub fn next(self) -> Self {
        match self {
            Focus::Projects => Focus::Todos,
            Focus::Todos => Focus::Calendar,
            Focus::Calendar => Focus::Timer,
            Focus::Timer => Focus::Notes,
            Focus::Notes => Focus::Projects,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Focus::Projects => Focus::Notes,
            Focus::Notes => Focus::Timer,
            Focus::Timer => Focus::Calendar,
            Focus::Calendar => Focus::Todos,
            Focus::Todos => Focus::Projects,
        }
    }
}

/// Qué se está escribiendo cuando estamos en modo de entrada de texto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    AddProject,
    AddTodo,
    AddSubtask,
    EditNotes,
    EditProject,
    EditTodo,
    /// Fecha de la tarea escrita a mano (vacío = quitar la fecha).
    SetDate,
    /// Token de API de Todoist (vacío = borrar el token).
    TodoistToken,
    Search,
}

/// A qué se asocian las notas que se muestran/editan en el panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotesScope {
    /// Notas del proyecto seleccionado.
    Project,
    /// Notas generales, sin proyecto.
    General,
}

/// Estado del temporizador pomodoro.
#[derive(Debug, Clone)]
pub struct Timer {
    pub running: bool,
    pub remaining: Duration,
    /// Duración a la que vuelve al reiniciar.
    pub preset: Duration,
    pub on_break: bool,
}

impl Default for Timer {
    fn default() -> Self {
        let work = Duration::from_secs(25 * 60);
        Self {
            running: false,
            remaining: work,
            preset: work,
            on_break: false,
        }
    }
}

impl Timer {
    pub fn toggle(&mut self) {
        self.running = !self.running;
    }

    pub fn reset(&mut self) {
        self.remaining = self.preset;
        self.running = false;
    }

    /// Cambia entre el preset de trabajo (25:00) y el de descanso (05:00).
    pub fn switch_mode(&mut self) {
        self.on_break = !self.on_break;
        self.preset = if self.on_break {
            Duration::from_secs(5 * 60)
        } else {
            Duration::from_secs(25 * 60)
        };
        self.remaining = self.preset;
        self.running = false;
    }

    /// Avanza el temporizador en `elapsed`. Devuelve true si acaba de llegar a cero.
    pub fn tick(&mut self, elapsed: Duration) -> bool {
        if !self.running {
            return false;
        }
        if elapsed >= self.remaining {
            self.remaining = Duration::ZERO;
            self.running = false;
            true
        } else {
            self.remaining -= elapsed;
            false
        }
    }

    pub fn label(&self) -> String {
        let secs = self.remaining.as_secs();
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}

/// Cronómetro que cuenta hacia adelante.
#[derive(Debug, Clone, Default)]
pub struct Stopwatch {
    pub running: bool,
    pub elapsed: Duration,
}

impl Stopwatch {
    pub fn toggle(&mut self) {
        self.running = !self.running;
    }

    pub fn reset(&mut self) {
        self.elapsed = Duration::ZERO;
        self.running = false;
    }

    pub fn tick(&mut self, elapsed: Duration) {
        if self.running {
            self.elapsed += elapsed;
        }
    }

    pub fn label(&self) -> String {
        let secs = self.elapsed.as_secs();
        if secs >= 3600 {
            format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
        } else {
            format!("{:02}:{:02}", secs / 60, secs % 60)
        }
    }
}

/// Cuál de los tres relojes de la columna está seleccionado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockSel {
    Pomodoro,
    Reloj,
    Cronometro,
}

impl ClockSel {
    fn next(self) -> Self {
        match self {
            ClockSel::Pomodoro => ClockSel::Reloj,
            ClockSel::Reloj => ClockSel::Cronometro,
            ClockSel::Cronometro => ClockSel::Pomodoro,
        }
    }

    fn prev(self) -> Self {
        match self {
            ClockSel::Pomodoro => ClockSel::Cronometro,
            ClockSel::Reloj => ClockSel::Pomodoro,
            ClockSel::Cronometro => ClockSel::Reloj,
        }
    }
}

/// Resultado de una sincronización con Todoist, enviado desde el hilo de red.
pub struct TodoistOutcome {
    /// Tareas creadas en Todoist: (proyecto, tarea, id remota, título).
    /// El título permite recolocar la id si la tarea se movió mientras tanto.
    created: Vec<(usize, usize, String, String)>,
    /// Ids remotas de tareas ya exportadas que están completadas en Todoist.
    remote_done: Vec<String>,
    /// Pendientes que ya estaban exportadas y se han saltado.
    skipped: usize,
    /// Si algo falló a medias, el mensaje (lo ya hecho cuenta igualmente).
    error: Option<String>,
}

/// Estado global de la aplicación.
pub struct App {
    pub store: Store,
    pub focus: Focus,
    pub mode: InputMode,
    pub project_idx: usize,
    pub todo_idx: usize,
    pub input: String,
    pub timer: Timer,
    pub stopwatch: Stopwatch,
    /// Reloj seleccionado dentro de la columna de relojes.
    pub clock_sel: ClockSel,
    /// Primer día del mes que se muestra en el calendario.
    pub calendar_anchor: NaiveDate,
    /// Día seleccionado en el calendario (cursor).
    pub calendar_cursor: NaiveDate,
    /// A qué se asocian las notas mostradas en el panel.
    pub notes_scope: NotesScope,
    /// Texto de filtro activo en la lista de to-dos ("" = sin filtro).
    pub search: String,
    /// Vista superpuesta activa (ayuda, agenda, papelera, etc.).
    pub overlay: Overlay,
    /// Si está activo, se está confirmando un borrado.
    pub confirm_delete: bool,
    /// Historial para deshacer: instantáneas del store antes de cada cambio.
    pub undo_stack: Vec<Store>,
    /// Tiempo acumulado desde el último guardado (para autoguardado periódico).
    since_save: Duration,
    /// To-do vinculado al pomodoro: (índice de proyecto, índice real de tarea).
    pub pomodoro_link: Option<(usize, usize)>,
    /// Configuración de usuario (tema y atajos).
    pub config: Config,
    /// Sincronización con Todoist en curso: receptor del hilo de red.
    todoist_sync: Option<mpsc::Receiver<TodoistOutcome>>,
    pub should_quit: bool,
    pub status: String,
}

impl App {
    pub fn new() -> Self {
        let store = Store::load();
        let today = Local::now().date_naive();
        let calendar_anchor = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
        let calendar_cursor = today;
        Self {
            store,
            focus: Focus::Projects,
            mode: InputMode::Normal,
            project_idx: 0,
            todo_idx: 0,
            input: String::new(),
            timer: Timer::default(),
            stopwatch: Stopwatch::default(),
            clock_sel: ClockSel::Pomodoro,
            calendar_anchor,
            calendar_cursor,
            notes_scope: NotesScope::Project,
            search: String::new(),
            overlay: Overlay::None,
            confirm_delete: false,
            undo_stack: Vec::new(),
            since_save: Duration::ZERO,
            pomodoro_link: None,
            config: Config::load(),
            todoist_sync: None,
            should_quit: false,
            status: "Tab: panel · a: añadir · e: editar · /: buscar · t: hoy · ?: ayuda · q: salir"
                .into(),
        }
    }

    pub fn save(&self) {
        if let Err(e) = self.store.save() {
            // No abortamos la app por un fallo de guardado; lo dejaría en el status
            // pero `save` se llama desde sitios sin &mut, así que sólo lo ignoramos aquí.
            let _ = e;
        }
    }

    // --- Deshacer -------------------------------------------------------------

    /// Guarda una instantánea del estado antes de un cambio, para poder deshacerlo.
    /// Se llama al principio de cada acción que muta los datos.
    fn record(&mut self) {
        const MAX_UNDO: usize = 50;
        self.undo_stack.push(self.store.clone());
        if self.undo_stack.len() > MAX_UNDO {
            self.undo_stack.remove(0);
        }
        // Reinicia el cronómetro de autoguardado: hubo un cambio reciente.
        self.since_save = Duration::ZERO;
    }

    /// Revierte el último cambio, si hay algo en la pila.
    fn undo(&mut self) {
        match self.undo_stack.pop() {
            Some(prev) => {
                self.store = prev;
                self.clamp_indices();
                self.save();
                self.status = "Cambio deshecho (u)".into();
            }
            None => self.status = "Nada que deshacer".into(),
        }
    }

    /// Mueve el to-do seleccionado al proyecto `dest_idx`.
    fn move_todo_to(&mut self, dest_idx: usize) {
        let Some(actual) = self.selected_todo_actual() else {
            return;
        };
        if dest_idx == self.project_idx || dest_idx >= self.store.projects.len() {
            self.status = "Mismo proyecto: sin cambios".into();
            return;
        }
        self.record();
        let todo = self.store.projects[self.project_idx].todos.remove(actual);
        let name = self.store.projects[dest_idx].name.clone();
        self.store.projects[dest_idx].todos.push(todo);
        self.clamp_indices();
        self.save();
        self.status = format!("Tarea movida a «{name}»");
    }

    pub fn current_project(&self) -> Option<&Project> {
        self.store.projects.get(self.project_idx)
    }

    /// Tema de colores activo.
    pub fn theme(&self) -> &Theme {
        &self.config.theme
    }

    // --- To-dos: filtro y selección ------------------------------------------

    /// Índices reales (en `project.todos`) de las tareas visibles tras el filtro.
    pub fn filtered_todo_indices(&self) -> Vec<usize> {
        match self.current_project() {
            Some(p) => {
                if self.search.is_empty() {
                    (0..p.todos.len()).collect()
                } else {
                    let q = self.search.to_lowercase();
                    // Si el filtro empieza por '#', se busca sólo entre las etiquetas.
                    let tag_query = q.strip_prefix('#');
                    p.todos
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| match tag_query {
                            Some(tq) => t.tags.iter().any(|tag| tag.contains(tq)),
                            None => {
                                t.title.to_lowercase().contains(&q)
                                    || t.tags.iter().any(|tag| tag.contains(&q))
                            }
                        })
                        .map(|(i, _)| i)
                        .collect()
                }
            }
            None => Vec::new(),
        }
    }

    /// Índice real de la tarea seleccionada (teniendo en cuenta el filtro).
    fn selected_todo_actual(&self) -> Option<usize> {
        self.filtered_todo_indices().get(self.todo_idx).copied()
    }

    /// Tareas asignadas a `date` en todos los proyectos, con su proyecto.
    pub fn agenda_items(&self, date: NaiveDate) -> Vec<(&str, &Todo)> {
        let mut out = Vec::new();
        for project in &self.store.projects {
            for todo in &project.todos {
                if todo.date == Some(date) {
                    out.push((project.name.as_str(), todo));
                }
            }
        }
        out
    }

    /// Como `agenda_items`, pero con índices (proyecto, tarea) para poder saltar.
    pub fn agenda_items_idx(&self, date: NaiveDate) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (pi, project) in self.store.projects.iter().enumerate() {
            for (ti, todo) in project.todos.iter().enumerate() {
                if todo.date == Some(date) {
                    out.push((pi, ti));
                }
            }
        }
        out
    }

    // --- Notas ----------------------------------------------------------------

    /// Si el ámbito es Proyecto pero no hay ninguno, se comporta como General.
    pub fn effective_notes_scope(&self) -> NotesScope {
        if self.notes_scope == NotesScope::Project && self.current_project().is_none() {
            NotesScope::General
        } else {
            self.notes_scope
        }
    }

    /// Texto de las notas que toca mostrar según el ámbito activo.
    pub fn active_notes(&self) -> &str {
        match self.effective_notes_scope() {
            NotesScope::General => &self.store.notes,
            NotesScope::Project => self
                .current_project()
                .map(|p| p.notes.as_str())
                .unwrap_or(""),
        }
    }

    fn set_active_notes(&mut self, text: String) {
        match self.effective_notes_scope() {
            NotesScope::General => self.store.notes = text,
            NotesScope::Project => {
                if let Some(p) = self.store.projects.get_mut(self.project_idx) {
                    p.notes = text;
                }
            }
        }
    }

    fn toggle_notes_scope(&mut self) {
        self.notes_scope = match self.notes_scope {
            NotesScope::Project => NotesScope::General,
            NotesScope::General => NotesScope::Project,
        };
    }

    // --- Navegación de listas -------------------------------------------------

    fn clamp_indices(&mut self) {
        if self.project_idx >= self.store.projects.len() {
            self.project_idx = self.store.projects.len().saturating_sub(1);
        }
        let todos_len = self.filtered_todo_indices().len();
        if self.todo_idx >= todos_len {
            self.todo_idx = todos_len.saturating_sub(1);
        }
    }

    fn move_selection(&mut self, delta: isize) {
        match self.focus {
            Focus::Projects => {
                let len = self.store.projects.len();
                if len > 0 {
                    self.project_idx = wrap(self.project_idx, delta, len);
                    self.todo_idx = 0;
                }
            }
            Focus::Todos => {
                let len = self.filtered_todo_indices().len();
                if len > 0 {
                    self.todo_idx = wrap(self.todo_idx, delta, len);
                }
            }
            Focus::Timer => {
                self.clock_sel = if delta >= 0 {
                    self.clock_sel.next()
                } else {
                    self.clock_sel.prev()
                };
            }
            _ => {}
        }
    }

    fn toggle_selected_clock(&mut self) {
        match self.clock_sel {
            ClockSel::Pomodoro => self.timer.toggle(),
            ClockSel::Cronometro => self.stopwatch.toggle(),
            ClockSel::Reloj => {}
        }
    }

    fn reset_selected_clock(&mut self) {
        match self.clock_sel {
            ClockSel::Pomodoro => self.timer.reset(),
            ClockSel::Cronometro => self.stopwatch.reset(),
            ClockSel::Reloj => {}
        }
    }

    /// Mueve el cursor del calendario `days` días y reajusta el mes mostrado.
    fn move_cursor(&mut self, days: i64) {
        if let Some(d) = self
            .calendar_cursor
            .checked_add_signed(chrono::Duration::days(days))
        {
            self.calendar_cursor = d;
            self.calendar_anchor = NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap();
        }
    }

    /// Mueve el calendario un mes adelante o atrás, ajustando el día del cursor
    /// si el mes destino es más corto (como los botones ◀ ▶ del escritorio).
    fn shift_month(&mut self, delta: i32) {
        let (mut y, mut m) = (
            self.calendar_cursor.year(),
            self.calendar_cursor.month() as i32,
        );
        m += delta;
        if m < 1 {
            m = 12;
            y -= 1;
        } else if m > 12 {
            m = 1;
            y += 1;
        }
        let last = crate::model::days_in_month(y, m as u32);
        let day = self.calendar_cursor.day().min(last);
        if let Some(d) = NaiveDate::from_ymd_opt(y, m as u32, day) {
            self.calendar_cursor = d;
            self.calendar_anchor = NaiveDate::from_ymd_opt(y, m as u32, 1).unwrap();
        }
    }

    /// Devuelve el calendario a hoy (como el botón «hoy» del escritorio).
    fn calendar_today(&mut self) {
        let today = Local::now().date_naive();
        self.calendar_cursor = today;
        self.calendar_anchor = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
        self.status = "Calendario en hoy".into();
    }

    /// Asigna (o desasigna, si ya estaba) el to-do seleccionado al día del cursor.
    fn assign_todo_date(&mut self) {
        let date = self.calendar_cursor;
        let Some(actual) = self.selected_todo_actual() else {
            return;
        };
        let mut new_date = None;
        let mut changed = false;
        self.record();
        if let Some(project) = self.store.projects.get_mut(self.project_idx) {
            if let Some(todo) = project.todos.get_mut(actual) {
                todo.date = if todo.date == Some(date) { None } else { Some(date) };
                new_date = todo.date;
                changed = true;
            }
        }
        if changed {
            self.save();
            self.status = match new_date {
                Some(d) => format!("Tarea asignada a {}", d.format("%d/%m/%Y")),
                None => "Fecha quitada de la tarea".into(),
            };
        }
    }

    // --- Acciones -------------------------------------------------------------

    fn toggle_todo(&mut self) {
        let Some(actual) = self.selected_todo_actual() else {
            return;
        };
        self.record();
        let today = Local::now().date_naive();
        if let Some(project) = self.store.projects.get_mut(self.project_idx) {
            match project.todos.get(actual).map(|t| t.done) {
                Some(true) => {
                    let todo = &mut project.todos[actual];
                    todo.done = false;
                    todo.completed_at = None;
                }
                Some(false) => {
                    if complete_todo(project, actual, today) {
                        self.status = "Tarea recurrente regenerada para la próxima fecha".into();
                    }
                }
                None => {}
            }
        }
        self.save();
    }

    /// Cambia la prioridad de la tarea seleccionada de forma cíclica.
    fn cycle_priority(&mut self) {
        let Some(actual) = self.selected_todo_actual() else {
            return;
        };
        self.record();
        if let Some(project) = self.store.projects.get_mut(self.project_idx) {
            if let Some(todo) = project.todos.get_mut(actual) {
                todo.priority = todo.priority.cycle();
                self.save();
            }
        }
    }

    /// Mueve el elemento seleccionado (proyecto o to-do) hacia arriba/abajo.
    fn move_item(&mut self, delta: isize) {
        match self.focus {
            Focus::Projects => {
                let len = self.store.projects.len();
                let i = self.project_idx;
                let j = i as isize + delta;
                if j >= 0 && (j as usize) < len {
                    self.record();
                    self.store.projects.swap(i, j as usize);
                    self.project_idx = j as usize;
                    self.save();
                }
            }
            Focus::Todos => {
                let Some(actual) = self.selected_todo_actual() else {
                    return;
                };
                let j = actual as isize + delta;
                let todos_len = self
                    .current_project()
                    .map(|p| p.todos.len())
                    .unwrap_or(0);
                if j < 0 || (j as usize) >= todos_len {
                    return;
                }
                self.record();
                let mut moved = false;
                if let Some(project) = self.store.projects.get_mut(self.project_idx) {
                    project.todos.swap(actual, j as usize);
                    moved = true;
                }
                if moved {
                    // Hacer que la selección siga a la tarea movida.
                    let newpos = (actual as isize + delta) as usize;
                    if let Some(p) = self.filtered_todo_indices().iter().position(|&x| x == newpos) {
                        self.todo_idx = p;
                    }
                    self.save();
                }
            }
            _ => {}
        }
    }

    /// Pide confirmación antes de borrar (si hay algo que borrar).
    fn request_delete(&mut self) {
        let has_target = match self.focus {
            Focus::Projects => !self.store.projects.is_empty(),
            Focus::Todos => self.selected_todo_actual().is_some(),
            _ => false,
        };
        if has_target {
            self.confirm_delete = true;
            self.status = "¿Borrar? (y/n)".into();
        }
    }

    fn delete_current(&mut self) {
        let today = Local::now().date_naive();
        match self.focus {
            Focus::Projects if self.project_idx < self.store.projects.len() => {
                self.record();
                let project = self.store.projects.remove(self.project_idx);
                self.store.trash.push(TrashItem {
                    kind: TrashKind::Project(project),
                    deleted_at: Some(today),
                });
                self.todo_idx = 0;
                self.clamp_indices();
                self.save();
                self.status = "Proyecto enviado a la papelera (u: deshacer)".into();
            }
            Focus::Todos => {
                if let Some(actual) = self.selected_todo_actual() {
                    self.record();
                    let project_name = self.store.projects[self.project_idx].name.clone();
                    let todo = self.store.projects[self.project_idx].todos.remove(actual);
                    self.store.trash.push(TrashItem {
                        kind: TrashKind::Todo {
                            project: project_name,
                            todo,
                        },
                        deleted_at: Some(today),
                    });
                    self.clamp_indices();
                    self.save();
                    self.status = "Tarea enviada a la papelera (u: deshacer)".into();
                }
            }
            _ => {}
        }
    }

    fn start_add(&mut self) {
        self.input.clear();
        match self.focus {
            Focus::Projects => self.mode = InputMode::AddProject,
            Focus::Todos => {
                if self.current_project().is_some() {
                    self.mode = InputMode::AddTodo;
                } else {
                    self.status = "Crea primero un proyecto (foco en proyectos + 'a')".into();
                }
            }
            _ => {}
        }
    }

    fn start_edit_notes(&mut self) {
        self.input = self.active_notes().to_string();
        self.mode = InputMode::EditNotes;
    }

    /// Inicia el renombrado del elemento seleccionado según el panel con foco.
    fn start_rename(&mut self) {
        match self.focus {
            Focus::Projects => {
                if let Some(p) = self.current_project() {
                    self.input = p.name.clone();
                    self.mode = InputMode::EditProject;
                }
            }
            Focus::Todos => {
                if let Some(actual) = self.selected_todo_actual() {
                    if let Some(t) = self
                        .current_project()
                        .and_then(|p| p.todos.get(actual))
                    {
                        // Reconstruye el texto con los tags para poder editarlos.
                        let mut text = t.title.clone();
                        for tag in &t.tags {
                            text.push_str(&format!(" #{tag}"));
                        }
                        self.input = text;
                        self.mode = InputMode::EditTodo;
                    }
                }
            }
            Focus::Notes => self.start_edit_notes(),
            _ => {}
        }
    }

    /// Abre el popup para escribir la fecha de la tarea seleccionada a mano
    /// (equivalente al selector de fecha del escritorio; vacío = quitar fecha).
    fn start_set_date(&mut self) {
        let Some(actual) = self.selected_todo_actual() else {
            self.status = "No hay tarea seleccionada".into();
            return;
        };
        let current = self.store.projects[self.project_idx].todos[actual].date;
        self.input = current
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        self.mode = InputMode::SetDate;
    }

    fn start_search(&mut self) {
        self.mode = InputMode::Search;
        self.todo_idx = 0;
    }

    fn commit_input(&mut self) {
        let text = self.input.trim().to_string();
        // Registra para deshacer salvo en modos que no mutan datos.
        if !matches!(self.mode, InputMode::Search | InputMode::Normal) {
            self.record();
        }
        match self.mode {
            InputMode::AddProject => {
                if !text.is_empty() {
                    self.store.projects.push(Project::new(text));
                    self.project_idx = self.store.projects.len() - 1;
                    self.todo_idx = 0;
                    self.save();
                }
            }
            InputMode::AddTodo => {
                if !text.is_empty() {
                    let (title, tags) = parse_tags(&text);
                    if let Some(project) = self.store.projects.get_mut(self.project_idx) {
                        let mut todo = Todo::new(title);
                        todo.tags = tags;
                        project.todos.push(todo);
                        self.save();
                    }
                }
            }
            InputMode::AddSubtask => {
                if !text.is_empty() {
                    if let Some(actual) = self.selected_todo_actual() {
                        if let Some(t) = self
                            .store
                            .projects
                            .get_mut(self.project_idx)
                            .and_then(|p| p.todos.get_mut(actual))
                        {
                            t.subtasks.push(Subtask::new(text));
                            self.save();
                        }
                    }
                }
            }
            InputMode::EditNotes => {
                // Para notas guardamos el texto tal cual (con saltos de línea)
                // en el ámbito activo (proyecto o general).
                self.set_active_notes(self.input.clone());
                self.save();
            }
            InputMode::EditProject => {
                if !text.is_empty() {
                    if let Some(p) = self.store.projects.get_mut(self.project_idx) {
                        p.name = text;
                        self.save();
                    }
                }
            }
            InputMode::EditTodo => {
                if !text.is_empty() {
                    let (title, tags) = parse_tags(&text);
                    if let Some(actual) = self.selected_todo_actual() {
                        if let Some(p) = self.store.projects.get_mut(self.project_idx) {
                            if let Some(t) = p.todos.get_mut(actual) {
                                t.title = title;
                                if !tags.is_empty() {
                                    t.tags = tags;
                                }
                                self.save();
                            }
                        }
                    }
                }
            }
            InputMode::SetDate => {
                if let Some(actual) = self.selected_todo_actual() {
                    let parsed = if text.is_empty() {
                        Ok(None)
                    } else {
                        NaiveDate::parse_from_str(&text, "%Y-%m-%d")
                            .or_else(|_| NaiveDate::parse_from_str(&text, "%d/%m/%Y"))
                            .map(Some)
                    };
                    match parsed {
                        Ok(date) => {
                            if let Some(t) = self
                                .store
                                .projects
                                .get_mut(self.project_idx)
                                .and_then(|p| p.todos.get_mut(actual))
                            {
                                t.date = date;
                                self.save();
                                self.status = match date {
                                    Some(d) => {
                                        format!("Tarea asignada a {}", d.format("%d/%m/%Y"))
                                    }
                                    None => "Fecha quitada de la tarea".into(),
                                };
                            }
                        }
                        Err(_) => {
                            self.status =
                                "Fecha no válida: usa AAAA-MM-DD o DD/MM/AAAA".into();
                        }
                    }
                }
            }
            InputMode::TodoistToken => {
                self.store.todoist_token = (!text.is_empty()).then(|| text.clone());
                self.save();
                self.status = if self.store.todoist_token.is_some() {
                    "Token de Todoist guardado".into()
                } else {
                    "Token de Todoist borrado".into()
                };
            }
            InputMode::Search | InputMode::Normal => {}
        }
        self.input.clear();
        self.mode = InputMode::Normal;
    }

    // --- Subtareas, recurrencia, mover ---------------------------------------

    fn open_move_todo(&mut self) {
        if self.selected_todo_actual().is_none() {
            self.status = "No hay tarea seleccionada".into();
            return;
        }
        if self.store.projects.len() < 2 {
            self.status = "Necesitas otro proyecto para mover la tarea".into();
            return;
        }
        self.overlay = Overlay::MoveTodo { sel: 0 };
    }

    fn open_subtasks(&mut self) {
        if self.selected_todo_actual().is_none() {
            self.status = "No hay tarea seleccionada".into();
            return;
        }
        self.overlay = Overlay::Subtasks { sel: 0 };
    }

    fn cycle_recurrence(&mut self) {
        let Some(actual) = self.selected_todo_actual() else {
            return;
        };
        self.record();
        let mut label = "";
        if let Some(t) = self
            .store
            .projects
            .get_mut(self.project_idx)
            .and_then(|p| p.todos.get_mut(actual))
        {
            t.recurrence = t.recurrence.cycle();
            label = t.recurrence.label();
        }
        self.save();
        self.status = match label {
            "" => "Recurrencia desactivada".into(),
            l => format!("Recurrencia: {l}"),
        };
    }

    /// To-do seleccionado actualmente, como referencia (para el editor de subtareas).
    pub fn selected_todo(&self) -> Option<&Todo> {
        let actual = self.selected_todo_actual()?;
        self.current_project()?.todos.get(actual)
    }

    fn toggle_subtask(&mut self, sel: usize) {
        let Some(actual) = self.selected_todo_actual() else {
            return;
        };
        self.record();
        if let Some(t) = self
            .store
            .projects
            .get_mut(self.project_idx)
            .and_then(|p| p.todos.get_mut(actual))
        {
            if let Some(s) = t.subtasks.get_mut(sel) {
                s.done = !s.done;
            }
        }
        self.save();
    }

    fn delete_subtask(&mut self, sel: usize) {
        let Some(actual) = self.selected_todo_actual() else {
            return;
        };
        self.record();
        if let Some(t) = self
            .store
            .projects
            .get_mut(self.project_idx)
            .and_then(|p| p.todos.get_mut(actual))
        {
            if sel < t.subtasks.len() {
                t.subtasks.remove(sel);
            }
        }
        self.save();
    }

    // --- Pendientes globales --------------------------------------------------

    /// Todas las tareas sin completar, como (índice de proyecto, índice de tarea).
    pub fn pending_items(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (pi, p) in self.store.projects.iter().enumerate() {
            if p.archived {
                continue;
            }
            for (ti, t) in p.todos.iter().enumerate() {
                if !t.done {
                    out.push((pi, ti));
                }
            }
        }
        out
    }

    /// Salta el foco a una tarea concreta (usado desde la vista de pendientes).
    fn jump_to_todo(&mut self, project: usize, todo_actual: usize) {
        if project >= self.store.projects.len() {
            return;
        }
        self.project_idx = project;
        self.search.clear();
        self.focus = Focus::Todos;
        // Recalcula el índice visible (sin filtro coincide con el real).
        self.todo_idx = todo_actual.min(self.store.projects[project].todos.len().saturating_sub(1));
        self.overlay = Overlay::None;
    }

    // --- Papelera -------------------------------------------------------------

    fn restore_trash(&mut self, idx: usize) {
        if idx >= self.store.trash.len() {
            return;
        }
        self.record();
        let item = self.store.trash.remove(idx);
        match item.kind {
            TrashKind::Project(p) => {
                self.store.projects.push(p);
                self.status = "Proyecto restaurado".into();
            }
            TrashKind::Todo { project, todo } => {
                // Busca el proyecto por nombre; si ya no existe, lo recrea.
                let pos = self.store.projects.iter().position(|p| p.name == project);
                match pos {
                    Some(i) => self.store.projects[i].todos.push(todo),
                    None => {
                        let mut p = Project::new(project);
                        p.todos.push(todo);
                        self.store.projects.push(p);
                    }
                }
                self.status = "Tarea restaurada".into();
            }
        }
        self.save();
    }

    fn purge_trash(&mut self, idx: usize) {
        if idx < self.store.trash.len() {
            self.record();
            self.store.trash.remove(idx);
            self.save();
            self.status = "Elemento eliminado definitivamente".into();
        }
    }

    // --- Pomodoro -------------------------------------------------------------

    fn link_pomodoro(&mut self) {
        let Some(actual) = self.selected_todo_actual() else {
            self.status = "No hay tarea seleccionada".into();
            return;
        };
        self.pomodoro_link = Some((self.project_idx, actual));
        let title = self.store.projects[self.project_idx].todos[actual].title.clone();
        self.status = format!("Pomodoro vinculado a «{title}»");
    }

    /// Título de la tarea vinculada al pomodoro, si el vínculo sigue siendo válido.
    pub fn pomodoro_link_title(&self) -> Option<&str> {
        let (pi, ti) = self.pomodoro_link?;
        self.store
            .projects
            .get(pi)?
            .todos
            .get(ti)
            .map(|t| t.title.as_str())
    }

    /// Etiquetas (proyecto, tarea) para registrar un foco completado.
    fn pomodoro_labels(&self) -> (Option<String>, Option<String>) {
        if let Some((pi, ti)) = self.pomodoro_link {
            if let Some(p) = self.store.projects.get(pi) {
                let todo = p.todos.get(ti).map(|t| t.title.clone());
                return (Some(p.name.clone()), todo);
            }
        }
        (self.current_project().map(|p| p.name.clone()), None)
    }

    /// Emite un aviso al terminar un foco o descanso del pomodoro.
    fn notify_finish(&self) {
        use std::io::Write;
        // Campana del terminal: parpadeo o beep en la mayoría de emuladores.
        let _ = write!(std::io::stdout(), "\x07");
        let _ = std::io::stdout().flush();
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("afplay")
                .arg("/System/Library/Sounds/Glass.aiff")
                .spawn();
        }
    }

    // --- Todoist ----------------------------------------------------------------

    /// Abre el popup para pegar (o borrar, dejándolo vacío) el token de Todoist.
    fn start_todoist_token(&mut self) {
        self.input = self.store.todoist_token.clone().unwrap_or_default();
        self.mode = InputMode::TodoistToken;
    }

    /// Lanza la sincronización con Todoist en un hilo aparte: envía las
    /// pendientes aún no exportadas y trae las completadas allí. El resultado
    /// se recoge en `tick()` sin congelar la interfaz.
    fn start_todoist_sync(&mut self) {
        if self.todoist_sync.is_some() {
            self.status = "Ya hay una sincronización con Todoist en curso".into();
            return;
        }
        let Some(token) = self.store.todoist_token.clone() else {
            // Como en escritorio: sin token, se abre primero la configuración.
            self.start_todoist_token();
            return;
        };

        // Recoge lo pendiente; el hilo no toca el store.
        let mut outgoing = Vec::new();
        let mut known_ids = Vec::new();
        for (pi, p) in self.store.projects.iter().enumerate() {
            for (ti, t) in p.todos.iter().enumerate() {
                if t.done {
                    continue;
                }
                if let Some(id) = &t.todoist_id {
                    known_ids.push(id.clone());
                    continue;
                }
                outgoing.push(todoist::Outgoing {
                    project: pi,
                    todo: ti,
                    project_name: p.name.clone(),
                    content: t.title.clone(),
                    due_date: t.date.map(|d| d.to_string()),
                    priority: todoist::priority(t.priority),
                    labels: t.tags.clone(),
                });
            }
        }
        if outgoing.is_empty() && known_ids.is_empty() {
            self.status = "Nada que sincronizar con Todoist".into();
            return;
        }
        let skipped = known_ids.len();

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            // Vuelta: qué tareas ya exportadas se han completado en Todoist.
            let (remote_done, pull_error) = if known_ids.is_empty() {
                (Vec::new(), None)
            } else {
                todoist::fetch_completed(&token, &known_ids)
            };
            // Ida: crea en Todoist lo que aún no está.
            let (created, push_error) = if outgoing.is_empty() {
                (Vec::new(), None)
            } else {
                todoist::export(&token, &outgoing)
            };
            // `export` crea en orden, así que la k-ésima creada es la k-ésima
            // saliente: se le adjunta el título para poder verificarla luego.
            let created = created
                .into_iter()
                .zip(outgoing.iter())
                .map(|((pi, ti, id), o)| (pi, ti, id, o.content.clone()))
                .collect();
            let _ = tx.send(TodoistOutcome {
                created,
                remote_done,
                skipped,
                error: push_error.or(pull_error),
            });
        });
        self.todoist_sync = Some(rx);
        self.status = "Sincronizando con Todoist…".into();
    }

    /// Aplica el resultado de la sincronización: registra las ids remotas de
    /// lo creado (aunque haya fallado a medias, para no duplicarlo después) y
    /// marca lo completado en Todoist. No pasa por la pila de deshacer: undo
    /// borraría las ids y el siguiente sync duplicaría tareas.
    fn apply_todoist(&mut self, outcome: TodoistOutcome) {
        let exported = outcome.created.len();
        for (pi, ti, id, title) in outcome.created {
            // Misma posición y mismo título: caso normal.
            let direct = self
                .store
                .projects
                .get(pi)
                .and_then(|p| p.todos.get(ti))
                .is_some_and(|t| t.title == title && t.todoist_id.is_none());
            if direct {
                self.store.projects[pi].todos[ti].todoist_id = Some(id);
                continue;
            }
            // La tarea se movió/reordenó durante la sincronización: por título.
            'search: for p in &mut self.store.projects {
                for t in &mut p.todos {
                    if t.todoist_id.is_none() && t.title == title {
                        t.todoist_id = Some(id);
                        break 'search;
                    }
                }
            }
        }

        // Completa por id remota: las inserciones de recurrentes mueven índices.
        let today = Local::now().date_naive();
        let mut completed = 0;
        for id in &outcome.remote_done {
            let mut found = None;
            for (pi, p) in self.store.projects.iter().enumerate() {
                if let Some(ti) = p
                    .todos
                    .iter()
                    .position(|t| !t.done && t.todoist_id.as_deref() == Some(id.as_str()))
                {
                    found = Some((pi, ti));
                    break;
                }
            }
            if let Some((pi, ti)) = found {
                complete_todo(&mut self.store.projects[pi], ti, today);
                completed += 1;
            }
        }

        if exported > 0 || completed > 0 {
            self.clamp_indices();
            self.save();
        }

        let mut parts = Vec::new();
        if exported > 0 {
            parts.push(format!("{exported} enviadas a Todoist"));
        }
        if completed > 0 {
            parts.push(format!("{completed} completadas desde Todoist"));
        }
        let summary = if parts.is_empty() {
            format!("Nada nuevo ({} ya estaban en Todoist)", outcome.skipped)
        } else {
            parts.join(", ")
        };
        self.status = match outcome.error {
            Some(e) => format!("Todoist: {e} ({summary})"),
            None => summary,
        };
    }

    // --- Export / Import ------------------------------------------------------

    fn export_markdown(&mut self) {
        let mut out = String::from("# Xietiao — export\n\n");
        for p in &self.store.projects {
            out.push_str(&format!("## {}\n\n", p.name));
            for t in &p.todos {
                let mark = if t.done { "x" } else { " " };
                out.push_str(&format!("- [{mark}] {}", t.title));
                if let Some(d) = t.date {
                    out.push_str(&format!(" (📅 {})", d.format("%d/%m/%Y")));
                }
                for tag in &t.tags {
                    out.push_str(&format!(" #{tag}"));
                }
                out.push('\n');
                for s in &t.subtasks {
                    let sm = if s.done { "x" } else { " " };
                    out.push_str(&format!("  - [{sm}] {}\n", s.title));
                }
            }
            if !p.notes.is_empty() {
                out.push_str(&format!("\n> {}\n", p.notes.replace('\n', "\n> ")));
            }
            out.push('\n');
        }
        match std::env::current_dir().map(|d| d.join("xietiao-export.md")) {
            Ok(path) => match std::fs::write(&path, out) {
                Ok(_) => self.status = format!("Exportado a {}", path.display()),
                Err(e) => self.status = format!("Error al exportar: {e}"),
            },
            Err(e) => self.status = format!("Error: {e}"),
        }
    }

    fn export_json(&mut self) {
        match serde_json::to_string_pretty(&self.store) {
            Ok(json) => match std::env::current_dir().map(|d| d.join("xietiao-export.json")) {
                Ok(path) => match std::fs::write(&path, json) {
                    Ok(_) => self.status = format!("Exportado a {}", path.display()),
                    Err(e) => self.status = format!("Error al exportar: {e}"),
                },
                Err(e) => self.status = format!("Error: {e}"),
            },
            Err(e) => self.status = format!("Error al serializar: {e}"),
        }
    }

    fn import_json(&mut self) {
        let path = match std::env::current_dir().map(|d| d.join("xietiao-import.json")) {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("Error: {e}");
                return;
            }
        };
        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str::<Store>(&contents) {
                Ok(store) => {
                    self.record();
                    self.store = store;
                    self.clamp_indices();
                    self.save();
                    self.status = format!("Importado desde {} (u: deshacer)", path.display());
                }
                Err(e) => self.status = format!("JSON inválido: {e}"),
            },
            Err(_) => {
                self.status = format!("No se encontró {}", path.display());
            }
        }
    }

    // --- Manejo de teclas de overlays ----------------------------------------

    fn on_key_overlay(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        let close = matches!(
            key.code,
            KeyCode::Esc | KeyCode::Char('q')
        );
        match self.overlay.clone() {
            Overlay::None => {}
            // Vistas informativas: cualquier tecla las cierra.
            Overlay::Help | Overlay::Stats => {
                self.overlay = Overlay::None;
            }
            // Agenda: navegable, con salto directo a la tarea (como en escritorio).
            Overlay::Agenda { date, sel } => {
                let items = self.agenda_items_idx(date);
                let n = items.len();
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') if n > 0 => {
                        self.overlay = Overlay::Agenda {
                            date,
                            sel: sel.saturating_sub(1),
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') if n > 0 => {
                        self.overlay = Overlay::Agenda {
                            date,
                            sel: (sel + 1).min(n.saturating_sub(1)),
                        }
                    }
                    KeyCode::Enter => match items.get(sel) {
                        Some(&(pi, ti)) => self.jump_to_todo(pi, ti),
                        None => self.overlay = Overlay::None,
                    },
                    _ => self.overlay = Overlay::None,
                }
            }
            Overlay::WeekAgenda { anchor } => match key.code {
                KeyCode::Left | KeyCode::Char('h') => {
                    self.overlay = Overlay::WeekAgenda {
                        anchor: anchor - chrono::Duration::days(7),
                    }
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    self.overlay = Overlay::WeekAgenda {
                        anchor: anchor + chrono::Duration::days(7),
                    }
                }
                _ => self.overlay = Overlay::None,
            },
            Overlay::MoveTodo { sel } => {
                let n = self.store.projects.len();
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.overlay = Overlay::MoveTodo {
                            sel: sel.saturating_sub(1),
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.overlay = Overlay::MoveTodo {
                            sel: (sel + 1).min(n.saturating_sub(1)),
                        }
                    }
                    KeyCode::Enter => {
                        self.move_todo_to(sel);
                        self.overlay = Overlay::None;
                    }
                    _ if close => self.overlay = Overlay::None,
                    _ => {}
                }
            }
            Overlay::Subtasks { sel } => {
                let len = self.selected_todo().map(|t| t.subtasks.len()).unwrap_or(0);
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.overlay = Overlay::Subtasks {
                            sel: sel.saturating_sub(1),
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.overlay = Overlay::Subtasks {
                            sel: (sel + 1).min(len.saturating_sub(1)),
                        }
                    }
                    KeyCode::Char(' ') | KeyCode::Enter => self.toggle_subtask(sel),
                    KeyCode::Char('a') => {
                        self.input.clear();
                        self.mode = InputMode::AddSubtask;
                    }
                    KeyCode::Char('d') => {
                        self.delete_subtask(sel);
                        let new_len = self.selected_todo().map(|t| t.subtasks.len()).unwrap_or(0);
                        self.overlay = Overlay::Subtasks {
                            sel: sel.min(new_len.saturating_sub(1)),
                        };
                    }
                    _ if close || key.code == KeyCode::Char('s') => self.overlay = Overlay::None,
                    _ => {}
                }
            }
            Overlay::Pending { sel } => {
                let items = self.pending_items();
                let n = items.len();
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.overlay = Overlay::Pending {
                            sel: sel.saturating_sub(1),
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.overlay = Overlay::Pending {
                            sel: (sel + 1).min(n.saturating_sub(1)),
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(&(pi, ti)) = items.get(sel) {
                            self.jump_to_todo(pi, ti);
                        }
                    }
                    KeyCode::Char(' ') => {
                        if let Some(&(pi, ti)) = items.get(sel) {
                            self.record();
                            if let Some(t) = self
                                .store
                                .projects
                                .get_mut(pi)
                                .and_then(|p| p.todos.get_mut(ti))
                            {
                                t.done = true;
                                t.completed_at = Some(Local::now().date_naive());
                            }
                            self.save();
                            self.overlay = Overlay::Pending {
                                sel: sel.min(n.saturating_sub(2).max(0)),
                            };
                        }
                    }
                    _ if close => self.overlay = Overlay::None,
                    _ => {}
                }
            }
            Overlay::Trash { sel } => {
                let n = self.store.trash.len();
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.overlay = Overlay::Trash {
                            sel: sel.saturating_sub(1),
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.overlay = Overlay::Trash {
                            sel: (sel + 1).min(n.saturating_sub(1)),
                        }
                    }
                    KeyCode::Char('r') => {
                        self.restore_trash(sel);
                        let nn = self.store.trash.len();
                        self.overlay = Overlay::Trash {
                            sel: sel.min(nn.saturating_sub(1)),
                        };
                    }
                    KeyCode::Char('d') | KeyCode::Char('x') => {
                        self.purge_trash(sel);
                        let nn = self.store.trash.len();
                        self.overlay = Overlay::Trash {
                            sel: sel.min(nn.saturating_sub(1)),
                        };
                    }
                    _ if close => self.overlay = Overlay::None,
                    _ => {}
                }
            }
            Overlay::Menu { sel } => {
                const N: usize = 5;
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.overlay = Overlay::Menu {
                            sel: sel.saturating_sub(1),
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.overlay = Overlay::Menu {
                            sel: (sel + 1).min(N - 1),
                        }
                    }
                    KeyCode::Enter => {
                        self.overlay = Overlay::None;
                        match sel {
                            0 => self.export_markdown(),
                            1 => self.export_json(),
                            2 => self.import_json(),
                            3 => self.start_todoist_token(),
                            _ => self.start_todoist_sync(),
                        }
                    }
                    _ if close => self.overlay = Overlay::None,
                    _ => {}
                }
            }
        }
    }

    // --- Manejo de teclas -----------------------------------------------------

    /// Procesa una tecla. Devuelve nada; muta el estado.
    pub fn on_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};

        // Entrada de texto (incl. búsqueda) tiene prioridad: así un overlay puede
        // alojar un campo de entrada (p. ej. añadir una subtarea sin cerrarlo).

        // Búsqueda en vivo: filtra la lista de to-dos mientras escribes.
        if self.mode == InputMode::Search {
            match key.code {
                KeyCode::Esc => {
                    self.search.clear();
                    self.mode = InputMode::Normal;
                    self.clamp_indices();
                }
                KeyCode::Enter => self.mode = InputMode::Normal, // mantiene el filtro
                KeyCode::Backspace => {
                    self.search.pop();
                    self.todo_idx = 0;
                }
                KeyCode::Char(c) => {
                    self.search.push(c);
                    self.todo_idx = 0;
                }
                _ => {}
            }
            return;
        }

        // Modo de entrada de texto: capturamos casi todo.
        if self.mode != InputMode::Normal {
            match key.code {
                KeyCode::Esc => {
                    self.input.clear();
                    self.mode = InputMode::Normal;
                }
                KeyCode::Enter => {
                    if self.mode == InputMode::EditNotes {
                        // En notas, Enter inserta salto de línea; Ctrl+S / Esc guardan.
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            self.commit_input();
                        } else {
                            self.input.push('\n');
                        }
                    } else {
                        self.commit_input();
                    }
                }
                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.commit_input();
                }
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Char(c) => {
                    self.input.push(c);
                }
                _ => {}
            }
            return;
        }

        // Confirmación de borrado.
        if self.confirm_delete {
            self.confirm_delete = false;
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.delete_current(),
                _ => self.status = "Borrado cancelado".into(),
            }
            return;
        }

        // Overlays (vistas superpuestas): tienen su propio manejo.
        if !self.overlay.is_none() {
            self.on_key_overlay(key);
            return;
        }

        // Modo normal: atajos críticos no reasignables primero.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.save();
            self.should_quit = true;
            return;
        }
        if key.code == KeyCode::Esc && !self.search.is_empty() {
            self.search.clear();
            self.clamp_indices();
            self.status = "Filtro quitado".into();
            return;
        }
        // 'i' edita las notas (alternativa a Enter en el panel de notas).
        if key.code == KeyCode::Char('i') && self.focus == Focus::Notes {
            self.start_edit_notes();
            return;
        }

        // Resto de teclas: se resuelven contra el keymap configurable.
        let chord = key_to_string(key.code, key.modifiers);
        if let Some(&action) = self.config.keymap.get(&chord) {
            self.run_action(action);
        }
    }

    /// Ejecuta una acción lógica (resuelta desde el keymap). Las acciones
    /// dependientes de panel comprueban el foco actual.
    fn run_action(&mut self, action: Action) {
        use Action::*;
        match action {
            Quit => {
                self.save();
                self.should_quit = true;
            }
            Help => self.overlay = Overlay::Help,
            Undo => self.undo(),
            NextPanel => self.focus = self.focus.next(),
            PrevPanel => self.focus = self.focus.prev(),
            Down => {
                if self.focus == Focus::Calendar {
                    self.move_cursor(7);
                } else {
                    self.move_selection(1);
                }
            }
            Up => {
                if self.focus == Focus::Calendar {
                    self.move_cursor(-7);
                } else {
                    self.move_selection(-1);
                }
            }
            Left => {
                if self.focus == Focus::Calendar {
                    self.move_cursor(-1);
                }
            }
            Right => {
                if self.focus == Focus::Calendar {
                    self.move_cursor(1);
                }
            }
            Add => self.start_add(),
            Rename => self.start_rename(),
            Delete => self.request_delete(),
            Activate => match self.focus {
                Focus::Todos => self.toggle_todo(),
                Focus::Notes => self.start_edit_notes(),
                Focus::Timer => self.toggle_selected_clock(),
                Focus::Calendar => {
                    self.overlay = Overlay::Agenda {
                        date: self.calendar_cursor,
                        sel: 0,
                    }
                }
                _ => {}
            },
            AssignDate => {
                if self.focus == Focus::Todos {
                    self.assign_todo_date();
                }
            }
            SetDate => {
                if self.focus == Focus::Todos {
                    self.start_set_date();
                }
            }
            PrevMonth => self.shift_month(-1),
            NextMonth => self.shift_month(1),
            CalendarToday => self.calendar_today(),
            TodoistSync => self.start_todoist_sync(),
            CyclePriority => {
                if self.focus == Focus::Todos {
                    self.cycle_priority();
                }
            }
            CycleRecurrence => {
                if self.focus == Focus::Todos {
                    self.cycle_recurrence();
                }
            }
            Subtasks => {
                if self.focus == Focus::Todos {
                    self.open_subtasks();
                }
            }
            MoveTodo => {
                if self.focus == Focus::Todos {
                    self.open_move_todo();
                }
            }
            LinkPomodoro => {
                if self.focus == Focus::Todos {
                    self.link_pomodoro();
                }
            }
            Search => {
                if self.focus == Focus::Todos {
                    self.start_search();
                }
            }
            ToggleNotesScope => {
                if self.focus == Focus::Notes {
                    self.toggle_notes_scope();
                }
            }
            MoveItemUp => self.move_item(-1),
            MoveItemDown => self.move_item(1),
            AgendaToday => {
                self.overlay = Overlay::Agenda {
                    date: Local::now().date_naive(),
                    sel: 0,
                }
            }
            WeekAgenda => {
                self.overlay = Overlay::WeekAgenda {
                    anchor: monday_of(Local::now().date_naive()),
                }
            }
            Pending => self.overlay = Overlay::Pending { sel: 0 },
            Stats => self.overlay = Overlay::Stats,
            Trash => self.overlay = Overlay::Trash { sel: 0 },
            Menu => self.overlay = Overlay::Menu { sel: 0 },
            ResetClock => {
                if self.focus == Focus::Timer {
                    self.reset_selected_clock();
                }
            }
            SwitchPomo => {
                if self.focus == Focus::Timer && self.clock_sel == ClockSel::Pomodoro {
                    self.timer.switch_mode();
                }
            }
        }
    }

    /// Avanza el reloj del temporizador.
    pub fn tick(&mut self, elapsed: Duration) {
        // Recoge el resultado de la sincronización con Todoist, si terminó.
        if let Some(rx) = &self.todoist_sync {
            match rx.try_recv() {
                Ok(outcome) => {
                    self.todoist_sync = None;
                    self.apply_todoist(outcome);
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.todoist_sync = None;
                    self.status = "Todoist: la sincronización falló".into();
                }
            }
        }

        self.stopwatch.tick(elapsed);
        if self.timer.tick(elapsed) {
            let was_break = self.timer.on_break;
            if !was_break {
                // Foco completado: lo registramos en el historial.
                let (project, todo) = self.pomodoro_labels();
                self.store.pomodoros.push(PomodoroSession {
                    date: Local::now().date_naive(),
                    project,
                    todo,
                });
                self.save();
            }
            self.status = if was_break {
                "Descanso terminado ⏰".into()
            } else {
                "Pomodoro terminado ⏰ ¡toca descanso!".into()
            };
            self.notify_finish();
        }

        // Autoguardado periódico de seguridad.
        self.since_save += elapsed;
        if self.since_save >= Duration::from_secs(15) {
            self.save();
            self.since_save = Duration::ZERO;
        }
    }
}

/// Marca una tarea como hecha. Si es recurrente, genera la siguiente
/// aparición justo debajo (misma lógica que la versión de escritorio).
/// Devuelve true si se regeneró una aparición nueva.
fn complete_todo(project: &mut Project, todo: usize, today: NaiveDate) -> bool {
    let mut regen: Option<(usize, Todo)> = None;
    if let Some(t) = project.todos.get_mut(todo) {
        t.done = true;
        t.completed_at = Some(today);
        if t.recurrence != Recurrence::None {
            let base = t.date.unwrap_or(today);
            if let Some(next) = t.recurrence.next_date(base) {
                let mut copy = t.clone();
                copy.done = false;
                copy.completed_at = None;
                copy.date = Some(next);
                copy.todoist_id = None; // la nueva aparición aún no está en Todoist
                for sub in &mut copy.subtasks {
                    sub.done = false;
                }
                regen = Some((todo + 1, copy));
            }
        }
    }
    match regen {
        Some((pos, copy)) => {
            project.todos.insert(pos.min(project.todos.len()), copy);
            true
        }
        None => false,
    }
}

/// Lunes de la semana que contiene `d`.
pub fn monday_of(d: NaiveDate) -> NaiveDate {
    let back = d.weekday().num_days_from_monday() as i64;
    d - chrono::Duration::days(back)
}

/// Separa los `#tags` del texto de un to-do. Devuelve (título sin tags, tags en minúsculas).
fn parse_tags(text: &str) -> (String, Vec<String>) {
    let mut title_words = Vec::new();
    let mut tags = Vec::new();
    for word in text.split_whitespace() {
        if let Some(tag) = word.strip_prefix('#') {
            let tag = tag.trim().to_lowercase();
            if !tag.is_empty() && !tags.contains(&tag) {
                tags.push(tag);
            }
        } else {
            title_words.push(word);
        }
    }
    let title = title_words.join(" ");
    // Si sólo había tags, conserva el texto original como título para no perderlo.
    if title.is_empty() {
        (text.trim().to_string(), tags)
    } else {
        (title, tags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tags_splits_title_and_tags() {
        let (title, tags) = parse_tags("comprar pan #casa #compras");
        assert_eq!(title, "comprar pan");
        assert_eq!(tags, vec!["casa".to_string(), "compras".to_string()]);
    }

    #[test]
    fn parse_tags_dedups_and_lowercases() {
        let (_title, tags) = parse_tags("x #Casa #casa");
        assert_eq!(tags, vec!["casa".to_string()]);
    }

    #[test]
    fn parse_tags_only_tags_keeps_text_as_title() {
        let (title, tags) = parse_tags("#solo");
        assert_eq!(title, "#solo");
        assert_eq!(tags, vec!["solo".to_string()]);
    }

    #[test]
    fn monday_of_returns_monday() {
        // 1 de julio de 2026 es miércoles → lunes es el 29 de junio.
        let wed = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        assert_eq!(monday_of(wed), NaiveDate::from_ymd_opt(2026, 6, 29).unwrap());
    }

    #[test]
    fn wrap_cycles() {
        assert_eq!(wrap(2, 1, 3), 0);
        assert_eq!(wrap(0, -1, 3), 2);
    }
}

/// Suma `delta` a `idx` con wrap-around dentro de `[0, len)`.
fn wrap(idx: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let len_i = len as isize;
    let mut next = idx as isize + delta;
    next = ((next % len_i) + len_i) % len_i;
    next as usize
}
