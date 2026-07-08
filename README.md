# Xietiao

**Xietiao** es un dashboard de productividad en la terminal (TUI), escrito en Rust con
[ratatui](https://ratatui.rs). Reúne en un solo pantallazo tus proyectos, to-dos,
notas, un calendario y temporizadores (pomodoro/cronómetro/reloj).

![Interfaz de Xietiao](Interfaz.png)

## Características

- **Proyectos y to-dos** con prioridad, fecha, reordenación y búsqueda/filtro.
- **Subtareas** (checklist) dentro de cada to-do, tabuladas bajo su tarea y
  con barra de progreso.
- **Etiquetas** `#tag`: las escribes en el título y filtras por ellas.
- **Recurrencia** diaria/semanal/mensual: al completar la tarea se regenera.
- **Calendario** con carga por día (puntos `•` contando todos los proyectos),
  navegación por meses, agenda por día (con salto directo a la tarea) y
  **agenda semanal**, más resaltado de tareas vencidas (rojo) y de hoy (ámbar).
- **Fechas** asignables desde el calendario (`f`) o escritas a mano (`D`),
  con quitado rápido (deja el campo vacío).
- **Notas** por proyecto y notas generales.
- **Pomodoro** foco(25)/break(5), cronómetro y reloj. Registra los focos
  completados, puede **vincularse a un to-do** y avisa al terminar.
- **Vista de pendientes** cruzando todos los proyectos.
- **Estadísticas** con racha de hábitos y mini-gráfico de los últimos 7 días.
- **Deshacer** (`u`) y **papelera** (borrado recuperable).
- **Export/Import** a Markdown y JSON.
- **Sincronización con Todoist** (`y`, o desde el menú `o`): las tareas
  pendientes se envían una sola vez —con su proyecto, fecha, prioridad y
  `#tags`— y las que completes en Todoist se marcan como hechas aquí.
- **Configurable**: tema de colores y atajos de teclado vía `config.toml`.

## Instalación

Necesitas [Rust](https://rustup.rs) (incluye `cargo`).

```bash
git clone https://github.com/Shikillo/Xietiao.git
cd Xietiao
cargo run --release
```

Para instalarlo como comando del sistema (`xietiao` desde cualquier sitio):

```bash
cargo install --path .
```

## Atajos principales

| Tecla | Acción |
|-------|--------|
| `Tab` / `Shift+Tab` | Cambiar de panel |
| `↑ ↓` / `j k` | Navegar lista (± semana en calendario) |
| `← →` / `h l` | Mover día en el calendario |
| `a` / `n` | Añadir proyecto o tarea (`#tags` en tareas) |
| `e` | Renombrar / editar notas |
| `d` | Borrar (a la papelera, con confirmación) |
| `u` | Deshacer |
| `Espacio` / `Enter` | Marcar tarea · play relojes · editar notas |
| `f` | Asignar la tarea al día del cursor |
| `D` | Escribir la fecha de la tarea (vacío: quitarla) |
| `p` / `R` | Prioridad · recurrencia |
| `s` | Editar subtareas |
| `m` | Mover la tarea a otro proyecto |
| `v` | Vincular pomodoro a la tarea |
| `/` | Buscar / filtrar (admite `#tag`) |
| `g` | Notas del proyecto ↔ generales |
| `t` / `w` | Agenda de hoy · de la semana |
| `[` `]` / `T` | Mes anterior/siguiente · hoy (calendario) |
| `P` | Todas las tareas pendientes |
| `S` | Estadísticas y racha |
| `x` / `o` | Papelera · menú (export/import/Todoist) |
| `y` | Sincronizar con Todoist |
| `r` / `b` | Reset · foco↔break (relojes) |
| `?` | Ayuda |
| `q` | Salir |

Pulsa `?` dentro de la app para ver la ayuda completa.

## Datos y configuración

Xietiao guarda su estado en `<config_dir>/xietiao/store.json`:

- **macOS:** `~/Library/Application Support/xietiao/`
- **Linux:** `~/.config/xietiao/`
- **Windows:** `%APPDATA%\xietiao\`

Para mover tus to-dos entre equipos, usa el menú (`o`) → exportar/importar JSON.

### Todoist

En el menú (`o`) → «Conectar Todoist» pega tu token de API (en Todoist:
Configuración → Integraciones → Desarrollador). Después, «Sincronizar con
Todoist» (o la tecla `y`) envía cada tarea pendiente una sola vez a un proyecto
remoto homónimo del local (se crea si no existe) y marca como hechas aquí las
que completes allí. El token y las ids remotas se guardan en `store.json`, el
mismo que usa la versión de escritorio, así que puedes sincronizar desde
cualquiera de las dos.

### Personalización

Copia [`config.example.toml`](config.example.toml) a `<config_dir>/xietiao/config.toml`
para cambiar el **tema de colores** y **reasignar teclas**. Todo es opcional; lo que
no definas usa los valores por defecto.

## Desarrollo

```bash
cargo build      # compilar
cargo test       # tests
cargo run        # ejecutar (modo debug)
```

Estructura:

- `src/main.rs` — event loop (tick de 250 ms).
- `src/app.rs` — estado, foco, modos de entrada, overlays y acciones.
- `src/model.rs` — datos (Project/Todo/Subtask…) y persistencia JSON.
- `src/todoist.rs` — sincronización con Todoist (API v1, en hilo aparte).
- `src/ui.rs` — renderizado por panel y overlays.
- `src/config.rs` — tema de colores y keymap (`config.toml`).

---

Hecho con 🦀 Rust + ratatui.

## Créditos

Desarrollado por [Shikillo](https://github.com/Shikillo) con la asistencia de Claude (Anthropic).
