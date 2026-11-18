// Single typed gateway to the Rust backend. Components never call `invoke` directly.
// The session token is held in Rust; the webview never sees it.
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
export interface AuditPage {
  total: number;
  rows: AuditRow[];
}
export interface Principal {
  user_id: string;
  username: string;
  full_name: string;
  must_change_password: boolean;
  roles: string[];
  permissions: string[];
}
export interface RestoreOutcome {
  applied: boolean;
  message: string;
  replaced_dir: string | null;
}
export interface SetupStatus {
  needs_setup: boolean;
  startup_notice: RestoreOutcome | null;
}
export interface UserRow {
  id: string;
  username: string;
  full_name: string;
  is_active: boolean;
  must_change_password: boolean;
  roles: string[];
  last_login_at: string | null;
  created_at: string;
}
export interface RoleRow {
  id: string;
  name: string;
  description: string;
  is_system: boolean;
  permissions: string[];
}
export interface PermissionRow {
  key: string;
  category: string;
  description: string;
}
export interface NewUser {
  username: string;
  full_name: string;
  password: string;
  role_ids: string[];
}
export interface Branding {
  shop_name: string;
  currency_symbol: string;
  language: string;
}
export interface BackupInfo {
  path: string;
  manifest_path: string;
  size_bytes: number;
  sha256: string;
  schema_version: number;
}
export interface Manifest {
  created_at: string;
  app_version: string;
  schema_version: number;
}
export interface Preview {
  path: string;
  size_bytes: number;
  sha256: string;
  manifest: Manifest | null;
  checksum_ok: boolean | null;
  schema_version: number | null;
  app_schema_version: number;
  users: number | null;
  audit_rows: number | null;
  problems: string[];
  warnings: string[];
  can_restore: boolean;
}
export interface HistoryRow {
  id: string;
  at: string;
  kind: string;
  path: string;
  size_bytes: number | null;
  status: string;
  note: string | null;
}
export interface BackupStatus {
  last_backup_at: string | null;
  reminder_days: number;
  overdue: boolean;
}
export interface RestoreMarker {
  staged_at: string;
  source: string;
  sha256: string;
  safety_copy: string;
}
export interface StagedRestore {
  marker: RestoreMarker;
  preview: Preview;
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

type Listener = () => void;
const sessionLost = new Set<Listener>();
/** Fires when any call reports the session ended (expired, deactivated). */
export function onSessionLost(l: Listener): () => void {
  sessionLost.add(l);
  return () => sessionLost.delete(l);
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inDesktop()) {
    throw { code: "no_runtime", message: "Open Singer Pro as the desktop app to access data." };
  }
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    const err = toApiError(e);
    if (err.code === "unauthenticated" && cmd !== "session") sessionLost.forEach((l) => l());
    throw err;
  }
}

export const api = {
  appInfo: () => call<AppInfo>("app_info"),
  setupStatus: () => call<SetupStatus>("setup_status"),
  setupOwner: (username: string, fullName: string, password: string) =>
    call<Principal>("setup_owner", { username, fullName, password }),
  login: (username: string, password: string) => call<Principal>("login", { username, password }),
  logout: () => call<void>("logout"),
  session: () => call<Principal | null>("session"),
  changePassword: (old: string, next: string) => call<void>("change_password", { old, new: next }),
  dbStatus: () => call<DbStatus>("db_status"),
  auditList: (limit: number, offset: number, actionPrefix?: string) =>
    call<AuditPage>("audit_list", { limit, offset, actionPrefix }),

  usersList: () => call<UserRow[]>("users_list"),
  rolesList: () => call<RoleRow[]>("roles_list"),
  permissionsList: () => call<PermissionRow[]>("permissions_list"),
  userCreate: (user: NewUser) => call<string>("user_create", { user }),
  userSetActive: (userId: string, active: boolean, reason: string) =>
    call<void>("user_set_active", { userId, active, reason }),
  userSetRoles: (userId: string, roleIds: string[]) =>
    call<void>("user_set_roles", { userId, roleIds }),
  userResetPassword: (userId: string, password: string) =>
    call<void>("user_reset_password", { userId, password }),
  roleCreate: (name: string, description: string, permissions: string[]) =>
    call<string>("role_create", { name, description, permissions }),
  roleSetPermissions: (roleId: string, permissions: string[]) =>
    call<void>("role_set_permissions", { roleId, permissions }),
  roleDelete: (roleId: string) => call<void>("role_delete", { roleId }),

  settingsGet: () => call<Record<string, string>>("settings_get"),
  settingsUpdate: (changes: Record<string, string>) =>
    call<string[]>("settings_update", { changes }),
  branding: () => call<Branding>("branding"),
  logoGet: () => call<string | null>("logo_get"),
  logoSet: (dataBase64: string) => call<void>("logo_set", { dataBase64 }),
  logoClear: () => call<void>("logo_clear"),

  backupCreate: (destDir?: string) => call<BackupInfo>("backup_create", { destDir }),
  backupHistory: () => call<HistoryRow[]>("backup_history"),
  backupStatus: () => call<BackupStatus>("backup_status"),
  backupDefaultDir: () => call<string>("backup_default_dir"),
  backupVerify: (path: string) => call<Preview>("backup_verify", { path }),
  restoreStage: (path: string) => call<StagedRestore>("restore_stage", { path }),
  restorePending: () => call<RestoreMarker | null>("restore_pending"),
  restoreCancel: () => call<void>("restore_cancel"),
  restartApp: () => call<void>("restart_app"),
};
