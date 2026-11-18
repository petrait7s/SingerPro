import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { open } from "@tauri-apps/plugin-dialog";
import { Download, Printer } from "lucide-react";
import { api, toApiError, type ReportData, type ReportDef, type ReportParams } from "../lib/api";
import { useLoad } from "../lib/useLoad";
import { formatPaisa, todayDhakaIso } from "../lib/format";
import { Card, EmptyState } from "../ui/Card";
import { Button, Field, Input, Select, useToast } from "../ui/controls";
import { useShopIdentity } from "../ui/PrintDoc";

function monthStart() {
  return todayDhakaIso().slice(0, 8) + "01";
}

function kindOf(data: ReportData, j: number): string {
  return data.columns[j]?.kind ?? "text";
}

function cell(kind: string, v: string | number | null): string {
  if (v === null || v === undefined) return "";
  if (kind === "money" && typeof v === "number") return formatPaisa(v);
  return String(v);
}

export function ReportsPage() {
  const toast = useToast();
  const cat = useLoad(() => api.reportsCatalogue(), []);
  const [id, setId] = useState("");
  const [from, setFrom] = useState(monthStart());
  const [to, setTo] = useState(todayDhakaIso());
  const [asOf, setAsOf] = useState(todayDhakaIso());
  const [data, setData] = useState<ReportData | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [printing, setPrinting] = useState(false);

  const defs = useMemo(() => cat.data ?? [], [cat.data]);
  const def: ReportDef | undefined = defs.find((d) => d.id === id) ?? defs[0];
  const groups = useMemo(() => [...new Set(defs.map((d) => d.group))], [defs]);

  function params(): ReportParams {
    if (def?.period === "range") return { from, to };
    if (def?.period === "as_of") return { as_of: asOf };
    return {};
  }
  async function run() {
    if (!def) return;
    setBusy(true);
    setErr(null);
    try {
      setData(await api.reportRun(def.id, params()));
    } catch (e) {
      setData(null);
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  async function exportAs(format: "csv" | "xlsx") {
    if (!def) return;
    try {
      const picked = await open({ directory: true, title: "Choose a folder for the export" });
      if (typeof picked !== "string") return;
      const path = await api.reportExport(def.id, params(), format, picked);
      toast("ok", `Saved ${path}`);
    } catch (e) {
      toast("bad", toApiError(e).message);
    }
  }

  return (
    <div className="space-y-4">
      <Card>
        <div className="flex flex-wrap items-end gap-3">
          <Field label="Report">
            {(fid) => (
              <Select
                id={fid}
                className="!w-72"
                value={def?.id ?? ""}
                onChange={(e) => {
                  setId(e.target.value);
                  setData(null);
                }}
              >
                {groups.map((g) => (
                  <optgroup key={g} label={g}>
                    {defs
                      .filter((d) => d.group === g)
                      .map((d) => (
                        <option key={d.id} value={d.id}>
                          {d.title}
                        </option>
                      ))}
                  </optgroup>
                ))}
              </Select>
            )}
          </Field>
          {def?.period === "range" && (
            <>
              <Field label="From">
                {(fid) => (
                  <Input
                    id={fid}
                    type="date"
                    value={from}
                    onChange={(e) => setFrom(e.target.value)}
                  />
                )}
              </Field>
              <Field label="To">
                {(fid) => (
                  <Input id={fid} type="date" value={to} onChange={(e) => setTo(e.target.value)} />
                )}
              </Field>
            </>
          )}
          {def?.period === "as_of" && (
            <Field label="As of">
              {(fid) => (
                <Input
                  id={fid}
                  type="date"
                  value={asOf}
                  onChange={(e) => setAsOf(e.target.value)}
                />
              )}
            </Field>
          )}
          <Button variant="primary" busy={busy} disabled={!def} onClick={() => void run()}>
            Show report
          </Button>
          {data && (
            <div className="ml-auto flex gap-2">
              <Button onClick={() => setPrinting(true)}>
                <Printer size={14} />
                Print / PDF
              </Button>
              <Button onClick={() => void exportAs("csv")}>
                <Download size={14} />
                CSV
              </Button>
              <Button onClick={() => void exportAs("xlsx")}>
                <Download size={14} />
                Excel
              </Button>
            </div>
          )}
        </div>
      </Card>
      {err && (
        <p role="alert" className="text-[13px] text-danger">
          {err}
        </p>
      )}
      {cat.data && cat.data.length === 0 && (
        <EmptyState title="No reports available" hint="Your role does not include report access." />
      )}
      {data && (
        <Card title={`${data.title} · ${data.period}`}>
          {data.rows.length === 0 ? (
            <p className="text-[13px] text-ink-2">Nothing to show for this period.</p>
          ) : (
            <ReportTable data={data} />
          )}
        </Card>
      )}
      {printing && data && <PrintReport data={data} onDone={() => setPrinting(false)} />}
    </div>
  );
}

function ReportTable({ data }: { data: ReportData }) {
  const num = (k: string) => (k === "money" || k === "int" ? "num" : "");
  return (
    <table className="w-full text-[12px]">
      <thead>
        <tr className="text-left text-ink-2">
          {data.columns.map((c) => (
            <th key={c.key} className={`pb-2 ${num(c.kind)}`}>
              {c.label}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {data.rows.map((r, i) => (
          <tr key={i} className="border-t border-line">
            {r.map((v, j) => (
              <td key={j} className={`py-1.5 ${num(kindOf(data, j))}`}>
                {cell(kindOf(data, j), v)}
              </td>
            ))}
          </tr>
        ))}
        {data.totals && (
          <tr className="border-t-2 border-navy font-bold">
            {data.totals.map((v, j) => (
              <td key={j} className={`py-1.5 ${num(kindOf(data, j))}`}>
                {cell(kindOf(data, j), v)}
              </td>
            ))}
          </tr>
        )}
      </tbody>
    </table>
  );
}

function PrintReport({ data, onDone }: { data: ReportData; onDone: () => void }) {
  const { branding, logo } = useShopIdentity();
  useEffect(() => {
    const style = document.createElement("style");
    style.textContent = "@page { size: A4; margin: 14mm; }";
    document.head.append(style);
    const t = setTimeout(() => window.print(), 200);
    const after = () => onDone();
    window.addEventListener("afterprint", after);
    return () => {
      clearTimeout(t);
      window.removeEventListener("afterprint", after);
      style.remove();
    };
  }, [onDone]);
  return createPortal(
    <div id="print-root" className="print-a4">
      <header className="mb-3">
        {logo && <img src={logo} alt="" className="mb-2 max-h-14" />}
        <h1 className="text-[16px] font-bold">{branding?.shop_name ?? "Singer Pro"}</h1>
        <h2 className="text-[14px] font-bold">{data.title}</h2>
        <p>{data.period}</p>
      </header>
      <table className="w-full border-collapse text-[11px]">
        <thead>
          <tr className="border-y border-black text-left">
            {data.columns.map((c) => (
              <th
                key={c.key}
                className={c.kind === "money" || c.kind === "int" ? "text-right" : ""}
              >
                {c.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {[...data.rows, ...(data.totals ? [data.totals] : [])].map((r, i) => (
            <tr
              key={i}
              className={
                data.totals && i === data.rows.length ? "border-t border-black font-bold" : ""
              }
            >
              {r.map((v, j) => (
                <td
                  key={j}
                  className={
                    kindOf(data, j) === "money" || kindOf(data, j) === "int" ? "text-right" : ""
                  }
                >
                  {cell(kindOf(data, j), v)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      <p className="mt-3 text-[10px]">
        Printed from Singer Pro. Amounts in {branding?.currency_symbol ?? "৳"}.
      </p>
    </div>,
    document.body,
  );
}
