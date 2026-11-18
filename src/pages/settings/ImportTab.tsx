import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, toApiError, type ImportReport } from "../../lib/api";
import { Card } from "../../ui/Card";
import { Button, Select, useToast } from "../../ui/controls";

type Kind = "products" | "customers";

export function ImportTab() {
  const toast = useToast();
  const [kind, setKind] = useState<Kind>("products");
  const [text, setText] = useState<string | null>(null);
  const [fileName, setFileName] = useState("");
  const [report, setReport] = useState<ImportReport | null>(null);
  const [busy, setBusy] = useState(false);

  async function saveTemplate() {
    try {
      const dir = await open({ directory: true, title: "Choose a folder for the template" });
      if (typeof dir !== "string") return;
      toast("ok", `Template saved: ${await api.importTemplateSave(kind, dir)}`);
    } catch (e) {
      toast("bad", toApiError(e).message);
    }
  }
  async function choose(file: File | undefined) {
    setReport(null);
    if (!file) return setText(null);
    setFileName(file.name);
    setText(await file.text());
  }
  async function run(commit: boolean) {
    if (text === null) return;
    setBusy(true);
    try {
      const r = await api.importRun(kind, text, commit);
      setReport(r);
      if (r.imported)
        toast("ok", `Imported ${r.created} ${kind === "products" ? "product(s)" : "customer(s)"}`);
    } catch (e) {
      toast("bad", toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  const clean = report !== null && report.errors.length === 0 && !report.imported;
  return (
    <div className="space-y-4">
      <Card title="Import from a spreadsheet (CSV)">
        <p className="mb-3 max-w-2xl text-[13px] text-ink-2">
          Download the template, fill it in with Excel, save it as CSV and choose it here. The file
          is checked completely first; if any row has a problem nothing is imported. Import only
          adds new records: existing SKUs and phone numbers are never overwritten.
        </p>
        <div className="flex flex-wrap items-center gap-3">
          <Select
            className="!w-48"
            aria-label="What to import"
            value={kind}
            onChange={(e) => {
              setKind(e.target.value as Kind);
              setReport(null);
              setText(null);
            }}
          >
            <option value="products">Products and variants</option>
            <option value="customers">Customers</option>
          </Select>
          <Button onClick={() => void saveTemplate()}>Save template…</Button>
          <label className="text-[13px] text-navy">
            <span className="mr-2 font-semibold">CSV file</span>
            <input
              type="file"
              accept=".csv,text/csv"
              aria-label="CSV file"
              onChange={(e) => void choose(e.target.files?.[0])}
            />
          </label>
        </div>
        {text !== null && (
          <div className="mt-4 flex items-center gap-3">
            <span className="text-[12px] text-ink-2">{fileName}</span>
            <Button busy={busy} onClick={() => void run(false)}>
              Check file
            </Button>
            {clean && (
              <Button variant="primary" busy={busy} onClick={() => void run(true)}>
                Import {report.rows} row(s)
              </Button>
            )}
          </div>
        )}
      </Card>
      {report && (
        <Card
          title={
            report.errors.length ? "Problems found" : report.imported ? "Done" : "File is valid"
          }
        >
          {report.errors.length === 0 ? (
            <p role="status" className="text-[13px] text-teal-deep">
              {report.imported
                ? `${report.created} record(s) added.`
                : `${report.rows} row(s) checked, no problems. Press Import to add them.`}
            </p>
          ) : (
            <table className="w-full text-[12px]">
              <thead>
                <tr className="text-left text-ink-2">
                  <th className="w-16 pb-2">Line</th>
                  <th>Problem</th>
                </tr>
              </thead>
              <tbody>
                {report.errors.map((e, i) => (
                  <tr key={i} className="border-t border-line">
                    <td className="py-1.5">{e.line}</td>
                    <td className="text-danger">{e.message}</td>
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
