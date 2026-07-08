//! Integración con Todoist (API unificada v1): sincronización de tareas.
//!
//! Ida (Xietiao → Todoist): cada tarea pendiente se crea una única vez en
//! Todoist (su id remota se recuerda en `Todo::todoist_id`), dentro de un
//! proyecto remoto homónimo del local (se crea si no existe).
//!
//! Vuelta (Todoist → Xietiao): las tareas ya exportadas que aparezcan
//! completadas en Todoist se marcan como hechas también aquí.
//!
//! (Portado de la versión de escritorio; aquí las peticiones son síncronas
//! con `ureq` y el llamador las ejecuta en un hilo aparte para no congelar
//! la interfaz.)

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::json;

use crate::model::Priority;

const API: &str = "https://api.todoist.com/api/v1";

/// Proyecto tal como lo devuelve la API (sólo los campos que usamos).
#[derive(Deserialize)]
struct RemoteProject {
    id: String,
    name: String,
}

/// Tarea tal como la devuelve la API (sólo los campos que usamos).
#[derive(Deserialize)]
struct RemoteTask {
    id: String,
}

/// Estado de una tarea remota ya exportada. La API v1 sigue devolviendo por id
/// las tareas completadas (`checked`) e incluso las borradas (`is_deleted`).
#[derive(Deserialize)]
struct RemoteTaskState {
    checked: bool,
    is_deleted: bool,
}

/// Página de resultados: los listados de la API v1 vienen paginados.
#[derive(Deserialize)]
struct Page<T> {
    results: Vec<T>,
    next_cursor: Option<String>,
}

/// Tarea local lista para exportar, con su posición dentro del `Store`.
pub struct Outgoing {
    pub project: usize,
    pub todo: usize,
    pub project_name: String,
    pub content: String,
    pub due_date: Option<String>,
    pub priority: u8,
    pub labels: Vec<String>,
}

/// Prioridad local → prioridad Todoist (1 normal … 4 urgente).
pub fn priority(p: Priority) -> u8 {
    match p {
        Priority::None => 1,
        Priority::Low => 2,
        Priority::Medium => 3,
        Priority::High => 4,
    }
}

/// Mensaje legible para un error de red/API.
fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(401 | 403, _) => "el token no es válido".into(),
        ureq::Error::Status(code, _) => format!("error HTTP {code}"),
        ureq::Error::Transport(t) => t.to_string(),
    }
}

fn fetch_projects(auth: &str) -> Result<HashMap<String, String>, String> {
    let mut projects = HashMap::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut req = ureq::get(&format!("{API}/projects")).set("Authorization", auth);
        if let Some(c) = &cursor {
            req = req.query("cursor", c);
        }
        let page: Page<RemoteProject> = req
            .call()
            .map_err(describe)?
            .into_json()
            .map_err(|e| e.to_string())?;
        projects.extend(page.results.into_iter().map(|p| (p.name, p.id)));
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => return Ok(projects),
        }
    }
}

fn post_json<T: serde::de::DeserializeOwned>(
    auth: &str,
    path: &str,
    body: serde_json::Value,
) -> Result<T, String> {
    ureq::post(&format!("{API}/{path}"))
        .set("Authorization", auth)
        .send_json(body)
        .map_err(describe)?
        .into_json()
        .map_err(|e| e.to_string())
}

/// Consulta en Todoist las tareas de `ids` y devuelve las que están
/// completadas (no borradas). Si algo falló a medias, el mensaje de error;
/// las ids ya comprobadas cuentan igualmente.
pub fn fetch_completed(token: &str, ids: &[String]) -> (Vec<String>, Option<String>) {
    let auth = format!("Bearer {token}");
    let mut completed = Vec::new();
    for id in ids {
        let response = match ureq::get(&format!("{API}/tasks/{id}"))
            .set("Authorization", &auth)
            .call()
        {
            Ok(r) => r,
            // Una id que ya no existe (purgada) no es un error: se ignora.
            Err(ureq::Error::Status(404, _)) => continue,
            Err(e) => return (completed, Some(describe(e))),
        };
        match response.into_json::<RemoteTaskState>() {
            Ok(s) if s.checked && !s.is_deleted => completed.push(id.clone()),
            Ok(_) => {}
            Err(e) => return (completed, Some(e.to_string())),
        }
    }
    (completed, None)
}

/// Exporta `outgoing` a Todoist en orden. Devuelve las tareas creadas como
/// `(project, todo, id_remota)` y, si algo falló a medias, el mensaje de
/// error; así el llamador registra lo ya creado y no lo duplica al reintentar.
pub fn export(token: &str, outgoing: &[Outgoing]) -> (Vec<(usize, usize, String)>, Option<String>) {
    let auth = format!("Bearer {token}");

    // Proyectos remotos existentes, por nombre.
    let mut project_ids = match fetch_projects(&auth) {
        Ok(map) => map,
        Err(e) => return (Vec::new(), Some(e)),
    };

    let mut created = Vec::new();
    for task in outgoing {
        // Asegura el proyecto remoto homónimo.
        if !project_ids.contains_key(&task.project_name) {
            match post_json::<RemoteProject>(&auth, "projects", json!({ "name": task.project_name }))
            {
                Ok(p) => {
                    project_ids.insert(p.name, p.id);
                }
                Err(e) => return (created, Some(e)),
            }
        }
        let mut body = json!({
            "content": task.content,
            "project_id": project_ids[&task.project_name],
            "priority": task.priority,
        });
        if let Some(d) = &task.due_date {
            body["due_date"] = json!(d);
        }
        if !task.labels.is_empty() {
            body["labels"] = json!(task.labels);
        }
        match post_json::<RemoteTask>(&auth, "tasks", body) {
            Ok(t) => created.push((task.project, task.todo, t.id)),
            Err(e) => return (created, Some(e)),
        }
    }
    (created, None)
}
