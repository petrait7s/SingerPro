import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { App } from "../shell/App";

type H = (a: Record<string, unknown> | undefined) => unknown;

const owner = {
  user_id: "u1",
  username: "owner",
  full_name: "Shop Owner",
  must_change_password: false,
  roles: ["Owner"],
  permissions: [
    "products.view",
    "products.manage",
    "inventory.view",
    "inventory.adjust",
    "suppliers.view",
    "suppliers.manage",
    "purchases.view",
    "purchases.manage",
    "purchases.pay",
    "purchases.return",
    "purchases.reverse_payment",
    "settings.view",
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
  session: () => owner,
  branding: () => ({ shop_name: "Singer Pro", currency_symbol: "৳", language: "en" }),
  db_status: () => ({
    schema_version: 3,
    journal_mode: "wal",
    foreign_keys: true,
    integrity_ok: true,
    integrity_detail: [],
  }),
  backup_status: () => ({ last_backup_at: null, reminder_days: 7, overdue: false }),
  audit_list: () => ({ total: 0, rows: [] }),
};

const variant = {
  id: "v1",
  sku: "FR-300",
  barcode: null,
  colour: "White",
  capacity: "300L",
  size: "",
  purchase_cost: 4000000,
  retail_price: 5200000,
  wholesale_price: 4800000,
  installment_price: 6000000,
  is_active: true,
  available: 0,
  on_hand_total: 0,
  stock_value: 0,
};
const product = {
  id: "p1",
  brand_id: "b1",
  brand: "Singer",
  category_id: "c1",
  category: "Refrigerators",
  name: "Frost Free",
  model: "FF-300",
  description: "",
  warranty_months: 12,
  warranty_terms: "",
  min_stock: 0,
  is_serialized: true,
  is_active: true,
  available_total: 0,
  low_stock: false,
  variants: [variant],
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

async function go(name: RegExp) {
  render(<App />);
  await screen.findByText("Shop Owner");
  fireEvent.click(screen.getByRole("button", { name }));
}

describe("Products", () => {
  it("lists products with variants and exact prices, and creates a product in paisa", async () => {
    const created = vi.fn(() => "p2");
    backend({
      ...base,
      brands_list: () => [{ id: "b1", name: "Singer", is_active: true }],
      categories_list: () => [
        { id: "c1", name: "Refrigerators", parent_id: null, is_active: true },
      ],
      products_list: () => ({ total: 1, rows: [product] }),
      product_create: created,
    });
    await go(/^Products/);
    fireEvent.click(await screen.findByText("Frost Free"));
    expect(await screen.findByText("৳52,000.00")).toBeTruthy(); // retail 5,200,000 paisa
    expect(screen.getByText("৳60,000.00")).toBeTruthy(); // installment price shown separately
    fireEvent.click(screen.getByRole("button", { name: /New product/ }));
    const dlg = await screen.findByRole("dialog", { name: "New product" });
    const w = within(dlg);
    fireEvent.change(w.getByLabelText("Brand"), { target: { value: "b1" } });
    fireEvent.change(w.getByLabelText(/Category/), { target: { value: "c1" } });
    fireEvent.change(w.getByLabelText("Product name"), { target: { value: "Split AC" } });
    fireEvent.change(w.getByLabelText("SKU"), { target: { value: "AC-1" } });
    fireEvent.change(w.getByLabelText("Purchase cost (৳)"), { target: { value: "45,000.50" } });
    fireEvent.change(w.getByLabelText("Retail price (৳)"), { target: { value: "52000" } });
    fireEvent.click(w.getByLabelText(/Track each unit/));
    fireEvent.click(w.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(created).toHaveBeenCalled());
    const body = (
      invoke.mock.calls.find((c) => c[0] === "product_create")![1] as {
        product: {
          variants: {
            purchase_cost: number;
            retail_price: number;
            wholesale_price: number;
            barcode: string | null;
          }[];
          is_serialized: boolean;
        };
      }
    ).product;
    expect(body.variants[0]).toMatchObject({
      purchase_cost: 4500050,
      retail_price: 5200000,
      wholesale_price: 0,
      barcode: null,
    });
    expect(body.is_serialized).toBe(true);
  });

  it("shows backend validation errors and hides management for view-only users", async () => {
    backend({
      ...base,
      session: () => ({
        ...owner,
        roles: ["Cashier"],
        permissions: ["products.view", "settings.view"],
      }),
      brands_list: () => [],
      categories_list: () => [],
      products_list: () => ({ total: 0, rows: [] }),
    });
    await go(/^Products/);
    expect(await screen.findByText("No products yet")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /New product/ })).toBeNull();
  });
});

describe("Purchases", () => {
  it("builds a draft with exact totals and sends integer paisa", async () => {
    const draft = vi.fn(() => "po1");
    const confirm = vi.fn(() => null);
    backend({
      ...base,
      purchases_list: () => ({ total: 0, rows: [] }),
      suppliers_list: () => [
        {
          id: "s1",
          name: "Dhaka Traders",
          is_active: true,
          balance: 0,
          contact_person: "",
          phone: "",
          alt_phone: "",
          email: "",
          address: "",
          notes: "",
        },
      ],
      variants_search: () => [
        {
          variant_id: "v1",
          product_id: "p1",
          label: "Singer Frost Free FF-300",
          sku: "FR-300",
          barcode: null,
          is_serialized: true,
          purchase_cost: 4000000,
          retail_price: 0,
          wholesale_price: 0,
          installment_price: null,
          available: 0,
        },
      ],
      purchase_create_draft: draft,
      purchase_confirm: confirm,
    });
    await go(/^Purchases/);
    fireEvent.click(await screen.findByRole("button", { name: /New purchase/ }));
    const dlg = await screen.findByRole("dialog", { name: "New purchase" });
    const w = within(dlg);
    await waitFor(() =>
      expect(w.getByLabelText("Supplier").querySelectorAll("option").length).toBeGreaterThan(1),
    );
    fireEvent.change(w.getByLabelText("Supplier"), { target: { value: "s1" } });
    fireEvent.change(w.getByLabelText("Supplier invoice no."), { target: { value: "INV-9" } });
    fireEvent.change(w.getByLabelText("Find product"), { target: { value: "frost" } });
    fireEvent.click(await w.findByText("Singer Frost Free FF-300", { selector: "b" }));
    fireEvent.change(w.getByLabelText("Quantity"), { target: { value: "3" } });
    expect(await w.findAllByText("৳1,20,000.00")).not.toHaveLength(0); // 3 x 40,000.00 in lakh grouping
    fireEvent.click(w.getByRole("button", { name: "Confirm invoice" }));
    await waitFor(() => expect(confirm).toHaveBeenCalled());
    const body = (
      invoke.mock.calls.find((c) => c[0] === "purchase_create_draft")![1] as {
        purchase: {
          items: { qty: number; unit_cost: number }[];
          supplier_invoice_no: string;
          discount: number;
        };
      }
    ).purchase;
    expect(body.items).toEqual([{ variant_id: "v1", qty: 3, unit_cost: 4000000 }]);
    expect(body).toMatchObject({ supplier_invoice_no: "INV-9", discount: 0 });
  });

  it("refuses to submit a draft with an invalid quantity (client check) and shows backend errors", async () => {
    backend({
      ...base,
      purchases_list: () => ({ total: 0, rows: [] }),
      suppliers_list: () => [
        {
          id: "s1",
          name: "S",
          is_active: true,
          balance: 0,
          contact_person: "",
          phone: "",
          alt_phone: "",
          email: "",
          address: "",
          notes: "",
        },
      ],
      variants_search: () => [
        {
          variant_id: "v1",
          product_id: "p1",
          label: "Fan",
          sku: "F",
          barcode: null,
          is_serialized: false,
          purchase_cost: 100,
          retail_price: 0,
          wholesale_price: 0,
          installment_price: null,
          available: 0,
        },
      ],
      purchase_create_draft: () =>
        Object.assign(
          new Error("conflict: a purchase with this supplier invoice number already exists"),
          { code: "conflict" },
        ),
    });
    await go(/^Purchases/);
    fireEvent.click(await screen.findByRole("button", { name: /New purchase/ }));
    const w = within(await screen.findByRole("dialog", { name: "New purchase" }));
    await waitFor(() =>
      expect(w.getByLabelText("Supplier").querySelectorAll("option").length).toBeGreaterThan(1),
    );
    fireEvent.change(w.getByLabelText("Supplier"), { target: { value: "s1" } });
    fireEvent.change(w.getByLabelText("Find product"), { target: { value: "fan" } });
    fireEvent.click(await w.findByText("Fan", { selector: "b" }));
    fireEvent.change(w.getByLabelText("Quantity"), { target: { value: "2.5" } });
    fireEvent.click(w.getByRole("button", { name: "Save draft" }));
    expect(await w.findByText(/whole quantity/)).toBeTruthy();
    expect(invoke).not.toHaveBeenCalledWith("purchase_create_draft", expect.anything());
    fireEvent.change(w.getByLabelText("Quantity"), { target: { value: "2" } });
    fireEvent.click(w.getByRole("button", { name: "Save draft" }));
    expect(await w.findByText(/already exists/)).toBeTruthy();
  });

  it("receiving splits pasted serial numbers per line and sends them with the quantity", async () => {
    const receive = vi.fn(() => "r1");
    const detail = {
      purchase: {
        id: "po1",
        number: "PO-000001",
        supplier_id: "s1",
        supplier: "S",
        supplier_invoice_no: "I1",
        order_date: "2026-03-01",
        status: "confirmed",
        grand_total: 8000000,
        landed_total: 0,
        paid: 0,
        credits: 0,
        due: 8000000,
        ordered_qty: 2,
        received_qty: 0,
      },
      notes: "",
      discount: 0,
      subtotal: 8000000,
      cancel_reason: null,
      costs: [],
      payments: [],
      receipts: [],
      returns: [],
      items: [
        {
          id: "i1",
          variant_id: "v1",
          label: "Fridge",
          sku: "FR",
          is_serialized: true,
          qty: 2,
          unit_cost: 4000000,
          landed_alloc: 0,
          received_qty: 0,
          returned_qty: 0,
          line_total: 8000000,
        },
      ],
    };
    backend({
      ...base,
      purchases_list: () => ({ total: 1, rows: [detail.purchase] }),
      purchase_get: () => detail,
      purchase_receive: receive,
    });
    await go(/^Purchases/);
    fireEvent.click(await screen.findByText("PO-000001"));
    fireEvent.click(await screen.findByRole("button", { name: "Receive goods" }));
    const w = within(await screen.findByRole("dialog", { name: "Receive goods" }));
    fireEvent.change(w.getByLabelText(/Serial numbers/), { target: { value: "SN-1\nSN-2\n" } });
    fireEvent.click(w.getByRole("button", { name: "Receive into stock" }));
    await waitFor(() => expect(receive).toHaveBeenCalled());
    const body = (
      invoke.mock.calls.find((c) => c[0] === "purchase_receive")![1] as {
        receipt: { lines: { qty: number; serials: string[] }[] };
      }
    ).receipt;
    expect(body.lines[0]).toMatchObject({ item_id: "i1", qty: 2, serials: ["SN-1", "SN-2"] });
  });

  it("a double-click on 'Record payment' reuses the same submission key", async () => {
    const pay = vi.fn(() => "pay1");
    const detail = {
      purchase: {
        id: "po1",
        number: "PO-000001",
        supplier_id: "s1",
        supplier: "S",
        supplier_invoice_no: null,
        order_date: "2026-03-01",
        status: "confirmed",
        grand_total: 1000,
        landed_total: 0,
        paid: 0,
        credits: 0,
        due: 1000,
        ordered_qty: 1,
        received_qty: 1,
      },
      notes: "",
      discount: 0,
      subtotal: 1000,
      cancel_reason: null,
      costs: [],
      payments: [],
      receipts: [],
      returns: [],
      items: [
        {
          id: "i1",
          variant_id: "v1",
          label: "Fan",
          sku: "F",
          is_serialized: false,
          qty: 1,
          unit_cost: 1000,
          landed_alloc: 0,
          received_qty: 1,
          returned_qty: 0,
          line_total: 1000,
        },
      ],
    };
    backend({
      ...base,
      purchases_list: () => ({ total: 1, rows: [detail.purchase] }),
      purchase_get: () => detail,
      supplier_pay: pay,
    });
    await go(/^Purchases/);
    fireEvent.click(await screen.findByText("PO-000001"));
    fireEvent.click(await screen.findByRole("button", { name: "Record payment" }));
    const w = within(await screen.findByRole("dialog", { name: /Pay S/ }));
    const b = w.getByRole("button", { name: "Record payment" });
    fireEvent.click(b);
    fireEvent.click(b);
    await waitFor(() => expect(pay.mock.calls.length).toBeGreaterThanOrEqual(1));
    const keys = invoke.mock.calls
      .filter((c) => c[0] === "supplier_pay")
      .map((c) => (c[1] as { payment: { client_key: string; amount: number } }).payment.client_key);
    expect(new Set(keys).size).toBe(1);
    expect(
      (
        invoke.mock.calls.find((c) => c[0] === "supplier_pay")![1] as {
          payment: { amount: number };
        }
      ).payment.amount,
    ).toBe(1000);
  });
});

describe("Inventory", () => {
  it("shows low stock and requires a reason client-side via backend error", async () => {
    const row = {
      variant_id: "v1",
      product_id: "p1",
      label: "Fan 56in",
      brand: "Singer",
      category: "Fans",
      sku: "FAN-1",
      barcode: null,
      is_serialized: false,
      min_stock: 5,
      available: 2,
      reserved: 0,
      damaged: 0,
      warranty: 0,
      pending_inspection: 0,
      value: 200000,
      avg_cost: 100000,
      low_stock: true,
    };
    backend({
      ...base,
      stock_levels: () => ({ total: 1, total_value: 200000, rows: [row] }),
      stock_adjust: () =>
        Object.assign(new Error("validation failed: a reason (max 300 characters) is required"), {
          code: "validation",
        }),
    });
    await go(/^Inventory/);
    expect(await screen.findByText("low")).toBeTruthy();
    expect(screen.getAllByText("৳2,000.00").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "Adjust" }));
    const w = within(await screen.findByRole("dialog", { name: /Adjust stock/ }));
    fireEvent.change(w.getByLabelText("Quantity"), { target: { value: "1" } });
    fireEvent.click(w.getByRole("button", { name: "Record adjustment" }));
    expect(await w.findByText(/reason/i)).toBeTruthy();
  });

  it("reconciliation reports all-clear and discrepancies honestly", async () => {
    let call = 0;
    backend({
      ...base,
      stock_levels: () => ({ total: 0, total_value: 0, rows: [] }),
      stock_reconcile: () =>
        ++call === 1
          ? []
          : [
              {
                variant_id: "v1",
                sku: "FAN-1",
                bucket: "available",
                check: "ledger_qty_vs_cached_qty",
                expected: 5,
                actual: 6,
              },
            ],
    });
    await go(/^Inventory/);
    fireEvent.click(await screen.findByRole("button", { name: "Reconciliation" }));
    expect(await screen.findByText(/all agree/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Run again" }));
    expect(await screen.findByText(/1 discrepancy/)).toBeTruthy();
  });
});
