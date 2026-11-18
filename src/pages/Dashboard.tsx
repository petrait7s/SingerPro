import { useEffect, useState } from "react";
import { Database, ShieldCheck, ScrollText, AlertTriangle } from "lucide-react";
import {
  api,
  toApiError,
  type AuditRow,
  type BackupStatus,
  type DashboardData,
  type DbStatus,
} from "../lib/api";
import { useAuth } from "../lib/auth";
import { useI18n } from "../lib/i18n";
import { formatDhaka, formatPaisa } from "../lib/format";
import { Badge, Card, EmptyState } from "../ui/Card";

export function Dashboard() {
  const { t } = useI18n();
  const { can } = useAuth();
  const [db, setDb] = useState<DbStatus | null>(null);
  const [audit, setAudit] = useState<AuditRow[] | null>(null);
  const [bk, setBk] = useState<BackupStatus | null>(null);
  const [kpi, setKpi] = useState<DashboardData | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const canAudit = can("audit.view");

  useEffect(() => {
    api
      .dbStatus()
      .then(setDb)
      .catch((e) => setErr(toApiError(e).message));
    api
      .dashboardGet()
      .then(setKpi)
      .catch(() => undefined);
    api
      .backupStatus()
      .then(setBk)
      .catch(() => undefined);
    if (canAudit)
      api
        .auditList(8, 0)
        .then((p) => setAudit(p.rows))
        .catch((e) => setErr(toApiError(e).message));
  }, [canAudit]);

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
      {bk?.overdue && (
        <div
          role="status"
          className="flex items-center gap-2 rounded-lg border border-amber-200 bg-amber-50 px-4 py-3 text-[13px] text-warn"
        >
          <AlertTriangle size={16} />
          {t("backup.overdue", "A backup is due")} —{" "}
          {bk.last_backup_at
            ? `${t("backup.last", "Last backup")}: ${formatDhaka(bk.last_backup_at)}`
            : t("backup.never", "never backed up")}
        </div>
      )}
      {kpi && <KpiTiles kpi={kpi} />}
      <div className="grid grid-cols-3 gap-4">
        <Card title={t("dash.db", "Database")}>
          <div className="flex items-center gap-3">
            <Database className="text-teal" size={22} />
            <div>
              <p className="metric-display text-[20px] font-bold text-navy">
                {db ? `Schema v${db.schema_version}` : "…"}
              </p>
              <p className="text-[12px] text-ink-2">
                {db
                  ? `${db.journal_mode.toUpperCase()} · foreign keys ${db.foreign_keys ? "on" : "OFF"}`
                  : t("common.loading", "Loading")}
              </p>
            </div>
          </div>
        </Card>
        <Card title={t("dash.integrity", "Integrity")}>
          <div className="flex items-center gap-3">
            <ShieldCheck className="text-teal" size={22} />
            <div>
              {db ? (
                <Badge tone={db.integrity_ok ? "ok" : "bad"}>
                  {db.integrity_ok ? t("dash.healthy", "Healthy") : "Problems found"}
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
            </div>
          </div>
        </Card>
        <Card title={t("dash.kpi", "Business KPIs")}>
          <div className="flex items-center gap-3">
            <ScrollText className="text-slate-400" size={22} />
            <p className="text-[12px] text-ink-2">
              {kpi?.sales || kpi?.money
                ? "Figures below are read from your sales and accounting records."
                : "Figures appear here for roles with sales or report access."}
            </p>
          </div>
        </Card>
      </div>
      {canAudit && (
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
      )}
    </div>
  );
}

function Tile({
  label,
  value,
  hint,
  bad,
}: {
  label: string;
  value: string;
  hint?: string;
  bad?: boolean;
}) {
  return (
    <div className="rounded-(--radius-card) border border-line bg-surface p-4 shadow-card">
      <p className="text-[11px] font-semibold tracking-wide text-ink-2 uppercase">{label}</p>
      <p
        className={`metric-display mt-1 text-[22px] font-bold ${bad ? "text-danger" : "text-navy"}`}
      >
        {value}
      </p>
      {hint && <p className="mt-0.5 text-[11px] text-ink-2">{hint}</p>}
    </div>
  );
}

function KpiTiles({ kpi }: { kpi: DashboardData }) {
  const max = Math.max(1, ...kpi.last_days.map((d) => d[1]));
  return (
    <div className="space-y-4" data-testid="kpis">
      <div className="grid grid-cols-4 gap-4">
        {kpi.sales && (
          <>
            <Tile
              label="Sales today (net)"
              value={formatPaisa(kpi.sales.today_net)}
              hint={`${kpi.sales.today_invoices} invoices`}
            />
            <Tile
              label="Sales this month (net)"
              value={formatPaisa(kpi.sales.month_net)}
              hint={`${kpi.sales.month_invoices} invoices`}
            />
          </>
        )}
        {kpi.money && (
          <>
            <Tile label="Cash and bank" value={formatPaisa(kpi.money.cash_and_bank)} />
            <Tile
              label="Profit this month"
              value={formatPaisa(kpi.money.month_profit)}
              bad={kpi.money.month_profit < 0}
            />
            <Tile label="Customers owe us" value={formatPaisa(kpi.money.receivables)} />
            <Tile label="We owe suppliers" value={formatPaisa(kpi.money.payables)} />
          </>
        )}
        {kpi.stock && (
          <Tile
            label="Stock value"
            value={formatPaisa(kpi.stock.value)}
            hint={`${kpi.stock.low_items} items at or below minimum`}
            bad={kpi.stock.low_items > 0}
          />
        )}
        {kpi.installments && (
          <Tile
            label="Overdue installments"
            value={formatPaisa(kpi.installments.overdue_amount)}
            hint={`${kpi.installments.overdue_count} agreements · ${formatPaisa(kpi.installments.due_this_week)} due this week`}
            bad={kpi.installments.overdue_count > 0}
          />
        )}
      </div>
      {kpi.sales && kpi.last_days.length > 0 && (
        <Card title="Net sales, last 14 days">
          <div className="flex h-24 items-end gap-1" role="img" aria-label="Net sales per day">
            {kpi.last_days.map(([d, v]) => (
              <div
                key={d}
                title={`${d}: ${formatPaisa(v)}`}
                className="flex-1 rounded-t bg-teal/70"
                style={{ height: `${Math.max(2, (Math.max(0, v) / max) * 100)}%` }}
              />
            ))}
          </div>
        </Card>
      )}
      {(kpi.pending.returns_awaiting_inspection ||
        kpi.pending.expenses_awaiting_approval ||
        kpi.pending.open_warranty_cases) && (
        <p className="text-[12px] text-ink-2">
          Needs attention:
          {kpi.pending.returns_awaiting_inspection
            ? ` ${kpi.pending.returns_awaiting_inspection} return(s) awaiting inspection`
            : ""}
          {kpi.pending.expenses_awaiting_approval
            ? ` · ${kpi.pending.expenses_awaiting_approval} expense(s) awaiting approval`
            : ""}
          {kpi.pending.open_warranty_cases
            ? ` · ${kpi.pending.open_warranty_cases} open warranty case(s)`
            : ""}
        </p>
      )}
    </div>
  );
}
