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
  Payment,
  PaymentInput,
  PurchaseDetail,
  PurchaseInput,
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
    };
  }

  private buildImpayes(filter: ImpayeFilter, limit?: number): ImpayeClient[] {
    const today = todayIso();
    const map = new Map<number, ImpayeClient>();
    const order: number[] = [];
    const overdue = this.installments
      .filter((i) => dayDiff(i.dueDate, today) < 0 && i.amount > i.paidAmount)
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
        const purchases = this.purchases.filter((p) => p.clientId === c.id);
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
    const purchases = this.purchases
      .filter((p) => p.clientId === id)
      .sort((a, b) => b.purchaseDate.localeCompare(a.purchaseDate) || b.id - a.id)
      .map((p) => this.buildPurchaseSummary(p.id));
    const totalPurchased = purchases.reduce((s, p) => s + p.totalPrice, 0);
    const totalPaid = purchases.reduce((s, p) => s + p.paidAmount, 0);
    const overdueCount = purchases.reduce((s, p) => s + p.overdueCount, 0);
    return {
      client: this.clientOut(client),
      purchases,
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

  listPurchases(search?: string): PurchaseSummary[] {
    const needle = search?.trim().toLowerCase();
    return this.purchases
      .slice()
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

  createPurchase(input: PurchaseInput): PurchaseDetail {
    validatePurchaseInput(input);

    // Mirrors `create_purchase_impl`: an archived client must not take on new
    // debt, which is what keeps "archived implies a zero balance" true.
    const client = this.clients.find((c) => c.id === input.clientId);
    if (!client) throw new Error("CLIENT_NOT_FOUND");
    if (client.archivedAt !== null) throw new Error("CLIENT_ARCHIVED");

    // Resolve amounts and due dates before mutating anything, so a rejected
    // request leaves no half-created purchase behind — the mock's stand-in for
    // the Rust side's transaction.
    let amounts: number[];
    let dueDates: string[];
    if (input.installments && input.installments.length > 0) {
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

  deletePurchase(id: number): void {
    const instIds = this.installments.filter((i) => i.purchaseId === id).map((i) => i.id);
    this.payments = this.payments.filter((p) => !instIds.includes(p.installmentId));
    this.installments = this.installments.filter((i) => i.purchaseId !== id);
    this.purchases = this.purchases.filter((p) => p.id !== id);
  }

  recordPayment(input: PaymentInput): PurchaseDetail {
    if (input.amount <= 0) throw new Error("INVALID_AMOUNT");
    assertIsoDate(input.paymentDate);
    const inst = this.installments.find((i) => i.id === input.installmentId);
    if (!inst) throw new Error("INSTALLMENT_NOT_FOUND");
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
      .slice()
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
    const totalPurchases = this.purchases.length;
    const totalSales = this.purchases.reduce((s, p) => s + p.totalPrice, 0);
    const totalCollected = this.payments.reduce((s, p) => s + p.amount, 0);
    const totalOutstanding = this.installments.reduce((s, i) => s + (i.amount - i.paidAmount), 0);
    const overdueInsts = this.installments.filter(
      (i) => dayDiff(i.dueDate, today) < 0 && i.amount > i.paidAmount,
    );
    const overdueClients = new Set(
      overdueInsts.map((i) => this.purchases.find((p) => p.id === i.purchaseId)!.clientId),
    ).size;
    const upcomingCount = this.installments.filter(
      (i) => i.dueDate >= today && i.dueDate <= horizon && i.amount > i.paidAmount,
    ).length;

    const recentIds = this.purchases
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
