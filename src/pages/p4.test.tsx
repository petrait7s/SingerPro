import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { App } from "../shell/App";
import { toTerms } from "./Pos";

type H = (a: Record<string, unknown> | undefined) => unknown;

const manager = {
  user_id: "u1",
  username: "mgr",
  full_name: "Manager One",
  must_change_password: false,
  roles: ["Manager"],
  permissions: [
    "sales.create",
    "sales.view",
    "sales.discount",
    "sales.wholesale",
    "products.view",
    "customers.view",
    "installments.view",
    "installments.create",
    "installments.collect",
    "installments.reschedule",
    "installments.reverse_payment",
    "installments.set_markup",
  ],
};
const cashier = {
  ...manager,
  roles: ["Cashier"],
  full_name: "Cashier One",
  permissions: [
    "sales.create",
    "sales.view",
    "products.view",
    "customers.view",
    "installments.view",
    "installments.collect",
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
  session: () => manager,
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
    schema_version: 5,
    journal_mode: "wal",
    foreign_keys: true,
    integrity_ok: true,
    integrity_detail: [],
  }),
  backup_status: () => ({ last_backup_at: null, reminder_days: 7, overdue: false }),
};

const row = {
  id: "a1",
  number: "INS-000001",
  sale_id: "s1",
  sale_number: "INV-000001",
  customer_id: "c1",
  customer: "Karim Uddin",
  customer_phone: "01711",
  status: "active",
  down_payment: 1000000,
  total_payable: 5000000,
  collected: 500000,
  outstanding: 4500000,
  next_due_date: "2026-04-01",
  next_due_amount: 750000,
  overdue_amount: 750000,
  max_days_overdue: 12,
  installment_count: 4,
  created_on: "2026-03-01",
};
const item = (seq: number, status: string, paid: number, over = 0) => ({
  id: `i${seq}`,
  seq,
  due_date: `2026-0${seq + 3}-01`,
  amount: 1250000,
  late_fee: 0,
  paid,
  superseded: false,
  effective: 1250000,
  remaining: 1250000 - paid,
  status,
  days_overdue: over,
});
const detail = {
  agreement: row,
  cash_price: 5200000,
  price_total: 6000000,
  principal: 5000000,
  markup_bp: 0,
  markup: 0,
  fees: [],
  late_fees_total: 0,
  rebates: 0,
  frequency: "monthly",
  first_due_date: "2026-04-01",
  recognition_policy: "proportional_to_collections",
  earned_markup: 0,
  unearned_markup: 0,
  notes: "",
  items: [
    item(1, "partial", 500000, 12),
    item(2, "pending", 0),
    item(3, "pending", 0),
    item(4, "pending", 0),
  ],
  collections: [
    {
      id: "col1",
      receipt_no: "RCT-000009",
      kind: "payment",
      amount: 500000,
      method: "cash",
      paid_on: "2026-04-02",
      reference: "",
      note: "",
      reverses_id: null,
    },
  ],
  reschedules: [],
  followups: [],
  grace_days: 0,
};

beforeEach(() => {
  invoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  localStorage.clear();
});
afterEach(() => {
  cleanup();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

describe("toTerms", () => {
  const base = {
    count: "6",
    frequency: "monthly",
    first: "2026-05-01",
    markupPct: "",
    fees: [] as { label: string; amount: number | null }[],
  };
  it("blank markup means the shop default", () =>
    expect(toTerms(base)).toMatchObject({ markup_bp: null, count: 6, frequency: "monthly" }));
  it("markup percent is converted exactly to basis points", () =>
    expect(toTerms({ ...base, markupPct: "7.5" })).toMatchObject({ markup_bp: 750 }));
  it("rejects bad input instead of guessing", () => {
    expect(typeof toTerms({ ...base, markupPct: "abc" })).toBe("string");
    expect(typeof toTerms({ ...base, fees: [{ label: "", amount: 5 }] })).toBe("string");
  });
});

describe("Installments", () => {
  it("lists agreements with overdue amounts and opens the contract with every component disclosed", async () => {
    backend({
      ...base,
      agreements_list: () => ({
        total: 1,
        overdue_total: 750000,
        outstanding_total: 4500000,
        rows: [row],
      }),
      agreement_get: () => detail,
    });
    render(<App />);
    await screen.findByText("Manager One");
    fireEvent.click(screen.getByRole("button", { name: /^Installments/ }));
    expect(await screen.findByText("Karim Uddin")).toBeTruthy();
    expect(screen.getAllByText(/12d/).length).toBeGreaterThan(0);
    fireEvent.click(screen.getByText("Karim Uddin"));
    const dlg = within(await screen.findByRole("dialog", { name: /INS-000001/ }));
    // cash price, installment price, down payment, financed amount, markup and total are all shown separately
    expect(await dlg.findByText("Cash price")).toBeTruthy();
    expect(dlg.getByText("৳52,000.00")).toBeTruthy();
    expect(dlg.getByText("৳60,000.00")).toBeTruthy();
    expect(dlg.getByText("-৳10,000.00")).toBeTruthy();
    expect(dlg.getByText(/Markup \(0%\)/)).toBeTruthy();
    expect(dlg.getByText("Total payable")).toBeTruthy();
    expect(dlg.getAllByText(/partial/).length).toBeGreaterThan(0);
  });

  it("collects a payment once even if the button is clicked twice (same key) and reports the allocation", async () => {
    const collect = vi.fn(() => ({
      receipt: "RCT-000010",
      allocations: [
        [1, 750000],
        [2, 250000],
      ],
      outstanding: 3500000,
      completed: false,
    }));
    backend({
      ...base,
      agreements_list: () => ({
        total: 1,
        overdue_total: 0,
        outstanding_total: 4500000,
        rows: [row],
      }),
      agreement_get: () => detail,
      installment_collect: collect,
      installment_settle_quote: () => ({ outstanding: 4500000, max_rebate: 0 }),
    });
    render(<App />);
    await screen.findByText("Manager One");
    fireEvent.click(screen.getByRole("button", { name: /^Installments/ }));
    fireEvent.click(await screen.findByText("Karim Uddin"));
    const dlg = within(await screen.findByRole("dialog", { name: /INS-000001/ }));
    fireEvent.click(await dlg.findByRole("button", { name: "Collect payment" }));
    const pay = within(await screen.findByRole("dialog", { name: /Collect payment/ }));
    fireEvent.change(pay.getByLabelText(/Amount/), { target: { value: "10,000" } });
    const b = pay.getByRole("button", { name: "Receive payment" });
    fireEvent.click(b);
    fireEvent.click(b);
    await waitFor(() => expect(collect).toHaveBeenCalled());
    const keys = collect.mock.calls.map(
      (c) => ((c as unknown[])[0] as { collect: { client_key: string; amount: number } }).collect,
    );
    expect(new Set(keys.map((k) => k.client_key)).size).toBe(1);
    expect(keys[0]?.amount).toBe(1000000);
  });

  it("hides rescheduling, late fees and reversals from users without those permissions", async () => {
    backend({
      ...base,
      session: () => cashier,
      agreements_list: () => ({
        total: 1,
        overdue_total: 750000,
        outstanding_total: 4500000,
        rows: [row],
      }),
      agreement_get: () => detail,
    });
    render(<App />);
    await screen.findByText("Cashier One");
    fireEvent.click(screen.getByRole("button", { name: /^Installments/ }));
    fireEvent.click(await screen.findByText("Karim Uddin"));
    const dlg = within(await screen.findByRole("dialog", { name: /INS-000001/ }));
    await dlg.findByRole("button", { name: "Collect payment" });
    expect(dlg.queryByRole("button", { name: "Reschedule" })).toBeNull();
    expect(dlg.queryByText("late fee")).toBeNull();
    expect(dlg.queryByText("reverse")).toBeNull();
  });

  it("shows the empty state for the overdue worklist honestly", async () => {
    backend({
      ...base,
      agreements_list: () => ({ total: 0, overdue_total: 0, outstanding_total: 0, rows: [] }),
    });
    render(<App />);
    await screen.findByText("Manager One");
    fireEvent.click(screen.getByRole("button", { name: /^Installments/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Overdue follow-up" }));
    expect(await screen.findByText("Nothing overdue")).toBeTruthy();
  });
});

describe("POS installment mode", () => {
  const fridge = {
    variant_id: "v2",
    product_id: "p1",
    label: "Singer Fridge 300L",
    sku: "FR",
    barcode: null,
    is_serialized: true,
    purchase_cost: 4000000,
    retail_price: 5200000,
    wholesale_price: 4800000,
    installment_price: 6000000,
    available: 3,
  };
  it("discloses financing before signing and sends the terms with the down payment", async () => {
    const create = vi.fn(() => ({
      id: "s1",
      number: "INV-000001",
      grand_total: 0,
      paid: 0,
      due: 0,
      receipts: [],
    }));
    const previewSale = (a: Record<string, unknown> | undefined) => {
      const s = (a as { sale: { items: { variant_id: string; qty: number }[] } }).sale;
      const total = s.items.reduce((t, i) => t + 6000000 * i.qty, 0);
      return {
        subtotal: total,
        discount_total: 0,
        tax_total: 0,
        grand_total: total,
        lines: s.items.map((i) => ({
          variant_id: i.variant_id,
          unit_price: 6000000,
          discount: 0,
          tax: 0,
          line_total: 6000000 * i.qty,
        })),
      };
    };
    backend({
      ...base,
      variants_search: () => [fridge],
      sale_preview: previewSale,
      sale_serials: () => [{ id: "sn1", serial_no: "F-1" }],
      customers_list: () => [
        {
          id: "c1",
          code: "CUS-00001",
          name: "Karim Uddin",
          phone: "017",
          alt_phone: "",
          address: "",
          customer_type: "retail",
          credit_limit: 0,
          nid: "",
          notes: "",
          is_active: true,
          registered_on: "2026-01-01",
          balance: 0,
          price_tier_id: null,
        },
      ],
      installment_preview: () => ({
        cash_price: 5200000,
        price_total: 6000000,
        down_payment: 1000000,
        principal: 5000000,
        markup_bp: 0,
        markup: 0,
        fees_total: 0,
        total_payable: 5000000,
        schedule: [
          ["2026-05-01", 833333],
          ["2026-06-01", 4166667],
        ],
      }),
      sale_create: create,
      sale_get: () => ({
        sale: {
          id: "s1",
          number: "INV-000001",
          mode: "installment",
          sale_date: "2026-03-01",
          customer_id: "c1",
          customer: "Karim Uddin",
          grand_total: 6000000,
          paid: 1000000,
          due: 5000000,
        },
        subtotal: 6000000,
        discount_total: 0,
        tax_total: 0,
        financing_charges: 0,
        notes: "",
        cashier: "mgr",
        customer_phone: null,
        customer_address: null,
        items: [],
        payments: [],
      }),
    });
    render(<App />);
    await screen.findByText("Manager One");
    fireEvent.click(screen.getByRole("button", { name: /New Sale/ }));
    fireEvent.click(await screen.findByRole("tab", { name: "Installment" }));
    fireEvent.change(await screen.findByLabelText("Find product"), { target: { value: "fridge" } });
    fireEvent.click(await screen.findByText("Singer Fridge 300L", { selector: "b" }));
    fireEvent.click(
      within(await screen.findByRole("dialog", { name: /Select serial numbers/ })).getByLabelText(
        "F-1",
      ),
    );
    fireEvent.click(screen.getByRole("button", { name: "Add to sale" }));
    fireEvent.change(await screen.findByLabelText("Find customer"), { target: { value: "karim" } });
    fireEvent.click(await screen.findByText("Karim Uddin", { selector: "b" }));
    fireEvent.change(screen.getByPlaceholderText("Amount"), { target: { value: "10000" } });
    const disclosure = within(await screen.findByLabelText("Agreement disclosure"));
    expect(await disclosure.findByText("Cash price")).toBeTruthy();
    expect(disclosure.getByText("৳52,000.00")).toBeTruthy();
    expect(disclosure.getByText("Financed amount")).toBeTruthy();
    expect(disclosure.getByText(/Markup \(0%\)/)).toBeTruthy();
    expect(disclosure.getByText("Total payable in installments")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Complete sale/ }));
    await waitFor(() => expect(create).toHaveBeenCalled());
    const body = (
      create.mock.calls[0] as unknown as [
        {
          sale: {
            mode: string;
            customer_id: string;
            payments: { amount: number }[];
            installment: { count: number; markup_bp: number | null; frequency: string };
          };
        },
      ]
    )[0].sale;
    expect(body.mode).toBe("installment");
    expect(body.customer_id).toBe("c1");
    expect(body.payments).toEqual([{ method: "cash", amount: 1000000, reference: "" }]);
    expect(body.installment).toMatchObject({ count: 6, markup_bp: null, frequency: "monthly" });
  });

  it("only offers the installment type to users who may create agreements", async () => {
    backend({ ...base, session: () => cashier });
    render(<App />);
    await screen.findByText("Cashier One");
    fireEvent.click(screen.getByRole("button", { name: /New Sale/ }));
    expect(screen.queryByRole("tab", { name: "Installment" })).toBeNull();
  });
});
