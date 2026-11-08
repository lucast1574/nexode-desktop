use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use nexode_sdk::{LaunchOptions, NexodeClient, RunOptions};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio_stream::wrappers::ReceiverStream;

type App = Arc<Mutex<Option<NexodeClient>>>;
type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn error(e: impl ToString) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error":e.to_string()})),
    )
}
fn client(state: &App) -> Result<NexodeClient, String> {
    let mut slot = state.lock().map_err(|e| e.to_string())?;
    if let Some(client) = &*slot {
        return Ok(client.clone());
    }
    let home = std::env::var_os("ORBIT_DESKTOP_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".nexode-desktop")
        });
    let instance = NexodeClient::launch(LaunchOptions {
        nexode_home: Some(home),
        binary: std::env::var_os("NEXODE_BINARY").map(PathBuf::from),
        client_name: "nexode-desktop/0.1.0".into(),
        // Cold provider discovery can exceed the SDK's 30-second default.
        request_timeout: Some(std::time::Duration::from_secs(120)),
        ..Default::default()
    })
    .map_err(|e| e.to_string())?;
    *slot = Some(instance.clone());
    Ok(instance)
}
async fn run<F>(state: App, f: F) -> ApiResult
where
    F: FnOnce(NexodeClient) -> Result<Value, String> + Send + 'static,
{
    tokio::task::spawn_blocking(move || f(client(&state)?))
        .await
        .map_err(error)?
        .map(Json)
        .map_err(error)
}
// Every API request needs a non-simple header. No CORS is enabled. Validate Host
// as well, so a hostile website cannot control the local agent through rebinding.
async fn local_only(req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let allowed = [
        "127.0.0.1:4318",
        "localhost:4318",
        "127.0.0.1:1420",
        "localhost:1420",
    ];
    let origin_ok = req
        .headers()
        .get("origin")
        .map(|v| {
            v.to_str()
                .ok()
                .is_some_and(|s| allowed.iter().any(|h| s == format!("http://{h}")))
        })
        .unwrap_or(true);
    if !allowed.contains(&host)
        || !origin_ok
        || req
            .headers()
            .get("x-orbit-client")
            .and_then(|v| v.to_str().ok())
            != Some("desktop")
    {
        return (
            StatusCode::FORBIDDEN,
            "Only the local Orbit interface may use this API",
        )
            .into_response();
    }
    next.run(req).await
}
async fn status(State(s): State<App>) -> ApiResult {
    run(s, |c| {
        c.ping().map_err(|e| e.to_string())?;
        Ok(json!({"connected":true,"name":"Nexode Orbit","isolated":true}))
    })
    .await
}
async fn sessions(State(s): State<App>) -> ApiResult {
    run(s, |c| {
        serde_json::to_value(c.list_sessions().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    })
    .await
}
#[derive(Deserialize)]
struct Create {
    path: String,
}
async fn create(State(s): State<App>, Json(body): Json<Create>) -> ApiResult {
    run(s, move |c| {
        let dir = std::fs::canonicalize(&body.path)
            .map_err(|e| format!("No se puede abrir la carpeta: {e}"))?;
        if !dir.is_dir() {
            return Err("Selecciona una carpeta de proyecto".into());
        }
        serde_json::to_value(
            c.create_session(Some(dir.to_string_lossy().into()))
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
    })
    .await
}
async fn history(State(s): State<App>, Path(id): Path<String>) -> ApiResult {
    run(s, move |c| {
        c.attach_session(&id).map_err(|e| e.to_string())?;
        serde_json::to_value(c.get_history(&id).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    })
    .await
}
async fn models(State(s): State<App>, Path(id): Path<String>) -> ApiResult {
    run(s, move |c| {
        let (models, current) = c.list_models(&id).map_err(|e| e.to_string())?;
        Ok(json!({"models":models,"current":current}))
    })
    .await
}
#[derive(Deserialize)]
struct Model {
    model: String,
}
async fn model(State(s): State<App>, Path(id): Path<String>, Json(b): Json<Model>) -> ApiResult {
    run(s, move |c| {
        c.set_model(&id, &b.model)
            .map(|_| json!({"ok":true}))
            .map_err(|e| e.to_string())
    })
    .await
}
async fn cancel(State(s): State<App>, Path(id): Path<String>) -> ApiResult {
    run(s, move |c| {
        c.cancel(&id)
            .map(|_| json!({"ok":true}))
            .map_err(|e| e.to_string())
    })
    .await
}
#[derive(Deserialize)]
struct Permission {
    request_id: String,
    decision: nexode_sdk::PermissionDecision,
}
async fn permission(
    State(s): State<App>,
    Path(id): Path<String>,
    Json(b): Json<Permission>,
) -> ApiResult {
    run(s, move |c| {
        c.respond_to_permission(&id, &b.request_id, b.decision)
            .map(|_| json!({"ok":true}))
            .map_err(|e| e.to_string())
    })
    .await
}
#[derive(Deserialize)]
struct Prompt {
    content: String,
}
async fn turn(State(s): State<App>, Path(id): Path<String>, Json(b): Json<Prompt>) -> Response {
    if b.content.trim().is_empty() {
        return error("Escribe una instrucción").into_response();
    }
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, Infallible>>(128);
    tokio::task::spawn_blocking(move || {
        let result = client(&s).and_then(|c| {
            let events = tx.clone();
            c.run(
                &id,
                &b.content,
                RunOptions {
                    on_event: Some(Box::new(move |event| {
                        if let Ok(line) = serde_json::to_string(event) {
                            let _ = events.blocking_send(Ok(format!("{line}\n")));
                        }
                    })),
                    auto_approve: false,
                    ..Default::default()
                },
            )
            .map(|r| json!({"ev":"desktop_done","text":r.final_text,"stop_reason":r.stop_reason}))
            .map_err(|e| e.to_string())
        });
        let final_event = result.unwrap_or_else(|e| json!({"ev":"desktop_error","message":e}));
        let _ = tx.blocking_send(Ok(format!("{final_event}\n")));
    });
    (
        [
            ("content-type", "application/x-ndjson"),
            ("cache-control", "no-store"),
        ],
        Body::from_stream(ReceiverStream::new(rx)),
    )
        .into_response()
}
async fn changes(State(s): State<App>, Path(id): Path<String>) -> ApiResult {
    run(s, move |c| {
        let session = c.list_sessions().map_err(|e|e.to_string())?.into_iter().find(|x| x.session_id == id).ok_or("Sesión no encontrada")?;
        let dir = session.working_dir.ok_or("Sesión sin carpeta")?;
        let git = |args: &[&str]| -> Result<String,String> {
            let output = std::process::Command::new("git").arg("-C").arg(&dir).args(args).env("GIT_OPTIONAL_LOCKS","0").output().map_err(|e|e.to_string())?;
            if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).into()); }
            Ok(String::from_utf8_lossy(&output.stdout).into())
        };
        Ok(json!({"status":git(&["status","--short"])?,"diff":git(&["diff","--no-ext-diff","--no-textconv","HEAD","--"])?,"branch":git(&["branch","--show-current"])?}))
    }).await
}
async fn projects() -> ApiResult {
    let root = std::env::var_os("ORBIT_PROJECTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap()
                .parent()
                .unwrap()
                .to_owned()
        });
    let mut projects = vec![];
    for entry in std::fs::read_dir(root).map_err(error)?.flatten() {
        if entry.path().join(".git").exists() {
            projects.push(json!({"name":entry.file_name().to_string_lossy(),"path":entry.path().to_string_lossy()}));
        }
    }
    projects.sort_by_key(|p| p["name"].as_str().unwrap_or("").to_owned());
    Ok(Json(json!(projects)))
}
#[tokio::main]
async fn main() {
    let state: App = Arc::new(Mutex::new(None));
    let api = Router::new()
        .route("/status", get(status))
        .route("/projects", get(projects))
        .route("/sessions", get(sessions).post(create))
        .route("/sessions/{id}/history", get(history))
        .route("/sessions/{id}/models", get(models).post(model))
        .route("/sessions/{id}/turn", post(turn))
        .route("/sessions/{id}/cancel", post(cancel))
        .route("/sessions/{id}/permission", post(permission))
        .route("/sessions/{id}/changes", get(changes))
        .layer(middleware::from_fn(local_only))
        .with_state(state.clone());
    let app = Router::new()
        .nest("/api", api)
        .fallback_service(tower_http::services::ServeDir::new("dist"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:4318")
        .await
        .expect("El puerto 4318 está ocupado");
    println!("Nexode Orbit UI: http://localhost:4318");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap();
    // The last SDK handle stops only the private runtime owned by this UI.
    state.lock().unwrap().take();
}
