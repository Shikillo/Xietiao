//! Renderizado de la interfaz: reproduce el layout de la maqueta.

use chrono::{Datelike, Local, NaiveDate};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app::{monday_of, App, ClockSel, Focus, InputMode, NotesScope, Overlay};
use crate::config::Theme;
use crate::model::{Priority, Recurrence};

/// Punto de entrada del dibujado.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Marco exterior de toda la app.
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(" Xietiao ")
        .title_style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(Color::Gray));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    // Cuerpo + línea de estado.
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(root[0]);

    draw_left_column(frame, app, columns[0]);
    draw_right_column(frame, app, columns[1]);
    draw_status(frame, app, root[1]);

    // Overlays de tipo "vista" (pantalla parcial).
    match &app.overlay {
        Overlay::None => {}
        Overlay::Help => draw_help(frame, app.theme(), area),
        Overlay::Agenda(date) => draw_agenda(frame, app, area, *date),
        Overlay::MoveTodo { sel } => draw_move_todo(frame, app, area, *sel),
        Overlay::Subtasks { sel } => draw_subtasks(frame, app, area, *sel),
        Overlay::Pending { sel } => draw_pending(frame, app, area, *sel),
        Overlay::WeekAgenda { anchor } => draw_week_agenda(frame, app, area, *anchor),
        Overlay::Stats => draw_stats(frame, app, area),
        Overlay::Trash { sel } => draw_trash(frame, app, area, *sel),
        Overlay::Menu { sel } => draw_menu(frame, app, area, *sel),
    }

    // Popups de entrada de texto (van por encima de todo).
    if matches!(
        app.mode,
        InputMode::AddProject
            | InputMode::AddTodo
            | InputMode::AddSubtask
            | InputMode::EditProject
            | InputMode::EditTodo
    ) {
        draw_input_popup(frame, app, area);
    }
}

fn draw_left_column(frame: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(area);

    draw_projects(frame, app, rows[0]);
    draw_todos(frame, app, rows[1]);
}

fn draw_right_column(frame: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(46),
            Constraint::Min(6),
            Constraint::Length(4),
        ])
        .split(area);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(16)])
        .split(rows[0]);

    draw_calendar(frame, app, top[0]);
    draw_clocks(frame, app, top[1]);
    draw_notes(frame, app, rows[1]);
    draw_progress(frame, app, rows[2]);
}

/// Construye un bloque con borde resaltado según el foco.
fn panel(theme: &Theme, title: &str, focused: bool) -> Block<'static> {
    let color = if focused { theme.focus } else { theme.idle };
    let title_style = if focused {
        Style::default().fg(theme.focus).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .title_style(title_style)
        .border_style(Style::default().fg(color))
}

fn highlight_style(theme: &Theme, focused: bool) -> Style {
    if focused {
        Style::default()
            .fg(Color::Black)
            .bg(theme.focus)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::REVERSED).fg(theme.idle)
    }
}

fn draw_projects(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Projects;
    let block = panel(app.theme(), "proyectos", focused);

    if app.store.projects.is_empty() {
        let p = Paragraph::new("Sin proyectos.\nPulsa 'a' para crear uno.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(p, area);
        return;
    }

    let items: Vec<ListItem> = app
        .store
        .projects
        .iter()
        .map(|p| {
            let line = Line::from(vec![
                Span::raw(p.name.clone()),
                Span::styled(
                    format!("  ({}/{})", p.done_count(), p.todos.len()),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(highlight_style(app.theme(), focused))
        .highlight_symbol("▌ ");

    let mut state = ListState::default();
    state.select(Some(app.project_idx));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_todos(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Todos;
    let base_title = match app.current_project() {
        Some(p) => format!("to-dos · {}", p.name),
        None => "to-dos".to_string(),
    };
    let title = if app.search.is_empty() {
        base_title
    } else {
        format!("{base_title}  /{}", app.search)
    };
    let block = panel(app.theme(), &title, focused);

    let Some(project) = app.current_project() else {
        let p = Paragraph::new("Selecciona o crea un proyecto.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(p, area);
        return;
    };

    if project.todos.is_empty() {
        let p = Paragraph::new("Sin tareas.\nPulsa 'a' para añadir.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(p, area);
        return;
    }

    let indices = app.filtered_todo_indices();
    if indices.is_empty() {
        let p = Paragraph::new("Sin coincidencias para el filtro.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(p, area);
        return;
    }

    let today = Local::now().date_naive();
    let theme = app.theme();

    let items: Vec<ListItem> = indices
        .iter()
        .map(|&i| {
            let t = &project.todos[i];
            let (mark, mark_style) = if t.done {
                ("[x] ", Style::default().fg(theme.done))
            } else {
                ("[ ] ", Style::default())
            };
            let mut text_style = mark_style;
            if t.done {
                text_style = text_style.add_modifier(Modifier::CROSSED_OUT);
            }

            let mut spans: Vec<Span> = Vec::new();

            // Marca de prioridad coloreada.
            if t.priority != Priority::None {
                spans.push(Span::styled(
                    format!("{} ", t.priority.marker()),
                    Style::default()
                        .fg(priority_color(app.theme(), t.priority))
                        .add_modifier(Modifier::BOLD),
                ));
            }

            spans.push(Span::styled(mark, mark_style));
            spans.push(Span::styled(t.title.clone(), text_style));

            // Progreso de subtareas, si tiene.
            let (sd, st) = t.subtask_progress();
            if st > 0 {
                let all = sd == st;
                spans.push(Span::styled(
                    format!("  ☑{sd}/{st}"),
                    Style::default().fg(if all { theme.done } else { theme.idle }),
                ));
            }

            // Marca de recurrencia.
            if t.recurrence != Recurrence::None {
                spans.push(Span::styled(
                    format!("  {}", t.recurrence.label()),
                    Style::default().fg(theme.recurrence),
                ));
            }

            if let Some(d) = t.date {
                // Rojo: vencida (pasada y sin completar). Ámbar: vence hoy. Amarillo: futura.
                let color = if !t.done && d < today {
                    theme.overdue
                } else if !t.done && d == today {
                    theme.today
                } else {
                    theme.due
                };
                spans.push(Span::styled(
                    format!("  · {}", d.format("%d/%m")),
                    Style::default().fg(color),
                ));
            }

            // Etiquetas.
            for tag in &t.tags {
                spans.push(Span::styled(
                    format!("  #{tag}"),
                    Style::default().fg(theme.tag),
                ));
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(highlight_style(theme, focused))
        .highlight_symbol("▌ ");

    let mut state = ListState::default();
    state.select(Some(app.todo_idx));
    frame.render_stateful_widget(list, area, &mut state);
}

fn priority_color(theme: &Theme, p: Priority) -> Color {
    match p {
        Priority::High => theme.priority_high,
        Priority::Medium => theme.priority_medium,
        Priority::Low => theme.priority_low,
        Priority::None => theme.idle,
    }
}

fn draw_calendar(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Calendar;
    let block = panel(app.theme(), "calendario", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 7 || inner.height < 4 {
        return;
    }

    let anchor = app.calendar_anchor;
    let year = anchor.year();
    let month = anchor.month();
    let today = Local::now().date_naive();

    let first_weekday = anchor.weekday().num_days_from_monday() as usize;
    let days_in_month = days_in_month(year, month);

    // Ancho de columna: repartimos el ancho disponible entre los 7 días.
    let col_w = (inner.width as usize / 7).max(3);

    // Cabecera del mes.
    let header = format!("{} {}", month_name_es(month), year);
    let mut header_lines: Vec<Line> = vec![Line::from(header).centered().style(
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    )];

    // Fila de días de la semana, cada etiqueta centrada en su columna.
    let weekdays = ["lu", "ma", "mi", "ju", "vi", "sá", "do"];
    let weekday_row: String = weekdays.iter().map(|d| center_in(d, col_w)).collect();
    header_lines.push(Line::from(weekday_row).style(Style::default().fg(Color::DarkGray)));

    // Filas de semanas.
    let mut weeks: Vec<Line> = Vec::new();
    let mut spans: Vec<Span> = Vec::new();
    for _ in 0..first_weekday {
        spans.push(Span::raw(" ".repeat(col_w)));
    }
    let mut col = first_weekday;
    for day in 1..=days_in_month {
        let date = NaiveDate::from_ymd_opt(year, month, day);
        let is_today = today.year() == year && today.month() == month && today.day() == day;
        let is_cursor = date == Some(app.calendar_cursor);
        let (has_todos, has_overdue) = date
            .map(|d| {
                app.current_project()
                    .map(|p| {
                        let any = p.todos.iter().any(|t| t.date == Some(d));
                        let overdue =
                            d < today && p.todos.iter().any(|t| t.date == Some(d) && !t.done);
                        (any, overdue)
                    })
                    .unwrap_or((false, false))
            })
            .unwrap_or((false, false));

        // Estilo del día combinando: tareas, vencidas, hoy y cursor.
        let theme = app.theme();
        let mut style = Style::default();
        if is_today {
            style = style.fg(theme.today).add_modifier(Modifier::BOLD);
        }
        if has_todos {
            style = style.fg(theme.done).add_modifier(Modifier::BOLD);
        }
        if has_overdue {
            style = style.fg(theme.overdue).add_modifier(Modifier::BOLD);
        }
        if is_cursor {
            style = style.add_modifier(Modifier::UNDERLINED);
            if focused {
                style = style.add_modifier(Modifier::REVERSED);
            }
        }

        let cell = center_in(&day.to_string(), col_w);
        spans.push(Span::styled(cell, style));
        col += 1;
        if col == 7 {
            weeks.push(Line::from(std::mem::take(&mut spans)));
            col = 0;
        }
    }
    if !spans.is_empty() {
        weeks.push(Line::from(spans));
    }

    // Reparte el alto sobrante como líneas en blanco entre semanas.
    let used = header_lines.len() + weeks.len();
    let spare = (inner.height as usize).saturating_sub(used);
    let gap = if !weeks.is_empty() { spare / weeks.len() } else { 0 };

    let mut lines = header_lines;
    for week in weeks {
        for _ in 0..gap {
            lines.push(Line::from(""));
        }
        lines.push(week);
    }

    let p = Paragraph::new(Text::from(lines));
    frame.render_widget(p, inner);
}

/// Columna de tres relojes: pomodoro, reloj y cronómetro.
fn draw_clocks(frame: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .split(area);

    let focused = app.focus == Focus::Timer;
    let theme = app.theme();

    // Pomodoro (cuenta atrás).
    let pomo_color = if app.timer.running {
        theme.done
    } else {
        Color::Gray
    };
    let base = if app.timer.on_break { "break" } else { "foco" };
    let today_count = app.store.pomodoros_on(Local::now().date_naive());
    let pomo_sub = format!("{base} · {today_count} hoy");
    clock_box(
        frame,
        theme,
        rows[0],
        "pomodoro",
        &app.timer.label(),
        pomo_color,
        Some(&pomo_sub),
        focused && app.clock_sel == ClockSel::Pomodoro,
    );

    // Reloj (hora actual).
    let now = Local::now().format("%H:%M:%S").to_string();
    clock_box(
        frame,
        theme,
        rows[1],
        "reloj",
        &now,
        Color::White,
        None,
        focused && app.clock_sel == ClockSel::Reloj,
    );

    // Cronómetro (cuenta adelante).
    let crono_color = if app.stopwatch.running {
        theme.done
    } else {
        Color::Gray
    };
    clock_box(
        frame,
        theme,
        rows[2],
        "cronómetro",
        &app.stopwatch.label(),
        crono_color,
        None,
        focused && app.clock_sel == ClockSel::Cronometro,
    );
}

/// Dibuja una caja pequeña de reloj con su título, valor y un subtítulo opcional.
fn clock_box(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    title: &str,
    value: &str,
    value_color: Color,
    subtitle: Option<&str>,
    selected: bool,
) {
    let block = panel(theme, title, selected);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let mut content: Vec<Line> = vec![Line::from(Span::styled(
        value.to_string(),
        Style::default()
            .fg(value_color)
            .add_modifier(Modifier::BOLD),
    ))
    .centered()];
    if let Some(sub) = subtitle {
        content.push(
            Line::from(Span::styled(
                sub.to_string(),
                Style::default().fg(Color::DarkGray),
            ))
            .centered(),
        );
    }

    let top_pad = (inner.height as usize).saturating_sub(content.len()) / 2;
    let mut lines: Vec<Line> = vec![Line::from(""); top_pad];
    lines.extend(content);

    let p = Paragraph::new(Text::from(lines)).alignment(Alignment::Center);
    frame.render_widget(p, inner);
}

fn draw_notes(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Notes;
    let editing = app.mode == InputMode::EditNotes;

    // Etiqueta del ámbito: proyecto concreto o "general".
    let scope_label = match app.effective_notes_scope() {
        NotesScope::General => "general".to_string(),
        NotesScope::Project => app
            .current_project()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "general".to_string()),
    };
    let title = if editing {
        format!("notas · {scope_label} · editando (Ctrl+S/Esc)")
    } else {
        format!("notas · {scope_label}  (g: cambiar)")
    };
    let block = panel(app.theme(), &title, focused || editing);

    let notes_empty = app.active_notes().is_empty();
    let content = if editing {
        let mut s = app.input.clone();
        s.push('▌');
        s
    } else if notes_empty {
        "Sin notas. Pulsa 'e' para editar.".to_string()
    } else {
        app.active_notes().to_string()
    };

    let style = if notes_empty && !editing {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };

    let p = Paragraph::new(content)
        .style(style)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

fn draw_progress(frame: &mut Frame, app: &App, area: Rect) {
    let project = app.current_project();
    let (done, total) = match project {
        Some(p) => (p.done_count(), p.todos.len()),
        None => (0, 0),
    };

    // El recuento va en el título; la barra queda limpia para el dibujo.
    let title = if total > 0 {
        format!(
            "barra de progreso · {done}/{total} ({:.0}%)",
            done as f64 / total as f64 * 100.0
        )
    } else {
        "barra de progreso".to_string()
    };
    let block = panel(app.theme(), &title, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 2 || inner.height < 1 {
        return;
    }

    let w = inner.width as usize;
    let n = total;
    let done_style = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let todo_style = Style::default().fg(Color::Gray);

    // Segmento al que pertenece la columna `c` (cada to-do es un segmento).
    let seg_of = |c: usize| -> usize {
        if n == 0 {
            0
        } else {
            (c * n / w).min(n - 1)
        }
    };

    let mut top: Vec<Span> = Vec::with_capacity(w);
    let mut base: Vec<Span> = Vec::with_capacity(w);

    for c in 0..w {
        let seg = seg_of(c);
        // Hay marca (tick) en los extremos y en cada cambio de segmento.
        let boundary = c != 0 && n > 0 && seg != seg_of(c - 1);
        // Se llena de izquierda a derecha: los primeros `done` segmentos.
        let style = if n > 0 && seg < done {
            done_style
        } else {
            todo_style
        };

        let base_ch = if c == 0 {
            "└"
        } else if c == w - 1 {
            "┘"
        } else if boundary {
            "┴"
        } else {
            "─"
        };
        let top_ch = if c == 0 || c == w - 1 || boundary {
            "│"
        } else {
            " "
        };

        top.push(Span::styled(top_ch.to_string(), style));
        base.push(Span::styled(base_ch.to_string(), style));
    }

    let lines = vec![Line::from(top), Line::from(base)];
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let overlay_hint = match &app.overlay {
        Overlay::None => None,
        Overlay::Help => Some("Ayuda · pulsa cualquier tecla para cerrar".to_string()),
        Overlay::Agenda(_) => Some("Agenda · pulsa cualquier tecla para cerrar".to_string()),
        Overlay::MoveTodo { .. } => {
            Some("Mover tarea · ↑↓: elegir · Enter: mover · Esc: cancelar".to_string())
        }
        Overlay::Subtasks { .. } => {
            Some("Subtareas · ↑↓ · Espacio: marcar · a: añadir · d: borrar · Esc: cerrar".to_string())
        }
        Overlay::Pending { .. } => {
            Some("Pendientes · ↑↓ · Enter: ir · Espacio: completar · Esc: cerrar".to_string())
        }
        Overlay::WeekAgenda { .. } => {
            Some("Semana · ←→: cambiar semana · Esc: cerrar".to_string())
        }
        Overlay::Stats => Some("Estadísticas · pulsa cualquier tecla para cerrar".to_string()),
        Overlay::Trash { .. } => {
            Some("Papelera · ↑↓ · r: restaurar · d: borrar def. · Esc: cerrar".to_string())
        }
        Overlay::Menu { .. } => {
            Some("Menú · ↑↓ · Enter: ejecutar · Esc: cerrar".to_string())
        }
    };

    let hint: String = if app.confirm_delete {
        "¿Borrar? · y: sí · n/Esc: no".to_string()
    } else if let Some(h) = overlay_hint {
        h
    } else {
        match app.mode {
            InputMode::AddProject => "Nuevo proyecto · Enter: crear · Esc: cancelar".to_string(),
            InputMode::AddTodo => {
                "Nueva tarea · #tags admitidos · Enter: crear · Esc: cancelar".to_string()
            }
            InputMode::AddSubtask => "Nueva subtarea · Enter: crear · Esc: cancelar".to_string(),
            InputMode::EditNotes => {
                "Editando notas · Ctrl+S: guardar · Esc: cancelar".to_string()
            }
            InputMode::EditProject => "Renombrar proyecto · Enter: ok · Esc: cancelar".to_string(),
            InputMode::EditTodo => "Renombrar tarea · Enter: ok · Esc: cancelar".to_string(),
            InputMode::Search => format!("Buscar: {}▌ · Enter: aplicar · Esc: limpiar", app.search),
            InputMode::Normal => app.status.clone(),
        }
    };
    let p = Paragraph::new(Line::from(format!(" {hint}")))
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(p, area);
}

fn draw_input_popup(frame: &mut Frame, app: &App, area: Rect) {
    let title = match app.mode {
        InputMode::AddProject => " nuevo proyecto ",
        InputMode::AddTodo => " nueva tarea (#tags) ",
        InputMode::AddSubtask => " nueva subtarea ",
        InputMode::EditProject => " renombrar proyecto ",
        InputMode::EditTodo => " renombrar tarea ",
        _ => " ",
    };
    let popup = centered_rect(50, area, 3);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(app.theme().accent));
    let mut text = app.input.clone();
    text.push('▌');
    let p = Paragraph::new(text).block(block);
    frame.render_widget(p, popup);
}

/// Overlay de ayuda con los atajos de teclado.
fn draw_help(frame: &mut Frame, theme: &Theme, area: Rect) {
    let popup = centered_rect_pct(62, 80, area);
    frame.render_widget(Clear, popup);

    let rows = [
        ("Tab / Shift+Tab", "cambiar de panel"),
        ("↑ ↓  / j k", "navegar lista (± semana en calendario)"),
        ("← →  / h l", "mover día en el calendario"),
        ("a / n", "añadir proyecto o tarea (#tags en tareas)"),
        ("e", "renombrar proyecto/tarea · editar notas"),
        ("d", "borrar (a la papelera, con confirmación)"),
        ("u", "deshacer último cambio"),
        ("Espacio / Enter", "marcar tarea · play relojes · editar notas"),
        ("f", "asignar tarea al día del cursor (calendario)"),
        ("p / R", "prioridad · recurrencia de la tarea"),
        ("s", "editar subtareas de la tarea"),
        ("m", "mover la tarea a otro proyecto"),
        ("v", "vincular pomodoro a la tarea"),
        ("J / K", "mover el elemento arriba / abajo"),
        ("/", "buscar / filtrar (admite #tag)"),
        ("g", "notas del proyecto ↔ generales"),
        ("t / w", "agenda de hoy · de la semana"),
        ("P", "todas las tareas pendientes"),
        ("S", "estadísticas y racha"),
        ("x / o", "papelera · menú (export/import)"),
        ("r / b", "reset · foco↔break (relojes)"),
        ("? ", "mostrar esta ayuda"),
        ("q", "salir"),
    ];

    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "Atajos de Xietiao",
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    ))
    .centered()];
    lines.push(Line::from(""));
    for (key, desc) in rows {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {key:<18}"),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled(desc.to_string(), Style::default().fg(Color::Gray)),
        ]));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" ayuda ")
        .title_style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(theme.accent));
    let p = Paragraph::new(Text::from(lines)).block(block);
    frame.render_widget(p, popup);
}

/// Overlay con la agenda de un día (tareas de todos los proyectos).
fn draw_agenda(frame: &mut Frame, app: &App, area: Rect, date: NaiveDate) {
    let popup = centered_rect_pct(60, 60, area);
    frame.render_widget(Clear, popup);

    let items = app.agenda_items(date);
    let today = Local::now().date_naive();

    let mut lines: Vec<Line> = Vec::new();
    if items.is_empty() {
        lines.push(Line::from(Span::styled(
            "Sin tareas asignadas a este día.",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (proj, t) in items {
            let mark_style = if t.done {
                Style::default().fg(Color::Green)
            } else if date < today {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            };
            let mut title_style = mark_style;
            if t.done {
                title_style = title_style.add_modifier(Modifier::CROSSED_OUT);
            }
            let mark = if t.done { "[x] " } else { "[ ] " };
            lines.push(Line::from(vec![
                Span::styled(mark, mark_style),
                Span::styled(t.title.clone(), title_style),
                Span::styled(format!("  ({proj})"), Style::default().fg(Color::DarkGray)),
            ]));
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" agenda · {} ", date.format("%d/%m/%Y")))
        .title_style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(app.theme().accent));
    let p = Paragraph::new(Text::from(lines))
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(p, popup);
}

/// Bloque estándar para overlays, con título resaltado.
fn overlay_block(theme: &Theme, title: String) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(theme.accent))
}

/// Overlay: selector de proyecto destino para mover una tarea.
fn draw_move_todo(frame: &mut Frame, app: &App, area: Rect, sel: usize) {
    let popup = centered_rect_pct(50, 60, area);
    frame.render_widget(Clear, popup);

    let items: Vec<ListItem> = app
        .store
        .projects
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut spans = vec![Span::raw(p.name.clone())];
            if i == app.project_idx {
                spans.push(Span::styled(
                    "  (actual)",
                    Style::default().fg(Color::DarkGray),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items)
        .block(overlay_block(app.theme(), " mover tarea a… ".into()))
        .highlight_style(highlight_style(app.theme(), true))
        .highlight_symbol("▌ ");
    let mut state = ListState::default();
    state.select(Some(sel));
    frame.render_stateful_widget(list, popup, &mut state);
}

/// Overlay: editor de subtareas (checklist) de la tarea seleccionada.
fn draw_subtasks(frame: &mut Frame, app: &App, area: Rect, sel: usize) {
    let popup = centered_rect_pct(60, 60, area);
    frame.render_widget(Clear, popup);

    let Some(todo) = app.selected_todo() else {
        return;
    };
    let (done, total) = todo.subtask_progress();
    let block = overlay_block(app.theme(), format!(" subtareas · {} ({}/{}) ", todo.title, done, total));

    if todo.subtasks.is_empty() {
        let p = Paragraph::new("Sin subtareas.\nPulsa 'a' para añadir una.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(p, popup);
        return;
    }

    let items: Vec<ListItem> = todo
        .subtasks
        .iter()
        .map(|s| {
            let (mark, style) = if s.done {
                ("[x] ", Style::default().fg(Color::Green).add_modifier(Modifier::CROSSED_OUT))
            } else {
                ("[ ] ", Style::default())
            };
            ListItem::new(Line::from(vec![
                Span::styled(mark, style),
                Span::styled(s.title.clone(), style),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(highlight_style(app.theme(), true))
        .highlight_symbol("▌ ");
    let mut state = ListState::default();
    state.select(Some(sel));
    frame.render_stateful_widget(list, popup, &mut state);
}

/// Overlay: todas las tareas pendientes cruzando proyectos.
fn draw_pending(frame: &mut Frame, app: &App, area: Rect, sel: usize) {
    let popup = centered_rect_pct(66, 70, area);
    frame.render_widget(Clear, popup);

    let items = app.pending_items();
    let today = Local::now().date_naive();
    let block = overlay_block(app.theme(), format!(" pendientes · {} ", items.len()));

    if items.is_empty() {
        let p = Paragraph::new("¡Sin pendientes! 🎉")
            .style(Style::default().fg(Color::Green))
            .block(block);
        frame.render_widget(p, popup);
        return;
    }

    let list_items: Vec<ListItem> = items
        .iter()
        .map(|&(pi, ti)| {
            let p = &app.store.projects[pi];
            let t = &p.todos[ti];
            let mut spans: Vec<Span> = Vec::new();
            if t.priority != Priority::None {
                spans.push(Span::styled(
                    format!("{} ", t.priority.marker()),
                    Style::default()
                        .fg(priority_color(app.theme(), t.priority))
                        .add_modifier(Modifier::BOLD),
                ));
            }
            spans.push(Span::raw(t.title.clone()));
            spans.push(Span::styled(
                format!("  ({})", p.name),
                Style::default().fg(Color::DarkGray),
            ));
            if let Some(d) = t.date {
                let overdue = d < today;
                let color = if overdue {
                    Color::Red
                } else if d == today {
                    Color::Rgb(255, 165, 0)
                } else {
                    Color::Yellow
                };
                spans.push(Span::styled(
                    format!("  · {}", d.format("%d/%m")),
                    Style::default().fg(color),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(list_items)
        .block(block)
        .highlight_style(highlight_style(app.theme(), true))
        .highlight_symbol("▌ ");
    let mut state = ListState::default();
    state.select(Some(sel));
    frame.render_stateful_widget(list, popup, &mut state);
}

/// Overlay: agenda de la semana que empieza en `anchor` (lunes).
fn draw_week_agenda(frame: &mut Frame, app: &App, area: Rect, anchor: NaiveDate) {
    let popup = centered_rect_pct(66, 76, area);
    frame.render_widget(Clear, popup);

    let monday = monday_of(anchor);
    let sunday = monday + chrono::Duration::days(6);
    let today = Local::now().date_naive();
    let names = ["Lunes", "Martes", "Miércoles", "Jueves", "Viernes", "Sábado", "Domingo"];

    let mut lines: Vec<Line> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let date = monday + chrono::Duration::days(i as i64);
        let is_today = date == today;
        let header_style = if is_today {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
        };
        lines.push(Line::from(Span::styled(
            format!("{name} {}", date.format("%d/%m")),
            header_style,
        )));
        let items = app.agenda_items(date);
        if items.is_empty() {
            lines.push(Line::from(Span::styled(
                "   —",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for (proj, t) in items {
                let mark = if t.done { "[x] " } else { "[ ] " };
                let mut style = if t.done {
                    Style::default().fg(Color::Green)
                } else if date < today {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default()
                };
                if t.done {
                    style = style.add_modifier(Modifier::CROSSED_OUT);
                }
                lines.push(Line::from(vec![
                    Span::raw("   "),
                    Span::styled(mark, style),
                    Span::styled(t.title.clone(), style),
                    Span::styled(format!("  ({proj})"), Style::default().fg(Color::DarkGray)),
                ]));
            }
        }
    }

    let title = format!(
        " semana · {} – {} ",
        monday.format("%d/%m"),
        sunday.format("%d/%m")
    );
    let p = Paragraph::new(Text::from(lines))
        .block(overlay_block(app.theme(), title))
        .wrap(Wrap { trim: false });
    frame.render_widget(p, popup);
}

/// Overlay: estadísticas y racha de hábitos.
fn draw_stats(frame: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect_pct(60, 66, area);
    frame.render_widget(Clear, popup);

    let today = Local::now().date_naive();

    // Totales de tareas.
    let mut total = 0usize;
    let mut done = 0usize;
    for p in &app.store.projects {
        total += p.todos.len();
        done += p.done_count();
    }
    let pending = total.saturating_sub(done);

    // Pomodoros de hoy y de la semana.
    let pomo_today = app.store.pomodoros_on(today);
    let monday = monday_of(today);
    let pomo_week: usize = (0..7)
        .map(|i| app.store.pomodoros_on(monday + chrono::Duration::days(i)))
        .sum();

    // Racha: días consecutivos hasta hoy con alguna tarea completada o pomodoro.
    let streak = habit_streak(app, today);

    let mut lines: Vec<Line> = Vec::new();
    let stat = |label: &str, value: String| {
        Line::from(vec![
            Span::styled(format!("  {label:<26}"), Style::default().fg(Color::Gray)),
            Span::styled(value, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ])
    };
    lines.push(stat("Tareas completadas", format!("{done}/{total}")));
    lines.push(stat("Tareas pendientes", pending.to_string()));
    lines.push(stat("Pomodoros hoy", pomo_today.to_string()));
    lines.push(stat("Pomodoros esta semana", pomo_week.to_string()));
    lines.push(stat("Racha de hábitos", format!("{streak} días 🔥")));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Pomodoros (últimos 7 días)",
        Style::default().fg(Color::Gray),
    )));

    // Mini-gráfico ASCII de barras de los últimos 7 días.
    let weekday_short = ["lu", "ma", "mi", "ju", "vi", "sá", "do"];
    for i in (0..7).rev() {
        let d = today - chrono::Duration::days(i);
        let n = app.store.pomodoros_on(d);
        let bar = "█".repeat(n.min(20));
        let wd = weekday_short[d.weekday().num_days_from_monday() as usize];
        lines.push(Line::from(vec![
            Span::styled(format!("  {wd} {}  ", d.format("%d/%m")), Style::default().fg(Color::DarkGray)),
            Span::styled(bar, Style::default().fg(Color::Green)),
            Span::styled(format!(" {n}"), Style::default().fg(Color::DarkGray)),
        ]));
    }

    let p = Paragraph::new(Text::from(lines)).block(overlay_block(app.theme(), " estadísticas ".into()));
    frame.render_widget(p, popup);
}

/// Días consecutivos (terminando hoy) con al menos una tarea completada o un pomodoro.
fn habit_streak(app: &App, today: NaiveDate) -> i64 {
    let active = |d: NaiveDate| -> bool {
        if app.store.pomodoros_on(d) > 0 {
            return true;
        }
        app.store
            .projects
            .iter()
            .flat_map(|p| p.todos.iter())
            .any(|t| t.completed_at == Some(d))
    };
    let mut streak = 0;
    let mut d = today;
    // Permite que la racha "siga viva" aunque hoy aún no haya actividad:
    // si hoy no hay nada, empezamos a contar desde ayer.
    if !active(d) {
        d -= chrono::Duration::days(1);
    }
    while active(d) {
        streak += 1;
        d -= chrono::Duration::days(1);
    }
    streak
}

/// Overlay: papelera con elementos borrados.
fn draw_trash(frame: &mut Frame, app: &App, area: Rect, sel: usize) {
    let popup = centered_rect_pct(66, 60, area);
    frame.render_widget(Clear, popup);

    let block = overlay_block(app.theme(), format!(" papelera · {} ", app.store.trash.len()));
    if app.store.trash.is_empty() {
        let p = Paragraph::new("Papelera vacía.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(p, popup);
        return;
    }

    let items: Vec<ListItem> = app
        .store
        .trash
        .iter()
        .map(|it| ListItem::new(Line::from(it.label())))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(highlight_style(app.theme(), true))
        .highlight_symbol("▌ ");
    let mut state = ListState::default();
    state.select(Some(sel));
    frame.render_stateful_widget(list, popup, &mut state);
}

/// Overlay: menú de acciones (export/import).
fn draw_menu(frame: &mut Frame, app: &App, area: Rect, sel: usize) {
    let popup = centered_rect_pct(54, 40, area);
    frame.render_widget(Clear, popup);

    let options = [
        "Exportar a Markdown (xietiao-export.md)",
        "Exportar datos a JSON (xietiao-export.json)",
        "Importar datos desde JSON (xietiao-import.json)",
    ];
    let items: Vec<ListItem> = options
        .iter()
        .map(|o| ListItem::new(Line::from(o.to_string())))
        .collect();
    let list = List::new(items)
        .block(overlay_block(app.theme(), " menú · acciones ".into()))
        .highlight_style(highlight_style(app.theme(), true))
        .highlight_symbol("▌ ");
    let mut state = ListState::default();
    state.select(Some(sel));
    frame.render_stateful_widget(list, popup, &mut state);
}

/// Rectángulo centrado de ancho `percent_x`% y alto fijo `height` líneas.
fn centered_rect(percent_x: u16, area: Rect, height: u16) -> Rect {
    let w = area.width * percent_x / 100;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width: w,
        height,
    }
}

/// Rectángulo centrado ocupando `px`% del ancho y `py`% del alto.
fn centered_rect_pct(px: u16, py: u16, area: Rect) -> Rect {
    let w = area.width * px / 100;
    let h = area.height * py / 100;
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

/// Centra `s` en un campo de ancho `w` rellenando con espacios.
fn center_in(s: &str, w: usize) -> String {
    let len = s.chars().count();
    if len >= w {
        return s.to_string();
    }
    let left = (w - len) / 2;
    let right = w - len - left;
    format!("{}{}{}", " ".repeat(left), s, " ".repeat(right))
}

// --- Utilidades de fecha ------------------------------------------------------

fn days_in_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_next = NaiveDate::from_ymd_opt(ny, nm, 1).unwrap();
    let first_this = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    (first_next - first_this).num_days() as u32
}

fn month_name_es(month: u32) -> &'static str {
    match month {
        1 => "Enero",
        2 => "Febrero",
        3 => "Marzo",
        4 => "Abril",
        5 => "Mayo",
        6 => "Junio",
        7 => "Julio",
        8 => "Agosto",
        9 => "Septiembre",
        10 => "Octubre",
        11 => "Noviembre",
        _ => "Diciembre",
    }
}
