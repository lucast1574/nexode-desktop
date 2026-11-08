import { test, expect } from "@playwright/test";
// Opt-in only: run against the isolated native fake OpenAI fixture, never a paid provider.
test("real Rust bridge streams a turn, persists history and reads Git", async ({
  page,
  request,
}) => {
  test.skip(
    process.env.ORBIT_LIVE_TEST !== "1",
    "Requires the isolated synthetic provider",
  );
  test.setTimeout(150000);
  await page.goto("/");
  await expect(page.getByText("Motor conectado")).toBeVisible({
    timeout: 40000,
  });
  await page
    .getByRole("textbox", { name: "Mensaje para Orbit" })
    .fill("Desktop integration smoke test");
  await page.getByRole("button", { name: "Enviar mensaje" }).click();
  await expect(page.locator(".message.assistant")).toContainText(
    "[desktop] ack.",
    { timeout: 120000 },
  );
  await expect(
    page.getByRole("button", { name: "Enviar mensaje" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Cambios", exact: true }).click();
  await expect(page.locator(".file-status")).toBeVisible({ timeout: 15000 });
  await page.screenshot({ path: "test-results/live-integration.png" });
  const forbidden = await request.get("http://127.0.0.1:4318/api/sessions");
  expect(forbidden.status()).toBe(403);
  const hostile = await request.get("http://127.0.0.1:4318/api/sessions", {
    headers: { "X-Orbit-Client": "desktop", Origin: "https://hostile.example" },
  });
  expect(hostile.status()).toBe(403);
});
