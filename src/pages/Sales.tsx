import { useState } from "react";
import { Printer } from "lucide-react";
import { api, toApiError, type SaleDetail } from "../lib/api";
import { useAuth } from "../lib/auth";
import { useLoad } from "../lib/useLoad";
import { formatPaisa } from "../lib/format";
import { Badge, Card, EmptyState } from "../ui/Card";
import { Button, Field, Input, Modal, Select, Toggle, useToast } from "../ui/controls";
import { PrintDoc, useShopIdentity, type PrintKind } from "../ui/PrintDoc";

const MODE_TONE: Record<string, "ok" | "neutral"> = {
  retail: "neutral",
  wholesale: "ok",
  installment: "ok",
};

export function SalesPage({ fixedMode }: { fixedMode?: "retail" | "wholesale" }) {
  const [search, setSearch] = useState("");
  const [mode, setMode] = useState(fixedMode ?? "");
  const [dueOnly, setDueOnly] = useState(false);
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const list = useLoad(
    () =>
      api.salesList({
        search: search || undefined,
        mode: mode || undefined,
        due_only: dueOnly,
        from: from || undefined,
        to: to || undefined,
        limit: 100,
      }),
    [search, mode, dueOnly, from, to],
  );
  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-3">
        <Input
          className="!w-64"
          placeholder="Search invoice no., customer, phone…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label="Search sales"
        />
        {!fixedMode && (
          <Select
            className="!w-40"
            value={mode}
            onChange={(e) => setMode(e.target.value)}
            aria-label="Sale type"
          >
            <option value="">All types</option>
            <option value="retail">Retail</option>
            <option value="wholesale">Wholesale</option>
            <option value="installment">Installment</option>
          </Select>
        )}
        <Input
          className="!w-40"
          type="date"
          value={from}
          onChange={(e) => setFrom(e.target.value)}
          aria-label="From date"
        />
        <Input
          className="!w-40"
          type="date"
          value={to}
          onChange={(e) => setTo(e.target.value)}
          aria-label="To date"
        />
        <Toggle label="With balance due" checked={dueOnly} onChange={setDueOnly} />
      </div>
      {list.error && (
        <p role="alert" className="text-[13px] text-danger">
          {list.error}
        </p>
      )}
      <Card>
        {list.data && list.data.rows.length === 0 ? (
          <EmptyState
            title="No sales found"
            hint="Completed sales appear here with their payment status."
          />
        ) : (
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th className="pb-2">Invoice</th>
                <th>Date</th>
                <th>Type</th>
                <th>Customer</th>
                <th className="num">Total</th>
                <th className="num">Paid</th>
                <th className="num">Due</th>
              </tr>
            </thead>
            <tbody>
              {list.data?.rows.map((r) => (
                <tr
                  key={r.id}
                  className="cursor-pointer border-t border-line hover:bg-mint/30"
                  onClick={() => setOpen(r.id)}
                >
                  <td className="py-2 font-medium text-navy">{r.number}</td>
                  <td>{r.sale_date}</td>
                  <td>
                    <Badge tone={MODE_TONE[r.mode] ?? "neutral"}>{r.mode}</Badge>
                  </td>
                  <td>{r.customer ?? "Walk-in"}</td>
                  <td className="num">{formatPaisa(r.grand_total)}</td>
                  <td className="num">{formatPaisa(r.paid)}</td>
                  <td className={`num ${r.due > 0 ? "font-semibold text-warn" : ""}`}>
                    {r.due > 0 ? formatPaisa(r.due) : "—"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {list.data && <p className="mt-3 text-[11px] text-ink-2">{list.data.total} sale(s)</p>}
      </Card>
      {open && <SaleModal saleId={open} onClose={() => setOpen(null)} onChanged={list.reload} />}
    </div>
  );
}

export function SaleModal({
  saleId,
  onClose,
  onChanged,
}: {
  saleId: string;
  onClose: () => void;
  onChanged?: () => void;
}) {
  const { can } = useAuth();
  const toast = useToast();
  const identity = useShopIdentity();
  const d = useLoad<SaleDetail>(() => api.saleGet(saleId), [saleId]);
  const [printing, setPrinting] = useState<PrintKind | null>(null);
  const [reverse, setReverse] = useState<string | null>(null);
  if (!d.data)
    return (
      <Modal title="Sale" onClose={onClose}>
        {d.error ? (
          <p role="alert" className="text-danger">
            {d.error}
          </p>
        ) : (
          <p className="text-ink-2">Loading…</p>
        )}
      </Modal>
    );
  const s = d.data;
  return (
    <Modal
      title={`${s.sale.number} · ${s.sale.customer ?? "Walk-in"}`}
      onClose={onClose}
      width={820}
    >
      <p className="mb-3 text-[12px] text-ink-2">
        {s.sale.sale_date} · {s.sale.mode}
        {s.cashier && ` · served by ${s.cashier}`}
      </p>
      <table className="w-full text-[12px]">
        <thead>
          <tr className="text-left text-ink-2">
            <th>Item</th>
            <th>Serial</th>
            <th className="num">Qty</th>
            <th className="num">Price</th>
            <th className="num">Discount</th>
            <th className="num">Tax</th>
            <th className="num">Total</th>
          </tr>
        </thead>
        <tbody>
          {s.items.map((i, n) => (
            <tr key={n} className="border-t border-line">
              <td className="py-1.5">
                {i.label} <span className="text-ink-2">{i.sku}</span>
              </td>
              <td>{i.serial_no ?? ""}</td>
              <td className="num">{i.qty}</td>
              <td className="num">{formatPaisa(i.unit_price)}</td>
              <td className="num">{formatPaisa(i.discount)}</td>
              <td className="num">{formatPaisa(i.tax)}</td>
              <td className="num">{formatPaisa(i.line_total)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="mt-3 grid grid-cols-2 gap-6 text-[12px]">
        <div>
          <p className="mb-1 font-semibold text-navy">Receipts</p>
          {s.payments.length === 0 ? (
            <p className="text-ink-2">No payments recorded.</p>
          ) : (
            s.payments.map((p) => (
              <p key={p.id} className="flex justify-between border-t border-line py-1">
                <span>
                  {p.receipt_no} · {p.paid_on} · {p.method}
                  {p.reverses_id && " (reversal)"}
                </span>
                <span className="flex items-center gap-2 tabular-nums">
                  {formatPaisa(p.amount)}
                  {can("customers.reverse_payment") &&
                    p.amount > 0 &&
                    !s.payments.some((r) => r.reverses_id === p.id) && (
                      <button className="text-danger underline" onClick={() => setReverse(p.id)}>
                        reverse
                      </button>
                    )}
                </span>
              </p>
            ))
          )}
        </div>
        <dl className="space-y-1 self-start rounded-lg bg-pearl p-3 text-[13px]">
          <div className="flex justify-between">
            <dt>Subtotal</dt>
            <dd className="tabular-nums">{formatPaisa(s.subtotal)}</dd>
          </div>
          {s.discount_total > 0 && (
            <div className="flex justify-between">
              <dt>Discount</dt>
              <dd className="tabular-nums">-{formatPaisa(s.discount_total)}</dd>
            </div>
          )}
          {s.tax_total > 0 && (
            <div className="flex justify-between">
              <dt>Tax</dt>
              <dd className="tabular-nums">{formatPaisa(s.tax_total)}</dd>
            </div>
          )}
          <div className="flex justify-between border-t border-line pt-1 font-semibold text-navy">
            <dt>Total</dt>
            <dd className="tabular-nums">{formatPaisa(s.sale.grand_total)}</dd>
          </div>
          <div className="flex justify-between">
            <dt>Paid</dt>
            <dd className="tabular-nums">{formatPaisa(s.sale.paid)}</dd>
          </div>
          <div className="flex justify-between font-semibold text-warn">
            <dt>Due</dt>
            <dd className="tabular-nums">{formatPaisa(s.sale.due)}</dd>
          </div>
        </dl>
      </div>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={() => setPrinting("receipt")}>
          <Printer size={14} />
          Receipt (80mm)
        </Button>
        <Button onClick={() => setPrinting("a4")}>
          <Printer size={14} />
          Invoice (A4)
        </Button>
        <Button onClick={onClose}>Close</Button>
      </div>
      {printing && (
        <PrintDoc
          sale={s}
          kind={printing}
          branding={identity.branding}
          logo={identity.logo}
          onDone={() => setPrinting(null)}
        />
      )}
      {reverse && (
        <ReverseModal
          onClose={() => setReverse(null)}
          run={async (reason) => {
            await api.salePaymentReverse(reverse, reason);
            toast("ok", "Payment reversed");
            setReverse(null);
            d.reload();
            onChanged?.();
          }}
        />
      )}
    </Modal>
  );
}

function ReverseModal({
  onClose,
  run,
}: {
  onClose: () => void;
  run: (reason: string) => Promise<void>;
}) {
  const [reason, setReason] = useState("");
  const [err, setErr] = useState<string | null>(null);
  return (
    <Modal title="Reverse payment" onClose={onClose} width={420}>
      <p className="mb-2 text-[12px] text-ink-2">
        The receipt stays on record; an opposite entry is added and the invoice balance reopens.
      </p>
      <Field label="Reason (audited)" error={err}>
        {(id) => (
          <Input id={id} value={reason} onChange={(e) => setReason(e.target.value)} autoFocus />
        )}
      </Field>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Back</Button>
        <Button
          variant="danger"
          onClick={() => void run(reason).catch((e) => setErr(toApiError(e).message))}
        >
          Reverse payment
        </Button>
      </div>
    </Modal>
  );
}
