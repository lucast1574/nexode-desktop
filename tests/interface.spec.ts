import { test, expect } from "@playwright/test";
const session = {
  session_id: "test-session",
  title: "Revisar el motor Rust",
  working_dir: "/workspace/nexode-orbit",
  status: "idle",
};
test.beforeEach(async ({ page }) => {
  let history: Array<{ role: string; content: string }> = [];
  await page.route("**/api/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    let body: unknown = { ok: true };
    if (path.endsWith("/status")) body = { connected: true };
    else if (path.endsWith("/projects"))
      body = [{ name: "nexode-orbit", path: session.working_dir }];
    else if (path.endsWith("/sessions"))
      body = route.request().method() === "POST" ? session : [session];
    else if (path.endsWith("/history")) body = history;
    else if (path.endsWith("/models"))
      body = { models: ["test-model", "another-model"], current: "test-model" };
    else if (path.endsWith("/changes"))
      body = {
        branch: "main",
        status: " M src/main.rs",
        diff: "diff --git a/src/main.rs b/src/main.rs\n@@ -1 +1 @@\n-old\n+new",
      };
    else if (path.endsWith("/turn")) {
      const content = route.request().postDataJSON().content;
      history = [
        ...history,
        { role: "user", content },
        {
          role: "assistant",
          content: "He revisado el proyecto. **El motor está en Rust.**",
        },
      ];
      return route.fulfill({
        contentType: "application/x-ndjson",
        body:
          [
            {
              ev: "text_delta",
              message_id: "a",
              text: "He revisado el proyecto.",
            },
            {
              ev: "text_replace",
              message_id: "a",
              text: "He revisado el proyecto. **El motor está en Rust.**",
            },
            { ev: "desktop_done" },
          ]
            .map((v) => JSON.stringify(v))
            .join("\n") + "\n",
      });
    }
    await route.fulfill({ json: body });
  });
});
test("starts a chat, restores history, selects a model and shows changes", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByText("Motor conectado")).toBeVisible();
  await page.getByRole("button", { name: "Entender este proyecto" }).click();
  await expect(
    page.getByRole("textbox", { name: "Mensaje para Orbit" }),
  ).toHaveValue(/Explora la estructura/);
  await page.getByRole("button", { name: "Enviar mensaje" }).click();
  await expect(
    page.getByText("El motor está en Rust.", { exact: false }),
  ).toBeVisible();
  await page.getByRole("button", { name: "test-model", exact: true }).click();
  await page.getByRole("menuitem", { name: "another-model" }).click();
  await expect(
    page.getByRole("button", { name: "another-model", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Cambios", exact: true }).click();
  await expect(page.locator(".addition")).toHaveText("+new");
  await page.getByRole("button", { name: "Nuevo chat", exact: true }).click();
  await page
    .getByRole("button", { name: "Revisar el motor Rust" })
    .first()
    .click();
  await expect(
    page.getByText("El motor está en Rust.", { exact: false }),
  ).toBeVisible();
  await page.screenshot({ path: "test-results/conversation.png" });
});
test("desktop welcome, keyboard dialog and mobile layout", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("Motor conectado")).toBeVisible();
  await page.screenshot({ path: "test-results/desktop.png" });
  await page.getByRole("button", { name: "Configuración" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).not.toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.reload();
  await expect(
    page.getByRole("heading", { name: "¿Qué hacemos hoy?" }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({ path: "test-results/mobile.png" });
});
test("connection failure is actionable and does not allow a send", async ({
  page,
}) => {
  await page.route("**/api/status", (route) =>
    route.fulfill({
      status: 400,
      json: { error: "No se encontró el ejecutable de Orbit" },
    }),
  );
  await page.goto("/");
  await expect(page.getByRole("alert")).toContainText("No se encontró");
  await page.getByRole("textbox", { name: "Mensaje para Orbit" }).fill("Hola");
  await expect(
    page.getByRole("button", { name: "Enviar mensaje" }),
  ).toBeDisabled();
});
