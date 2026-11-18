import { useEffect, useMemo, useState } from "react";
import { Printer, Trash2 } from "lucide-react";
import {
  api,
  toApiError,
  type CustomerRow,
  type SaleDetail,
  type SaleIn,
  type SalePreview,
  type SerialChoice,
  type VariantPick,
} from "../lib/api";
import { useAuth } from "../lib/auth";
import { formatPaisa, newKey } from "../lib/format";
import { Card } from "../ui/Card";
import { CustomerPicker } from "../ui/CustomerPicker";
import { Badge } from "../ui/Card";
import { Button, Field, Input, Modal, Select, useToast } from "../ui/controls";
import { MoneyInput } from "../ui/MoneyInput";
import { PrintDoc, useShopIdentity, type PrintKind } from "../ui/PrintDoc";
import { VariantPicker } from "../ui/VariantPicker";

interface CartLine {
  pick: VariantPick;
  qty: number;
  discount: number;
  serials: SerialChoice[];
}
interface PayRow {
  method: string;
  amount: number | null;
  reference: string;
}

/** Applies entered amounts in order, never beyond `total`. */
export function applyInOrder(entered: number[], total: number): number[] {
  const out: number[] = [];
  let left = total;
  for (const e of entered) {
    const a = Math.max(0, Math.min(e, left));
    out.push(a);
    left -= a;
  }
  return out;
}

export function PosPage() {
  const { can } = useAuth();
  const toast = useToast();
  const identity = useShopIdentity();
  const canWholesale = can("sales.wholesale");
  const canDiscount = can("sales.discount");
  const canOverride = can("sales.credit_override");
  const [mode, setMode] = useState<"retail" | "wholesale">("retail");
  const [customer, setCustomer] = useState<CustomerRow | null>(null);
  const [cart, setCart] = useState<CartLine[]>([]);
  const [pays, setPays] = useState<PayRow[]>([{ method: "cash", amount: null, reference: "" }]);
  const [override, setOverride] = useState(false);
  const [notes, setNotes] = useState("");
  const [preview, setPreview] = useState<SalePreview | null>(null);
  const [previewErr, setPreviewErr] = useState<string | null>(null);
  const [key, setKey] = useState(newKey);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [serialFor, setSerialFor] = useState<VariantPick | null>(null);
  const [done, setDone] = useState<{ sale: SaleDetail; change: number } | null>(null);
  const [printing, setPrinting] = useState<PrintKind | null>(null);

  const items = useMemo(
    () =>
      cart.map((l) => ({
        variant_id: l.pick.variant_id,
        qty: l.pick.is_serialized ? l.serials.length : l.qty,
        discount: l.discount,
        serial_ids: l.serials.map((s) => s.id),
      })),
    [cart],
  );
  const itemsKey = JSON.stringify(items) + mode;

  useEffect(() => {
    if (items.length === 0) return;
    let live = true;
    const t = setTimeout(() => {
      const body: SaleIn = {
        mode,
        customer_id: null,
        sale_date: null,
        items,
        payments: [],
        notes: "",
        client_key: "preview",
        credit_override: false,
      };
      api
        .salePreview(body)
        .then((p) => {
          if (live) {
            setPreview(p);
            setPreviewErr(null);
          }
        })
        .catch((e) => {
          if (live) setPreviewErr(toApiError(e).message);
        });
    }, 120);
    return () => {
      live = false;
      clearTimeout(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [itemsKey]);

  const shownPreview = cart.length === 0 ? null : preview;
  const grand = shownPreview?.grand_total ?? 0;
  // apply entered amounts in order, never beyond the total; whatever is left over is change
  const applied = applyInOrder(
    pays.map((p) => p.amount ?? 0),
    grand,
  );
  const paid = applied.reduce((a, b) => a + b, 0);
  const entered = pays.reduce((a, p) => a + (p.amount ?? 0), 0);
  const change = Math.max(0, entered - grand);
  const due = grand - paid;

  function addPick(p: VariantPick) {
    if (p.is_serialized) return setSerialFor(p);
    setCart((c) =>
      c.some((l) => l.pick.variant_id === p.variant_id)
        ? c.map((l) => (l.pick.variant_id === p.variant_id ? { ...l, qty: l.qty + 1 } : l))
        : [...c, { pick: p, qty: 1, discount: 0, serials: [] }],
    );
  }
  function setPaysFull() {
    setPays([{ method: "cash", amount: grand, reference: "" }]);
  }
  function reset() {
    setCart([]);
    setCustomer(null);
    setPays([{ method: "cash", amount: null, reference: "" }]);
    setOverride(false);
    setNotes("");
    setPreview(null);
    setErr(null);
    setKey(newKey());
  }

  async function complete() {
    setErr(null);
    if (cart.length === 0) return setErr("Add at least one item");
    if (mode === "wholesale" && !customer) return setErr("Wholesale sales need a customer");
    setBusy(true);
    try {
      const body: SaleIn = {
        mode,
        customer_id: customer?.id ?? null,
        sale_date: null,
        items,
        notes,
        client_key: key,
        credit_override: override,
        payments: pays
          .map((p, i) => ({ method: p.method, amount: applied[i] ?? 0, reference: p.reference }))
          .filter((p) => p.amount > 0),
      };
      const r = await api.saleCreate(body);
      const sale = await api.saleGet(r.id);
      toast("ok", `Sale ${r.number} completed`);
      setDone({ sale, change });
      reset();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      if (e.key === "F9") {
        e.preventDefault();
        document.getElementById("pos-complete")?.click();
      }
      if (e.key === "F2") {
        e.preventDefault();
        document.querySelector<HTMLInputElement>("input[aria-label='Find product']")?.focus();
      }
    };
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  }, []);

  return (
    <div className="grid grid-cols-[1fr_380px] gap-4">
      <div className="space-y-3">
        <Card>
          <div className="mb-3 flex items-center gap-3">
            {canWholesale && (
              <div
                role="tablist"
                className="flex rounded-lg border border-line p-0.5 text-[12px] font-semibold"
              >
                {(["retail", "wholesale"] as const).map((m) => (
                  <button
                    key={m}
                    role="tab"
                    aria-selected={mode === m}
                    onClick={() => setMode(m)}
                    className={`rounded-md px-3 py-1 ${mode === m ? "bg-navy text-white" : "text-ink-2"}`}
                  >
                    {m === "retail" ? "Retail" : "Wholesale"}
                  </button>
                ))}
              </div>
            )}
            <div className="flex-1">
              <VariantPicker
                autoFocus
                onPick={addPick}
                placeholder="Scan barcode or search product…  (F2)"
              />
            </div>
          </div>
          {cart.length === 0 ? (
            <p className="py-10 text-center text-[13px] text-ink-2">
              Scan or search to add items. Prices follow the {mode} price list.
            </p>
          ) : (
            <table className="w-full text-[12px]">
              <thead>
                <tr className="text-left text-ink-2">
                  <th className="pb-2">Item</th>
                  <th className="w-24 num">Qty</th>
                  <th className="num">Price</th>
                  {canDiscount && <th className="w-32 num">Discount</th>}
                  <th className="num">Total</th>
                  <th className="w-8" />
                </tr>
              </thead>
              <tbody>
                {cart.map((l, i) => {
                  const pl = shownPreview?.lines.find((x) => x.variant_id === l.pick.variant_id);
                  return (
                    <tr key={l.pick.variant_id} className="border-t border-line align-top">
                      <td className="py-2">
                        <b className="text-navy">{l.pick.label}</b>
                        <div className="text-ink-2">
                          {l.pick.sku}
                          {l.pick.is_serialized && (
                            <>
                              {" "}
                              · S/N {l.serials.map((s) => s.serial_no).join(", ")}{" "}
                              <button className="underline" onClick={() => setSerialFor(l.pick)}>
                                change
                              </button>
                            </>
                          )}
                        </div>
                      </td>
                      <td className="num">
                        {l.pick.is_serialized ? (
                          l.serials.length
                        ) : (
                          <Input
                            className="text-right"
                            inputMode="numeric"
                            aria-label={`Quantity ${l.pick.sku}`}
                            value={String(l.qty)}
                            onChange={(e) => {
                              const n = Number(e.target.value.replace(/\D/g, ""));
                              setCart(cart.map((x, j) => (j === i ? { ...x, qty: n } : x)));
                            }}
                          />
                        )}
                      </td>
                      <td className="num">{pl ? formatPaisa(pl.unit_price) : "…"}</td>
                      {canDiscount && (
                        <td>
                          <MoneyInput
                            value={l.discount}
                            onChange={(d) =>
                              setCart(
                                cart.map((x, j) => (j === i ? { ...x, discount: d ?? 0 } : x)),
                              )
                            }
                          />
                        </td>
                      )}
                      <td className="num font-semibold">{pl ? formatPaisa(pl.line_total) : "…"}</td>
                      <td>
                        <button
                          aria-label={`Remove ${l.pick.sku}`}
                          onClick={() => setCart(cart.filter((_, j) => j !== i))}
                          className="text-ink-2 hover:text-danger"
                        >
                          <Trash2 size={14} />
                        </button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
          {previewErr && cart.length > 0 && (
            <p role="alert" className="mt-2 text-[12px] text-danger">
              {previewErr}
            </p>
          )}
        </Card>
      </div>

      <div className="space-y-3">
        <Card title="Customer">
          <CustomerPicker value={customer} onChange={setCustomer} required={mode === "wholesale"} />
        </Card>
        <Card title="Payment">
          <dl className="space-y-1 text-[13px]">
            <div className="flex justify-between">
              <dt>Subtotal</dt>
              <dd className="tabular-nums">{formatPaisa(shownPreview?.subtotal ?? 0)}</dd>
            </div>
            {(shownPreview?.discount_total ?? 0) > 0 && (
              <div className="flex justify-between">
                <dt>Discount</dt>
                <dd className="tabular-nums">-{formatPaisa(shownPreview!.discount_total)}</dd>
              </div>
            )}
            {(shownPreview?.tax_total ?? 0) > 0 && (
              <div className="flex justify-between">
                <dt>Tax</dt>
                <dd className="tabular-nums">{formatPaisa(shownPreview!.tax_total)}</dd>
              </div>
            )}
            <div className="metric-display flex justify-between border-t border-line pt-2 text-[20px] font-bold text-navy">
              <dt>Total</dt>
              <dd className="tabular-nums">{formatPaisa(grand)}</dd>
            </div>
          </dl>
          <div className="mt-3 space-y-2">
            {pays.map((p, i) => (
              <div key={i} className="grid grid-cols-[110px_1fr_auto] items-center gap-2">
                <Select
                  aria-label="Payment method"
                  value={p.method}
                  onChange={(e) =>
                    setPays(pays.map((x, j) => (j === i ? { ...x, method: e.target.value } : x)))
                  }
                >
                  <option value="cash">Cash</option>
                  <option value="bank">Bank</option>
                  <option value="mobile_wallet">Wallet</option>
                  <option value="cheque">Cheque</option>
                </Select>
                <MoneyInput
                  value={p.amount}
                  onChange={(a) => setPays(pays.map((x, j) => (j === i ? { ...x, amount: a } : x)))}
                  placeholder="Amount"
                />
                {pays.length > 1 ? (
                  <button
                    aria-label="Remove payment"
                    onClick={() => setPays(pays.filter((_, j) => j !== i))}
                    className="text-ink-2 hover:text-danger"
                  >
                    <Trash2 size={14} />
                  </button>
                ) : (
                  <span />
                )}
              </div>
            ))}
            <div className="flex gap-2">
              <Button
                onClick={() => setPays([...pays, { method: "bank", amount: null, reference: "" }])}
              >
                Split payment
              </Button>
              <Button onClick={setPaysFull} disabled={grand === 0}>
                Pay full (cash)
              </Button>
            </div>
          </div>
          <dl className="mt-3 space-y-1 text-[13px]">
            <div className="flex justify-between">
              <dt>Paid</dt>
              <dd className="tabular-nums">{formatPaisa(paid)}</dd>
            </div>
            {change > 0 && (
              <div className="flex justify-between font-semibold text-teal-deep">
                <dt>Change to return</dt>
                <dd className="tabular-nums">{formatPaisa(change)}</dd>
              </div>
            )}
            <div className={`flex justify-between font-semibold ${due > 0 ? "text-warn" : ""}`}>
              <dt>Balance due</dt>
              <dd className="tabular-nums">{formatPaisa(due)}</dd>
            </div>
          </dl>
          {mode === "wholesale" && due > 0 && canOverride && (
            <label className="mt-2 flex items-center gap-2 text-[12px]">
              <input
                type="checkbox"
                className="accent-teal"
                checked={override}
                onChange={(e) => setOverride(e.target.checked)}
              />
              Override credit limit (audited)
            </label>
          )}
          <div className="mt-2">
            <Field label="Notes">
              {(id) => <Input id={id} value={notes} onChange={(e) => setNotes(e.target.value)} />}
            </Field>
          </div>
          {err && (
            <p role="alert" className="mt-2 text-[12px] text-danger">
              {err}
            </p>
          )}
          <Button
            id="pos-complete"
            variant="primary"
            busy={busy}
            disabled={cart.length === 0}
            className="mt-3 w-full !py-3 text-[15px]"
            onClick={() => void complete()}
          >
            Complete sale (F9)
          </Button>
        </Card>
      </div>

      {serialFor && (
        <SerialModal
          pick={serialFor}
          initial={cart.find((l) => l.pick.variant_id === serialFor.variant_id)?.serials ?? []}
          onClose={() => setSerialFor(null)}
          onPick={(serials) => {
            setCart((c) => [
              ...c.filter((l) => l.pick.variant_id !== serialFor.variant_id),
              {
                pick: serialFor,
                qty: serials.length,
                discount: c.find((l) => l.pick.variant_id === serialFor.variant_id)?.discount ?? 0,
                serials,
              },
            ]);
            setSerialFor(null);
          }}
        />
      )}
      {done && (
        <Modal
          title={`Sale ${done.sale.sale.number} completed`}
          onClose={() => setDone(null)}
          width={420}
        >
          <p className="text-[13px]">
            Total <b>{formatPaisa(done.sale.sale.grand_total)}</b> · paid{" "}
            {formatPaisa(done.sale.sale.paid)}
            {done.sale.sale.due > 0 && (
              <>
                {" "}
                · <span className="text-warn">due {formatPaisa(done.sale.sale.due)}</span>
              </>
            )}
          </p>
          {done.change > 0 && (
            <p className="mt-2 rounded-lg bg-mint p-3 text-[16px] font-bold text-teal-deep">
              Change: {formatPaisa(done.change)}
            </p>
          )}
          <div className="mt-4 flex justify-end gap-2">
            <Button onClick={() => setPrinting("receipt")}>
              <Printer size={14} />
              Receipt (80mm)
            </Button>
            <Button onClick={() => setPrinting("a4")}>
              <Printer size={14} />
              Invoice (A4)
            </Button>
            <Button variant="primary" onClick={() => setDone(null)}>
              New sale
            </Button>
          </div>
        </Modal>
      )}
      {printing && done && (
        <PrintDoc
          sale={done.sale}
          kind={printing}
          branding={identity.branding}
          logo={identity.logo}
          onDone={() => setPrinting(null)}
        />
      )}
    </div>
  );
}

function SerialModal({
  pick,
  initial,
  onClose,
  onPick,
}: {
  pick: VariantPick;
  initial: SerialChoice[];
  onClose: () => void;
  onPick: (s: SerialChoice[]) => void;
}) {
  const [rows, setRows] = useState<SerialChoice[] | null>(null);
  const [sel, setSel] = useState<Set<string>>(new Set(initial.map((s) => s.id)));
  const [q, setQ] = useState("");
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    api
      .saleSerials(pick.variant_id)
      .then(setRows)
      .catch((e) => setErr(toApiError(e).message));
  }, [pick.variant_id]);
  const shown = (rows ?? []).filter((r) => r.serial_no.toLowerCase().includes(q.toLowerCase()));
  return (
    <Modal title={`Select serial numbers: ${pick.label}`} onClose={onClose} width={440}>
      <Input
        autoFocus
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder="Scan or type a serial number"
        aria-label="Find serial"
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            const m = (rows ?? []).find(
              (r) => r.serial_no.toLowerCase() === q.trim().toLowerCase(),
            );
            if (m) {
              setSel(new Set([...sel, m.id]));
              setQ("");
            }
          }
        }}
      />
      {err && (
        <p role="alert" className="mt-2 text-[12px] text-danger">
          {err}
        </p>
      )}
      {rows && rows.length === 0 && (
        <p className="mt-3 text-[13px] text-danger">
          No units of this product are available to sell.
        </p>
      )}
      <ul className="mt-3 max-h-64 space-y-1 overflow-y-auto">
        {shown.map((r) => (
          <li key={r.id}>
            <label className="flex cursor-pointer items-center gap-2 rounded-lg px-2 py-1.5 text-[13px] hover:bg-mint/50">
              <input
                type="checkbox"
                className="accent-teal"
                checked={sel.has(r.id)}
                onChange={(e) =>
                  setSel((s) => {
                    const n = new Set(s);
                    if (e.target.checked) n.add(r.id);
                    else n.delete(r.id);
                    return n;
                  })
                }
              />
              {r.serial_no}
            </label>
          </li>
        ))}
      </ul>
      <div className="mt-4 flex items-center justify-between">
        <Badge tone="neutral">{sel.size} selected</Badge>
        <div className="flex gap-2">
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabled={sel.size === 0}
            onClick={() => onPick((rows ?? []).filter((r) => sel.has(r.id)))}
          >
            Add to sale
          </Button>
        </div>
      </div>
    </Modal>
  );
}
