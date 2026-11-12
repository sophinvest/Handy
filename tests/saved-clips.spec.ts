import { test, expect } from "./fixtures/browser";

const ids = ["300.wav", "200.wav", "100.wav"];
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (v: unknown) => void>();
    const state = {
      clips: [300, 200, 100].map((n) => ({
        id: `${n}.wav`,
        saved_at_ms: 1794500000000 + n * 1000,
        duration_seconds: n / 10,
      })),
      calls: [] as { command: string; id?: string | null }[],
      fail: "",
      delay: 0,
      nextListDelay: 0,
    };
    Object.assign(window, {
      fixture: state,
      __TAURI_INTERNALS__: {
        transformCallback: (fn: (v: unknown) => void) => {
          const id = callbacks.size + 1;
          callbacks.set(id, fn);
          return id;
        },
        unregisterCallback: () => {},
        invoke: async (command: string, args?: { clipId?: string | null }) => {
          if (command === "plugin:event|listen") return 1;
          if (command === "plugin:event|unlisten") return;
          if (command === "gemini_list_clips") {
            const snapshot = structuredClone(state.clips);
            const delay = state.nextListDelay;
            state.nextListDelay = 0;
            if (delay) await new Promise((r) => setTimeout(r, delay));
            return snapshot;
          }
          state.calls.push({ command, id: args?.clipId });
          if (state.delay) await new Promise((r) => setTimeout(r, state.delay));
          if (state.fail) throw state.fail;
          if (
            command === "gemini_retry_clip" ||
            command === "gemini_delete_clip"
          )
            state.clips = state.clips.filter(
              (c) => c.id !== (args?.clipId ?? state.clips[0]?.id),
            );
          return "delivered";
        },
      },
    });
  });
  await page.goto("/tests/fixtures/saved-clips.html");
});

test("newest action and metadata, with individual older retry and delete", async ({
  page,
}) => {
  const rows = page.locator("li[data-clip-id]");
  await expect(rows).toHaveCount(3);
  for (const [n, id] of ids.entries())
    await expect(rows.nth(n)).toHaveAttribute("data-clip-id", id);
  await expect(rows.nth(0)).toContainText("0:30");
  await rows.nth(2).getByRole("button", { name: "Retry", exact: true }).click();
  await expect(rows).toHaveCount(2);
  await rows
    .nth(1)
    .getByRole("button", { name: "Delete", exact: true })
    .click();
  await expect(rows).toHaveCount(1);
  await page
    .getByRole("button", { name: "Retry last failed clip", exact: true })
    .click();
  await expect(rows).toHaveCount(0);
  await expect(
    page.getByText("No saved failed clips.", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Retry last failed clip", exact: true }),
  ).toBeDisabled();
  const calls = await page.evaluate(
    () =>
      (window as unknown as { fixture: { calls: unknown[] } }).fixture.calls,
  );
  expect(calls).toEqual([
    { command: "gemini_retry_clip", id: "100.wav" },
    { command: "gemini_delete_clip", id: "200.wav" },
    { command: "gemini_retry_clip", id: null },
  ]);
});

test("failure retains rows and duplicate clicks cannot start another request", async ({
  page,
}) => {
  await page.evaluate(() =>
    Object.assign((window as unknown as { fixture: object }).fixture, {
      fail: "Text delivery failed; clip is still saved",
      delay: 150,
    }),
  );
  const button = page.getByRole("button", {
    name: "Retry last failed clip",
    exact: true,
  });
  await button.dblclick({ force: true });
  await expect(
    page.getByText("Text delivery failed; clip is still saved", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.locator("li[data-clip-id]")).toHaveCount(3);
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { fixture: { calls: unknown[] } }).fixture.calls,
    ),
  ).toHaveLength(1);
});

test("a delayed refresh cannot restore a deleted row", async ({ page }) => {
  const rows = page.locator("li[data-clip-id]");
  await expect(rows).toHaveCount(3);
  await page.evaluate(() => {
    Object.assign((window as unknown as { fixture: object }).fixture, {
      nextListDelay: 500,
    });
    window.dispatchEvent(new Event("focus"));
  });
  await rows
    .nth(2)
    .getByRole("button", { name: "Delete", exact: true })
    .click();
  await expect(rows).toHaveCount(2);
  await page.waitForTimeout(600);
  await expect(rows).toHaveCount(2);
  await expect(page.locator('[data-clip-id="100.wav"]')).toHaveCount(0);
});

test("main action resolves newest natively when the displayed list is stale", async ({
  page,
}) => {
  await expect(page.locator("li[data-clip-id]")).toHaveCount(3);
  await page.evaluate(() => {
    const state = (window as unknown as { fixture: { clips: object[] } })
      .fixture;
    state.clips.unshift({
      id: "400.wav",
      saved_at_ms: 1794500900000,
      duration_seconds: 40,
    });
  });
  await page
    .getByRole("button", { name: "Retry last failed clip", exact: true })
    .click();
  await expect(page.locator('[data-clip-id="300.wav"]')).toHaveCount(1);
  const call = await page.evaluate(
    () =>
      (
        window as unknown as {
          fixture: { calls: { command: string; id: string | null }[] };
        }
      ).fixture.calls[0],
  );
  expect(call).toEqual({ command: "gemini_retry_clip", id: null });
});

test("a fast successful main action cannot drain a second clip on double click", async ({
  page,
}) => {
  const button = page.getByRole("button", {
    name: "Retry last failed clip",
    exact: true,
  });
  await expect(button).toBeEnabled();
  await button.dblclick({ force: true });
  await expect(page.locator("li[data-clip-id]")).toHaveCount(2);
  await expect(button).toBeEnabled();
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { fixture: { calls: unknown[] } }).fixture.calls,
    ),
  ).toHaveLength(1);
});
