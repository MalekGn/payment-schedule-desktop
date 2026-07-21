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
// Usage: node e2e/run.mjs   (spawns Vite itself, tears it down on exit)

import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// Dedicated port so the suite never collides with (or accidentally tests) a
// dev server the user already has running on Vite's default 5173.
const PORT = 5199;
const BASE = `http://localhost:${PORT}`;
const ARTIFACTS = path.join(ROOT, "e2e", "artifacts");
mkdirSync(ARTIFACTS, { recursive: true });

// --- tiny test harness -------------------------------------------------------

const tests = [];
const test = (name, fn) => tests.push({ name, fn });

function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}
function assertEqual(actual, expected, msg) {
  if (actual !== expected) {
    throw new Error(`${msg}\n    expected: ${JSON.stringify(expected)}\n    actual:   ${JSON.stringify(actual)}`);
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
  assertEqual(await page.locator("h1.page-title").innerText(), NAV.dashboard, "header title on dashboard");
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
  assertEqual(await page.locator("#pay-amount").inputValue(), "400", "amount pre-filled with remaining");
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

  await page.getByRole("button", { name: "Nouvel achat" }).click();
  const dialog = page.locator('[role="dialog"]');
  await dialog.waitFor({ state: "visible", timeout: 5000 });
  assertEqual(await dialog.locator(".modal-head h2").innerText(), "Nouvel achat", "modal title");

  await page.locator("#np-client").selectOption("1");
  await page.locator("#np-product").fill("Aspirateur Dyson");
  await page.locator("#np-total").fill("3000");
  await page.locator("#np-count").fill("3");

  // Amounts auto-split 3000 / 3 = 1000 each and the running sum matches the total.
  await page.waitForFunction(
    () => document.querySelectorAll(".inst-row").length === 3,
    undefined,
    { timeout: 5000 },
  );
  await page.locator(".inst-sum.ok").waitFor({ timeout: 5000 });
  assertEqual(await page.locator(".inst-amount").first().inputValue(), "1000", "first tranche auto-split to 1000");

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
  assertEqual(await page.locator("h1.page-title").innerText(), "A-000009", "navigated to new purchase detail");
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
  assertEqual(await dialog.locator(".modal-head h2").innerText(), "Supprimer le client", "confirm dialog title");
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
  assertEqual(await page.locator(".brand-line1").innerText(), "الدفع", "brand re-renders in Arabic");
  assertEqual(
    await page.locator(".nav-item", { hasText: "لوحة التحكم" }).count(),
    1,
    "dashboard nav label is Arabic",
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
        results.push({ name: t.name, ok: false, ms: Date.now() - started, error: err.message, shot, consoleErrors });
        console.log(`  \x1b[31mFAIL\x1b[0m ${t.name}\n       ${err.message.replace(/\n/g, "\n       ")}`);
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
