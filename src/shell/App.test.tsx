import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { App } from "./App";

type Handler = (args: Record<string, unknown> | undefined) => unknown;

function principal(
  over: Partial<{ roles: string[]; permissions: string[]; must_change_password: boolean }> = {},
) {
  return {
    user_id: "u1",
    username: "tester",
    full_name: "Test User",
    must_change_password: false,
    roles: ["Cashier"],
    permissions: ["sales.create", "sales.view"],
    ...over,
  };
}

function backend(handlers: Record<string, Handler>) {
  invoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    const h = handlers[cmd];
    if (!h) throw { code: "unknown", message: `unhandled command ${cmd}` };
    const r = h(args);
    if (r instanceof Error)
      throw { code: (r as Error & { code?: string }).code ?? "unknown", message: r.message };
    return r;
  });
}
const fail = (code: string, message: string) => Object.assign(new Error(message), { code });

const base = {
  setup_status: () => ({ needs_setup: false, startup_notice: null }),
  branding: () => ({ shop_name: "Singer Pro", currency_symbol: "৳", language: "en" }),
  db_status: () => ({
    schema_version: 2,
    journal_mode: "wal",
    foreign_keys: true,
    integrity_ok: true,
    integrity_detail: [],
  }),
  backup_status: () => ({ last_backup_at: null, reminder_days: 7, overdue: false }),
  audit_list: () => ({ total: 0, rows: [] }),
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

describe("auth gate", () => {
  it("shows first-run setup (no default password) and validates confirmation locally", async () => {
    backend({ ...base, setup_status: () => ({ needs_setup: true, startup_notice: null }) });
    render(<App />);
    expect(await screen.findByText("Welcome to Singer Pro")).toBeTruthy();
    expect(screen.getByText(/no default password/i)).toBeTruthy();
    fireEvent.change(screen.getByLabelText(/Full name/), { target: { value: "Shop Owner" } });
    fireEvent.change(screen.getByLabelText(/^Username/), { target: { value: "owner" } });
    fireEvent.change(screen.getByLabelText(/^Password$/), { target: { value: "Correct-horse-1" } });
    fireEvent.change(screen.getByLabelText(/Confirm password/), { target: { value: "different" } });
    fireEvent.click(screen.getByRole("button", { name: /Create owner account/ }));
    expect(await screen.findByText(/do not match/i)).toBeTruthy();
    expect(invoke).not.toHaveBeenCalledWith("setup_owner", expect.anything());
  });

  it("setup creates the owner via the backend and enters the app", async () => {
    backend({
      ...base,
      setup_status: () => ({ needs_setup: true, startup_notice: null }),
      setup_owner: () => principal({ roles: ["Owner"], permissions: ["settings.view"] }),
    });
    render(<App />);
    await screen.findByText("Welcome to Singer Pro");
    fireEvent.change(screen.getByLabelText(/Full name/), { target: { value: "Shop Owner" } });
    fireEvent.change(screen.getByLabelText(/^Username/), { target: { value: "owner" } });
    fireEvent.change(screen.getByLabelText(/^Password$/), { target: { value: "Correct-horse-1" } });
    fireEvent.change(screen.getByLabelText(/Confirm password/), {
      target: { value: "Correct-horse-1" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Create owner account/ }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("setup_owner", {
        username: "owner",
        fullName: "Shop Owner",
        password: "Correct-horse-1",
      }),
    );
    expect(await screen.findByText("Test User")).toBeTruthy();
  });

  it("shows the backend's login error and keeps the user signed out", async () => {
    backend({
      ...base,
      session: () => null,
      login: () => fail("validation", "invalid username or password"),
    });
    render(<App />);
    fireEvent.change(await screen.findByLabelText(/Username/), { target: { value: "x" } });
    fireEvent.change(screen.getByLabelText(/Password/), { target: { value: "y" } });
    fireEvent.click(screen.getByRole("button", { name: /Sign in/ }));
    expect(await screen.findByText("invalid username or password")).toBeTruthy();
    expect(screen.queryByText("Dashboard")).toBeNull();
  });

  it("forces a password change before anything else", async () => {
    backend({ ...base, session: () => principal({ must_change_password: true }) });
    render(<App />);
    expect(await screen.findByText("Choose a new password")).toBeTruthy();
    expect(screen.queryByRole("navigation")).toBeNull();
  });

  it("returns to the login screen when the session is lost mid-use", async () => {
    let n = 0;
    backend({
      ...base,
      session: () => principal(),
      db_status: () => {
        n++;
        return fail("unauthenticated", "not signed in or session expired");
      },
    });
    render(<App />);
    await screen.findByText("Test User");
    await waitFor(() => expect(n).toBeGreaterThan(0));
    expect(await screen.findByRole("button", { name: /Sign in/ })).toBeTruthy();
  });
});

describe("permission-aware shell", () => {
  it("hides modules the user cannot use", async () => {
    backend({ ...base, session: () => principal() });
    render(<App />);
    await screen.findByText("Test User");
    expect(screen.getByRole("button", { name: /New Sale/ })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Staff & Users/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /Settings/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /Reports/ })).toBeNull();
  });

  it("owner-level users see administration", async () => {
    backend({
      ...base,
      session: () =>
        principal({
          roles: ["Owner"],
          permissions: ["users.view", "settings.view", "audit.view", "sales.view"],
        }),
    });
    render(<App />);
    await screen.findByText("Test User");
    expect(screen.getByRole("button", { name: /Staff & Users/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Settings/ })).toBeTruthy();
  });
});

describe("bilingual UI", () => {
  it("switches the interface to Bangla and back", async () => {
    backend({ ...base, session: () => null });
    render(<App />);
    expect(await screen.findByRole("button", { name: /Sign in/ })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Language/ }));
    expect(await screen.findByRole("button", { name: "সাইন ইন করুন" })).toBeTruthy();
    expect(document.documentElement.lang).toBe("bn");
    expect(localStorage.getItem("sp.lang")).toBe("bn");
    fireEvent.click(screen.getByRole("button", { name: /ভাষা/ }));
    expect(await screen.findByRole("button", { name: "Sign in" })).toBeTruthy();
  });
});

describe("outside the desktop runtime", () => {
  it("shows an error, never fake data", async () => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    render(<App />);
    expect(await screen.findByRole("alert")).toBeTruthy();
  });
});
