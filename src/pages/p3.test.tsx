import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { App } from "../shell/App";
import { applyInOrder } from "./Pos";

type H = (a: Record<string, unknown> | undefined) => unknown;

const cashier = {
  user_id: "u1",
  username: "cash",
  full_name: "Counter One",
  must_change_password: false,
  roles: ["Cashier"],
  permissions: [
    "sales.create",
    "sales.view",
    "products.view",
    "customers.view",
    "customers.collect_payment",
  ],
};
const manager = {
  ...cashier,
  roles: ["Manager"],
  permissions: [
    ...cashier.permissions,
    "sales.discount",
    "sales.wholesale",
    "sales.credit_override",
    "accounts.view",
    "accounts.manage",
  ],
};

function backend(h: Record<string, H>) {
  invoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    const fn = h[cmd];
    if (!fn) throw { code: "unknown", message: `unhandled ${cmd}` };
    const r = fn(args);
    if (r instanceof Error)
      throw { code: (r as Error & { code?: string }).code ?? "unknown", message: r.message };
    return r;
  });
}

const base = {
  setup_status: () => ({ needs_setup: false, startup_notice: null }),
  session: () => cashier,
  branding: () => ({
    shop_name: "Singer Pro",
    shop_address: "",
    shop_phone: "",
    shop_email: "",
    invoice_footer: "",
    currency_symbol: "৳",
    language: "en",
  }),
  logo_get: () => null,
  db_status: () => ({
    schema_version: 4,
    journal_mode: "wal",
    foreign_keys: true,
    integrity_ok: true,
    integrity_detail: [],
  }),
  backup_status: () => ({ last_backup_at: null, reminder_days: 7, overdue: false }),
};

const fan = {
  variant_id: "v1",
  product_id: "p1",
  label: "Singer Fan 56in",
  sku: "FAN",
  barcode: "8901",
  is_serialized: false,
  purchase_cost: 100000,
  retail_price: 150000,
  wholesale_price: 130000,
  installment_price: null,
  available: 10,
};
const fridge = {
  ...fan,
  variant_id: "v2",
  label: "Singer Fridge 300L",
  sku: "FR",
  barcode: null,
  is_serialized: true,
  retail_price: 5200000,
  available: 3,
};

function previewFor(items: { variant_id: string; qty: number; discount: number }[], mode: string) {
  const price = (id: string) => (id === "v2" ? 5200000 : mode === "wholesale" ? 130000 : 150000);
  const lines = items.map((i) => ({
    variant_id: i.variant_id,
    unit_price: price(i.variant_id),
    discount: i.discount,
    tax: 0,
    line_total: price(i.variant_id) * i.qty - i.discount,
  }));
  const subtotal = items.reduce((s, i) => s + price(i.variant_id) * i.qty, 0);
  const disc = items.reduce((s, i) => s + i.discount, 0);
  return { subtotal, discount_total: disc, tax_total: 0, grand_total: subtotal - disc, lines };
}

beforeEach(() => {
  invoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  localStorage.clear();
});
afterEach(() => {
  cleanup();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

async function openPos(session = cashier, extra: Record<string, H> = {}) {
  const created = vi.fn((a: Record<string, unknown> | undefined) => ({
    id: "s1",
    number: "INV-000001",
    grand_total: (a as { sale: { payments: { amount: number }[] } }).sale.payments.reduce(
      (x, p) => x + p.amount,
      0,
    ),
    paid: 0,
    due: 0,
    receipts: ["RCT-000001"],
  }));
  backend({
    ...base,
    session: () => session,
    variants_search: () => [fan, fridge],
    sale_preview: (a) => {
      const s = (
        a as {
          sale: { items: { variant_id: string; qty: number; discount: number }[]; mode: string };
        }
      ).sale;
      return previewFor(s.items, s.mode);
    },
    sale_create: created,
    sale_get: () => ({
      sale: {
        id: "s1",
        number: "INV-000001",
        mode: "retail",
        sale_date: "2026-03-01",
        customer_id: null,
        customer: null,
        grand_total: 450000,
        paid: 450000,
        due: 0,
      },
      subtotal: 450000,
      discount_total: 0,
      tax_total: 0,
      notes: "",
      cashier: "cash",
      customer_phone: null,
      customer_address: null,
      items: [],
      payments: [],
    }),
    sale_serials: () => [
      { id: "sn1", serial_no: "F-1" },
      { id: "sn2", serial_no: "F-2" },
    ],
    customers_list: () => [
      {
        id: "c1",
        code: "CUS-00001",
        name: "Dealer Co",
        phone: "017",
        alt_phone: "",
        address: "",
        customer_type: "dealer",
        credit_limit: 500000,
        nid: "",
        notes: "",
        is_active: true,
        registered_on: "2026-01-01",
        balance: 0,
      },
    ],
    ...extra,
  });
  render(<App />);
  await screen.findByText(session.full_name);
  fireEvent.click(screen.getByRole("button", { name: /New Sale/ }));
  return created;
}

describe("applyInOrder", () => {
  it.each([
    [[100, 200], 250, [100, 150]],
    [[500], 300, [300]],
    [[0, 0], 300, [0, 0]],
    [[100], 0, [0]],
    [[-5, 10], 7, [0, 7]],
  ])("%j against %i -> %j", (entered, total, expected) =>
    expect(applyInOrder(entered, total)).toEqual(expected),
  );
});

describe("POS", () => {
  it("adds an item, shows server-computed totals, and records only what is applied (change is not a payment)", async () => {
    const created = await openPos();
    const find = await screen.findByLabelText("Find product");
    fireEvent.change(find, { target: { value: "fan" } });
    fireEvent.click(await screen.findByText("Singer Fan 56in", { selector: "b" }));
    fireEvent.change(screen.getByLabelText("Quantity FAN"), { target: { value: "3" } });
    await waitFor(() => expect(screen.getAllByText("৳4,500.00").length).toBeGreaterThan(0)); // 3 x 1,500.00 from the backend preview
    fireEvent.change(screen.getByPlaceholderText("Amount"), { target: { value: "5000" } }); // customer hands over 5,000
    expect(await screen.findByText("Change to return")).toBeTruthy();
    expect(screen.getByText("৳500.00")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Complete sale/ }));
    await waitFor(() => expect(created).toHaveBeenCalled());
    const body = (
      created.mock.calls[0]![0] as {
        sale: {
          mode: string;
          payments: { method: string; amount: number }[];
          items: { qty: number; variant_id: string }[];
          client_key: string;
          customer_id: string | null;
        };
      }
    ).sale;
    expect(body.payments).toEqual([{ method: "cash", amount: 450000, reference: "" }]);
    expect(body.items).toEqual([{ variant_id: "v1", qty: 3, discount: 0, serial_ids: [] }]);
    expect(body.mode).toBe("retail");
    expect(body.client_key.length).toBeGreaterThan(10);
    expect(await screen.findByRole("dialog", { name: /Sale INV-000001 completed/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Receipt \(80mm\)/ })).toBeTruthy();
  });

  it("scanner Enter adds the exact barcode match immediately", async () => {
    await openPos();
    const find = await screen.findByLabelText("Find product");
    fireEvent.change(find, { target: { value: "8901" } });
    await screen.findByText("Singer Fan 56in", { selector: "b" });
    fireEvent.keyDown(find, { key: "Enter" });
    expect(await screen.findByLabelText("Quantity FAN")).toBeTruthy();
  });

  it("serial-tracked items require choosing the exact units", async () => {
    const created = await openPos();
    const find = await screen.findByLabelText("Find product");
    fireEvent.change(find, { target: { value: "fridge" } });
    fireEvent.click(await screen.findByText("Singer Fridge 300L", { selector: "b" }));
    const dlg = within(await screen.findByRole("dialog", { name: /Select serial numbers/ }));
    expect(dlg.getByRole("button", { name: "Add to sale" }).hasAttribute("disabled")).toBe(true);
    fireEvent.click(dlg.getByLabelText("F-2"));
    fireEvent.click(dlg.getByRole("button", { name: "Add to sale" }));
    await waitFor(() => expect(screen.getAllByText("৳52,000.00").length).toBeGreaterThan(0));
    fireEvent.click(screen.getByRole("button", { name: "Pay full (cash)" }));
    fireEvent.click(screen.getByRole("button", { name: /Complete sale/ }));
    await waitFor(() => expect(created).toHaveBeenCalled());
    const body = (
      created.mock.calls[0]![0] as { sale: { items: { qty: number; serial_ids: string[] }[] } }
    ).sale;
    expect(body.items[0]).toMatchObject({ qty: 1, serial_ids: ["sn2"] });
  });

  it("shows backend rejections (e.g. stock, credit limit) instead of pretending success", async () => {
    await openPos(cashier, {
      sale_create: () =>
        Object.assign(new Error("conflict: not enough stock of FAN: 2 available, 3 requested"), {
          code: "conflict",
        }),
    });
    const find = await screen.findByLabelText("Find product");
    fireEvent.change(find, { target: { value: "fan" } });
    fireEvent.click(await screen.findByText("Singer Fan 56in", { selector: "b" }));
    await screen.findByLabelText("Quantity FAN");
    fireEvent.click(screen.getByRole("button", { name: "Pay full (cash)" }));
    fireEvent.click(screen.getByRole("button", { name: /Complete sale/ }));
    expect(await screen.findByText(/not enough stock/)).toBeTruthy();
    expect(screen.queryByRole("dialog", { name: /completed/ })).toBeNull();
  });

  it("cashiers see no discount column or wholesale switch; managers do", async () => {
    await openPos(cashier);
    expect(screen.queryByRole("tab", { name: "Wholesale" })).toBeNull();
    cleanup();
    await openPos(manager);
    expect(await screen.findByRole("tab", { name: "Wholesale" })).toBeTruthy();
  });

  it("wholesale needs a customer and uses the wholesale price list", async () => {
    const created = await openPos(manager);
    fireEvent.click(await screen.findByRole("tab", { name: "Wholesale" }));
    const find = await screen.findByLabelText("Find product");
    fireEvent.change(find, { target: { value: "fan" } });
    fireEvent.click(await screen.findByText("Singer Fan 56in", { selector: "b" }));
    await waitFor(() => expect(screen.getAllByText("৳1,300.00").length).toBeGreaterThan(0));
    fireEvent.click(screen.getByRole("button", { name: /Complete sale/ }));
    expect(await screen.findByText(/need a customer/)).toBeTruthy();
    expect(created).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("Find customer"), { target: { value: "dealer" } });
    fireEvent.click(await screen.findByText("Dealer Co", { selector: "b" }));
    fireEvent.click(screen.getByRole("button", { name: /Complete sale/ }));
    await waitFor(() => expect(created).toHaveBeenCalled());
    const body = (
      created.mock.calls[0]![0] as {
        sale: { mode: string; customer_id: string; payments: unknown[] };
      }
    ).sale;
    expect(body).toMatchObject({ mode: "wholesale", customer_id: "c1", payments: [] });
  });
});

describe("Customers & Accounts", () => {
  it("customer list shows what each customer owes and hides management from view-only users", async () => {
    backend({
      ...base,
      customers_list: () => [
        {
          id: "c1",
          code: "CUS-00001",
          name: "Dealer Co",
          phone: "017",
          alt_phone: "",
          address: "",
          customer_type: "dealer",
          credit_limit: 500000,
          nid: "",
          notes: "",
          is_active: true,
          registered_on: "2026-01-01",
          balance: 420000,
        },
      ],
    });
    render(<App />);
    await screen.findByText("Counter One");
    fireEvent.click(screen.getByRole("button", { name: /^Customers/ }));
    expect(await screen.findByText("Dealer Co")).toBeTruthy();
    expect(screen.getAllByText("৳4,200.00").length).toBeGreaterThan(0);
    expect(screen.queryByRole("button", { name: /New customer/ })).toBeNull();
  });

  it("trial balance and reconciliation report honestly", async () => {
    let n = 0;
    backend({
      ...base,
      session: () => manager,
      trial_balance: () => ({
        as_of: null,
        rows: [
          {
            id: "a",
            code: "1000",
            name: "Cash in Hand",
            account_type: "asset",
            normal_side: "debit",
            is_system: true,
            is_active: true,
            debit: 500000,
            credit: 0,
            balance: 500000,
          },
          {
            id: "b",
            code: "4000",
            name: "Sales Revenue",
            account_type: "income",
            normal_side: "credit",
            is_system: true,
            is_active: true,
            debit: 0,
            credit: 500000,
            balance: 500000,
          },
        ],
        total_debit: 500000,
        total_credit: 500000,
        balanced: true,
      }),
      accounting_reconcile: () =>
        ++n === 1
          ? []
          : [
              {
                check: "stock_value_vs_inventory_account",
                detail: "stock balances vs account 1200",
                expected: 100,
                actual: 107,
              },
            ],
      accounts_list: () => [],
    });
    render(<App />);
    await screen.findByText("Counter One");
    fireEvent.click(screen.getByRole("button", { name: /^Accounts/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Trial balance" }));
    expect(await screen.findByText("Balanced")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Reconciliation" }));
    expect(await screen.findByText(/Journals balance/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Run again" }));
    expect(await screen.findByText(/1 issue/)).toBeTruthy();
  });
});
