import { useEffect, useRef, useState } from "react";
import * as Menu from "@radix-ui/react-dropdown-menu";
import * as Dialog from "@radix-ui/react-dialog";
import {
  ArrowUp,
  Check,
  ChevronDown,
  ChevronRight,
  Code2,
  Folder,
  FolderPlus,
  GitBranch,
  GitCompareArrows,
  LoaderCircle,
  MessageSquare,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  RefreshCw,
  Search,
  Settings2,
  ShieldCheck,
  Square,
  Terminal,
  X,
} from "lucide-react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  api,
  streamTurn,
  type Session,
  type Project,
  type Message,
} from "./api";

const basename = (path = "") =>
  path.split(/[\\/]/).filter(Boolean).pop() || "Proyecto";
function Picker({
  label,
  children,
}: {
  label: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <Menu.Root>
      <Menu.Trigger className="picker">
        {label}
        <ChevronDown size={13} />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content className="menu" sideOffset={8} align="start">
          {children}
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  );
}
export default function App() {
  const [projects, setProjects] = useState<Project[]>([]),
    [sessions, setSessions] = useState<Session[]>([]);
  const [project, setProject] = useState(""),
    [session, setSession] = useState<Session | null>(null),
    [messages, setMessages] = useState<Message[]>([]);
  const [draft, setDraft] = useState(""),
    [busy, setBusy] = useState(false),
    [loading, setLoading] = useState(false),
    [connected, setConnected] = useState(false);
  const [error, setError] = useState(""),
    [activity, setActivity] = useState(""),
    [live, setLive] = useState("");
  const [models, setModels] = useState<string[]>([]),
    [model, setModel] = useState("");
  const [sidebar, setSidebar] = useState(() => window.innerWidth > 700),
    [panel, setPanel] = useState(false),
    [settings, setSettings] = useState(false),
    [search, setSearch] = useState("");
  const [adding, setAdding] = useState(false),
    [folder, setFolder] = useState("");
  const [changes, setChanges] = useState<{
      status: string;
      diff: string;
      branch: string;
    } | null>(null),
    [diffError, setDiffError] = useState("");
  const [permission, setPermission] = useState<{
    request_id: string;
    description: string;
  } | null>(null);
  const end = useRef<HTMLDivElement>(null),
    composer = useRef<HTMLTextAreaElement>(null),
    generation = useRef(0);
  const fail = (e: unknown) =>
    setError(e instanceof Error ? e.message : String(e));
  async function refresh() {
    setError("");
    try {
      await api("/status");
      setConnected(true);
      setSessions(await api<Session[]>("/sessions"));
    } catch (e) {
      setConnected(false);
      fail(e);
    }
  }
  useEffect(() => {
    void refresh();
    api<Project[]>("/projects")
      .then((p) => {
        setProjects(p);
        setProject(
          p.find((x) => x.name === "nexode-orbit")?.path || p[0]?.path || "",
        );
      })
      .catch(fail);
  }, []);
  useEffect(() => {
    end.current?.scrollIntoView({ behavior: "auto" });
  }, [messages, live, activity]);
  async function loadModels(id: string) {
    const m = await api<{ models: string[]; current: string | null }>(
      `/sessions/${id}/models`,
    );
    setModels(m.models);
    setModel(m.current || "");
  }
  async function open(s: Session) {
    if (busy) return;
    const gen = ++generation.current;
    setSession(s);
    setProject(s.working_dir || project);
    setLoading(true);
    setMessages([]);
    setChanges(null);
    setError("");
    setModels([]);
    setModel("");
    try {
      const history = await api<Message[]>(`/sessions/${s.session_id}/history`);
      if (gen === generation.current) {
        setMessages(history);
        await loadModels(s.session_id);
      }
    } catch (e) {
      fail(e);
    } finally {
      if (gen === generation.current) setLoading(false);
    }
  }
  function newChat() {
    if (busy) return;
    ++generation.current;
    setSession(null);
    setMessages([]);
    setLive("");
    setChanges(null);
    setError("");
    setModels([]);
    setModel("");
    setLoading(false);
    composer.current?.focus();
  }
  async function readChanges(id = session?.session_id) {
    if (!id) return;
    setDiffError("");
    try {
      setChanges(await api(`/sessions/${id}/changes`));
    } catch (e) {
      setDiffError(String(e));
    }
  }
  useEffect(() => {
    if (panel && session) void readChanges();
  }, [panel, session?.session_id]);
  async function send() {
    if (!draft.trim() || busy || !project || !connected || loading) return;
    const content = draft.trim();
    setError("");
    setBusy(true);
    setDraft("");
    setLive("");
    setActivity("Preparando el contexto");
    let active = session;
    let completed = false;
    let streamError = "";
    const text = new Map<string, string>();
    try {
      if (!active) {
        active = await api<Session>("/sessions", { path: project });
        setSession(active);
        setSessions((p) => [active!, ...p]);
        await loadModels(active.session_id).catch(fail);
      }
      setMessages((p) => [...p, { role: "user", content }]);
      await streamTurn(active.session_id, content, (event) => {
        const kind = String(event.ev),
          key = String(event.message_id || "legacy");
        if (kind === "text_delta" || kind === "text_replace") {
          text.set(
            key,
            kind === "text_replace"
              ? String(event.text)
              : (text.get(key) || "") + String(event.text),
          );
          setLive([...text.values()].filter(Boolean).join("\n\n"));
          setActivity("Escribiendo");
        }
        if (kind === "tool_start" || kind === "tool_exec")
          setActivity(`Ejecutando ${event.name}`);
        if (kind === "tool_done") setActivity(`Completado: ${event.name}`);
        if (kind === "permission_request")
          setPermission({
            request_id: String(event.request_id),
            description: String(event.description),
          });
        if (kind === "desktop_error") {
          streamError = String(event.message);
          setError(streamError);
        }
        if (kind === "desktop_done") completed = true;
      });
      if (!completed && !streamError)
        throw new Error(
          "Se interrumpió la conexión. Reabre el chat para recuperar el historial.",
        );
      setMessages(
        await api<Message[]>(`/sessions/${active.session_id}/history`),
      );
      setSessions(await api<Session[]>("/sessions"));
      if (panel) await readChanges(active.session_id);
    } catch (e) {
      fail(e);
      if (!active) setDraft(content);
    } finally {
      setBusy(false);
      setActivity("");
      setLive("");
      setPermission(null);
    }
  }
  async function answer(decision: "allow" | "deny") {
    if (!session || !permission) return;
    try {
      await api(`/sessions/${session.session_id}/permission`, {
        request_id: permission.request_id,
        decision,
      });
      setPermission(null);
    } catch (e) {
      fail(e);
    }
  }
  const visibleSessions = sessions.filter(
    (s) =>
      !search ||
      `${s.title} ${s.working_dir}`
        .toLowerCase()
        .includes(search.toLowerCase()),
  );
  return (
    <div
      className={`app ${sidebar ? "" : "without-sidebar"} ${panel ? "with-panel" : ""}`}
    >
      {sidebar && (
        <aside className="sidebar">
          <div className="brand">
            <img src="/nexode.svg" alt="" />
            <span>
              nexode<span className="brand-orbit">orbit</span>
            </span>
            <button
              className="icon"
              aria-label="Ocultar barra lateral"
              onClick={() => setSidebar(false)}
            >
              <PanelLeftClose size={17} />
            </button>
          </div>
          <button className="new-chat" onClick={newChat} disabled={busy}>
            <Plus size={17} />
            Nuevo chat
          </button>
          <label className="search">
            <Search size={15} />
            <input
              aria-label="Buscar chats"
              placeholder="Buscar chats"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          </label>
          <div className="side-heading">
            PROYECTOS
            <button
              className="icon"
              aria-label="Añadir carpeta"
              onClick={() => setAdding(true)}
            >
              <FolderPlus size={15} />
            </button>
          </div>
          <nav aria-label="Proyectos y conversaciones" className="project-list">
            {projects.map((p) => (
              <div className="project-group" key={p.path}>
                <button
                  disabled={busy}
                  className={`project ${project === p.path ? "selected" : ""}`}
                  onClick={() => {
                    setProject(p.path);
                    newChat();
                  }}
                >
                  <ChevronDown size={12} />
                  <Folder size={15} />
                  <span>{p.name}</span>
                </button>
                {visibleSessions
                  .filter((s) => s.working_dir === p.path)
                  .map((s) => (
                    <button
                      disabled={busy}
                      key={s.session_id}
                      className={`thread ${session?.session_id === s.session_id ? "active" : ""}`}
                      onClick={() => void open(s)}
                    >
                      <MessageSquare size={13} />
                      <span>{s.title || "Nuevo chat"}</span>
                    </button>
                  ))}
              </div>
            ))}
            {visibleSessions
              .filter((s) => !projects.some((p) => p.path === s.working_dir))
              .map((s) => (
                <button
                  key={s.session_id}
                  disabled={busy}
                  className={`thread ${session?.session_id === s.session_id ? "active" : ""}`}
                  onClick={() => void open(s)}
                >
                  <MessageSquare size={13} />
                  <span>{s.title || basename(s.working_dir)}</span>
                </button>
              ))}
            {projects.length === 0 && (
              <p className="side-empty">
                Añade una carpeta para empezar a trabajar con Orbit.
              </p>
            )}
          </nav>
          <div className="sidebar-bottom">
            <div className="runtime">
              <span className={`status-dot ${connected ? "online" : ""}`} />
              <span>
                {connected ? "Motor conectado" : "Motor desconectado"}
              </span>
              <button
                className="icon"
                aria-label="Reconectar motor"
                onClick={() => void refresh()}
              >
                <RefreshCw size={13} />
              </button>
            </div>
            <button className="settings-link" onClick={() => setSettings(true)}>
              <Settings2 size={16} />
              Configuración<span className="keycap">Local</span>
            </button>
          </div>
        </aside>
      )}
      <main className="workspace">
        <header className="topbar">
          <div className="breadcrumb">
            {!sidebar && (
              <button
                className="icon"
                aria-label="Mostrar barra lateral"
                onClick={() => setSidebar(true)}
              >
                <PanelLeftOpen size={18} />
              </button>
            )}
            <Folder size={15} />
            <span>{basename(project)}</span>
            <ChevronRight size={12} />
            <span className="muted">{session?.title || "Nuevo chat"}</span>
          </div>
          <button
            className={`toolbar-button ${panel ? "pressed" : ""}`}
            onClick={() => setPanel(!panel)}
            aria-pressed={panel}
          >
            <GitCompareArrows size={16} />
            <span>Cambios</span>
          </button>
        </header>
        <div
          className={`conversation ${messages.length ? "has-messages" : ""}`}
        >
          {!messages.length && !loading && (
            <div className="welcome">
              <div className="welcome-mark">
                <img src="/nexode.svg" alt="Nexode" />
              </div>
              <p className="eyebrow">TU ESPACIO PARA CONSTRUIR</p>
              <h1>¿Qué hacemos hoy?</h1>
              <p className="welcome-copy">
                De la idea al código, con el contexto de tu proyecto.
              </p>
              <div className="suggestions">
                {[
                  {
                    icon: Code2,
                    title: "Entender este proyecto",
                    text: "Explora la estructura del proyecto y explícame cómo funciona, sin modificar archivos.",
                  },
                  {
                    icon: GitCompareArrows,
                    title: "Revisar los cambios",
                    text: "Revisa los cambios actuales de Git y señala posibles errores. No modifiques archivos.",
                  },
                  {
                    icon: Terminal,
                    title: "Encontrar qué mejorar",
                    text: "Analiza el proyecto y propón tres mejoras concretas, con sus archivos y prioridades. No modifiques archivos.",
                  },
                ].map(({ icon: Icon, title, text }) => (
                  <button
                    key={title}
                    onClick={() => {
                      setDraft(text);
                      composer.current?.focus();
                    }}
                  >
                    <Icon size={18} />
                    <span>{title}</span>
                    <ChevronRight size={13} />
                  </button>
                ))}
              </div>
            </div>
          )}
          {loading && (
            <div className="loading">
              <LoaderCircle size={18} className="spin" /> Cargando conversación
            </div>
          )}
          {messages
            .filter((m) => m.role === "user" || m.role === "assistant")
            .map((m, i) => (
              <article key={i} className={`message ${m.role}`}>
                <div className="message-author">
                  {m.role === "assistant" ? (
                    <>
                      <img src="/nexode.svg" alt="" />
                      Orbit
                    </>
                  ) : (
                    "Tú"
                  )}
                </div>
                <div className="markdown">
                  <Markdown remarkPlugins={[remarkGfm]}>{m.content}</Markdown>
                </div>
              </article>
            ))}
          {busy && (
            <article className="message assistant">
              <div className="message-author">
                <img src="/nexode.svg" alt="" />
                Orbit{" "}
                <span className="activity">
                  <LoaderCircle className="spin" size={12} />
                  {activity}
                </span>
              </div>
              {live && (
                <div className="markdown">
                  <Markdown remarkPlugins={[remarkGfm]}>{live}</Markdown>
                </div>
              )}
            </article>
          )}
          <div ref={end} />
        </div>
        <div className="composer-area">
          {error && (
            <div className="error" role="alert">
              <span>{error}</span>
              <button
                className="icon"
                aria-label="Cerrar error"
                onClick={() => setError("")}
              >
                <X size={14} />
              </button>
            </div>
          )}
          {permission && (
            <div className="permission">
              <ShieldCheck size={20} />
              <div>
                <strong>Orbit necesita tu autorización</strong>
                <p>{permission.description}</p>
              </div>
              <button onClick={() => void answer("deny")}>Denegar</button>
              <button className="primary" onClick={() => void answer("allow")}>
                Permitir
              </button>
            </div>
          )}
          <form
            className="composer"
            onSubmit={(e) => {
              e.preventDefault();
              void send();
            }}
          >
            <textarea
              ref={composer}
              aria-label="Mensaje para Orbit"
              placeholder="Pídele a Orbit que construya, revise o resuelva algo…"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (
                  e.key === "Enter" &&
                  !e.shiftKey &&
                  !e.nativeEvent.isComposing
                ) {
                  e.preventDefault();
                  void send();
                }
              }}
            />
            <div className="composer-controls">
              <div className="composer-pickers">
                <Picker
                  label={
                    <>
                      <Folder size={14} />
                      {basename(project)}
                    </>
                  }
                >
                  {projects.map((p) => (
                    <Menu.Item
                      disabled={busy || !!session}
                      className="menu-item"
                      key={p.path}
                      onSelect={() => setProject(p.path)}
                    >
                      {p.name}
                      {project === p.path && <Check size={13} />}
                    </Menu.Item>
                  ))}
                  <Menu.Separator className="menu-separator" />
                  <Menu.Item
                    className="menu-item"
                    onSelect={() => setAdding(true)}
                  >
                    Añadir carpeta
                    <FolderPlus size={14} />
                  </Menu.Item>
                </Picker>
                {session ? (
                  <Picker label={model || "Modelo"}>
                    <Menu.Label className="menu-label">
                      MODELOS DEL MOTOR
                    </Menu.Label>
                    {models.map((m) => (
                      <Menu.Item
                        disabled={busy}
                        key={m}
                        className="menu-item"
                        onSelect={() => {
                          void api(`/sessions/${session.session_id}/models`, {
                            model: m,
                          })
                            .then(() => setModel(m))
                            .catch(fail);
                        }}
                      >
                        {m}
                        {model === m && <Check size={13} />}
                      </Menu.Item>
                    ))}
                    <Menu.Separator className="menu-separator" />
                    <Menu.Item
                      className="menu-item"
                      onSelect={() =>
                        void loadModels(session.session_id).catch(fail)
                      }
                    >
                      Actualizar modelos
                      <RefreshCw size={13} />
                    </Menu.Item>
                  </Picker>
                ) : (
                  <span className="model-default">Modelo de Orbit</span>
                )}
              </div>
              {busy ? (
                <button
                  type="button"
                  className="send stop"
                  aria-label="Detener respuesta"
                  onClick={() =>
                    void api(
                      `/sessions/${session?.session_id}/cancel`,
                      {},
                    ).catch(fail)
                  }
                >
                  <Square size={14} />
                </button>
              ) : (
                <button
                  className="send"
                  aria-label="Enviar mensaje"
                  disabled={!draft.trim() || !project || !connected || loading}
                >
                  <ArrowUp size={19} />
                </button>
              )}
            </div>
          </form>
          <div className="composer-footer">
            <span>
              <ShieldCheck size={12} /> Entorno local · Permisos controlados por
              Orbit
            </span>
            <span>
              Enter para enviar <span className="separator">/</span> Shift +
              Enter para nueva línea
            </span>
          </div>
        </div>
        <footer className="statusbar">
          <span>
            <GitBranch size={12} />
            {changes?.branch.trim() || basename(project)}
          </span>
          <span>
            <span className={`status-dot ${connected ? "online" : ""}`} />
            {connected ? "Orbit listo" : "Sin conexión"}
          </span>
        </footer>
      </main>
      {panel && (
        <aside className="changes-panel">
          <header>
            <span>
              <GitCompareArrows size={16} />
              Cambios del proyecto
            </span>
            <button
              className="icon"
              aria-label="Cerrar panel de cambios"
              onClick={() => setPanel(false)}
            >
              <X size={16} />
            </button>
          </header>
          <div className="changes-body">
            <div className="changes-label">
              ÁRBOL DE TRABAJO
              <button
                className="icon"
                aria-label="Actualizar cambios"
                disabled={!session}
                onClick={() => void readChanges()}
              >
                <RefreshCw size={14} />
              </button>
            </div>
            {diffError ? (
              <p className="error">{diffError}</p>
            ) : changes ? (
              <>
                <pre className="file-status">
                  {changes.status || "No hay cambios pendientes."}
                </pre>
                {changes.diff ? (
                  <pre className="diff">
                    {changes.diff.split("\n").map((l, i) => (
                      <div
                        key={i}
                        className={
                          l.startsWith("+")
                            ? "addition"
                            : l.startsWith("-")
                              ? "deletion"
                              : l.startsWith("@@")
                                ? "hunk"
                                : ""
                        }
                      >
                        {l || " "}
                      </div>
                    ))}
                  </pre>
                ) : (
                  <div className="panel-empty">
                    <Check size={24} />
                    <p>Todo en su sitio</p>
                    <span>
                      {changes.status
                        ? "Los archivos nuevos aparecen en la lista."
                        : "Tu árbol de trabajo está limpio."}
                    </span>
                  </div>
                )}
              </>
            ) : (
              <div className="panel-empty">
                <GitCompareArrows size={28} />
                <p>El código, a la vista</p>
                <span>
                  Abre un chat para revisar los cambios de su proyecto.
                </span>
              </div>
            )}
          </div>
        </aside>
      )}
      <Dialog.Root
        open={settings || adding}
        onOpenChange={(open) => {
          if (!open) {
            setSettings(false);
            setAdding(false);
          }
        }}
      >
        <Dialog.Portal>
          <Dialog.Overlay className="modal-backdrop" />
          <Dialog.Content className="modal" aria-describedby={undefined}>
            <Dialog.Title className="sr-only">
              {adding ? "Añadir proyecto" : "Configuración"}
            </Dialog.Title>
            <button
              autoFocus
              className="icon modal-close"
              aria-label="Cerrar ventana"
              onClick={() => {
                setSettings(false);
                setAdding(false);
              }}
            >
              <X size={18} />
            </button>
            {adding ? (
              <>
                <FolderPlus size={26} />
                <h2>Abre tu proyecto</h2>
                <p>
                  Escribe la ruta de la carpeta en el sistema donde se ejecuta
                  Orbit.
                </p>
                <label className="field">
                  Carpeta del proyecto
                  <input
                    placeholder="/mnt/d/vscode/mi-proyecto"
                    value={folder}
                    onChange={(e) => setFolder(e.target.value)}
                  />
                </label>
                <button
                  className="primary"
                  disabled={!folder.trim() || busy}
                  onClick={() => {
                    const path = folder.trim();
                    setProjects((p) =>
                      p.some((x) => x.path === path)
                        ? p
                        : [...p, { name: basename(path), path }],
                    );
                    setProject(path);
                    newChat();
                    setAdding(false);
                    setFolder("");
                  }}
                >
                  Abrir carpeta
                </button>
              </>
            ) : (
              <>
                <img className="settings-logo" src="/nexode.svg" alt="" />
                <h2>Tu espacio, tu motor.</h2>
                <p>
                  Nexode Orbit ejecuta el agente en tu equipo. Esta interfaz
                  utiliza una instancia independiente y conserva sus
                  conversaciones.
                </p>
                <dl>
                  <dt>Conexión</dt>
                  <dd>{connected ? "Conectado" : "Desconectado"}</dd>
                  <dt>Interfaz</dt>
                  <dd>Local · Rust + React</dd>
                  <dt>Proveedor y credenciales</dt>
                  <dd>Gestionados por Orbit</dd>
                </dl>
                <p className="small">
                  El selector de modelos aparece al crear el primer chat. Las
                  credenciales existentes se heredan del motor; nunca se envían
                  al navegador.
                </p>
                <button className="primary" onClick={() => void refresh()}>
                  Comprobar conexión
                </button>
              </>
            )}
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </div>
  );
}
