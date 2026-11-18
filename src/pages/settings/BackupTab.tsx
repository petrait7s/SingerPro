import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { AlertTriangle, CheckCircle2, XCircle } from "lucide-react";
import {
  api,
  toApiError,
  type BackupStatus,
  type HistoryRow,
  type Preview,
  type RestoreMarker,
} from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { useI18n } from "../../lib/i18n";
import { formatBytes, formatDhaka } from "../../lib/format";
import { Badge, Card } from "../../ui/Card";
import { Button, Modal, useToast } from "../../ui/controls";

export function BackupTab() {
  const { t } = useI18n();
  const { can } = useAuth();
  const toast = useToast();
  const [status, setStatus] = useState<BackupStatus | null>(null);
  const [history, setHistory] = useState<HistoryRow[]>([]);
  const [pending, setPending] = useState<RestoreMarker | null>(null);
  const [defDir, setDefDir] = useState("");
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState<Preview | null>(null);

  const load = useCallback(() => {
    api
      .backupStatus()
      .then(setStatus)
      .catch(() => undefined);
    api
      .backupHistory()
      .then(setHistory)
      .catch(() => undefined);
    api
      .restorePending()
      .then(setPending)
      .catch(() => undefined);
    api
      .backupDefaultDir()
      .then(setDefDir)
      .catch(() => undefined);
  }, []);
  useEffect(load, [load]);

  async function backup(choose: boolean) {
    let dir: string | undefined;
    if (choose) {
      const picked = await open({
        directory: true,
        title: t("backup.choose", "Choose backup folder"),
      });
      if (typeof picked !== "string") return;
      dir = picked;
    }
    setBusy(true);
    try {
      const r = await api.backupCreate(dir);
      toast("ok", `Backup verified and saved (${formatBytes(r.size_bytes)}): ${r.path}`);
      load();
    } catch (e) {
      toast("bad", toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  async function pickRestore() {
    const picked = await open({
      multiple: false,
      title: t("backup.restore", "Restore from backup"),
      filters: [{ name: "Singer Pro backup", extensions: ["spbak"] }],
    });
    if (typeof picked !== "string") return;
    try {
      setPreview(await api.backupVerify(picked));
    } catch (e) {
      toast("bad", toApiError(e).message);
    }
  }
  async function cancelPending() {
    try {
      await api.restoreCancel();
      toast("ok", "Pending restore cancelled");
      load();
    } catch (e) {
      toast("bad", toApiError(e).message);
    }
  }

  return (
    <div className="space-y-4">
      <Card title="Backup">
        <div className="flex items-start justify-between gap-6">
          <div className="text-[13px]">
            <p>
              {t("backup.last", "Last backup")}:{" "}
              <b>
                {status?.last_backup_at
                  ? formatDhaka(status.last_backup_at)
                  : t("backup.never", "never")}
              </b>{" "}
              {status?.overdue && <Badge tone="bad">{t("backup.overdue", "due")}</Badge>}
            </p>
            <p className="mt-1 text-[12px] text-ink-2">
              Default folder: {defDir}. Backups are consistent snapshots, verified before they are
              saved, and never overwrite an existing file. Copy them to an external drive for
              safety.
            </p>
          </div>
          {can("backup.create") && (
            <div className="flex shrink-0 gap-2">
              <Button variant="primary" busy={busy} onClick={() => void backup(false)}>
                {t("backup.now", "Back up now")}
              </Button>
              <Button disabled={busy} onClick={() => void backup(true)}>
                {t("backup.choose", "Back up to folder…")}
              </Button>
            </div>
          )}
        </div>
      </Card>

      {can("backup.restore") && (
        <Card title={t("backup.restore", "Restore from backup")}>
          {pending ? (
            <div className="rounded-lg bg-amber-50 p-3 text-[13px] text-warn">
              <p className="flex items-center gap-2 font-semibold">
                <AlertTriangle size={15} />A restore is staged and will be applied when Singer Pro
                restarts.
              </p>
              <p className="mt-1 text-[12px]">
                Source: {pending.source}
                <br />A safety copy of your current data was saved to: {pending.safety_copy}
              </p>
              <div className="mt-3 flex gap-2">
                <Button variant="danger" onClick={() => void api.restartApp()}>
                  Restart now to apply
                </Button>
                <Button onClick={() => void cancelPending()}>Cancel restore</Button>
              </div>
            </div>
          ) : (
            <div className="flex items-center justify-between gap-6">
              <p className="text-[12px] text-ink-2">
                Choose a backup file to inspect. Nothing changes until you confirm, and your current
                data is always saved to a safety copy first and kept after the restore.
              </p>
              <Button onClick={() => void pickRestore()}>Choose backup file…</Button>
            </div>
          )}
        </Card>
      )}

      <Card title={t("backup.history", "History")}>
        {history.length === 0 ? (
          <p className="text-[13px] text-ink-2">No backups or restores recorded yet.</p>
        ) : (
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th className="pb-2">When (Dhaka)</th>
                <th>Event</th>
                <th>Status</th>
                <th className="num">Size</th>
                <th>File</th>
              </tr>
            </thead>
            <tbody>
              {history.map((h) => (
                <tr key={h.id} className="border-t border-line">
                  <td className="py-1.5">{formatDhaka(h.at)}</td>
                  <td>{h.kind.replace("_", " ")}</td>
                  <td>
                    <Badge tone={h.status === "ok" ? "ok" : "bad"}>{h.status}</Badge>
                  </td>
                  <td className="num">{h.size_bytes ? formatBytes(h.size_bytes) : "—"}</td>
                  <td className="max-w-[320px] truncate" title={h.note ?? h.path}>
                    {h.path}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>

      {preview && (
        <RestorePreview
          preview={preview}
          onClose={() => setPreview(null)}
          onStaged={() => {
            setPreview(null);
            load();
          }}
        />
      )}
    </div>
  );
}

function RestorePreview({
  preview,
  onClose,
  onStaged,
}: {
  preview: Preview;
  onClose: () => void;
  onStaged: () => void;
}) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const [ack, setAck] = useState(false);
  async function stage() {
    setBusy(true);
    try {
      await api.restoreStage(preview.path);
      toast("ok", "Restore staged. Restart to apply it.");
      onStaged();
    } catch (e) {
      toast("bad", toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title="Restore preview" onClose={onClose} width={560}>
      <dl className="space-y-1 text-[12px]">
        <div className="flex justify-between gap-4">
          <dt className="text-ink-2">File</dt>
          <dd className="truncate" title={preview.path}>
            {preview.path}
          </dd>
        </div>
        <div className="flex justify-between">
          <dt className="text-ink-2">Size</dt>
          <dd>{formatBytes(preview.size_bytes)}</dd>
        </div>
        <div className="flex justify-between">
          <dt className="text-ink-2">Created</dt>
          <dd>
            {preview.manifest ? formatDhaka(preview.manifest.created_at) : "unknown (no manifest)"}
          </dd>
        </div>
        <div className="flex justify-between">
          <dt className="text-ink-2">Schema</dt>
          <dd>
            v{preview.schema_version ?? "?"} (this app: v{preview.app_schema_version})
          </dd>
        </div>
        <div className="flex justify-between">
          <dt className="text-ink-2">Users / audit events</dt>
          <dd>
            {preview.users ?? "?"} / {preview.audit_rows ?? "?"}
          </dd>
        </div>
        <div className="flex justify-between">
          <dt className="text-ink-2">Checksum</dt>
          <dd>
            {preview.checksum_ok === true
              ? "matches manifest"
              : preview.checksum_ok === false
                ? "MISMATCH"
                : "not confirmed"}
          </dd>
        </div>
      </dl>
      {preview.problems.length > 0 && (
        <ul className="mt-3 space-y-1 rounded-lg bg-red-50 p-3 text-[12px] text-danger">
          {preview.problems.map((p) => (
            <li key={p} className="flex gap-2">
              <XCircle size={14} className="mt-0.5 shrink-0" />
              {p}
            </li>
          ))}
        </ul>
      )}
      {preview.warnings.length > 0 && (
        <ul className="mt-3 space-y-1 rounded-lg bg-amber-50 p-3 text-[12px] text-warn">
          {preview.warnings.map((p) => (
            <li key={p} className="flex gap-2">
              <AlertTriangle size={14} className="mt-0.5 shrink-0" />
              {p}
            </li>
          ))}
        </ul>
      )}
      {preview.can_restore && (
        <div className="mt-4 space-y-3">
          <p className="flex items-center gap-2 text-[12px] text-teal-deep">
            <CheckCircle2 size={14} />
            Integrity verified.
          </p>
          <label className="flex items-start gap-2 text-[12px]">
            <input
              type="checkbox"
              className="mt-0.5 accent-teal"
              checked={ack}
              onChange={(e) => setAck(e.target.checked)}
            />
            I understand everything entered after this backup will be replaced. My current data is
            saved to a safety copy and kept.
          </label>
        </div>
      )}
      <div className="mt-5 flex justify-end gap-2">
        <Button onClick={onClose}>Close</Button>
        <Button
          variant="danger"
          busy={busy}
          disabled={!preview.can_restore || !ack}
          onClick={() => void stage()}
        >
          Stage restore
        </Button>
      </div>
    </Modal>
  );
}
