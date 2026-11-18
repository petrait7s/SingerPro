import { useEffect, useState } from "react";
import { api, toApiError, type VariantPick } from "../lib/api";
import { Input } from "./controls";

/** Type-ahead product/variant search (SKU, barcode, name, model). Exact barcode/SKU scans first. */
export function VariantPicker({
  onPick,
  placeholder,
  autoFocus,
}: {
  onPick: (v: VariantPick) => void;
  placeholder?: string;
  autoFocus?: boolean;
}) {
  const [q, setQ] = useState("");
  const [rows, setRows] = useState<VariantPick[]>([]);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    if (!q.trim()) return;
    let live = true;
    const t = setTimeout(() => {
      api
        .variantsSearch(q)
        .then((r) => {
          if (live) {
            setRows(r);
            setErr(null);
          }
        })
        .catch((e) => {
          if (live) setErr(toApiError(e).message);
        });
    }, 150);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [q]);
  const shown = q.trim() ? rows : [];
  return (
    <div className="relative">
      <Input
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder={placeholder ?? "Scan barcode or search product…"}
        aria-label="Find product"
        autoFocus={autoFocus}
        onKeyDown={(e) => {
          if (e.key !== "Enter") return;
          const term = q.trim().toLowerCase();
          // a scanner types the barcode/SKU then Enter: add the exact match straight away
          const hit =
            rows.find((r) => r.barcode === q.trim() || r.sku.toLowerCase() === term) ??
            (rows.length === 1 ? rows[0] : undefined);
          if (hit) {
            e.preventDefault();
            onPick(hit);
            setQ("");
            setRows([]);
          }
        }}
      />
      {err && (
        <p role="alert" className="mt-1 text-[12px] text-danger">
          {err}
        </p>
      )}
      {shown.length > 0 && (
        <ul className="absolute z-30 mt-1 max-h-64 w-full overflow-y-auto rounded-lg border border-line bg-surface shadow-lift">
          {shown.map((r) => (
            <li key={r.variant_id}>
              <button
                type="button"
                onClick={() => {
                  onPick(r);
                  setQ("");
                  setRows([]);
                }}
                className="flex w-full items-center justify-between px-3 py-2 text-left text-[12px] hover:bg-mint/60"
              >
                <span>
                  <b className="text-navy">{r.label}</b>
                  <span className="ml-2 text-ink-2">{r.sku}</span>
                </span>
                <span className="text-ink-2">
                  {r.is_serialized ? "serial · " : ""}
                  {r.available} in stock
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
