use crate::settings::{Settings, data_dir, engine_binary};
use nexode_sdk::{
    ApiEvent, ConnectOptions, HistoryMessage, LaunchOptions, LaunchedInstance, NexodeClient,
    PermissionDecision, RunOptions, SessionInfo, launch_instance,
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    time::Duration,
};

pub enum Command {
    Connect(Settings),
    Open(String),
    Send {
        session: Option<String>,
        path: PathBuf,
        content: String,
    },
    Cancel(String),
    Models(String),
    Model(String, String),
    Changes(String, PathBuf),
    Permission(String, String, PermissionDecision),
    PickFolder,
    PickBinary,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Operation {
    Connect,
    Open,
    Turn,
    Models,
    Changes,
    Other,
}
pub enum Event {
    Connected(Vec<SessionInfo>),
    Opened(SessionInfo, Vec<HistoryMessage>),
    Created(SessionInfo),
    Models(String, Vec<String>, Option<String>),
    Stream(String, ApiEvent),
    Finished(String, Vec<HistoryMessage>, Vec<SessionInfo>),
    Changes(String, GitChanges),
    Folder(PathBuf),
    Binary(PathBuf),
    Error(Operation, String),
}
#[derive(Clone, Default)]
pub struct GitChanges {
    pub branch: String,
    pub status: String,
    pub diff: String,
}

#[derive(Clone)]
pub struct Backend {
    client: Arc<Mutex<Option<NexodeClient>>>,
    instance: Arc<Mutex<Option<LaunchedInstance>>>,
    closing: Arc<AtomicBool>,
    launches: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    sender: SyncSender<Event>,
    repaint: Arc<dyn Fn() + Send + Sync>,
}
impl Backend {
    pub fn new(repaint: impl Fn() + Send + Sync + 'static) -> (Self, Receiver<Event>) {
        let (sender, receiver) = sync_channel(512);
        (
            Self {
                client: Default::default(),
                instance: Default::default(),
                closing: Default::default(),
                launches: Default::default(),
                sender,
                repaint: Arc::new(repaint),
            },
            receiver,
        )
    }
    fn emit(&self, event: Event) {
        if self.closing.load(Ordering::Acquire) {
            return;
        }
        if self.sender.send(event).is_ok() {
            (self.repaint)();
        }
    }
    fn client(&self) -> Result<NexodeClient, String> {
        self.client
            .lock()
            .map_err(|e| e.to_string())?
            .clone()
            .ok_or("Conecta el motor de Orbit desde Configuración".into())
    }
    pub fn send(&self, command: Command) {
        if self.closing.load(Ordering::Acquire) {
            return;
        }
        let launching = matches!(&command, Command::Connect(_));
        let worker = self.clone();
        let job = std::thread::spawn(move || {
            let op = match &command {
                Command::Connect(_) => Operation::Connect,
                Command::Open(_) => Operation::Open,
                Command::Send { .. } => Operation::Turn,
                Command::Models(_) | Command::Model(..) => Operation::Models,
                Command::Changes(..) => Operation::Changes,
                _ => Operation::Other,
            };
            if let Err(message) = worker.execute(command) {
                worker.emit(Event::Error(op, message));
            }
        });
        if launching {
            self.launches.lock().unwrap().push(job);
        }
    }
    fn execute(&self, command: Command) -> Result<(), String> {
        match command {
            Command::PickFolder => {
                if let Some(p) = rfd::FileDialog::new()
                    .set_title("Abrir proyecto")
                    .pick_folder()
                {
                    self.emit(Event::Folder(p));
                }
                return Ok(());
            }
            Command::PickBinary => {
                if let Some(p) = rfd::FileDialog::new()
                    .set_title("Seleccionar ejecutable de Orbit")
                    .pick_file()
                {
                    self.emit(Event::Binary(p));
                }
                return Ok(());
            }
            Command::Connect(settings) => {
                // Drop an old private instance on a worker, never on the render thread.
                self.client.lock().map_err(|e| e.to_string())?.take();
                self.instance.lock().map_err(|e| e.to_string())?.take();
                let instance = launch_instance(&LaunchOptions {
                    nexode_home: Some(data_dir().join("engine")),
                    binary: Some(engine_binary(&settings)),
                    request_timeout: Some(Duration::from_secs(120)),
                    client_name: "nexode-desktop-native/0.2".into(),
                    ..Default::default()
                })
                .map_err(|e| e.to_string())?;
                let c = NexodeClient::connect(ConnectOptions {
                    socket_path: Some(instance.socket_path.clone()),
                    ensure_runtime: false,
                    request_timeout: Some(Duration::from_secs(120)),
                    client_name: "nexode-desktop-native/0.2".into(),
                    ..Default::default()
                })
                .map_err(|e| e.to_string())?;
                let sessions = c.list_sessions().map_err(|e| e.to_string())?;
                if self.closing.load(Ordering::Acquire) {
                    return Ok(());
                }
                *self.instance.lock().map_err(|e| e.to_string())? = Some(instance);
                *self.client.lock().map_err(|e| e.to_string())? = Some(c);
                self.emit(Event::Connected(sessions));
                return Ok(());
            }
            _ => {}
        }
        let c = self.client()?;
        match command {
            Command::Open(id) => {
                let s = c.attach_session(&id).map_err(|e| e.to_string())?;
                let history = c.get_history(&id).map_err(|e| e.to_string())?;
                self.emit(Event::Opened(s, history));
                self.publish_models(&c, &id)?;
            }
            Command::Send {
                session,
                path,
                content,
            } => {
                let id = match session {
                    Some(id) => id,
                    None => {
                        let path = std::fs::canonicalize(path)
                            .map_err(|e| format!("No se puede abrir el proyecto: {e}"))?;
                        let s = c
                            .create_session(Some(path.to_string_lossy().into()))
                            .map_err(|e| e.to_string())?;
                        let id = s.session_id.clone();
                        self.emit(Event::Created(s));
                        id
                    }
                };
                let worker = self.clone();
                let event_id = id.clone();
                let result = c.run(
                    &id,
                    &content,
                    RunOptions {
                        auto_approve: false,
                        on_event: Some(Box::new(move |event| {
                            worker.emit(Event::Stream(event_id.clone(), event.clone()))
                        })),
                        ..Default::default()
                    },
                );
                // Restore durable history even when a provider fails or a turn is cancelled.
                self.emit(Event::Finished(
                    id.clone(),
                    c.get_history(&id).map_err(|e| e.to_string())?,
                    c.list_sessions().map_err(|e| e.to_string())?,
                ));
                result.map_err(|e| e.to_string())?;
                self.publish_models(&c, &id)?;
            }
            Command::Cancel(id) => c.cancel(&id).map_err(|e| e.to_string())?,
            Command::Permission(id, request, decision) => c
                .respond_to_permission(&id, &request, decision)
                .map_err(|e| e.to_string())?,
            Command::Models(id) => {
                c.runtime_command(&id, nexode_sdk::RuntimeCommand::RefreshModelList, vec![])
                    .map_err(|e| e.to_string())?;
                self.publish_models(&c, &id)?;
            }
            Command::Model(id, model) => {
                c.set_model(&id, &model).map_err(|e| e.to_string())?;
                self.publish_models(&c, &id)?;
            }
            Command::Changes(id, path) => self.emit(Event::Changes(id, git_changes(&path)?)),
            _ => unreachable!(),
        }
        Ok(())
    }
    fn publish_models(&self, c: &NexodeClient, id: &str) -> Result<(), String> {
        let (models, current) = c.list_models(id).map_err(|e| e.to_string())?;
        self.emit(Event::Models(id.into(), models, current));
        Ok(())
    }
    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::Release);
        // A close during startup must also reap the instance being launched.
        let launches = std::mem::take(&mut *self.launches.lock().unwrap());
        for job in launches {
            let _ = job.join();
        }
        // The window owns the process lifetime, independently of streaming clients.
        self.instance
            .lock()
            .ok()
            .and_then(|mut instance| instance.take());
        self.client.lock().ok().and_then(|mut client| client.take());
    }
}
pub fn git_changes(path: &std::path::Path) -> Result<GitChanges, String> {
    let git = |args: &[&str]| -> Result<String, String> {
        let mut command = std::process::Command::new("git");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let o = command
            .arg("-C")
            .arg(path)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .map_err(|e| e.to_string())?;
        if !o.status.success() {
            return Err(String::from_utf8_lossy(&o.stderr).into());
        }
        Ok(String::from_utf8_lossy(&o.stdout).into())
    };
    let diff = if git(&["rev-parse", "--verify", "HEAD"]).is_ok() {
        git(&["diff", "--no-ext-diff", "--no-textconv", "HEAD", "--"])?
    } else {
        git(&["diff", "--no-ext-diff", "--no-textconv", "--cached", "--"])?
    };
    Ok(GitChanges {
        branch: git(&["branch", "--show-current"])?,
        status: git(&["status", "--short"])?,
        diff,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn git_review_handles_new_repositories_and_untracked_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .arg(dir.path())
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(dir.path().join("hello.rs"), "fn main() {}\n").unwrap();
        let changes = git_changes(dir.path()).unwrap();
        assert!(changes.status.contains("?? hello.rs"));
        assert!(changes.diff.is_empty());
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(["add", "hello.rs"])
                .status()
                .unwrap()
                .success()
        );
        assert!(
            git_changes(dir.path())
                .unwrap()
                .diff
                .contains("+fn main() {}")
        );
    }
    #[test]
    fn git_review_reports_invalid_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(git_changes(dir.path()).is_err());
    }
    #[test]
    #[ignore = "requires the isolated native fake OpenAI provider; never run against a paid provider"]
    fn native_engine_stream_history_cancel_and_shutdown() {
        #[cfg(windows)]
        crate::windows_job::contain_child_processes().unwrap();
        assert_eq!(std::env::var("ORBIT_NATIVE_TEST").as_deref(), Ok("1"));
        let (backend, events) = Backend::new(|| {});
        backend.send(Command::Connect(Settings::default()));
        match events.recv_timeout(Duration::from_secs(120)).unwrap() {
            Event::Connected(_) => {}
            Event::Error(_, e) => panic!("{e}"),
            _ => panic!("expected connected"),
        };
        let dir = tempfile::tempdir().unwrap();
        backend.send(Command::Send {
            session: None,
            path: dir.path().into(),
            content: "native desktop integration test".into(),
        });
        let mut id = String::new();
        let mut streamed = false;
        loop {
            match events.recv_timeout(Duration::from_secs(120)).unwrap() {
                Event::Created(s) => id = s.session_id,
                Event::Stream(_, ApiEvent::TextDelta { text, .. }) => {
                    streamed |= text.contains("[native] ack.");
                }
                Event::Finished(_, history, _) => {
                    assert!(streamed);
                    assert!(
                        history
                            .iter()
                            .any(|m| m.role == "assistant" && m.content.contains("[native] ack."))
                    );
                    break;
                }
                Event::Error(_, e) => panic!("{e}"),
                _ => {}
            }
        }
        backend.send(Command::Send {
            session: Some(id.clone()),
            path: dir.path().into(),
            content: "[e2e:hold:native-cancel]".into(),
        });
        let control = PathBuf::from(std::env::var_os("FAKE_CONTROL_DIR").expect("control dir"));
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        while !control.join("native-cancel.held").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "provider gate never reached"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        backend.send(Command::Cancel(id.clone()));
        loop {
            match events.recv_timeout(Duration::from_secs(30)).unwrap() {
                Event::Finished(turn, _, _) if turn == id => break,
                Event::Error(_, e) => panic!("{e}"),
                _ => {}
            }
        }
        backend.shutdown();
        // socket_accepts only checks the file type; a stale Unix socket is not
        // a live process. Probe the actual API without auto-starting anything.
        assert!(
            NexodeClient::connect(ConnectOptions {
                socket_path: Some(data_dir().join("engine/run/nexode-api.sock")),
                ensure_runtime: false,
                request_timeout: Some(Duration::from_secs(1)),
                ..Default::default()
            })
            .is_err()
        );
        std::fs::write(control.join("native-cancel.release"), "").unwrap();
    }
}
