export type Session = {
  session_id: string;
  title?: string;
  working_dir?: string;
  status: string;
};
export type Project = { name: string; path: string };
export type Message = { role: string; content: string };
export const headers = {
  "X-Orbit-Client": "desktop",
  "Content-Type": "application/json",
};
export async function api<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(`/api${path}`, {
    headers,
    ...(body !== undefined
      ? { method: "POST", body: JSON.stringify(body) }
      : {}),
  });
  const data = await response.json();
  if (!response.ok)
    throw new Error(data.error || "No se pudo conectar con Orbit");
  return data;
}
export async function streamTurn(
  id: string,
  content: string,
  onEvent: (event: Record<string, unknown>) => void,
) {
  const response = await fetch(`/api/sessions/${encodeURIComponent(id)}/turn`, {
    method: "POST",
    headers,
    body: JSON.stringify({ content }),
  });
  if (!response.ok || !response.body)
    throw new Error("No se pudo iniciar el mensaje");
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  while (true) {
    const { done, value } = await reader.read();
    buffer += decoder.decode(value, { stream: !done });
    const lines = buffer.split("\n");
    buffer = lines.pop() || "";
    for (const line of lines) if (line.trim()) onEvent(JSON.parse(line));
    if (done) {
      if (buffer.trim()) onEvent(JSON.parse(buffer));
      break;
    }
  }
}
