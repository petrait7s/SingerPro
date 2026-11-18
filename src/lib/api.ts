// Single typed gateway to the Rust backend. Components never call `invoke` directly.
import { invoke } from "@tauri-apps/api/core";

export interface ApiError {
  code: string;
  message: string;
}

export interface AppInfo {
  product: string;
  developer: string;
  version: string;
  data_dir: string;
}
export interface DbStatus {
  schema_version: number;
  journal_mode: string;
  foreign_keys: boolean;
  integrity_ok: boolean;
  integrity_detail: string[];
}
export interface AuditRow {
  id: string;
  at: string;
  actor_id: string | null;
  action: string;
  entity_type: string;
  entity_id: string | null;
  reason: string | null;
}

export function inDesktop(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function toApiError(e: unknown): ApiError {
  if (typeof e === "object" && e !== null && "message" in e) {
    const o = e as { code?: unknown; message: unknown };
    return { code: typeof o.code === "string" ? o.code : "unknown", message: String(o.message) };
  }
  return { code: "unknown", message: String(e) };
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inDesktop()) {
    throw { code: "no_runtime", message: "Open Singer Pro as the desktop app to access data." };
  }
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toApiError(e);
  }
}

export const api = {
  appInfo: () => call<AppInfo>("app_info"),
  dbStatus: () => call<DbStatus>("db_status"),
  recentAudit: (limit = 20) => call<AuditRow[]>("recent_audit", { limit }),
};
