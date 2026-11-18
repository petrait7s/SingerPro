import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
const openDialog = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (...a: unknown[]) => openDialog(...a) }));

import { App } from "../shell/App";

type H = (a: Record<string, unknown> | undefined) => unknown;

const owner = {
  user_id: "u1",
  username: "owner",
  full_name: "Owner One",
  must_change_password: false,
  roles: ["Owner"],
  permissions: ["reports.operational", "reports.financial", "sales.view"],
};

function backend(h: Record<string, H>) {
  invoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    const fn = h[cmd];
    if (!fn) throw { code: "unknown", message: `unhandled ${cmd}` };
    return fn(args);
  });
}

const base = {
  setup_status: () => ({ needs_setup: false, startup_notice: null }),
  session: () => owner,
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
  audit_list: () => ({ total: 0, rows: [] }),
};

const kpi = {
  today: "2026-04-02",
  sales: { today_net: 1100000, today_invoices: 4, month_net: 2500000, month_invoices: 9 },
  money: { cash_and_bank: 7500000, receivables: 4200000, payables: 300000, month_profit: -50000 },
  stock: { value: 90000000, low_items: 2 },
  installments: { overdue_count: 3, overdue_amount: 750000, due_this_week: 100000 },
  pending: {
    returns_awaiting_inspection: 1,
    expenses_awaiting_approval: 2,
    open_warranty_cases: 0,
  },
  last_days: [["2026-04-02", 1100000]],
};

const report = {
  id: "sales_by_mode",
  title: "Sales by mode",
  period: "2026-04-01 to 2026-04-02",
  columns: [
    { key: "mode", label: "Mode", kind: "text" },
    { key: "invoices", label: "Invoices", kind: "int" },
    { key: "net", label: "Net sales", kind: "money" },
  ],
  rows: [
    ["retail", 3, 1100000],
    ["wholesale", 1, 650000],
  ],
  totals: ["Total", 4, 1750000],
};

beforeEach(() => {
  invoke.mockReset();
  openDialog.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  localStorage.clear();
});
afterEach(() => {
  cleanup();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

describe("Dashboard KPIs", () => {
  it("shows figures from the backend and flags problems", async () => {
    backend({ ...base, dashboard_get: () => kpi });
    render(<App />);
    const tiles = await screen.findByTestId("kpis");
    expect(within(tiles).getByText("৳11,000.00")).toBeTruthy();
    expect(within(tiles).getByText("৳75,000.00")).toBeTruthy();
    expect(within(tiles).getByText(/3 agreements/)).toBeTruthy();
    expect(within(tiles).getByText(/2 items at or below minimum/)).toBeTruthy();
    expect(within(tiles).getByText(/1 return\(s\) awaiting inspection/)).toBeTruthy();
    expect(within(tiles).getByText("-৳500.00")).toBeTruthy();
  });

  it("shows only the tiles the backend returned for a limited role", async () => {
    backend({
      ...base,
      dashboard_get: () => ({ ...kpi, money: null, stock: null, installments: null }),
    });
    render(<App />);
    const tiles = await screen.findByTestId("kpis");
    expect(within(tiles).getByText("Sales today (net)")).toBeTruthy();
    expect(within(tiles).queryByText("Cash and bank")).toBeNull();
    expect(within(tiles).queryByText("Stock value")).toBeNull();
  });
});

describe("Reports", () => {
  it("runs the chosen report with its period and renders totals", async () => {
    const run = vi.fn(() => report);
    backend({
      ...base,
      dashboard_get: () => kpi,
      reports_catalogue: () => [
        { id: "sales_by_mode", title: "Sales by mode", group: "Sales", period: "range" },
        { id: "balance_sheet", title: "Balance sheet", group: "Financial", period: "as_of" },
      ],
      report_run: run,
    });
    render(<App />);
    await screen.findByText("Owner One");
    fireEvent.click(screen.getByRole("button", { name: /^Reports/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Show report" }));
    expect(await screen.findByText("wholesale")).toBeTruthy();
    expect(screen.getByText("৳17,500.00")).toBeTruthy();
    const args = (
      run.mock.calls[0] as unknown as [{ reportId: string; params: Record<string, string> }]
    )[0];
    expect(args.reportId).toBe("sales_by_mode");
    expect(args.params.from).toMatch(/^\d{4}-\d{2}-01$/);
    expect(args.params.to).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });

  it("exports to the folder the user picks, and a failure is shown, not hidden", async () => {
    const exp = vi.fn(() => "C:\\Exports\\sales_by_mode.csv");
    backend({
      ...base,
      dashboard_get: () => kpi,
      reports_catalogue: () => [
        { id: "sales_by_mode", title: "Sales by mode", group: "Sales", period: "range" },
      ],
      report_run: () => report,
      report_export: exp,
    });
    openDialog.mockResolvedValue("C:\\Exports");
    render(<App />);
    await screen.findByText("Owner One");
    fireEvent.click(screen.getByRole("button", { name: /^Reports/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Show report" }));
    fireEvent.click(await screen.findByRole("button", { name: "CSV" }));
    await waitFor(() => expect(exp).toHaveBeenCalled());
    expect(exp).toHaveBeenCalledWith(
      expect.objectContaining({ reportId: "sales_by_mode", format: "csv", destDir: "C:\\Exports" }),
    );
    expect(await screen.findByText(/Saved C:\\Exports\\sales_by_mode.csv/)).toBeTruthy();
  });

  it("shows the backend's message when a report is refused", async () => {
    backend({
      ...base,
      dashboard_get: () => kpi,
      reports_catalogue: () => [
        { id: "profit_loss", title: "Profit and loss", group: "Financial", period: "range" },
      ],
      report_run: () => {
        throw { code: "validation", message: "the start date is after the end date" };
      },
    });
    render(<App />);
    await screen.findByText("Owner One");
    fireEvent.click(screen.getByRole("button", { name: /^Reports/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Show report" }));
    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      "the start date is after the end date",
    );
  });
});

describe("Import", () => {
  const importer = {
    ...base,
    session: () => ({
      ...owner,
      permissions: ["settings.view", "products.manage", "customers.manage"],
    }),
    dashboard_get: () => ({ ...kpi, sales: null, money: null, stock: null, installments: null }),
  };

  async function openImport() {
    render(<App />);
    await screen.findByText("Owner One");
    fireEvent.click(screen.getByRole("button", { name: /^Settings/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Import data" }));
  }
  const pick = (name: string, body: string) =>
    fireEvent.change(screen.getByLabelText("CSV file"), {
      target: { files: [new File([body], name, { type: "text/csv" })] },
    });

  it("lists row problems and offers no import button until the file is clean", async () => {
    const run = vi.fn((a) =>
      (a as { commit: boolean }).commit
        ? { kind: "products", rows: 2, errors: [], imported: true, created: 2 }
        : {
            kind: "products",
            rows: 2,
            errors: [{ line: 3, message: "sku 'X' already exists" }],
            imported: false,
            created: 0,
          },
    );
    backend({ ...importer, import_run: run });
    await openImport();
    pick("p.csv", "brand,sku\nA,X");
    fireEvent.click(await screen.findByRole("button", { name: "Check file" }));
    expect(await screen.findByText("sku 'X' already exists")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^Import \d+ row/ })).toBeNull();
    expect(run).toHaveBeenCalledWith({ kind: "products", text: "brand,sku\nA,X", commit: false });
  });

  it("imports only after a clean check", async () => {
    const run = vi.fn((a) =>
      (a as { commit: boolean }).commit
        ? { kind: "products", rows: 2, errors: [], imported: true, created: 2 }
        : { kind: "products", rows: 2, errors: [], imported: false, created: 0 },
    );
    backend({ ...importer, import_run: run });
    await openImport();
    pick("p.csv", "brand,sku\nA,Y");
    fireEvent.click(await screen.findByRole("button", { name: "Check file" }));
    fireEvent.click(await screen.findByRole("button", { name: "Import 2 row(s)" }));
    await waitFor(() => expect(run).toHaveBeenCalledTimes(2));
    expect(run).toHaveBeenLastCalledWith({
      kind: "products",
      text: "brand,sku\nA,Y",
      commit: true,
    });
    expect(await screen.findByText("2 record(s) added.")).toBeTruthy();
  });
});
