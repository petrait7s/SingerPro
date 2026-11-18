import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { App } from "../shell/App";

type H = (a: Record<string, unknown> | undefined) => unknown;

const manager = {
  user_id: "u1",
  username: "mgr",
  full_name: "Manager One",
  must_change_password: false,
  roles: ["Manager"],
  permissions: [
    "returns.process",
    "returns.approve",
    "warranty.manage",
    "expenses.create",
    "expenses.approve",
    "cash.session",
    "sales.view",
  ],
};
const clerk = {
  ...manager,
  full_name: "Clerk One",
  roles: ["Clerk"],
  permissions: ["returns.process", "expenses.create", "sales.view"],
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
    schema_version: 6,
    journal_mode: "wal",
    foreign_keys: true,
    integrity_ok: true,
    integrity_detail: [],
  }),
  backup_status: () => ({ last_backup_at: null, reminder_days: 7, overdue: false }),
};

const ret = {
  id: "r1",
  number: "RET-000001",
  sale_id: "s1",
  sale_number: "INV-000001",
  customer: "Karim Uddin",
  status: "requested",
  return_date: "2026-04-02",
  reason: "Not cooling",
  value: 5200000,
};
const retDetail = {
  row: ret,
  decision_note: null,
  items: [
    {
      id: "ri1",
      label: "Singer Fridge",
      sku: "FRIDGE",
      serial_no: "F-1",
      qty: 1,
      value: 5200000,
      outcome: null,
      note: null,
    },
  ],
  refunds: [],
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

describe("Returns & Warranty", () => {
  it("an approver must choose an inspection outcome for every item before approving", async () => {
    const approve = vi.fn(() => ({ value: 5200000, refund_receipt: "RCT-000010" }));
    backend({
      ...base,
      returns_list: () => [ret],
      return_get: () => retDetail,
      return_approve: approve,
    });
    render(<App />);
    await screen.findByText("Manager One");
    fireEvent.click(screen.getByRole("button", { name: /^Returns/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Inspect" }));
    const dlg = await screen.findByRole("dialog", { name: "Return RET-000001" });
    fireEvent.click(within(dlg).getByRole("button", { name: "Approve" }));
    expect(await within(dlg).findByText(/inspection outcome for every item/)).toBeTruthy();
    expect(approve).not.toHaveBeenCalled();
    fireEvent.change(within(dlg).getByLabelText("Outcome for Singer Fridge"), {
      target: { value: "restock" },
    });
    fireEvent.click(within(dlg).getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(approve).toHaveBeenCalled());
    const args = (
      approve.mock.calls[0] as unknown as [{ returnId: string; approval: Record<string, unknown> }]
    )[0];
    expect(args.returnId).toBe("r1");
    expect(args.approval.decisions).toEqual([{ item_id: "ri1", outcome: "restock", note: "" }]);
    expect(args.approval.refund_amount).toBe(0);
  });

  it("a user without approval permission can only view a pending return", async () => {
    backend({
      ...base,
      session: () => clerk,
      returns_list: () => [ret],
      return_get: () => retDetail,
      warranty_list: () => [],
    });
    render(<App />);
    await screen.findByText("Clerk One");
    fireEvent.click(screen.getByRole("button", { name: /^Returns/ }));
    expect(await screen.findByText("awaiting inspection")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "View" }));
    const dlg = await screen.findByRole("dialog", { name: "Return RET-000001" });
    expect(within(dlg).queryByRole("button", { name: "Approve" })).toBeNull();
    expect(within(dlg).queryByRole("button", { name: "Reject return" })).toBeNull();
  });

  it("records a return for an invoice found by serial number, sending only chosen quantities", async () => {
    const record = vi.fn(() => "r9");
    backend({
      ...base,
      returns_list: () => [],
      return_find_sale_by_serial: () => ["s1", "INV-000001"],
      returnable_items: () => [
        {
          sale_item_id: "si1",
          label: "Singer Fridge",
          sku: "FRIDGE",
          serial_no: "F-1",
          serial_status: "sold",
          sold_qty: 1,
          returnable_qty: 1,
          unit_value: 5200000,
        },
      ],
      return_record: record,
    });
    render(<App />);
    await screen.findByText("Manager One");
    fireEvent.click(screen.getByRole("button", { name: /^Returns/ }));
    fireEvent.click(await screen.findByRole("button", { name: /New return/ }));
    const dlg = await screen.findByRole("dialog", { name: "New return" });
    fireEvent.change(within(dlg).getByLabelText("Invoice number or serial number"), {
      target: { value: "F-1" },
    });
    fireEvent.click(within(dlg).getByRole("button", { name: "Find" }));
    fireEvent.change(await within(dlg).findByLabelText("Return quantity for Singer Fridge"), {
      target: { value: "1" },
    });
    fireEvent.click(within(dlg).getByRole("button", { name: "Record return" }));
    expect(await within(dlg).findByText(/reason for the return/)).toBeTruthy();
    expect(record).not.toHaveBeenCalled();
    fireEvent.change(within(dlg).getByLabelText("Reason"), { target: { value: "Faulty" } });
    fireEvent.click(within(dlg).getByRole("button", { name: "Record return" }));
    await waitFor(() => expect(record).toHaveBeenCalled());
    const req = (record.mock.calls[0] as unknown as [{ request: Record<string, unknown> }])[0]
      .request;
    expect(req.sale_id).toBe("s1");
    expect(req.lines).toEqual([{ sale_item_id: "si1", qty: 1 }]);
    expect(String(req.client_key).length).toBeGreaterThan(8);
  });

  it("warranty lookup shows coverage and blocks cases on unsold units", async () => {
    const cov = {
      serial_id: "x",
      serial_no: "F-1",
      variant_id: "v",
      label: "Singer Fridge",
      sale_id: "s1",
      sale_number: "INV-000001",
      customer_id: "c1",
      customer: "Karim Uddin",
      purchased_on: "2026-01-10",
      warranty_months: 12,
      warranty_terms: "Parts and labour",
      expires_on: "2027-01-10",
      in_warranty: true,
      status: "sold",
    };
    backend({
      ...base,
      returns_list: () => [],
      warranty_list: () => [],
      warranty_lookup: (a) =>
        a?.serialNo === "F-1"
          ? cov
          : { ...cov, serial_no: "F-9", sale_id: null, sale_number: null },
    });
    render(<App />);
    await screen.findByText("Manager One");
    fireEvent.click(screen.getByRole("button", { name: /^Returns/ }));
    fireEvent.click(await screen.findByRole("tab", { name: "Warranty" }));
    fireEvent.click(await screen.findByRole("button", { name: /Check serial/ }));
    const dlg = await screen.findByRole("dialog", { name: "Warranty lookup" });
    fireEvent.change(within(dlg).getByLabelText("Serial number"), { target: { value: "F-1" } });
    fireEvent.click(within(dlg).getByRole("button", { name: "Check" }));
    expect(await within(dlg).findByText("In warranty")).toBeTruthy();
    expect(within(dlg).getByText(/until 2027-01-10/)).toBeTruthy();
    expect(within(dlg).getByRole("button", { name: "Open warranty case" })).toBeTruthy();
    fireEvent.change(within(dlg).getByLabelText("Serial number"), { target: { value: "F-9" } });
    fireEvent.click(within(dlg).getByRole("button", { name: "Check" }));
    expect(await within(dlg).findByText(/no claim possible/)).toBeTruthy();
    expect(within(dlg).queryByRole("button", { name: "Open warranty case" })).toBeNull();
  });
});

describe("Expenses and cash session", () => {
  const exp = {
    id: "e1",
    number: "EXP-000001",
    expense_date: "2026-04-02",
    category: "Rent",
    amount: 2500000,
    method: "bank",
    payee: "Landlord",
    description: "April rent",
    status: "pending",
    created_by: "clerk",
    decided_by: null,
    decision_note: null,
  };

  it("approvers see approve/reject on pending expenses; creators alone do not", async () => {
    const approve = vi.fn(() => undefined);
    backend({
      ...base,
      expenses_list: () => ({ rows: [exp], approved_total: 0, pending_count: 1 }),
      expense_approve: approve,
    });
    const { unmount } = render(<App />);
    await screen.findByText("Manager One");
    fireEvent.click(screen.getByRole("button", { name: /^Expenses/ }));
    expect(await screen.findByText("April rent")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(approve).toHaveBeenCalledWith({ expenseId: "e1" }));
    unmount();
    cleanup();
    backend({
      ...base,
      session: () => clerk,
      expenses_list: () => ({ rows: [exp], approved_total: 0, pending_count: 1 }),
    });
    render(<App />);
    await screen.findByText("Clerk One");
    fireEvent.click(screen.getByRole("button", { name: /^Expenses/ }));
    expect(await screen.findByText("April rent")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Approve" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Categories" })).toBeNull();
  });

  it("closing a session shows the variance live and sends the counted amount in paisa", async () => {
    const session = {
      id: "cs1",
      number: "CSH-000001",
      status: "open",
      business_date: "2026-04-02",
      opening_cash: 1000000,
      opening_note: "",
      opened_by: "mgr",
      opened_at: "2026-04-02T03:00:00.000Z",
      closed_by: null,
      closed_at: null,
      expected_cash: null,
      counted_cash: null,
      variance: null,
      close_note: null,
    };
    const summary = {
      session,
      sales_by_mode: [],
      cash_sale_receipts: 5200000,
      cash_installment_receipts: 0,
      cash_refunds: 0,
      cash_expenses: 120000,
      cash_supplier_payments: 0,
      expected_cash: 6080000,
      receipts_by_method: [],
      refunds_total: 0,
      expenses_total: 120000,
    };
    const close = vi.fn(() => summary);
    backend({
      ...base,
      session: () => ({ ...manager, permissions: ["cash.session"] }),
      cash_current: () => session,
      cash_sessions: () => [session],
      cash_summary: () => summary,
      cash_close: close,
    });
    render(<App />);
    await screen.findByText("Manager One");
    fireEvent.click(screen.getByRole("button", { name: /^Expenses/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Count and close" }));
    const dlg = await screen.findByRole("dialog", { name: "Count and close" });
    fireEvent.change(within(dlg).getByLabelText("Counted cash (৳)"), {
      target: { value: "60,795.50" },
    });
    expect(await within(dlg).findByText(/Shortage of/)).toBeTruthy();
    fireEvent.click(within(dlg).getByRole("button", { name: "Close session" }));
    await waitFor(() => expect(close).toHaveBeenCalled());
    expect(close).toHaveBeenCalledWith({
      sessionId: "cs1",
      countedCash: 6079550,
      note: "",
    });
  });
});
