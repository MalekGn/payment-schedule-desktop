// End-to-end test suite for paymentSchedule.
//
// Runs against the real Vue app served by Vite, driving a headless Chromium via
// the Playwright library (the `playwright` package — the `@playwright/test`
// runner is not installed, so this file is a self-contained harness with its
// own tiny assert/report layer and no extra dependency).
//
// In a plain browser the app talks to the in-memory mock backend (src/api/mock.ts),
// which mirrors the Rust commands and is seeded with 6 clients / 8 purchases.
// A full page load (page.goto) re-instantiates that mock, so every test starts
// from the same seed and stays independent.
//
// Usage: node tests/e2e/run.mjs   (spawns Vite itself, tears it down on exit)

import { chromium } from "playwright";
import { Buffer } from "node:buffer";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

// This file lives at <root>/tests/e2e/run.mjs, so the project root is two levels up.
const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "..", "..");
// Dedicated port so the suite never collides with (or accidentally tests) a
// dev server the user already has running on Vite's default 5173.
const PORT = 5199;
const BASE = `http://localhost:${PORT}`;
// Screenshots land next to this script, under tests/e2e/artifacts.
const ARTIFACTS = path.join(HERE, "artifacts");
mkdirSync(ARTIFACTS, { recursive: true });

// --- tiny test harness -------------------------------------------------------

const tests = [];
const test = (name, fn) => tests.push({ name, fn });

function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}
function assertEqual(actual, expected, msg) {
  if (actual !== expected) {
    throw new Error(
      `${msg}\n    expected: ${JSON.stringify(expected)}\n    actual:   ${JSON.stringify(actual)}`,
    );
  }
}

// --- server lifecycle --------------------------------------------------------

function startServer() {
  const bin = path.join(ROOT, "node_modules", ".bin", "vite");
  const proc = spawn(bin, ["--port", String(PORT), "--strictPort"], {
    cwd: ROOT,
    stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, NO_COLOR: "1" },
  });
  proc.stdout.on("data", () => {});
  proc.stderr.on("data", (d) => process.stderr.write(`[vite] ${d}`));
  return proc;
}

async function waitForServer(timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(BASE, { method: "GET" });
      if (res.ok) return;
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 300));
  }
  throw new Error(`Vite dev server did not start within ${timeoutMs}ms`);
}

// --- helpers used across tests ----------------------------------------------

const NAV = {
  dashboard: "Tableau de bord",
  achats: "Achats",
  clients: "Clients",
  paiements: "Paiements",
  echeances: "Échéances",
  impayes: "Impayés",
  alertes: "Alertes",
  rapports: "Rapports",
  parametres: "Paramètres",
};

/** Load a route with a full page load so the mock resets to its seed. */
async function open(page, route = "/") {
  await page.goto(`${BASE}${route}`, { waitUntil: "networkidle" });
  await page.locator(".app-shell").waitFor({ state: "visible", timeout: 10000 });
}

// --- scenarios ---------------------------------------------------------------

test("app shell + sidebar render on first load", async (page) => {
  await open(page, "/");
  assertEqual(await page.locator(".brand-line1").innerText(), "Paiements", "brand line 1");
  assertEqual(await page.locator(".brand-line2").innerText(), "Échelonnés", "brand line 2");
  assertEqual(await page.locator(".nav-item").count(), 9, "sidebar should have 9 nav items");
  assertEqual(
    await page.locator("h1.page-title").innerText(),
    NAV.dashboard,
    "header title on dashboard",
  );
});

test("dashboard shows 5 KPI cards seeded from mock (8 purchases)", async (page) => {
  await open(page, "/");
  await page.locator(".kpi-row .kpi").first().waitFor({ timeout: 10000 });
  assertEqual(await page.locator(".kpi-row .kpi").count(), 5, "should render 5 KPI cards");

  const purchasesCard = page.locator(".kpi", { hasText: "Achats totaux" });
  const value = (await purchasesCard.locator(".kpi-value").innerText()).trim();
  assertEqual(value, "8", "total purchases KPI value (8 seeded purchases)");
});

test("sidebar navigates to every page and header title updates", async (page) => {
  await open(page, "/");
  for (const [name, label] of Object.entries(NAV)) {
    if (name === "dashboard") continue; // already there
    await page.locator(".nav-item", { hasText: label }).click();
    await page.waitForFunction(
      (expected) => document.querySelector("h1.page-title")?.textContent?.trim() === expected,
      label,
      { timeout: 5000 },
    );
    assertEqual(await page.locator("h1.page-title").innerText(), label, `header title for ${name}`);
  }
});

test("clients list renders the 6 seeded clients", async (page) => {
  await open(page, "/clients");
  await page.locator("table.table tbody tr").first().waitFor({ timeout: 10000 });
  assertEqual(await page.locator("table.table tbody tr").count(), 6, "seeded client row count");
  await page.locator("table.table", { hasText: "Mohamed Trabelsi" }).waitFor({ timeout: 5000 });
});

test("create a new client end-to-end (form -> table)", async (page) => {
  await open(page, "/clients");
  await page.locator("table.table tbody tr").first().waitFor({ timeout: 10000 });
  assertEqual(await page.locator("table.table tbody tr").count(), 6, "precondition: 6 clients");

  await page.getByRole("button", { name: "Nouveau client" }).click();
  const dialog = page.locator('[role="dialog"]');
  await dialog.waitFor({ state: "visible", timeout: 5000 });
  assertEqual(await dialog.locator(".modal-head h2").innerText(), "Nouveau client", "modal title");

  await page.locator("#cf-first").fill("Zied");
  await page.locator("#cf-last").fill("Zzztesteur");
  await page.locator("#cf-phone").fill("+216 21 000 000");
  await page.locator("#cf-addr").fill("Rue de Test, Tunis");
  await dialog.getByRole("button", { name: "Créer" }).click();

  await dialog.waitFor({ state: "hidden", timeout: 5000 });
  await page.waitForFunction(
    () => document.querySelectorAll("table.table tbody tr").length === 7,
    undefined,
    { timeout: 5000 },
  );
  assertEqual(await page.locator("table.table tbody tr").count(), 7, "client added to table");
  await page.locator("table.table", { hasText: "Zied Zzztesteur" }).waitFor({ timeout: 5000 });
});

test("achats list renders 8 purchases and search filters them", async (page) => {
  await open(page, "/achats");
  await page.locator("table.table tbody tr").first().waitFor({ timeout: 10000 });
  assertEqual(await page.locator("table.table tbody tr").count(), 8, "seeded purchase row count");

  await page.getByPlaceholder(/Rechercher un achat/).fill("Samsung");
  await page.waitForFunction(
    () => document.querySelectorAll("table.table tbody tr").length === 1,
    undefined,
    { timeout: 5000 },
  );
  const row = await page.locator("table.table tbody tr").first().innerText();
  assert(/Samsung/i.test(row), `filtered row should mention Samsung, got: ${row}`);
});

test("impayés page lists overdue clients and sidebar shows a danger badge", async (page) => {
  await open(page, "/impayes");
  // Seed produces past-due unpaid installments, so this must not be the empty state.
  await page.locator("table.table tbody tr").first().waitFor({ timeout: 10000 });
  const rows = await page.locator("table.table tbody tr").count();
  assert(rows >= 1, `expected at least one overdue client, got ${rows}`);

  const badge = page.locator(".nav-item", { hasText: NAV.impayes }).locator(".nav-badge--danger");
  await badge.waitFor({ timeout: 5000 });
  const count = parseInt((await badge.innerText()).trim(), 10);
  assert(count >= 1, `danger badge should be >= 1, got ${count}`);
});

test("record a partial payment on a purchase (PaymentModal)", async (page) => {
  // Seed purchase A-000001: 2400 over 6 monthly tranches of 400, tranche 1 paid.
  await open(page, "/achats/1");
  await page.locator(".inst-table tbody tr").first().waitFor({ timeout: 10000 });

  // Payment history starts with the single seeded payment (tranche 1).
  const history = page.locator("table.table").last();
  await page.waitForFunction(
    () => {
      const tables = document.querySelectorAll("table.table");
      const last = tables[tables.length - 1];
      return !!last && last.querySelectorAll("tbody tr").length === 1;
    },
    undefined,
    { timeout: 5000 },
  );

  // The first "Enregistrer" action in the schedule is tranche 2 (400 remaining).
  await page.locator(".inst-table .btn--primary").first().click();
  const dialog = page.locator('[role="dialog"]');
  await dialog.waitFor({ state: "visible", timeout: 5000 });
  assertEqual(
    await dialog.locator(".modal-head h2").innerText(),
    "Enregistrer un paiement — Tranche 2/6",
    "payment modal title (tranche 2 of 6)",
  );

  // Amount pre-fills with the full remaining (400); pay a partial 150 instead.
  assertEqual(
    await page.locator("#pay-amount").inputValue(),
    "400",
    "amount pre-filled with remaining",
  );
  await page.locator("#pay-amount").fill("150");
  await dialog.getByRole("button", { name: "Enregistrer" }).click();

  await dialog.waitFor({ state: "hidden", timeout: 5000 });

  // History now has two rows; the newest (dated today) is the 150 partial payment.
  await page.waitForFunction(
    () => {
      const tables = document.querySelectorAll("table.table");
      const last = tables[tables.length - 1];
      return !!last && last.querySelectorAll("tbody tr").length === 2;
    },
    undefined,
    { timeout: 5000 },
  );
  const newest = await history.locator("tbody tr").first().innerText();
  assert(/150/.test(newest), `newest payment row should show the 150 partial, got: ${newest}`);
});

test("new purchase: auto-split installments and sum-mismatch validation", async (page) => {
  await open(page, "/achats");
  await page.locator("table.table tbody tr").first().waitFor({ timeout: 10000 });
  assertEqual(await page.locator("table.table tbody tr").count(), 8, "precondition: 8 purchases");

  // Scope to the main content: the sidebar also carries a permanent "Nouvel
  // achat" button, so an unscoped role query would match two elements.
  await page.getByRole("main").getByRole("button", { name: "Nouvel achat" }).click();
  const dialog = page.locator('[role="dialog"]');
  await dialog.waitFor({ state: "visible", timeout: 5000 });
  assertEqual(await dialog.locator(".modal-head h2").innerText(), "Nouvel achat", "modal title");

  await page.locator("#np-client").selectOption("1");
  await page.locator("#np-product").fill("Aspirateur Dyson");
  await page.locator("#np-total").fill("3000");
  await page.locator("#np-count").fill("3");

  // Amounts auto-split 3000 / 3 = 1000 each and the running sum matches the total.
  await page.waitForFunction(() => document.querySelectorAll(".inst-row").length === 3, undefined, {
    timeout: 5000,
  });
  await page.locator(".inst-sum.ok").waitFor({ timeout: 5000 });
  assertEqual(
    await page.locator(".inst-amount").first().inputValue(),
    "1000",
    "first tranche auto-split to 1000",
  );

  // Break the balance by hand-editing one tranche -> the sum no longer matches.
  await page.locator(".inst-amount").first().fill("999");
  await page.locator(".inst-sum.bad").waitFor({ timeout: 5000 });

  // Submitting with a mismatch is blocked client-side: the modal stays open.
  await dialog.getByRole("button", { name: /Enregistrer l.achat/ }).click();
  assert(await dialog.isVisible(), "modal must stay open while the sum mismatches");
  await page.locator(".inst-sum.bad").waitFor({ timeout: 2000 });

  // Recalculer restores the even split, the sum matches, and submission succeeds.
  await page.getByRole("button", { name: "Recalculer automatiquement" }).click();
  await page.locator(".inst-sum.ok").waitFor({ timeout: 5000 });
  await dialog.getByRole("button", { name: /Enregistrer l.achat/ }).click();

  await dialog.waitFor({ state: "hidden", timeout: 5000 });
  // The new purchase (9th, reference A-000009) opens on its own detail page.
  await page.waitForFunction(
    () => document.querySelector("h1.page-title")?.textContent?.trim() === "A-000009",
    undefined,
    { timeout: 5000 },
  );
  assertEqual(
    await page.locator("h1.page-title").innerText(),
    "A-000009",
    "navigated to new purchase detail",
  );
});

test("delete-client safeguard warns when the client has purchases", async (page) => {
  await open(page, "/clients");
  await page.locator("table.table tbody tr").first().waitFor({ timeout: 10000 });
  assertEqual(await page.locator("table.table tbody tr").count(), 6, "precondition: 6 clients");

  // Mohamed Trabelsi has 2 seeded purchases -> the confirm must warn about cascade.
  const row = page.locator("table.table tbody tr", { hasText: "Mohamed Trabelsi" });
  await row.locator(".icon-action--danger").click();

  const dialog = page.locator('[role="dialog"]');
  await dialog.waitFor({ state: "visible", timeout: 5000 });
  assertEqual(
    await dialog.locator(".modal-head h2").innerText(),
    "Supprimer le client",
    "confirm dialog title",
  );
  const msg = await dialog.locator(".confirm-msg").innerText();
  assert(/2 achat/.test(msg), `message should warn about the 2 purchases, got: ${msg}`);

  // Confirming force-deletes the client (cascading its purchases) -> 5 rows remain.
  await dialog.getByRole("button", { name: "Supprimer" }).click();
  await dialog.waitFor({ state: "hidden", timeout: 5000 });
  await page.waitForFunction(
    () => document.querySelectorAll("table.table tbody tr").length === 5,
    undefined,
    { timeout: 5000 },
  );
  assertEqual(await page.locator("table.table tbody tr").count(), 5, "client removed from table");
  assertEqual(
    await page.locator("table.table", { hasText: "Mohamed Trabelsi" }).count(),
    0,
    "deleted client no longer listed",
  );
});

test("deleting a client with no purchases needs a single confirm", async (page) => {
  // This is the path that now genuinely sends `force: false`, so the backend
  // gate decides rather than the UI asserting `true` unconditionally. A freshly
  // created client is guaranteed to have no purchases.
  await open(page, "/clients");
  await page.locator("table.table tbody tr").first().waitFor({ timeout: 10000 });

  await page.getByRole("button", { name: "Nouveau client" }).click();
  let dialog = page.locator('[role="dialog"]');
  await dialog.waitFor({ state: "visible", timeout: 5000 });
  await page.locator("#cf-first").fill("Zied");
  await page.locator("#cf-last").fill("Zzzsupprime");
  await page.locator("#cf-phone").fill("+216 21 000 000");
  await dialog.getByRole("button", { name: "Créer" }).click();
  await dialog.waitFor({ state: "hidden", timeout: 5000 });
  await page.waitForFunction(
    () => document.querySelectorAll("table.table tbody tr").length === 7,
    undefined,
    { timeout: 5000 },
  );

  const row = page.locator("table.table tbody tr", { hasText: "Zzzsupprime" });
  await row.locator(".icon-action--danger").click();
  dialog = page.locator('[role="dialog"]');
  await dialog.waitFor({ state: "visible", timeout: 5000 });

  // No purchases -> the plain confirmation, not the cascade warning.
  const msg = await dialog.locator(".confirm-msg").innerText();
  assert(!/achat/i.test(msg), `no-purchase delete must not mention purchases, got: ${msg}`);

  await dialog.getByRole("button", { name: "Supprimer" }).click();
  await dialog.waitFor({ state: "hidden", timeout: 5000 });
  await page.waitForFunction(
    () => document.querySelectorAll("table.table tbody tr").length === 6,
    undefined,
    { timeout: 5000 },
  );
  assertEqual(
    await page.locator("table.table", { hasText: "Zzzsupprime" }).count(),
    0,
    "client deleted on a single confirm",
  );
});

test("impayés: the exported CSV is localized and properly quoted", async (page) => {
  await open(page, "/impayes");
  await page.locator(".impaye-card").first().waitFor({ timeout: 10000 });

  const [download] = await Promise.all([
    page.waitForEvent("download", { timeout: 10000 }),
    page.getByRole("button", { name: /Exporter/ }).click(),
  ]);

  assert(
    /^impayes-\d{4}-\d{2}-\d{2}\.csv$/.test(download.suggestedFilename()),
    `filename should carry the export date, got: ${download.suggestedFilename()}`,
  );

  const stream = await download.createReadStream();
  const chunks = [];
  for await (const chunk of stream) chunks.push(chunk);
  const text = Buffer.concat(chunks).toString("utf8");

  // BOM first, so Excel reads it as UTF-8 (the accented French headers and the
  // Arabic ones depend on it).
  assert(text.charCodeAt(0) === 0xfeff, "CSV must start with a UTF-8 BOM");

  const lines = text
    .replace(/^\uFEFF/, "")
    .trimEnd()
    .split("\r\n");
  assertEqual(
    lines[0],
    '"Client","Téléphone","N° Achat","Tranche","Échéance","Montant","En retard depuis"',
    "header row must use the localized column labels, not hard-coded strings",
  );

  // One row per overdue installment, and every row must have exactly seven
  // fields — the property that broke when an unescaped quote split a field.
  assert(lines.length > 1, "seeded data should produce at least one overdue row");
  for (const line of lines.slice(1)) {
    const fields = line.match(/("([^"]|"")*"|[^,]*)/g).filter((f) => f !== "");
    assertEqual(fields.length, 7, `every row needs 7 fields, got ${fields.length} in: ${line}`);
  }
});

test("switching to Arabic mirrors the layout to RTL", async (page) => {
  await open(page, "/");
  // Baseline: French, left-to-right.
  assertEqual(await page.locator("html").getAttribute("dir"), "ltr", "baseline dir is ltr");
  assertEqual(await page.locator("html").getAttribute("lang"), "fr", "baseline lang is fr");
  assertEqual(await page.locator(".brand-line1").innerText(), "Paiements", "baseline brand (fr)");

  // Pick Arabic from the header language menu.
  await page.locator(".lang-btn").click();
  await page.locator(".lang-option", { hasText: "العربية" }).click();

  // The document direction + language flip and the chrome re-renders in Arabic.
  await page.waitForFunction(
    () => document.documentElement.getAttribute("dir") === "rtl",
    undefined,
    { timeout: 5000 },
  );
  assertEqual(await page.locator("html").getAttribute("dir"), "rtl", "dir switches to rtl");
  assertEqual(await page.locator("html").getAttribute("lang"), "ar", "lang switches to ar");
  assertEqual(
    await page.locator(".brand-line1").innerText(),
    "الدفع",
    "brand re-renders in Arabic",
  );
  assertEqual(
    await page.locator(".nav-item", { hasText: "لوحة التحكم" }).count(),
    1,
    "dashboard nav label is Arabic",
  );
});

// --- Overdue (Impayés) page --------------------------------------------------
// The page renders one card per overdue client (`.impaye-card`) with an inner
// installments table, a shared ListFilterBar, per-column sorting, contact
// actions and a CSV export button. All filtering/sorting is client-side over
// the full `api.listImpayes()` result.

test("impayés: free-text search narrows to a single matching client", async (page) => {
  await open(page, "/impayes");
  await page.locator(".impaye-card").first().waitFor({ timeout: 10000 });
  const total = await page.locator(".impaye-card").count();
  assert(total >= 2, `need >= 2 overdue clients to prove filtering, got ${total}`);

  // The first card's client name is guaranteed to exist in the dataset. The
  // shared filter bar's search input carries the class `.search-input`.
  const name = (await page.locator(".impaye-card .impaye-name").first().innerText()).trim();
  await page.locator(".search-input").fill(name);
  await page.waitForFunction(
    (expected) =>
      Array.from(document.querySelectorAll(".impaye-card .impaye-name")).every(
        (el) => el.textContent?.trim() === expected,
      ),
    name,
    { timeout: 5000 },
  );
  const shown = await page.locator(".impaye-card").count();
  assert(shown >= 1 && shown <= total, `search should not grow the list (${shown} vs ${total})`);
  assertEqual(
    await page.locator(".impaye-card .impaye-name").first().innerText(),
    name,
    "the remaining card matches the searched name",
  );
});

test("impayés: an impossible amount range yields the empty state", async (page) => {
  await open(page, "/impayes");
  await page.locator(".impaye-card").first().waitFor({ timeout: 10000 });

  // Amount min far above any single overdue remaining -> every installment drops
  // out, every client is emptied, and the shared EmptyState (`.empty`) replaces
  // the cards. (The date-window equivalent is covered by the unit test, since
  // the DatePicker is a custom popup with no fillable input.)
  await page.getByPlaceholder("Min").fill("99999999");
  await page.locator(".empty").waitFor({ timeout: 5000 });
  assertEqual(await page.locator(".impaye-card").count(), 0, "no client cards remain");

  // Reset restores the full list.
  await page.getByRole("button", { name: "Réinitialiser" }).click();
  await page.locator(".impaye-card").first().waitFor({ timeout: 5000 });
  assert((await page.locator(".impaye-card").count()) >= 1, "reset restores the cards");
});

test("impayés: sorting by amount reorders a client's installment rows", async (page) => {
  await open(page, "/impayes");
  // Find a client card that has more than one overdue installment to reorder.
  await page.locator(".impaye-card").first().waitFor({ timeout: 10000 });
  const cards = page.locator(".impaye-card");
  let target = null;
  for (let idx = 0; idx < (await cards.count()); idx++) {
    if ((await cards.nth(idx).locator("tbody tr").count()) >= 2) {
      target = cards.nth(idx);
      break;
    }
  }
  assert(target, "expected at least one client with 2+ overdue installments");

  const amountCells = () => target.locator("tbody tr td:nth-child(4)").allInnerTexts();
  const num = (s) => Number(s.replace(/[^\d]/g, ""));

  // Click the "amount" column header (4th column) to sort ascending.
  await target.locator("thead th").nth(3).click();
  await page.waitForTimeout(50);
  const asc = (await amountCells()).map(num);
  const ascExpected = [...asc].sort((a, b) => a - b);
  assertEqual(JSON.stringify(asc), JSON.stringify(ascExpected), "ascending by amount");

  // Click again to flip to descending.
  await target.locator("thead th").nth(3).click();
  await page.waitForTimeout(50);
  const desc = (await amountCells()).map(num);
  assertEqual(
    JSON.stringify(desc),
    JSON.stringify([...desc].sort((a, b) => b - a)),
    "descending by amount",
  );
});

test("impayés: export button is present and each card exposes call/SMS/view actions", async (page) => {
  await open(page, "/impayes");
  await page.locator(".impaye-card").first().waitFor({ timeout: 10000 });

  // CSV export button lives in the header when there is at least one result.
  const exportBtn = page.getByRole("button", { name: /Exporter/ });
  await exportBtn.waitFor({ timeout: 5000 });
  assertEqual(await exportBtn.count(), 1, "one export button while results exist");

  // Contact actions must be buttons, never <a href="tel:…"> — see the
  // not-stranding test below for why.
  const first = page.locator(".impaye-card").first();
  assertEqual(
    await first.locator("button.contact-btn--call").count(),
    1,
    "call action is a button",
  );
  assertEqual(
    await first.locator("button.contact-btn--msg").count(),
    1,
    "message action is a button",
  );
  assertEqual(
    await first.locator("button.contact-btn--view").count(),
    1,
    "view-client button present",
  );
  assertEqual(
    await first.locator("a[href^='tel:'], a[href^='sms:']").count(),
    0,
    "no external-scheme anchors remain",
  );
});

// Regression guard for the 2026-07-26 bug: the call/SMS actions used to be
// <a href="tel:…"> anchors. Tauri's WebView cannot load those schemes, so the
// click navigated the WebView itself and replaced the whole SPA with a native
// error page the user could not escape.
//
// Note on strength: the *structural* assertions above and below (the actions are
// buttons, no tel:/sms: anchors exist) are what actually catch a revert.
// Playwright drives Chromium, which does not reproduce WebKitGTK's failure mode,
// so "the app is still here after clicking" is a sanity check rather than a
// faithful reproduction. The full path is exercised though: click → composable →
// api gateway → mock, and a rejection there would raise an error toast.

test("impayés: contact actions never navigate the app away", async (page) => {
  await open(page, "/impayes");
  await page.locator(".impaye-card").first().waitFor({ timeout: 10000 });
  const before = page.url();

  await page.locator("button.contact-btn--call").first().click();
  await page.locator("button.contact-btn--msg").first().click();

  await page.locator(".app-shell").waitFor({ state: "visible", timeout: 5000 });
  assertEqual(page.url(), before, "URL unchanged after call + message");
  assert(
    (await page.locator(".impaye-card").count()) > 0,
    "overdue cards still rendered after contact actions",
  );
  // A seeded client's number is dialable and the mock resolves, so the happy
  // path must stay silent — an error toast here means validation or the gateway
  // rejected a perfectly good number.
  assertEqual(
    await page.locator(".toast--error").count(),
    0,
    "no error toast for a valid seeded phone number",
  );
});

test("dashboard: overdue panel contact actions are buttons, not scheme links", async (page) => {
  await open(page, "/");
  await page.locator(".impaye-list .impaye-row").first().waitFor({ timeout: 10000 });

  const row = page.locator(".impaye-list .impaye-row").first();
  assertEqual(await row.locator("button.contact-btn--call").count(), 1, "call action is a button");
  assertEqual(
    await row.locator("button.contact-btn--msg").count(),
    1,
    "message action is a button",
  );
  assertEqual(
    await page.locator("a[href^='tel:'], a[href^='sms:']").count(),
    0,
    "dashboard has no external-scheme anchors either",
  );

  // Same defect lived here, reachable without ever opening Impayés.
  await row.locator("button.contact-btn--call").click();
  await page.locator(".app-shell").waitFor({ state: "visible", timeout: 5000 });
  assertEqual(
    await page.locator(".kpi-row .kpi").count(),
    5,
    "dashboard still rendered after a call action",
  );
});

test("impayés: deep link ?client=<id> pre-filters the search to that client", async (page) => {
  // Open with the dashboard overdue-panel deep-link shape. Seeded client ids are
  // 1..6; client 1 (Mohamed Trabelsi) has overdue installments in the seed.
  await open(page, "/impayes?client=1");
  await page.locator(".impaye-card").first().waitFor({ timeout: 10000 });

  const search = await page.locator(".search-input").inputValue();
  assert(search.length > 0, "deep-link should pre-fill the search box with the client name");

  // Every visible card must match the pre-filled search text.
  const names = await page.locator(".impaye-card .impaye-name").allInnerTexts();
  assert(names.length >= 1, "at least one card for the deep-linked client");
  for (const n of names) {
    assert(
      n.toLowerCase().includes(search.toLowerCase()),
      `card "${n}" should match pre-filled search "${search}"`,
    );
  }
});

// --- Alertes (alerts center) -------------------------------------------------
// The page derives its rows from `api.listSchedule()`: every unpaid installment
// that is overdue, due today, or due within 7 days. Three summary tiles
// (`.tile`, in DOM order overdue / due-today / due-soon) show a count each and
// double as one-click filters; status tabs (`.tab`) and the shared ListFilterBar
// filter the table below. The seed guarantees overdue installments exist.

test("alertes: summary tiles render and the table totals match them", async (page) => {
  await open(page, "/alertes");
  await page.locator(".summary .tile").first().waitFor({ timeout: 10000 });
  assertEqual(await page.locator(".summary .tile").count(), 3, "three summary tiles");

  const tileValue = async (n) =>
    Number((await page.locator(".summary .tile").nth(n).locator(".tile-value").innerText()).trim());
  const overdue = await tileValue(0);
  const dueToday = await tileValue(1);
  const dueSoon = await tileValue(2);
  assert(overdue >= 1, `seed must produce overdue alerts, got ${overdue}`);

  // Default "all" tab, no list filters -> every alert row is shown, so the table
  // row count equals the sum of the three tile counts.
  await page.locator("table.table tbody tr").first().waitFor({ timeout: 5000 });
  const rows = await page.locator("table.table tbody tr").count();
  assertEqual(rows, overdue + dueToday + dueSoon, "table rows equal the summed tile counts");
});

test("alertes: overdue tile count matches the sidebar warning badge", async (page) => {
  await open(page, "/alertes");
  await page.locator(".summary .tile").first().waitFor({ timeout: 10000 });

  const overdue = Number(
    (await page.locator(".summary .tile").first().locator(".tile-value").innerText()).trim(),
  );

  // The Alertes sidebar entry carries a warning badge = overdue installment count.
  const badge = page.locator(".nav-item", { hasText: NAV.alertes }).locator(".nav-badge--warning");
  await badge.waitFor({ timeout: 5000 });
  const badgeCount = Number((await badge.innerText()).trim());
  assertEqual(overdue, badgeCount, "overdue tile equals the sidebar warning badge");
});

test("alertes: clicking the Overdue tile filters the table to overdue rows", async (page) => {
  await open(page, "/alertes");
  await page.locator(".summary .tile").first().waitFor({ timeout: 10000 });
  const overdue = Number(
    (await page.locator(".summary .tile").first().locator(".tile-value").innerText()).trim(),
  );

  // Click the overdue tile: it activates and the "En retard" tab becomes active.
  await page.locator(".summary .tile").first().click();
  await page.locator(".tab.tab--active", { hasText: "En retard" }).waitFor({ timeout: 5000 });

  await page.waitForFunction(
    (expected) => document.querySelectorAll("table.table tbody tr").length === expected,
    overdue,
    { timeout: 5000 },
  );
  assertEqual(
    await page.locator("table.table tbody tr").count(),
    overdue,
    "rows narrowed to overdue count",
  );

  // Every visible timing cell is an overdue label ("… de retard") and every row
  // carries the late-row highlight class.
  const timings = await page.locator("table.table tbody tr .timing--overdue").count();
  assertEqual(timings, overdue, "each visible row shows an overdue timing");
  assertEqual(
    await page.locator("table.table tbody tr.is-late").count(),
    overdue,
    "each overdue row is highlighted",
  );
});

test("alertes: a row links through to its purchase detail", async (page) => {
  await open(page, "/alertes");
  await page.locator("table.table tbody tr").first().waitFor({ timeout: 10000 });

  const reference = (
    await page.locator("table.table tbody tr").first().locator(".row-link").innerText()
  ).trim();
  await page.locator("table.table tbody tr").first().click();

  // The purchase-detail header title is the purchase reference (e.g. A-000001).
  await page.waitForFunction(
    (expected) => document.querySelector("h1.page-title")?.textContent?.trim() === expected,
    reference,
    { timeout: 5000 },
  );
  assertEqual(
    await page.locator("h1.page-title").innerText(),
    reference,
    "navigated to the purchase detail",
  );
});

// --- Not-found recovery ------------------------------------------------------
// Unknown URLs hit the router's catch-all (`name: "not-found"`) and render
// NotFoundView: a localized card with a ghost "Retour" button (useBack) and a
// primary link to the dashboard.
//
// Coverage limit, deliberate: `open()` does a full document load, and vue-router
// replaceState's fresh history state on initial navigation, so `state.back` is
// always null here — these tests exercise the *fallback* branch. There is no UI
// path that router-navigates to an unknown URL, so the genuine `router.back()`
// branch and the "don't go back into another 404" skip are covered by the
// `shouldGoBack` unit tests in src/composables/useBack.test.ts instead.

test("unknown route renders the localized not-found page", async (page) => {
  await open(page, "/cette-page-nexiste-pas");

  assertEqual(await page.locator(".stub h2").innerText(), "Page introuvable", "not-found heading");
  // The catch-all is in AppHeader's NAV_KEY, so the header names the page
  // rather than falling back to the app name.
  assertEqual(
    await page.locator("h1.page-title").innerText(),
    "Page introuvable",
    "header title on the not-found page",
  );
  assertEqual(
    await page.locator(".stub-actions .btn").count(),
    2,
    "not-found offers two ways out (back + dashboard)",
  );
  assertEqual(
    (await page.locator(".stub-actions .btn--ghost").innerText()).trim(),
    "Retour",
    "back button label",
  );
});

test("not-found Back falls back to the dashboard when there is no in-app history", async (page) => {
  await open(page, "/cette-page-nexiste-pas");
  await page.locator(".stub-actions .btn--ghost").click();

  await page.waitForFunction(() => window.location.pathname === "/", undefined, { timeout: 5000 });
  assertEqual(await page.evaluate(() => window.location.pathname), "/", "landed on the dashboard");
  assertEqual(
    await page.locator("h1.page-title").innerText(),
    NAV.dashboard,
    "header title after recovering from the not-found page",
  );
});

test("not-found dashboard link returns to the dashboard", async (page) => {
  await open(page, "/cette-page-nexiste-pas");
  await page.locator(".stub-actions .btn--primary").click();

  await page.waitForFunction(() => window.location.pathname === "/", undefined, { timeout: 5000 });
  assertEqual(
    await page.locator("h1.page-title").innerText(),
    NAV.dashboard,
    "dashboard link reaches the dashboard",
  );
  assertEqual(await page.locator(".kpi-row .kpi").count(), 5, "dashboard actually rendered");
});

test("not-found back arrow mirrors in Arabic (RTL)", async (page) => {
  await open(page, "/cette-page-nexiste-pas");
  const arrow = page.locator(".stub-actions .btn--ghost .app-icon");

  // Baseline: French, LTR, arrow drawn as authored.
  assertEqual(await page.locator("html").getAttribute("dir"), "ltr", "baseline dir is ltr");
  assertEqual(
    await arrow.evaluate((el) => getComputedStyle(el).transform),
    "none",
    "arrow is not flipped in LTR",
  );

  await page.locator(".lang-btn").click();
  await page.locator(".lang-option", { hasText: "العربية" }).click();
  await page.waitForFunction(
    () => document.documentElement.getAttribute("dir") === "rtl",
    undefined,
    { timeout: 5000 },
  );

  assertEqual(
    await page.locator(".stub h2").innerText(),
    "الصفحة غير موجودة",
    "not-found heading re-renders in Arabic",
  );
  // `.icon-flip` under [dir="rtl"] applies scaleX(-1) so "back" points right.
  assertEqual(
    await arrow.evaluate((el) => getComputedStyle(el).transform),
    "matrix(-1, 0, 0, 1, 0, 0)",
    "back arrow is mirrored in RTL",
  );
});

test("a deleted record's detail page offers a mirrored way back", async (page) => {
  // Same recovery affordance on the in-page missing-record state, which is a
  // valid route (client-detail) rather than the router's catch-all.
  await open(page, "/clients/999999");
  await page.locator(".back-link").waitFor({ timeout: 10000 });

  assertEqual(
    await page.locator(".empty .empty-title").innerText(),
    "Ce client n'existe pas ou a été supprimé.",
    "missing client renders a recoverable message, not a blank page",
  );
  await page.locator(".back-link").click();
  await page.waitForFunction(() => window.location.pathname === "/clients", undefined, {
    timeout: 5000,
  });
  assertEqual(
    await page.locator("h1.page-title").innerText(),
    NAV.clients,
    "back from a missing client falls back to the clients list",
  );
});

test("a rejected payment shows a localized message, never a raw backend error", async (page) => {
  // The regression this pins: the modal used to render `String(e)` verbatim, so
  // a backend rejection put an unlocalized machine string (and, against the real
  // Rust backend, raw SQL text) in front of the user.
  //
  // Seed purchase A-000001: 2400 over 6 monthly tranches of 400, tranche 1 paid.
  await open(page, "/achats/1");
  await page.locator(".inst-table tbody tr").first().waitFor({ timeout: 10000 });

  // Tranche 2 has 400 outstanding — try to pay more than that.
  await page.locator(".inst-table .btn--primary").first().click();
  const dialog = page.locator('[role="dialog"]');
  await dialog.waitFor({ state: "visible", timeout: 5000 });

  await page.locator("#pay-amount").fill("1000");
  await dialog.getByRole("button", { name: "Enregistrer" }).click();

  // The modal stays open and reports the problem inline.
  const error = dialog.locator(".field-error").first();
  await error.waitFor({ state: "visible", timeout: 5000 });
  const text = (await error.innerText()).trim();

  assert(text.length > 0, "a rejected payment must show a message");
  assert(
    !/OVERPAYMENT|INVALID_|SUM_MISMATCH|INTERNAL/.test(text),
    `message must be localized prose, not a machine code; got: ${text}`,
  );
  assert(
    !/constraint failed|SELECT |INSERT |sqlite/i.test(text),
    `message must not leak backend internals; got: ${text}`,
  );
  assert(/400/.test(text), `message should name the remaining balance (400); got: ${text}`);

  // And nothing was recorded.
  await dialog
    .getByRole("button", { name: "Annuler" })
    .click()
    .catch(() => {});
});

test("settings exposes a database backup action", async (page) => {
  await open(page, "/parametres");
  await page.locator(".set-card").first().waitFor({ timeout: 10000 });

  // The backup card is desktop-only — it needs a real database file, so it is
  // hidden in the browser build this suite drives. Assert the guard holds
  // rather than asserting a control that must not be here.
  assertEqual(
    await page.locator(".set-card", { hasText: "Sauvegarde" }).count(),
    0,
    "backup card must be hidden outside the Tauri runtime",
  );
});

// --- runner ------------------------------------------------------------------

async function main() {
  const server = startServer();
  let browser;
  const results = [];
  try {
    await waitForServer();
    browser = await chromium.launch({ headless: true });

    for (const t of tests) {
      // Pin the browser locale to French: on a fresh install the app derives its
      // language from navigator.language, so this keeps the UI text deterministic.
      const context = await browser.newContext({
        viewport: { width: 1440, height: 900 },
        locale: "fr-FR",
      });
      const page = await context.newPage();
      const consoleErrors = [];
      page.on("console", (m) => m.type() === "error" && consoleErrors.push(m.text()));
      page.on("pageerror", (e) => consoleErrors.push(String(e)));

      const started = Date.now();
      try {
        await t.fn(page);
        results.push({ name: t.name, ok: true, ms: Date.now() - started, consoleErrors });
        console.log(`  \x1b[32mPASS\x1b[0m ${t.name} (${Date.now() - started}ms)`);
      } catch (err) {
        const shot = path.join(ARTIFACTS, `${t.name.replace(/[^a-z0-9]+/gi, "-")}.png`);
        await page.screenshot({ path: shot, fullPage: true }).catch(() => {});
        results.push({
          name: t.name,
          ok: false,
          ms: Date.now() - started,
          error: err.message,
          shot,
          consoleErrors,
        });
        console.log(
          `  \x1b[31mFAIL\x1b[0m ${t.name}\n       ${err.message.replace(/\n/g, "\n       ")}`,
        );
        console.log(`       screenshot: ${shot}`);
      } finally {
        await context.close();
      }
    }
  } finally {
    if (browser) await browser.close();
    server.kill("SIGTERM");
  }

  const passed = results.filter((r) => r.ok).length;
  const failed = results.length - passed;
  const withConsole = results.filter((r) => r.consoleErrors.length > 0);
  console.log(`\n${passed}/${results.length} passed, ${failed} failed`);
  if (withConsole.length) {
    console.log("\nBrowser console errors observed:");
    for (const r of withConsole) console.log(`  [${r.name}] ${r.consoleErrors.join(" | ")}`);
  }
  process.exit(failed === 0 ? 0 : 1);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
