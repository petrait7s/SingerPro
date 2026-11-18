import { useCallback, useEffect, useState } from "react";
import { Plus, ShieldCheck } from "lucide-react";
import { api, toApiError, type PermissionRow, type RoleRow, type UserRow } from "../lib/api";
import { useAuth } from "../lib/auth";
import { useI18n } from "../lib/i18n";
import { formatDhaka } from "../lib/format";
import { Badge, Card } from "../ui/Card";
import { Button, Field, Input, Modal, Toggle, useToast } from "../ui/controls";

type Tab = "users" | "roles";

export function UsersPage() {
  const { t } = useI18n();
  const { can } = useAuth();
  const [tab, setTab] = useState<Tab>("users");
  return (
    <div className="space-y-4">
      <div className="flex gap-1 border-b border-line">
        {(["users", "roles"] as Tab[]).map((k) => (
          <button
            key={k}
            onClick={() => setTab(k)}
            aria-current={tab === k}
            className={`-mb-px border-b-2 px-4 py-2 text-[13px] font-semibold ${tab === k ? "border-teal text-navy" : "border-transparent text-ink-2 hover:text-navy"}`}
          >
            {k === "users" ? t("users.title", "Users") : t("users.roles", "Roles & permissions")}
          </button>
        ))}
      </div>
      {tab === "users" ? (
        <UsersTab canManage={can("users.manage")} />
      ) : (
        <RolesTab canManage={can("roles.manage")} />
      )}
    </div>
  );
}

function UsersTab({ canManage }: { canManage: boolean }) {
  const { t } = useI18n();
  const toast = useToast();
  const [users, setUsers] = useState<UserRow[] | null>(null);
  const [roles, setRoles] = useState<RoleRow[]>([]);
  const [modal, setModal] = useState<
    null | "new" | { edit: UserRow } | { reset: UserRow } | { deactivate: UserRow }
  >(null);
  const load = useCallback(() => {
    Promise.all([api.usersList(), api.rolesList()])
      .then(([u, r]) => {
        setUsers(u);
        setRoles(r);
      })
      .catch((e) => toast("bad", toApiError(e).message));
  }, [toast]);
  useEffect(load, [load]);

  async function reactivate(u: UserRow) {
    try {
      await api.userSetActive(u.id, true, "");
      toast("ok", `${u.username}: ${t("common.active", "active")}`);
      load();
    } catch (e) {
      toast("bad", toApiError(e).message);
    }
  }

  return (
    <Card
      title={t("users.title", "Users")}
      action={
        canManage ? (
          <Button variant="primary" onClick={() => setModal("new")}>
            <Plus size={14} />
            {t("users.new", "New user")}
          </Button>
        ) : undefined
      }
    >
      <table className="w-full text-[12px]">
        <thead>
          <tr className="text-left text-ink-2">
            <th className="pb-2">{t("users.fullName", "Full name")}</th>
            <th>{t("users.username", "Username")}</th>
            <th>{t("common.roles", "Roles")}</th>
            <th>{t("users.lastLogin", "Last login")}</th>
            <th>Status</th>
            {canManage && <th className="text-right">{t("common.actions", "Actions")}</th>}
          </tr>
        </thead>
        <tbody>
          {users?.map((u) => (
            <tr key={u.id} className="border-t border-line">
              <td className="py-2 font-medium text-navy">{u.full_name}</td>
              <td>{u.username}</td>
              <td>{u.roles.join(", ")}</td>
              <td>{u.last_login_at ? formatDhaka(u.last_login_at) : "—"}</td>
              <td>
                <Badge tone={u.is_active ? "ok" : "neutral"}>
                  {u.is_active ? t("common.active", "Active") : t("common.inactive", "Inactive")}
                </Badge>
                {u.must_change_password && (
                  <span className="ml-1">
                    <Badge tone="neutral">must change pw</Badge>
                  </span>
                )}
              </td>
              {canManage && (
                <td className="space-x-1 text-right">
                  <Button variant="ghost" onClick={() => setModal({ edit: u })}>
                    Roles
                  </Button>
                  <Button variant="ghost" onClick={() => setModal({ reset: u })}>
                    Reset password
                  </Button>
                  {u.is_active ? (
                    <Button variant="ghost" onClick={() => setModal({ deactivate: u })}>
                      Deactivate
                    </Button>
                  ) : (
                    <Button variant="ghost" onClick={() => void reactivate(u)}>
                      Reactivate
                    </Button>
                  )}
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
      {modal === "new" && (
        <NewUserModal
          roles={roles}
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            load();
          }}
        />
      )}
      {modal && typeof modal === "object" && "edit" in modal && (
        <EditRolesModal
          user={modal.edit}
          roles={roles}
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            load();
          }}
        />
      )}
      {modal && typeof modal === "object" && "reset" in modal && (
        <ResetModal
          user={modal.reset}
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            load();
          }}
        />
      )}
      {modal && typeof modal === "object" && "deactivate" in modal && (
        <DeactivateModal
          user={modal.deactivate}
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            load();
          }}
        />
      )}
    </Card>
  );
}

function RolePicker({
  roles,
  value,
  onChange,
}: {
  roles: RoleRow[];
  value: string[];
  onChange: (v: string[]) => void;
}) {
  return (
    <div className="space-y-1.5">
      {roles.map((r) => (
        <Toggle
          key={r.id}
          label={`${r.name} (${r.permissions.length})`}
          checked={value.includes(r.id)}
          onChange={(c) => onChange(c ? [...value, r.id] : value.filter((x) => x !== r.id))}
        />
      ))}
    </div>
  );
}

function NewUserModal({
  roles,
  onClose,
  onDone,
}: {
  roles: RoleRow[];
  onClose: () => void;
  onDone: () => void;
}) {
  const { t } = useI18n();
  const toast = useToast();
  const [f, setF] = useState({ username: "", full_name: "", password: "" });
  const [sel, setSel] = useState<string[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function go() {
    setBusy(true);
    setErr(null);
    try {
      await api.userCreate({ ...f, role_ids: sel });
      toast("ok", `${f.username} created`);
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title={t("users.new", "New user")} onClose={onClose}>
      <div className="space-y-3">
        <Field label={t("users.fullName", "Full name")}>
          {(id) => (
            <Input
              id={id}
              value={f.full_name}
              onChange={(e) => setF({ ...f, full_name: e.target.value })}
              autoFocus
            />
          )}
        </Field>
        <Field label={t("users.username", "Username")}>
          {(id) => (
            <Input
              id={id}
              value={f.username}
              onChange={(e) => setF({ ...f, username: e.target.value })}
            />
          )}
        </Field>
        <Field
          label={t("users.tempPassword", "Temporary password")}
          hint="The user must replace it at first sign-in."
        >
          {(id) => (
            <Input
              id={id}
              type="password"
              value={f.password}
              onChange={(e) => setF({ ...f, password: e.target.value })}
              autoComplete="new-password"
            />
          )}
        </Field>
        <Field label={t("common.roles", "Roles")} error={err}>
          {() => <RolePicker roles={roles} value={sel} onChange={setSel} />}
        </Field>
        <div className="flex justify-end gap-2">
          <Button onClick={onClose}>{t("common.cancel", "Cancel")}</Button>
          <Button variant="primary" busy={busy} onClick={() => void go()}>
            {t("common.create", "Create")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

function EditRolesModal({
  user,
  roles,
  onClose,
  onDone,
}: {
  user: UserRow;
  roles: RoleRow[];
  onClose: () => void;
  onDone: () => void;
}) {
  const { t } = useI18n();
  const toast = useToast();
  const [sel, setSel] = useState<string[]>(
    roles.filter((r) => user.roles.includes(r.name)).map((r) => r.id),
  );
  const [err, setErr] = useState<string | null>(null);
  async function go() {
    try {
      await api.userSetRoles(user.id, sel);
      toast("ok", "Roles updated");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal title={`${t("common.roles", "Roles")}: ${user.username}`} onClose={onClose}>
      <Field label={t("common.roles", "Roles")} error={err}>
        {() => <RolePicker roles={roles} value={sel} onChange={setSel} />}
      </Field>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>{t("common.cancel", "Cancel")}</Button>
        <Button variant="primary" onClick={() => void go()}>
          {t("common.save", "Save")}
        </Button>
      </div>
    </Modal>
  );
}

function ResetModal({
  user,
  onClose,
  onDone,
}: {
  user: UserRow;
  onClose: () => void;
  onDone: () => void;
}) {
  const { t } = useI18n();
  const toast = useToast();
  const [pw, setPw] = useState("");
  const [err, setErr] = useState<string | null>(null);
  async function go() {
    try {
      await api.userResetPassword(user.id, pw);
      toast("ok", `Password reset for ${user.username}; they must change it at next sign-in`);
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal title={`Reset password: ${user.username}`} onClose={onClose}>
      <Field
        label={t("users.tempPassword", "Temporary password")}
        error={err}
        hint="All of this user's sessions are ended."
      >
        {(id) => (
          <Input
            id={id}
            type="password"
            value={pw}
            onChange={(e) => setPw(e.target.value)}
            autoComplete="new-password"
            autoFocus
          />
        )}
      </Field>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>{t("common.cancel", "Cancel")}</Button>
        <Button variant="danger" onClick={() => void go()}>
          Reset password
        </Button>
      </div>
    </Modal>
  );
}

function DeactivateModal({
  user,
  onClose,
  onDone,
}: {
  user: UserRow;
  onClose: () => void;
  onDone: () => void;
}) {
  const { t } = useI18n();
  const toast = useToast();
  const [reason, setReason] = useState("");
  const [err, setErr] = useState<string | null>(null);
  async function go() {
    try {
      await api.userSetActive(user.id, false, reason);
      toast("ok", `${user.username} deactivated`);
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal title={`Deactivate ${user.username}?`} onClose={onClose}>
      <p className="mb-3 text-[13px] text-ink-2">
        They are signed out immediately and cannot sign in. History is kept.
      </p>
      <Field label={t("common.reason", "Reason")} error={err}>
        {(id) => (
          <Input id={id} value={reason} onChange={(e) => setReason(e.target.value)} autoFocus />
        )}
      </Field>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>{t("common.cancel", "Cancel")}</Button>
        <Button variant="danger" onClick={() => void go()}>
          Deactivate
        </Button>
      </div>
    </Modal>
  );
}

function RolesTab({ canManage }: { canManage: boolean }) {
  const { t } = useI18n();
  const { principal } = useAuth();
  const toast = useToast();
  const [roles, setRoles] = useState<RoleRow[]>([]);
  const [perms, setPerms] = useState<PermissionRow[]>([]);
  const [sel, setSel] = useState<string | null>(null);
  const [edits, setEdits] = useState<Record<string, string[]>>({});
  const [creating, setCreating] = useState(false);
  const load = useCallback(() => {
    Promise.all([api.rolesList(), api.permissionsList()])
      .then(([r, p]) => {
        setRoles(r);
        setPerms(p);
      })
      .catch((e) => toast("bad", toApiError(e).message));
  }, [toast]);
  useEffect(load, [load]);
  const role = roles.find((r) => r.id === sel) ?? null;
  const draft = role ? (edits[role.id] ?? role.permissions) : [];
  const setDraft = (v: string[]) => role && setEdits((e) => ({ ...e, [role.id]: v }));
  const locked = !canManage || role?.name === "Owner";
  const categories = [...new Set(perms.map((p) => p.category))];
  const mine = new Set(principal?.permissions ?? []);
  const isOwner = principal?.roles.includes("Owner") ?? false;

  async function save() {
    if (!role) return;
    try {
      await api.roleSetPermissions(role.id, draft);
      toast("ok", `${role.name}: permissions saved`);
      load();
    } catch (e) {
      toast("bad", toApiError(e).message);
    }
  }
  async function remove() {
    if (!role) return;
    try {
      await api.roleDelete(role.id);
      toast("ok", `${role.name} deleted`);
      setSel(null);
      load();
    } catch (e) {
      toast("bad", toApiError(e).message);
    }
  }
  return (
    <div className="grid grid-cols-[260px_1fr] gap-4">
      <Card
        title={t("users.roles", "Roles")}
        action={
          canManage ? (
            <Button variant="ghost" onClick={() => setCreating(true)}>
              <Plus size={14} />
            </Button>
          ) : undefined
        }
      >
        <ul className="space-y-1">
          {roles.map((r) => (
            <li key={r.id}>
              <button
                onClick={() => setSel(r.id)}
                className={`flex w-full items-center justify-between rounded-lg px-3 py-2 text-left text-[13px] ${sel === r.id ? "bg-mint font-semibold text-navy" : "hover:bg-slate-50"}`}
              >
                <span className="flex items-center gap-2">
                  {r.is_system && <ShieldCheck size={13} className="text-teal-deep" />}
                  {r.name}
                </span>
                <span className="text-[11px] text-ink-2">{r.permissions.length}</span>
              </button>
            </li>
          ))}
        </ul>
      </Card>
      <Card
        title={role ? role.name : "Permissions"}
        action={
          role && !locked ? (
            <div className="flex gap-2">
              {!role.is_system && (
                <Button variant="danger" onClick={() => void remove()}>
                  Delete role
                </Button>
              )}
              <Button variant="primary" onClick={() => void save()}>
                {t("common.save", "Save")}
              </Button>
            </div>
          ) : undefined
        }
      >
        {!role && <p className="text-[13px] text-ink-2">Select a role to view its permissions.</p>}
        {role && (
          <div className="space-y-4">
            {role.name === "Owner" && (
              <p className="text-[12px] text-ink-2">
                The Owner role always holds every permission and cannot be edited.
              </p>
            )}
            {categories.map((c) => (
              <div key={c}>
                <p className="mb-1 text-[11px] font-semibold uppercase tracking-wider text-ink-2">
                  {c}
                </p>
                <div className="grid grid-cols-2 gap-x-6 gap-y-1">
                  {perms
                    .filter((p) => p.category === c)
                    .map((p) => (
                      <Toggle
                        key={p.key}
                        label={p.description}
                        checked={draft.includes(p.key)}
                        disabled={locked || (!isOwner && !mine.has(p.key))}
                        onChange={(v) =>
                          setDraft(v ? [...draft, p.key] : draft.filter((k) => k !== p.key))
                        }
                      />
                    ))}
                </div>
              </div>
            ))}
          </div>
        )}
      </Card>
      {creating && (
        <NewRoleModal
          perms={perms}
          mine={mine}
          isOwner={isOwner}
          onClose={() => setCreating(false)}
          onDone={() => {
            setCreating(false);
            load();
          }}
        />
      )}
    </div>
  );
}

function NewRoleModal({
  perms,
  mine,
  isOwner,
  onClose,
  onDone,
}: {
  perms: PermissionRow[];
  mine: Set<string>;
  isOwner: boolean;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const { t } = useI18n();
  const [name, setName] = useState("");
  const [desc, setDesc] = useState("");
  const [sel, setSel] = useState<string[]>([]);
  const [err, setErr] = useState<string | null>(null);
  async function go() {
    try {
      await api.roleCreate(name, desc, sel);
      toast("ok", `Role ${name} created`);
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal title="New role" onClose={onClose} width={560}>
      <div className="space-y-3">
        <Field label={t("common.name", "Name")}>
          {(id) => (
            <Input id={id} value={name} onChange={(e) => setName(e.target.value)} autoFocus />
          )}
        </Field>
        <Field label="Description">
          {(id) => <Input id={id} value={desc} onChange={(e) => setDesc(e.target.value)} />}
        </Field>
        <Field label="Permissions" error={err}>
          {() => (
            <div className="grid max-h-64 grid-cols-2 gap-x-4 gap-y-1 overflow-y-auto rounded-lg border border-line p-3">
              {perms.map((p) => (
                <Toggle
                  key={p.key}
                  label={p.description}
                  checked={sel.includes(p.key)}
                  disabled={!isOwner && !mine.has(p.key)}
                  onChange={(v) => setSel(v ? [...sel, p.key] : sel.filter((k) => k !== p.key))}
                />
              ))}
            </div>
          )}
        </Field>
        <div className="flex justify-end gap-2">
          <Button onClick={onClose}>{t("common.cancel", "Cancel")}</Button>
          <Button variant="primary" onClick={() => void go()}>
            {t("common.create", "Create")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
