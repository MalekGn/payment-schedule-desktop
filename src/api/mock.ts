// In-memory mock backend used when the app runs in a plain browser (no Tauri
// runtime): dev-server preview, screenshots, and unit/integration tests.
// It mirrors the behaviour of src-tauri/src/commands.rs closely enough to
// exercise every screen. The real desktop app always uses the Rust backend.

import {
  addInterval,
  dayDiff,
  installmentStatus,
  purchaseStatus,
  rebalanceAmounts,
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
const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

/** Throw `INVALID_DATE` unless `value` is a real `YYYY-MM-DD` calendar date. */
function assertIsoDate(value: string): void {
  if (!ISO_DATE.test(value) || Number.isNaN(Date.parse(`${value}T00:00:00Z`))) {
    throw new Error("INVALID_DATE");
  }
}

function validatePurchaseInput(input: PurchaseInput): void {
  if (input.totalPrice <= 0) throw new Error("INVALID_TOTAL_PRICE");
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
  private seq = { client: 0, purchase: 0, installment: 0, payment: 0 };

  constructor() {
    this.seed();
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
    if (input.installments && input.installments.length > 0) {
      const sum = input.installments.reduce((s, i) => s + i.amount, 0);
      if (sum !== input.totalPrice) throw new Error(`SUM_MISMATCH:${sum}:${input.totalPrice}`);
      return {
        amounts: input.installments.map((i) => i.amount),
        dueDates: input.installments.map((i) => {
          assertIsoDate(i.dueDate);
          return i.dueDate;
        }),
      };
    }
    const amounts = splitAmounts(input.totalPrice, input.installmentCount);
    return {
      amounts,
      dueDates: amounts.map((_, i) =>
        addInterval(input.purchaseDate, input.intervalKind, input.intervalDays, i),
      ),
    };
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

  updatePurchase(id: number, input: PurchaseInput): PurchaseDetail {
    validatePurchaseInput(input);
    const row = this.purchases.find((p) => p.id === id);
    if (!row) throw new Error("PURCHASE_NOT_FOUND");
    if (row.archivedAt !== null) throw new Error("PURCHASE_ARCHIVED");

    // Resolve before mutating, so a rejected request changes nothing — the
    // mock's stand-in for the Rust transaction.
    const { amounts, dueDates } = this.resolveSchedule(input);

    const reschedule = this.scheduleChanged(row, input, amounts, dueDates);
    if (reschedule) {
      const paid = this.purchasePaymentCount(id);
      if (paid > 0) throw new Error(`PURCHASE_HAS_PAYMENTS:${paid}`);
    }

    row.productLabel = input.productLabel.trim();
    row.totalPrice = input.totalPrice;
    row.installmentCount = input.installmentCount;
    row.intervalKind = input.intervalKind;
    row.intervalDays = input.intervalDays;
    row.purchaseDate = input.purchaseDate;

    if (reschedule) {
      // Safe only because the guard above proved there are no payments.
      this.installments = this.installments.filter((i) => i.purchaseId !== id);
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
    }
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
   * Edit one installment in place — the only write path that still works after
   * a payment has been recorded. Mirrors `update_installment_impl` in
   * `src-tauri/src/commands.rs` guard for guard and code for code; the
   * integration suite asserts against these strings.
   *
   * The fields split in two: the *schedule* (amount, due date) unlocks while
   * the installment is unsettled and ignores its neighbours; the *money* (paid
   * amount, payment date, note) is gated on the previous installment and
   * ignores this one's own status. A moved paid amount writes a correction row
   * into the ledger, so `SUM(payments) === SUM(paidAmount)` survives the edit.
   */
  updateInstallment(id: number, edit: InstallmentEdit): PurchaseDetail {
    if (edit.dueDate !== undefined) assertIsoDate(edit.dueDate);
    if (edit.paymentDate !== undefined) {
      assertIsoDate(edit.paymentDate);
      if (dayDiff(edit.paymentDate, todayIso()) > 0) throw new Error("FUTURE_PAID_DATE");
    }
    if ((edit.amount !== undefined && edit.amount < 0) || (edit.paidAmount ?? 0) < 0) {
      throw new Error("INVALID_AMOUNT");
    }

    const target = this.installments.find((i) => i.id === id);
    if (!target) throw new Error("INSTALLMENT_NOT_FOUND");
    const owner = this.purchases.find((p) => p.id === target.purchaseId);
    if (owner?.archivedAt != null) throw new Error("PURCHASE_ARCHIVED");

    const rows = this.installments
      .filter((i) => i.purchaseId === target.purchaseId)
      .sort((a, b) => a.index - b.index);
    const pos = rows.findIndex((i) => i.id === id);
    const settled = target.paidAmount >= target.amount;

    // -- the schedule half: gated on this installment being unsettled --------
    const amountChanged = edit.amount !== undefined && edit.amount !== target.amount;
    const dueChanged = edit.dueDate !== undefined && edit.dueDate !== target.dueDate;
    if (settled) {
      if (amountChanged) throw new Error("AMOUNT_LOCKED");
      if (dueChanged) throw new Error("DUE_DATE_LOCKED");
    }
    if (dueChanged) {
      const due = edit.dueDate!;
      const below = pos > 0 && due < rows[pos - 1].dueDate;
      const above = pos + 1 < rows.length && due > rows[pos + 1].dueDate;
      if (below || above) throw new Error("DUE_DATE_OUT_OF_ORDER");
    }

    // -- the money half: gated on the previous installment being settled -----
    const paidChanged = edit.paidAmount !== undefined && edit.paidAmount !== target.paidAmount;
    if (paidChanged || edit.paymentDate !== undefined || edit.note !== undefined) {
      const prev = pos > 0 ? rows[pos - 1] : null;
      if (prev && prev.paidAmount < prev.amount) throw new Error(`PREVIOUS_UNPAID:${prev.index}`);
    }

    // -- resolve everything before mutating anything --------------------------
    const finalPaid = edit.paidAmount ?? target.paidAmount;
    const finalAmount = edit.amount ?? target.amount;
    if (finalPaid > finalAmount) {
      throw new Error(
        paidChanged ? `PAID_ABOVE_AMOUNT:${finalAmount}` : `BELOW_PAID:${target.paidAmount}`,
      );
    }

    let nextAmounts: number[] | null = null;
    if (amountChanged) {
      const paidAmounts = rows.map((i) => i.paidAmount);
      // The edited row's own floor is what this edit lands on, not what is
      // stored, so lowering the amount and the collected figure together is not
      // refused for a conflict the request itself resolves.
      paidAmounts[pos] = finalPaid;
      nextAmounts = rebalanceAmounts(
        rows.map((i) => i.amount),
        paidAmounts,
        pos,
        finalAmount,
      );
      if (nextAmounts === null) throw new Error("NO_REBALANCE_ROOM");
    }

    const latest = this.latestPayment(id);
    if ((edit.paymentDate !== undefined || edit.note !== undefined) && !paidChanged && !latest) {
      throw new Error("NO_PAYMENT_TO_DATE");
    }

    // -- writes ---------------------------------------------------------------
    if (edit.dueDate !== undefined) target.dueDate = edit.dueDate;
    if (nextAmounts) {
      rows.forEach((row, i) => {
        row.amount = nextAmounts![i];
      });
    }

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
    } else if (latest) {
      // Nothing to correct, so a date or a note amends the entry already there.
      if (edit.paymentDate !== undefined) latest.paymentDate = edit.paymentDate;
      if (note !== null) latest.note = note;
    }

    // `paidDate` is derived, so re-run it for every row whose numbers moved.
    this.syncPaidDate(target);
    if (nextAmounts) rows.forEach((row) => this.syncPaidDate(row));

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
    return this.payments
      .slice()
      .sort((a, b) => b.paymentDate.localeCompare(a.paymentDate) || b.id - a.id)
      .slice(0, limit)
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
    };
  }

  updateSettings(patch: SettingsPatch): Settings {
    if (patch.language !== undefined) {
      this.settings.language = patch.language;
      this.settings.language_is_default = "0";
    }
    if (patch.currencyCode !== undefined) this.settings.currency_code = patch.currencyCode;
    if (patch.dateFormat !== undefined) this.settings.date_format = patch.dateFormat;
    if (patch.shopName !== undefined) this.settings.shop_name = patch.shopName;
    if (patch.shopInfo !== undefined) this.settings.shop_info = patch.shopInfo;
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
   * here, so this only records the destination for tests to assert on.
   */
  backupDatabase(dest: string): void {
    this.lastBackupPath = dest;
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
}

export const mockDb = new MockDb();
