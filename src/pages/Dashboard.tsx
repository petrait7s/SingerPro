import { useEffect, useState } from "react";
import { Database, ShieldCheck, ScrollText } from "lucide-react";
import { api, toApiError, type AuditRow, type DbStatus } from "../lib/api";
import { formatDhaka } from "../lib/format";
import { Badge, Card, EmptyState } from "../ui/Card";

export function Dashboard() {
  const [db, setDb] = useState<DbStatus | null>(null);
  const [audit, setAudit] = useState<AuditRow[] | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([api.dbStatus(), api.recentAudit(8)])
      .then(([d, a]) => {
        setDb(d);
        setAudit(a);
      })
      .catch((e) => setErr(toApiError(e).message));
  }, []);

  return (
    <div className="space-y-4">
      {err && (
        <div
          role="alert"
          className="rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-[13px] text-danger"
        >
          {err}
        </div>
      )}
      <div className="grid grid-cols-3 gap-4">
        <Card title="Database">
          <div className="flex items-center gap-3">
            <Database className="text-teal" size={22} />
            <div>
              <p className="metric-display text-[20px] font-bold text-navy">
                {db ? `Schema v${db.schema_version}` : "…"}
              </p>
              <p className="text-[12px] text-ink-2">
                {db
                  ? `${db.journal_mode.toUpperCase()} · foreign keys ${db.foreign_keys ? "on" : "OFF"}`
                  : "Loading"}
              </p>
            </div>
          </div>
        </Card>
        <Card title="Integrity">
          <div className="flex items-center gap-3">
            <ShieldCheck className="text-teal" size={22} />
            <div>
              {db ? (
                <Badge tone={db.integrity_ok ? "ok" : "bad"}>
                  {db.integrity_ok ? "Healthy" : "Problems found"}
                </Badge>
              ) : (
                "…"
              )}
              {db && !db.integrity_ok && (
                <ul className="mt-2 text-[12px] text-danger">
                  {db.integrity_detail.map((d) => (
                    <li key={d}>{d}</li>
                  ))}
                </ul>
              )}
              <p className="mt-1 text-[12px] text-ink-2">Checked at startup of this view</p>
            </div>
          </div>
        </Card>
        <Card title="Business KPIs">
          <div className="flex items-center gap-3">
            <ScrollText className="text-slate-400" size={22} />
            <p className="text-[12px] text-ink-2">
              No transactions recorded yet. KPIs appear here from real records once sales and
              purchases exist.
            </p>
          </div>
        </Card>
      </div>
      <Card title="Recent audit events">
        {audit && audit.length === 0 && (
          <EmptyState
            title="No audit events yet"
            hint="Privileged and financial actions are recorded here automatically."
          />
        )}
        {audit && audit.length > 0 && (
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th className="pb-2">When (Dhaka)</th>
                <th>Action</th>
                <th>Entity</th>
              </tr>
            </thead>
            <tbody>
              {audit.map((r) => (
                <tr key={r.id} className="border-t border-line">
                  <td className="py-1.5">{formatDhaka(r.at)}</td>
                  <td>{r.action}</td>
                  <td>{r.entity_type}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
    </div>
  );
}
