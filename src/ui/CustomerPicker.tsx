import { useEffect, useState } from "react";
import { api, toApiError, type CustomerRow } from "../lib/api";
import { formatPaisa } from "../lib/format";
import { Input } from "./controls";

export function CustomerPicker({
  value,
  onChange,
  required,
}: {
  value: CustomerRow | null;
  onChange: (c: CustomerRow | null) => void;
  required?: boolean;
}) {
  const [q, setQ] = useState("");
  const [rows, setRows] = useState<CustomerRow[]>([]);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    if (!q.trim()) return;
    let live = true;
    const t = setTimeout(() => {
      api
        .customersList({ search: q })
        .then((r) => {
          if (live) {
            setRows(r.slice(0, 8));
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
  if (value) {
    return (
      <div className="flex items-center justify-between rounded-lg border border-line bg-mint/40 px-3 py-2 text-[12px]">
        <span>
          <b className="text-navy">{value.name}</b>{" "}
          <span className="text-ink-2">
            {value.code} · {value.phone}
          </span>
          {value.balance > 0 && (
            <span className="ml-2 text-warn">owes {formatPaisa(value.balance)}</span>
          )}
        </span>
        <button type="button" onClick={() => onChange(null)} className="text-ink-2 underline">
          change
        </button>
      </div>
    );
  }
  const shown = q.trim() ? rows : [];
  return (
    <div className="relative">
      <Input
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder={
          required ? "Find customer (required)…" : "Customer (optional) — name or phone…"
        }
        aria-label="Find customer"
      />
      {err && (
        <p role="alert" className="mt-1 text-[12px] text-danger">
          {err}
        </p>
      )}
      {shown.length > 0 && (
        <ul className="absolute z-30 mt-1 w-full rounded-lg border border-line bg-surface shadow-lift">
          {shown.map((c) => (
            <li key={c.id}>
              <button
                type="button"
                onClick={() => {
                  onChange(c);
                  setQ("");
                  setRows([]);
                }}
                className="flex w-full justify-between px-3 py-2 text-left text-[12px] hover:bg-mint/60"
              >
                <span>
                  <b className="text-navy">{c.name}</b>{" "}
                  <span className="text-ink-2">{c.phone}</span>
                </span>
                <span className="text-ink-2">{c.code}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
