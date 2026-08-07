// In-memory mock backend used when the app runs in a plain browser (no Tauri
// runtime): dev-server preview, screenshots, and unit/integration tests.
// It mirrors the behaviour of src-tauri/src/commands.rs closely enough to
// exercise every screen. The real desktop app always uses the Rust backend.

import {
  addInterval,
  dayDiff,
  installmentStatus,
  purchaseStatus,
  splitAmounts,
  todayIso,
} from "@/lib/finance";
import type {
  Client,
  ClientDetail,
  ClientInput,
  ClientScope,
  ClientSummary,
  Dashboard,
  DueAlert,
  ImpayeClient,
  ImpayeFilter,
  Installment,
  InstallmentEdit,
  LicenseInfo,
  LicenseStatusTag,
  Payment,
  PaymentInput,
  PurchaseDetail,
  PurchaseInput,
  PurchaseScope,
  PurchaseSummary,
  ScheduleRow,
  Settings,
  SettingsPatch,
} from "@/types/models";

// --- validation, mirroring src-tauri/src/commands.rs -----------------------
//
// These bounds are not cosmetic on the Rust side: `installmentCount` sizes an
// allocation and an insert loop, and `intervalDays` is multiplied by the
// installment index before reaching date math. The mock has to reject the same
// inputs with the same codes, otherwise the integration and E2E suites would
// pass against behaviour the real backend does not have.

const INTERVAL_KINDS = ["weekly", "monthly", "custom"];
const INSTALLMENT_COUNT_MAX = 120;
const INTERVAL_DAYS_MIN = 1;
const INTERVAL_DAYS_MAX = 365;
// Mirrors `MONEY_RANGE` in db.rs. The cap is not about plausible shop prices: it
// keeps any sum of at most INSTALLMENT_COUNT_MAX terms far below the i64 range
// the Rust side computes it in, so a wrapping sum can never satisfy the
// SUM_MISMATCH equality that is supposed to prove the schedule adds up.
const MONEY_MIN = 0;
const MONEY_MAX = 1_000_000_000;
// Mirrors SHORT_TEXT_MAX / LONG_TEXT_MAX and the three vocabularies in db.rs.
// Counted in code points, matching the Rust side's `chars()` — a byte cap would
// give French and Arabic users a different limit from an ASCII one.
const SHORT_TEXT_MAX = 120;
const LONG_TEXT_MAX = 500;
const PAYMENT_LIMIT_MIN = 1;
const PAYMENT_LIMIT_MAX = 5000;
const LANGUAGES = ["fr", "en", "ar"];
const CURRENCY_CODES = ["TND", "EUR", "USD", "FCFA", "DZD", "MAD"];
const DATE_FORMAT_VALUES = ["dd/MM/yyyy", "MM/dd/yyyy", "yyyy-MM-dd", "dd-MM-yyyy"];
const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

/** Throw `TEXT_TOO_LONG:{max}` when `value` exceeds `max` code points. */
function assertBounded(value: string, max: number): void {
  if ([...value].length > max) throw new Error(`TEXT_TOO_LONG:${max}`);
}

/** Throw `TEXT_REQUIRED` when a field that must carry something is empty. */
function assertRequired(value: string): void {
  if (value === "") throw new Error("TEXT_REQUIRED");
}

/** Mirrors `validate_client_input` in commands.rs, guard for guard. */
function validateClientInput(input: ClientInput): void {
  const first = input.firstName.trim();
  const last = input.lastName.trim();
  assertRequired(first);
  assertRequired(last);
  assertBounded(first, SHORT_TEXT_MAX);
  assertBounded(last, SHORT_TEXT_MAX);
  assertBounded(input.phone.trim(), SHORT_TEXT_MAX);
  assertBounded(input.address.trim(), LONG_TEXT_MAX);
  if (input.email != null) assertBounded(input.email.trim(), SHORT_TEXT_MAX);
}

/** Throw `INVALID_DATE` unless `value` is a real `YYYY-MM-DD` calendar date. */
function assertIsoDate(value: string): void {
  if (!ISO_DATE.test(value) || Number.isNaN(Date.parse(`${value}T00:00:00Z`))) {
    throw new Error("INVALID_DATE");
  }
}

function validatePurchaseInput(input: PurchaseInput): void {
  if (input.totalPrice <= 0 || input.totalPrice > MONEY_MAX) {
    throw new Error("INVALID_TOTAL_PRICE");
  }
  assertBounded(input.productLabel.trim(), SHORT_TEXT_MAX);
  if (input.installmentCount < 1 || input.installmentCount > INSTALLMENT_COUNT_MAX) {
    throw new Error("INVALID_INSTALLMENT_COUNT");
  }
  if (!INTERVAL_KINDS.includes(input.intervalKind)) throw new Error("INVALID_INTERVAL_KIND");
  if (input.intervalKind === "custom") {
    const days = input.intervalDays ?? 30;
    if (days < INTERVAL_DAYS_MIN || days > INTERVAL_DAYS_MAX) {
      throw new Error("INVALID_INTERVAL_DAYS");
    }
  }
  assertIsoDate(input.purchaseDate);
}

interface ClientRow {
  id: number;
  firstName: string;
  lastName: string;
  phone: string;
  address: string;
  email: string | null;
  createdAt: string;
  archivedAt: string | null;
}
interface PurchaseRow {
  id: number;
  reference: string;
  clientId: number;
  productLabel: string;
  totalPrice: number;
  installmentCount: number;
  intervalKind: "weekly" | "monthly" | "custom";
  intervalDays: number | null;
  purchaseDate: string;
  createdAt: string;
  archivedAt: string | null;
}
interface InstallmentRow {
  id: number;
  purchaseId: number;
  index: number;
  amount: number;
  dueDate: string;
  paidAmount: number;
  paidDate: string | null;
}
interface PaymentRow {
  id: number;
  installmentId: number;
  amount: number;
  paymentDate: string;
  note: string | null;
  createdAt: string;
}

class MockDb {
  clients: ClientRow[] = [];
  purchases: PurchaseRow[] = [];
  installments: InstallmentRow[] = [];
  payments: PaymentRow[] = [];
  settings: Record<string, string> = {};
  /** Last URI passed to `openExternal`, for assertions in tests. */
  lastExternalUrl: string | null = null;
  lastBackupPath: string | null = null;
  /** See `getLicenseStatus` for why this starts valid. */
  private license: LicenseInfo = {
    status: "valid",
    license: null,
    expiredOn: null,
    machineId: null,
  };
  private seq = { client: 0, purchase: 0, installment: 0, payment: 0 };

  constructor() {
    this.seed();
    this.setLicense("valid");
  }

  private nextId(k: keyof typeof this.seq): number {
    this.seq[k] += 1;
    return this.seq[k];
  }

  private seed() {
    // Every seeded client starts active; the archive scenarios archive one
    // themselves, so row-count assertions elsewhere stay stable at 6.
    const clients: Omit<ClientRow, "id" | "createdAt" | "archivedAt">[] = [
      {
        firstName: "Mohamed",
        lastName: "Trabelsi",
        phone: "+216 20 123 456",
        address: "Cité El Ghazala, Ariana",
        email: "mohamed.trabelsi@email.tn",
      },
      {
        firstName: "Fatma",
        lastName: "Ben Salah",
        phone: "+216 22 345 678",
        address: "Avenue Habib Bourguiba, Tunis",
        email: "fatma.bensalah@email.tn",
      },
      {
        firstName: "Ahmed",
        lastName: "Gharbi",
        phone: "+216 24 567 890",
        address: "Rue de Marseille, Sfax",
        email: null,
      },
      {
        firstName: "Salma",
        lastName: "Jlassi",
        phone: "+216 26 789 012",
        address: "Menzah 6, Tunis",
        email: "salma.jlassi@email.tn",
      },
      {
        firstName: "Youssef",
        lastName: "Hamdi",
        phone: "+216 28 901 234",
        address: "Médina, Sousse",
        email: "youssef.hamdi@email.tn",
      },
      {
        firstName: "Nour",
        lastName: "Khelifi",
        phone: "+216 29 012 345",
        address: "La Marsa, Tunis",
        email: "nour.khelifi@email.tn",
      },
    ];
    const clientIds = clients.map((c) => {
      const id = this.nextId("client");
      this.clients.push({ ...c, id, createdAt: todayIso(), archivedAt: null });
      return id;
    });

    const purchases = [
      {
        ci: 0,
        product: "Réfrigérateur Samsung 260L",
        total: 2400,
        count: 6,
        monthsAgo: 5,
        paid: 1,
      },
      { ci: 1, product: "Machine à laver LG 8kg", total: 1800, count: 5, monthsAgo: 4, paid: 2 },
      { ci: 2, product: 'Téléviseur Smart 55"', total: 3200, count: 8, monthsAgo: 6, paid: 3 },
      { ci: 3, product: "Cuisinière 4 feux", total: 1200, count: 4, monthsAgo: 4, paid: 4 },
      { ci: 4, product: "Climatiseur 1.5 CV", total: 2100, count: 6, monthsAgo: 3, paid: 1 },
      { ci: 5, product: "Congélateur 200L", total: 1500, count: 5, monthsAgo: 1, paid: 1 },
      { ci: 0, product: "Four électrique", total: 900, count: 3, monthsAgo: 0, paid: 0 },
      { ci: 1, product: "Lave-vaisselle Bosch", total: 1600, count: 4, monthsAgo: 2, paid: 1 },
    ];

    for (const p of purchases) {
      const purchaseDate = addInterval(todayIso(), "monthly", null, -p.monthsAgo);
      const id = this.nextId("purchase");
      this.purchases.push({
        id,
        reference: `A-${String(id).padStart(6, "0")}`,
        clientId: clientIds[p.ci],
        productLabel: p.product,
        totalPrice: p.total,
        installmentCount: p.count,
        intervalKind: "monthly",
        intervalDays: null,
        purchaseDate,
        createdAt: todayIso(),
        archivedAt: null,
      });
      const amounts = splitAmounts(p.total, p.count);
      amounts.forEach((amount, i) => {
        const idx = i + 1;
        const due = addInterval(purchaseDate, "monthly", null, i);
        const fullyPaid = idx <= p.paid;
        const instId = this.nextId("installment");
        this.installments.push({
          id: instId,
          purchaseId: id,
          index: idx,
          amount,
          dueDate: due,
          paidAmount: fullyPaid ? amount : 0,
          paidDate: fullyPaid ? due : null,
        });
        if (fullyPaid) {
          this.payments.push({
            id: this.nextId("payment"),
            installmentId: instId,
            amount,
            paymentDate: due,
            note: null,
            createdAt: due,
          });
        }
      });
    }

    this.settings = {
      language: "fr",
      language_is_default: "1",
      currency_code: "TND",
      date_format: "dd/MM/yyyy",
      shop_name: "Électro Ménager",
      shop_info: "",
      logo_path: "",
      alert_soon_days: "7",
    };
  }

  // ---- builders -----------------------------------------------------------

  private clientOut(r: ClientRow): Client {
    return { ...r };
  }

  private loadInstallments(purchaseId: number): Installment[] {
    const today = todayIso();
    return this.installments
      .filter((i) => i.purchaseId === purchaseId)
      .sort((a, b) => a.index - b.index)
      .map((i) => ({
        id: i.id,
        purchaseId: i.purchaseId,
        index: i.index,
        amount: i.amount,
        dueDate: i.dueDate,
        paidAmount: i.paidAmount,
        paidDate: i.paidDate,
        status: installmentStatus(i.amount, i.paidAmount, i.dueDate, today),
      }));
  }

  buildPurchaseDetail(purchaseId: number): PurchaseDetail {
    const p = this.purchases.find((x) => x.id === purchaseId);
    if (!p) throw new Error("PURCHASE_NOT_FOUND");
    const client = this.clients.find((c) => c.id === p.clientId)!;
    const installments = this.loadInstallments(purchaseId);
    const totalPaid = installments.reduce((s, i) => s + i.paidAmount, 0);
    const remaining = Math.max(0, p.totalPrice - totalPaid);
    const status = purchaseStatus(
      installments.map((i) => i.status),
      totalPaid > 0,
    );
    return {
      purchase: { ...p },
      client: this.clientOut(client),
      installments,
      totalPaid,
      remaining,
      status,
    };
  }

  buildPurchaseSummary(purchaseId: number): PurchaseSummary {
    const d = this.buildPurchaseDetail(purchaseId);
    return {
      id: d.purchase.id,
      reference: d.purchase.reference,
      clientId: d.purchase.clientId,
      clientName: `${d.client.firstName} ${d.client.lastName}`,
      productLabel: d.purchase.productLabel,
      totalPrice: d.purchase.totalPrice,
      paidAmount: d.totalPaid,
      remaining: d.remaining,
      installmentCount: d.purchase.installmentCount,
      purchaseDate: d.purchase.purchaseDate,
      status: d.status,
      overdueCount: d.installments.filter((i) => i.status === "late").length,
      archivedAt: d.purchase.archivedAt,
    };
  }

  /**
   * Purchases that still count. Archiving removes a purchase from every money
   * view — unlike an archived *client*, who is settled and so contributes
   * nothing either way. Every aggregate below goes through this or `isLive`.
   */
  private livePurchases(): PurchaseRow[] {
    return this.purchases.filter((p) => p.archivedAt === null);
  }

  /** Whether an installment belongs to a purchase that still counts. */
  private isLive(inst: InstallmentRow): boolean {
    return this.purchases.find((p) => p.id === inst.purchaseId)?.archivedAt === null;
  }

  private buildImpayes(filter: ImpayeFilter, limit?: number): ImpayeClient[] {
    const today = todayIso();
    const map = new Map<number, ImpayeClient>();
    const order: number[] = [];
    const overdue = this.installments
      .filter((i) => this.isLive(i) && dayDiff(i.dueDate, today) < 0 && i.amount > i.paidAmount)
      .sort((a, b) => a.dueDate.localeCompare(b.dueDate));

    for (const inst of overdue) {
      const purchase = this.purchases.find((p) => p.id === inst.purchaseId)!;
      const client = this.clients.find((c) => c.id === purchase.clientId)!;
      if (filter.clientId && client.id !== filter.clientId) continue;
      if (filter.dateFrom && inst.dueDate < filter.dateFrom) continue;
      if (filter.dateTo && inst.dueDate > filter.dateTo) continue;

      if (!map.has(client.id)) {
        order.push(client.id);
        map.set(client.id, {
          clientId: client.id,
          clientName: `${client.firstName} ${client.lastName}`,
          phone: client.phone,
          address: client.address,
          email: client.email,
          reference: purchase.reference,
          totalOverdue: 0,
          overdueCount: 0,
          installments: [],
        });
      }
      const entry = map.get(client.id)!;
      const remaining = inst.amount - inst.paidAmount;
      entry.totalOverdue += remaining;
      entry.overdueCount += 1;
      entry.installments.push({
        installmentId: inst.id,
        purchaseId: purchase.id,
        purchaseReference: purchase.reference,
        index: inst.index,
        installmentCount: purchase.installmentCount,
        dueDate: inst.dueDate,
        amount: inst.amount,
        remaining,
        daysLate: -dayDiff(inst.dueDate, today),
      });
    }
    let result = order.map((id) => map.get(id)!);
    result = result.sort((a, b) => b.totalOverdue - a.totalOverdue);
    return limit ? result.slice(0, limit) : result;
  }

  // ---- commands -----------------------------------------------------------

  listClients(scope: ClientScope = "active"): ClientSummary[] {
    const today = todayIso();
    return this.clients
      .filter((c) => (scope === "all" ? true : (scope === "archived") === (c.archivedAt !== null)))
      .sort((a, b) => `${a.lastName}${a.firstName}`.localeCompare(`${b.lastName}${b.firstName}`))
      .map((c) => {
        const purchases = this.livePurchases().filter((p) => p.clientId === c.id);
        const insts = this.installments.filter((i) => purchases.some((p) => p.id === i.purchaseId));
        const outstanding = insts.reduce((s, i) => s + (i.amount - i.paidAmount), 0);
        const overdue = insts.filter(
          (i) => dayDiff(i.dueDate, today) < 0 && i.amount > i.paidAmount,
        ).length;
        return {
          ...this.clientOut(c),
          purchaseCount: purchases.length,
          totalOutstanding: outstanding,
          overdueCount: overdue,
        };
      });
  }

  getClientDetail(id: number): ClientDetail {
    const client = this.clients.find((c) => c.id === id);
    if (!client) throw new Error("CLIENT_NOT_FOUND");
    const all = this.purchases
      .filter((p) => p.clientId === id)
      .sort((a, b) => b.purchaseDate.localeCompare(a.purchaseDate) || b.id - a.id)
      .map((p) => this.buildPurchaseSummary(p.id));
    // Archived purchases are listed separately and counted in no total —
    // the client no longer owes them.
    const purchases = all.filter((p) => p.archivedAt === null);
    const archivedPurchases = all.filter((p) => p.archivedAt !== null);
    const totalPurchased = purchases.reduce((s, p) => s + p.totalPrice, 0);
    const totalPaid = purchases.reduce((s, p) => s + p.paidAmount, 0);
    const overdueCount = purchases.reduce((s, p) => s + p.overdueCount, 0);
    return {
      client: this.clientOut(client),
      purchases,
      archivedPurchases,
      totalPurchased,
      totalPaid,
      totalOutstanding: Math.max(0, totalPurchased - totalPaid),
      overdueCount,
    };
  }

  createClient(input: ClientInput): Client {
    validateClientInput(input);
    const id = this.nextId("client");
    const row: ClientRow = {
      id,
      firstName: input.firstName.trim(),
      lastName: input.lastName.trim(),
      phone: input.phone.trim(),
      address: input.address.trim(),
      email: input.email?.trim() || null,
      createdAt: todayIso(),
      archivedAt: null,
    };
    this.clients.push(row);
    return this.clientOut(row);
  }

  updateClient(id: number, input: ClientInput): Client {
    validateClientInput(input);
    const row = this.clients.find((c) => c.id === id);
    if (!row) throw new Error("CLIENT_NOT_FOUND");
    Object.assign(row, {
      firstName: input.firstName.trim(),
      lastName: input.lastName.trim(),
      phone: input.phone.trim(),
      address: input.address.trim(),
      email: input.email?.trim() || null,
    });
    return this.clientOut(row);
  }

  /** Total still owed across every purchase of `clientId`; 0 when they have none. */
  private clientOutstanding(clientId: number): number {
    const purchaseIds = this.purchases.filter((p) => p.clientId === clientId).map((p) => p.id);
    return this.installments
      .filter((i) => purchaseIds.includes(i.purchaseId))
      .reduce((s, i) => s + (i.amount - i.paidAmount), 0);
  }

  archiveClient(id: number): void {
    const row = this.clients.find((c) => c.id === id);
    if (!row) throw new Error("CLIENT_NOT_FOUND");
    const outstanding = this.clientOutstanding(id);
    if (outstanding > 0) throw new Error(`ARCHIVE_HAS_OUTSTANDING:${outstanding}`);
    // Re-archiving must not move the stamp — see `archive_client_impl`.
    row.archivedAt ??= todayIso();
  }

  restoreClient(id: number): void {
    const row = this.clients.find((c) => c.id === id);
    if (!row) throw new Error("CLIENT_NOT_FOUND");
    row.archivedAt = null;
  }

  deleteClient(id: number): void {
    const row = this.clients.find((c) => c.id === id);
    if (!row) throw new Error("CLIENT_NOT_FOUND");
    const count = this.purchases.filter((p) => p.clientId === id).length;
    if (count > 0) throw new Error(`CLIENT_HAS_PURCHASES:${count}`);
    // No cascade to mirror: a client that reaches here has nothing attached.
    this.clients = this.clients.filter((c) => c.id !== id);
  }

  listPurchases(scope: PurchaseScope = "active", search?: string): PurchaseSummary[] {
    const needle = search?.trim().toLowerCase();
    return this.purchases
      .filter((p) => (scope === "all" ? true : (scope === "archived") === (p.archivedAt !== null)))
      .sort((a, b) => b.purchaseDate.localeCompare(a.purchaseDate) || b.id - a.id)
      .map((p) => this.buildPurchaseSummary(p.id))
      .filter((s) =>
        !needle
          ? true
          : `${s.reference} ${s.clientName} ${s.productLabel}`.toLowerCase().includes(needle),
      );
  }

  getPurchaseDetail(id: number): PurchaseDetail {
    return this.buildPurchaseDetail(id);
  }

  /**
   * Resolve a request into the installment amounts and due dates to write.
   *
   * Mirrors `resolve_schedule` in commands.rs, and is shared by create and
   * update for the same reason: a rescheduling edit must produce exactly what
   * creating the same purchase from scratch would. Runs before any mutation,
   * so a rejected request leaves nothing half-written — the mock's stand-in
   * for the Rust side's transaction.
   */
  private resolveSchedule(input: PurchaseInput): { amounts: number[]; dueDates: string[] } {
    let amounts: number[];
    let dueDates: string[];
    if (input.installments && input.installments.length > 0) {
      // The list, not `installmentCount`, is what sizes the schedule — so the
      // 1..=120 bound only binds if the two agree. See `resolve_schedule`.
      if (input.installments.length !== input.installmentCount) {
        throw new Error(
          `INSTALLMENT_COUNT_MISMATCH:${input.installments.length}:${input.installmentCount}`,
        );
      }
      // Before the sum, because the sum is the thing being protected.
      if (input.installments.some((i) => i.amount < MONEY_MIN || i.amount > MONEY_MAX)) {
        throw new Error("INVALID_AMOUNT");
      }
      const sum = input.installments.reduce((s, i) => s + i.amount, 0);
      if (sum !== input.totalPrice) throw new Error(`SUM_MISMATCH:${sum}:${input.totalPrice}`);
      amounts = input.installments.map((i) => i.amount);
      dueDates = input.installments.map((i) => {
        assertIsoDate(i.dueDate);
        return i.dueDate;
      });
    } else {
      amounts = splitAmounts(input.totalPrice, input.installmentCount);
      dueDates = amounts.map((_, i) =>
        addInterval(input.purchaseDate, input.intervalKind, input.intervalDays, i),
      );
    }
    // Position order and chronological order have to stay the same thing; only
    // a hand-edited schedule can break it.
    if (dueDates.some((due, i) => i > 0 && dueDates[i - 1] > due)) {
      throw new Error("DUE_DATE_OUT_OF_ORDER");
    }
    return { amounts, dueDates };
  }

  createPurchase(input: PurchaseInput): PurchaseDetail {
    validatePurchaseInput(input);

    // Mirrors `create_purchase_impl`: an archived client must not take on new
    // debt, which is what keeps "archived implies a zero balance" true.
    const client = this.clients.find((c) => c.id === input.clientId);
    if (!client) throw new Error("CLIENT_NOT_FOUND");
    if (client.archivedAt !== null) throw new Error("CLIENT_ARCHIVED");

    const { amounts, dueDates } = this.resolveSchedule(input);

    const id = this.nextId("purchase");
    this.purchases.push({
      id,
      reference: `A-${String(id).padStart(6, "0")}`,
      clientId: input.clientId,
      productLabel: input.productLabel.trim(),
      totalPrice: input.totalPrice,
      installmentCount: input.installmentCount,
      intervalKind: input.intervalKind,
      intervalDays: input.intervalDays,
      purchaseDate: input.purchaseDate,
      createdAt: todayIso(),
      archivedAt: null,
    });

    amounts.forEach((amount, i) => {
      this.installments.push({
        id: this.nextId("installment"),
        purchaseId: id,
        index: i + 1,
        amount,
        dueDate: dueDates[i],
        paidAmount: 0,
        paidDate: null,
      });
    });
    return this.buildPurchaseDetail(id);
  }

  /** Payments recorded against any installment of `purchaseId`. */
  private purchasePaymentCount(purchaseId: number): number {
    const instIds = this.installments.filter((i) => i.purchaseId === purchaseId).map((i) => i.id);
    return this.payments.filter((p) => instIds.includes(p.installmentId)).length;
  }

  /**
   * Mirrors `schedule_changed` in commands.rs: compares the *resolved* rows
   * against what is stored, because the editor always sends the installments
   * it is displaying — a label-only edit carries an identical list.
   */
  private scheduleChanged(
    row: PurchaseRow,
    input: PurchaseInput,
    amounts: number[],
    dueDates: string[],
  ): boolean {
    if (
      row.totalPrice !== input.totalPrice ||
      row.installmentCount !== input.installmentCount ||
      row.intervalKind !== input.intervalKind ||
      row.intervalDays !== input.intervalDays ||
      row.purchaseDate !== input.purchaseDate
    ) {
      return true;
    }
    const current = this.installments
      .filter((i) => i.purchaseId === row.id)
      .sort((a, b) => a.index - b.index);
    return (
      current.length !== amounts.length ||
      current.some((inst, i) => inst.amount !== amounts[i] || inst.dueDate !== dueDates[i])
    );
  }

  /** The installment rows of `purchaseId`, in position order. */
  private rowsOf(purchaseId: number): InstallmentRow[] {
    return this.installments
      .filter((i) => i.purchaseId === purchaseId)
      .sort((a, b) => a.index - b.index);
  }

  /**
   * Mirrors the guard half of `apply_schedule_in_place` in commands.rs: whether
   * a resolved schedule may replace the stored rows, position by position.
   *
   * A settled row is history and the incoming schedule has to agree with it; no
   * row may fall below what it has collected; and a row may only be dropped
   * while it has no ledger history, because the payment rows hang off it.
   */
  private assertScheduleApplies(purchaseId: number, amounts: number[], dueDates: string[]): void {
    const rows = this.rowsOf(purchaseId);
    for (let i = 0; i < Math.min(rows.length, amounts.length); i++) {
      const row = rows[i];
      if (row.paidAmount >= row.amount) {
        if (amounts[i] !== row.amount) throw new Error("AMOUNT_LOCKED");
        if (dueDates[i] !== row.dueDate) throw new Error("DUE_DATE_LOCKED");
      }
      if (amounts[i] < row.paidAmount) throw new Error(`BELOW_PAID:${row.paidAmount}`);
    }
    // Counted from the ledger, not from `paidAmount`: a row corrected back down
    // to zero still holds the entries that took the money and gave it back.
    const droppedWithHistory = rows
      .slice(amounts.length)
      .filter((r) => this.payments.some((p) => p.installmentId === r.id)).length;
    if (droppedWithHistory > 0) throw new Error(`PURCHASE_HAS_PAYMENTS:${droppedWithHistory}`);
  }

  /**
   * Mirrors the write half of `apply_schedule_in_place`: update the surviving
   * rows in place, drop the surplus, append what is new. Updating rather than
   * regenerating is what keeps the payment ledger attached.
   */
  private applyScheduleInPlace(purchaseId: number, amounts: number[], dueDates: string[]): void {
    const rows = this.rowsOf(purchaseId);
    rows.slice(0, amounts.length).forEach((row, i) => {
      const amountMoved = row.amount !== amounts[i];
      row.amount = amounts[i];
      row.dueDate = dueDates[i];
      // `paidDate` is derived from the amount as much as from the ledger.
      if (amountMoved) this.syncPaidDate(row);
    });
    const dropped = new Set(rows.slice(amounts.length).map((r) => r.id));
    if (dropped.size > 0) this.installments = this.installments.filter((i) => !dropped.has(i.id));
    for (let i = rows.length; i < amounts.length; i++) {
      this.installments.push({
        id: this.nextId("installment"),
        purchaseId,
        index: i + 1,
        amount: amounts[i],
        dueDate: dueDates[i],
        paidAmount: 0,
        paidDate: null,
      });
    }
  }

  updatePurchase(id: number, input: PurchaseInput): PurchaseDetail {
    validatePurchaseInput(input);
    const row = this.purchases.find((p) => p.id === id);
    if (!row) throw new Error("PURCHASE_NOT_FOUND");
    if (row.archivedAt !== null) throw new Error("PURCHASE_ARCHIVED");

    // Resolve before mutating, so a rejected request changes nothing — the
    // mock's stand-in for the Rust transaction.
    const { amounts, dueDates } = this.resolveSchedule(input);

    const reschedule = this.scheduleChanged(row, input, amounts, dueDates);
    // Judge the whole schedule before touching the purchase row, so a refusal
    // leaves the label and the totals alone too.
    if (reschedule) this.assertScheduleApplies(id, amounts, dueDates);

    row.productLabel = input.productLabel.trim();
    row.totalPrice = input.totalPrice;
    row.installmentCount = input.installmentCount;
    row.intervalKind = input.intervalKind;
    row.intervalDays = input.intervalDays;
    row.purchaseDate = input.purchaseDate;

    if (reschedule) this.applyScheduleInPlace(id, amounts, dueDates);
    return this.buildPurchaseDetail(id);
  }

  archivePurchase(id: number): void {
    const row = this.purchases.find((p) => p.id === id);
    if (!row) throw new Error("PURCHASE_NOT_FOUND");
    const paid = this.purchasePaymentCount(id);
    if (paid > 0) throw new Error(`PURCHASE_HAS_PAYMENTS:${paid}`);
    // Re-archiving must not move the stamp — see `archive_purchase_impl`.
    row.archivedAt ??= todayIso();
  }

  restorePurchase(id: number): void {
    const row = this.purchases.find((p) => p.id === id);
    if (!row) throw new Error("PURCHASE_NOT_FOUND");
    row.archivedAt = null;
  }

  deletePurchase(id: number): void {
    const row = this.purchases.find((p) => p.id === id);
    if (!row) throw new Error("PURCHASE_NOT_FOUND");
    if (row.archivedAt === null) throw new Error("PURCHASE_NOT_ARCHIVED");
    const instIds = this.installments.filter((i) => i.purchaseId === id).map((i) => i.id);
    this.payments = this.payments.filter((p) => !instIds.includes(p.installmentId));
    this.installments = this.installments.filter((i) => i.purchaseId !== id);
    this.purchases = this.purchases.filter((p) => p.id !== id);
  }

  /**
   * Record money against one installment — the only write path that still works
   * after a payment has been recorded. Mirrors `update_installment_impl` in
   * `src-tauri/src/commands.rs` guard for guard and code for code; the
   * integration suite asserts against these strings.
   *
   * It deals only in money. The schedule (amount, due date) belongs to
   * `updatePurchase` and is refused here with `SCHEDULE_VIA_PURCHASE`. The paid
   * amount is gated on the previous installment being settled, and a payment
   * date may only date the ledger entry this edit creates — an entry already on
   * record keeps its date (`PAYMENT_DATE_LOCKED`). A moved paid amount writes a
   * correction row, so `SUM(payments) === SUM(paidAmount)` survives the edit.
   */
  updateInstallment(id: number, edit: InstallmentEdit): PurchaseDetail {
    // Refused on presence, not on "differs from stored": a caller sending a
    // schedule field still believes this command owns one.
    if (edit.amount !== undefined || edit.dueDate !== undefined) {
      throw new Error("SCHEDULE_VIA_PURCHASE");
    }
    if (edit.paymentDate !== undefined) {
      assertIsoDate(edit.paymentDate);
      if (dayDiff(edit.paymentDate, todayIso()) > 0) throw new Error("FUTURE_PAID_DATE");
    }
    if ((edit.paidAmount ?? 0) < 0) throw new Error("INVALID_AMOUNT");

    const target = this.installments.find((i) => i.id === id);
    if (!target) throw new Error("INSTALLMENT_NOT_FOUND");
    const owner = this.purchases.find((p) => p.id === target.purchaseId);
    if (owner?.archivedAt != null) throw new Error("PURCHASE_ARCHIVED");

    const rows = this.rowsOf(target.purchaseId);
    const pos = rows.findIndex((i) => i.id === id);

    // -- the money: gated on the previous installment being settled ----------
    const paidChanged = edit.paidAmount !== undefined && edit.paidAmount !== target.paidAmount;
    if (paidChanged || edit.paymentDate !== undefined || edit.note !== undefined) {
      const prev = pos > 0 ? rows[pos - 1] : null;
      if (prev && prev.paidAmount < prev.amount) throw new Error(`PREVIOUS_UNPAID:${prev.index}`);
    }

    // -- resolve everything before mutating anything --------------------------
    const finalPaid = edit.paidAmount ?? target.paidAmount;
    if (finalPaid > target.amount) throw new Error(`PAID_ABOVE_AMOUNT:${target.amount}`);

    const latest = this.latestPayment(id);
    // A payment date dates the correction entry below, and nothing else: an
    // entry already on record keeps the date it was collected on, and with no
    // entry either way there is nothing for the date to land on.
    if (edit.paymentDate !== undefined && !paidChanged) {
      throw new Error(latest ? "PAYMENT_DATE_LOCKED" : "NO_PAYMENT_TO_DATE");
    }
    // A note carries no such history and may still amend the latest entry — but
    // it still needs one to amend.
    if (edit.note !== undefined && !paidChanged && !latest) {
      throw new Error("NO_PAYMENT_TO_DATE");
    }

    // -- writes ---------------------------------------------------------------
    const note = edit.note?.trim() || null;
    if (paidChanged) {
      this.payments.push({
        id: this.nextId("payment"),
        installmentId: id,
        amount: finalPaid - target.paidAmount,
        paymentDate: edit.paymentDate ?? todayIso(),
        note,
        createdAt: todayIso(),
      });
      target.paidAmount = finalPaid;
    } else if (latest && note !== null) {
      // Nothing to correct, so a note amends the entry already there.
      latest.note = note;
    }

    // `paidDate` is derived from the collected figure, so re-run it here.
    this.syncPaidDate(target);

    return this.buildPurchaseDetail(target.purchaseId);
  }

  /** Re-derive an installment's `paidDate`: its last payment, or null. */
  private syncPaidDate(inst: InstallmentRow): void {
    inst.paidDate =
      inst.paidAmount >= inst.amount ? (this.latestPayment(inst.id)?.paymentDate ?? null) : null;
  }

  /**
   * The installment's most recent ledger entry. Ties break on insertion id, so
   * this matches the Rust `ORDER BY payment_date DESC, id DESC LIMIT 1`.
   */
  private latestPayment(installmentId: number): PaymentRow | undefined {
    return this.payments
      .filter((p) => p.installmentId === installmentId)
      .sort((a, b) =>
        a.paymentDate === b.paymentDate ? a.id - b.id : a.paymentDate < b.paymentDate ? -1 : 1,
      )
      .pop();
  }

  recordPayment(input: PaymentInput): PurchaseDetail {
    if (input.amount <= 0) throw new Error("INVALID_AMOUNT");
    assertIsoDate(input.paymentDate);
    const inst = this.installments.find((i) => i.id === input.installmentId);
    if (!inst) throw new Error("INSTALLMENT_NOT_FOUND");
    // The other half of "an archived purchase carries zero payments".
    const owner = this.purchases.find((p) => p.id === inst.purchaseId);
    if (owner?.archivedAt != null) throw new Error("PURCHASE_ARCHIVED");
    // Mirrors the Rust guard: an uncapped paidAmount makes `amount - paidAmount`
    // negative, which then cancels out other clients' debt in the aggregates.
    const remaining = inst.amount - inst.paidAmount;
    if (input.amount > remaining) throw new Error(`OVERPAYMENT:${Math.max(remaining, 0)}`);
    this.payments.push({
      id: this.nextId("payment"),
      installmentId: inst.id,
      amount: input.amount,
      paymentDate: input.paymentDate,
      note: input.note?.trim() || null,
      createdAt: todayIso(),
    });
    inst.paidAmount += input.amount;
    inst.paidDate = inst.paidAmount >= inst.amount ? input.paymentDate : null;
    return this.buildPurchaseDetail(inst.purchaseId);
  }

  private mapPayment(p: PaymentRow): Payment {
    const inst = this.installments.find((i) => i.id === p.installmentId)!;
    const purchase = this.purchases.find((pu) => pu.id === inst.purchaseId)!;
    const client = this.clients.find((c) => c.id === purchase.clientId)!;
    return {
      id: p.id,
      installmentId: p.installmentId,
      installmentIndex: inst.index,
      purchaseId: purchase.id,
      purchaseReference: purchase.reference,
      clientId: client.id,
      clientName: `${client.firstName} ${client.lastName}`,
      amount: p.amount,
      paymentDate: p.paymentDate,
      note: p.note,
      createdAt: p.createdAt,
    };
  }

  listPaymentsForPurchase(purchaseId: number): Payment[] {
    const instIds = this.installments.filter((i) => i.purchaseId === purchaseId).map((i) => i.id);
    return this.payments
      .filter((p) => instIds.includes(p.installmentId))
      .sort((a, b) => b.paymentDate.localeCompare(a.paymentDate) || b.id - a.id)
      .map((p) => this.mapPayment(p));
  }

  listAllPayments(limit = 500): Payment[] {
    // Mirrors `PAYMENT_LIMIT_RANGE` in db.rs. Worth mirroring even though the
    // failure modes differ: a negative LIMIT means *no* limit in SQLite, while
    // `.slice(0, -1)` here would quietly drop the last row instead. Left alone,
    // the mock would disagree with the backend in both directions at once.
    const clamped = Math.min(PAYMENT_LIMIT_MAX, Math.max(PAYMENT_LIMIT_MIN, Math.trunc(limit)));
    return this.payments
      .slice()
      .sort((a, b) => b.paymentDate.localeCompare(a.paymentDate) || b.id - a.id)
      .slice(0, clamped)
      .map((p) => this.mapPayment(p));
  }

  listPaymentsForClient(clientId: number): Payment[] {
    const purchaseIds = this.purchases.filter((p) => p.clientId === clientId).map((p) => p.id);
    const instIds = this.installments
      .filter((i) => purchaseIds.includes(i.purchaseId))
      .map((i) => i.id);
    return this.payments
      .filter((p) => instIds.includes(p.installmentId))
      .sort((a, b) => b.paymentDate.localeCompare(a.paymentDate) || b.id - a.id)
      .map((p) => this.mapPayment(p));
  }

  listImpayes(filter?: ImpayeFilter): ImpayeClient[] {
    return this.buildImpayes(filter ?? {});
  }

  listSchedule(): ScheduleRow[] {
    const today = todayIso();
    return this.installments
      .filter((i) => this.isLive(i))
      .sort((a, b) => a.dueDate.localeCompare(b.dueDate) || a.id - b.id)
      .map((i) => {
        const purchase = this.purchases.find((p) => p.id === i.purchaseId)!;
        const client = this.clients.find((c) => c.id === purchase.clientId)!;
        return {
          installmentId: i.id,
          purchaseId: purchase.id,
          reference: purchase.reference,
          clientId: client.id,
          clientName: `${client.firstName} ${client.lastName}`,
          index: i.index,
          installmentCount: purchase.installmentCount,
          dueDate: i.dueDate,
          amount: i.amount,
          paidAmount: i.paidAmount,
          remaining: i.amount - i.paidAmount,
          status: installmentStatus(i.amount, i.paidAmount, i.dueDate, today),
        };
      });
  }

  getDashboard(upcomingDays = 7): Dashboard {
    const today = todayIso();
    const horizon = addInterval(today, "custom", upcomingDays, 1);
    const live = this.livePurchases();
    const totalPurchases = live.length;
    const totalSales = live.reduce((s, p) => s + p.totalPrice, 0);
    // Unfiltered on purpose: archiving is refused once a payment exists and an
    // archived purchase cannot take one, so it has no payments to exclude.
    const totalCollected = this.payments.reduce((s, p) => s + p.amount, 0);
    const totalOutstanding = this.installments
      .filter((i) => this.isLive(i))
      .reduce((s, i) => s + (i.amount - i.paidAmount), 0);
    const overdueInsts = this.installments.filter(
      (i) => this.isLive(i) && dayDiff(i.dueDate, today) < 0 && i.amount > i.paidAmount,
    );
    const overdueClients = new Set(
      overdueInsts.map((i) => this.purchases.find((p) => p.id === i.purchaseId)!.clientId),
    ).size;
    const upcomingCount = this.installments.filter(
      (i) =>
        this.isLive(i) && i.dueDate >= today && i.dueDate <= horizon && i.amount > i.paidAmount,
    ).length;

    const recentIds = live
      .slice()
      .sort((a, b) => b.purchaseDate.localeCompare(a.purchaseDate) || b.id - a.id)
      .slice(0, 5)
      .map((p) => p.id);
    const recentPurchases = recentIds.map((id) => this.buildPurchaseSummary(id));

    const featuredId =
      overdueInsts
        .map((i) => this.purchases.find((p) => p.id === i.purchaseId)!)
        .sort((a, b) => b.purchaseDate.localeCompare(a.purchaseDate) || b.id - a.id)[0]?.id ??
      recentIds[0];
    const featuredPurchase = featuredId ? this.buildPurchaseDetail(featuredId) : null;

    const dueAlerts: DueAlert[] = overdueInsts
      .slice()
      .sort((a, b) => a.dueDate.localeCompare(b.dueDate))
      .slice(0, 4)
      .map((i) => {
        const purchase = this.purchases.find((p) => p.id === i.purchaseId)!;
        const client = this.clients.find((c) => c.id === purchase.clientId)!;
        return {
          purchaseId: purchase.id,
          reference: purchase.reference,
          clientName: `${client.firstName} ${client.lastName}`,
          index: i.index,
          installmentCount: purchase.installmentCount,
          dueDate: i.dueDate,
          daysLate: -dayDiff(i.dueDate, today),
        };
      });

    return {
      stats: {
        totalPurchases,
        totalSales,
        totalCollected,
        totalOutstanding,
        overdueCount: overdueInsts.length,
        overdueClients,
        upcomingCount,
      },
      recentPurchases,
      featuredPurchase,
      dueAlerts,
      impayes: this.buildImpayes({}, 5),
    };
  }

  getSettings(): Settings {
    const s = this.settings;
    return {
      language: s.language ?? "fr",
      currencyCode: s.currency_code ?? "TND",
      dateFormat: s.date_format ?? "dd/MM/yyyy",
      logoPath: s.logo_path ? s.logo_path : null,
      shopName: s.shop_name ?? "",
      shopInfo: s.shop_info ?? "",
      alertSoonDays: Number(s.alert_soon_days ?? "7"),
      languageIsDefault: (s.language_is_default ?? "1") === "1",
      lastBackupAt: s.last_backup_at ? s.last_backup_at : null,
    };
  }

  updateSettings(patch: SettingsPatch): Settings {
    // Resolve and validate everything before writing anything, as the Rust side
    // does before opening its transaction — a rejected patch must not leave
    // half the settings applied.
    const language = patch.language?.trim();
    if (language !== undefined && !LANGUAGES.includes(language)) {
      throw new Error("INVALID_SETTING_VALUE");
    }
    const currencyCode = patch.currencyCode?.trim();
    if (currencyCode !== undefined && !CURRENCY_CODES.includes(currencyCode)) {
      throw new Error("INVALID_SETTING_VALUE");
    }
    const dateFormat = patch.dateFormat?.trim();
    if (dateFormat !== undefined && !DATE_FORMAT_VALUES.includes(dateFormat)) {
      throw new Error("INVALID_SETTING_VALUE");
    }
    const shopName = patch.shopName?.trim();
    if (shopName !== undefined) assertBounded(shopName, SHORT_TEXT_MAX);
    const shopInfo = patch.shopInfo?.trim();
    if (shopInfo !== undefined) assertBounded(shopInfo, LONG_TEXT_MAX);

    if (language !== undefined) {
      this.settings.language = language;
      this.settings.language_is_default = "0";
    }
    if (currencyCode !== undefined) this.settings.currency_code = currencyCode;
    if (dateFormat !== undefined) this.settings.date_format = dateFormat;
    if (shopName !== undefined) this.settings.shop_name = shopName;
    if (shopInfo !== undefined) this.settings.shop_info = shopInfo;
    if (patch.alertSoonDays !== undefined) {
      // Mirror the backend's defensive clamp (1..90).
      this.settings.alert_soon_days = String(
        Math.min(90, Math.max(1, Math.round(patch.alertSoonDays))),
      );
    }
    return this.getSettings();
  }

  setLogo(sourcePath: string): Settings {
    this.settings.logo_path = sourcePath;
    return this.getSettings();
  }

  clearLogo(): Settings {
    this.settings.logo_path = "";
    return this.getSettings();
  }

  /**
   * Browser stand-in for the `VACUUM INTO` snapshot. There is no file to write
   * here, so this only records the destination for tests to assert on — but it
   * must still stamp `last_backup_at` and return the settings, because that is
   * how the real command tells the renderer to clear the staleness banner.
   */
  backupDatabase(dest: string): Settings {
    this.lastBackupPath = dest;
    this.settings.last_backup_at = todayIso();
    return this.getSettings();
  }

  // -- system --

  /**
   * Browser stand-in for Tauri's opener. Records the URI and does nothing else —
   * actually navigating to it here would unload the SPA, which is precisely the
   * failure the opener call exists to avoid.
   */
  openExternal(url: string): void {
    this.lastExternalUrl = url;
  }

  // -- licence --

  /**
   * Browser stand-in for the licence verdict.
   *
   * **Licensed by default, on purpose.** The mock backs every unit, integration
   * and E2E test, and all of them exercise screens that are licensed features.
   * Defaulting to unlicensed would turn the whole suite red and, worse, would
   * make "the licence gate is showing" the expected output of tests that are
   * really about purchases and payments. Licence behaviour is tested by flipping
   * this explicitly with `setLicense`.
   *
   * The mock does not reimplement Ed25519 — signature verification is Rust's
   * job and is covered by `cargo test`. This only models the *verdict*.
   */
  getLicenseStatus(): LicenseInfo {
    return { ...this.license };
  }

  /**
   * Browser stand-in for the import flow. There is no file to read, so a path
   * ending in `.json` is accepted and anything else is refused with the same
   * `INVALID_LICENSE:{status}` shape the Rust command uses.
   */
  importLicense(sourcePath: string): LicenseInfo {
    if (!sourcePath.toLowerCase().endsWith(".json")) {
      throw new Error("INVALID_LICENSE:malformed");
    }
    this.setLicense("valid");
    return this.getLicenseStatus();
  }

  /** Test hook: put the mock into a given licence state. */
  setLicense(status: LicenseStatusTag): void {
    this.license = {
      status,
      license:
        status === "valid" || status === "expired" || status === "machineMismatch"
          ? {
              licenseId: "PS-MOCK-0001",
              licensee: "Boutique de démonstration",
              issuedAt: "2026-01-01",
              expiresAt: status === "expired" ? "2026-02-01" : "2999-12-31",
              machineId: status === "machineMismatch" ? "other-machine" : null,
              features: ["*"],
            }
          : null,
      expiredOn: status === "expired" ? "2026-02-01" : null,
      machineId: MOCK_MACHINE_ID,
    };
  }
}

/** Stable stand-in for a real machine fingerprint (64 lower-case hex chars). */
const MOCK_MACHINE_ID = "m0ckm0ck".repeat(8);

export const mockDb = new MockDb();
