import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import {
  api,
  toApiError,
  type PurchaseDetail,
  type PurchaseIn,
  type PurchaseRow,
  type VariantPick,
} from "../lib/api";
import { useAuth } from "../lib/auth";
import { useLoad } from "../lib/useLoad";
import { formatPaisa, todayDhakaIso } from "../lib/format";
import { Badge, Card, EmptyState } from "../ui/Card";
import { Button, Field, Input, Modal, Select, useToast } from "../ui/controls";
import { MoneyInput } from "../ui/MoneyInput";
import { VariantPicker } from "../ui/VariantPicker";
import { PayModal } from "./Suppliers";

const STATUS_TONE: Record<string, "ok" | "bad" | "neutral"> = {
  confirmed: "ok",
  cancelled: "bad",
  draft: "neutral",
};

export function PurchasesPage() {
  const { can } = useAuth();
  const [status, setStatus] = useState("");
  const [search, setSearch] = useState("");
  const [modal, setModal] = useState<null | "new" | { edit: string } | { view: string }>(null);
  const list = useLoad(
    () =>
      api.purchasesList({ status: status || undefined, search: search || undefined, limit: 100 }),
    [status, search],
  );
  const canManage = can("purchases.manage");
  const done = () => {
    setModal(null);
    list.reload();
  };
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <Input
          className="!w-64"
          placeholder="Search PO no., invoice, supplier…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label="Search purchases"
        />
        <Select
          className="!w-44"
          value={status}
          onChange={(e) => setStatus(e.target.value)}
          aria-label="Status"
        >
          <option value="">All statuses</option>
          <option value="draft">Draft</option>
          <option value="confirmed">Confirmed</option>
          <option value="cancelled">Cancelled</option>
        </Select>
        {canManage && (
          <Button className="ml-auto" variant="primary" onClick={() => setModal("new")}>
            <Plus size={14} />
            New purchase
          </Button>
        )}
      </div>
      {list.error && (
        <p role="alert" className="text-[13px] text-danger">
          {list.error}
        </p>
      )}
      <Card>
        {list.data && list.data.rows.length === 0 ? (
          <EmptyState
            title="No purchases yet"
            hint="Create a purchase, confirm the supplier invoice, then receive the goods into stock."
          />
        ) : (
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th className="pb-2">PO</th>
                <th>Date</th>
                <th>Supplier</th>
                <th>Invoice</th>
                <th>Status</th>
                <th className="num">Received</th>
                <th className="num">Total</th>
                <th className="num">Due</th>
              </tr>
            </thead>
            <tbody>
              {list.data?.rows.map((r) => (
                <tr
                  key={r.id}
                  className="cursor-pointer border-t border-line hover:bg-mint/30"
                  onClick={() =>
                    setModal(r.status === "draft" && canManage ? { edit: r.id } : { view: r.id })
                  }
                >
                  <td className="py-2 font-medium text-navy">{r.number}</td>
                  <td>{r.order_date}</td>
                  <td>{r.supplier}</td>
                  <td>{r.supplier_invoice_no ?? "—"}</td>
                  <td>
                    <Badge tone={STATUS_TONE[r.status] ?? "neutral"}>{r.status}</Badge>
                  </td>
                  <td className="num">
                    {r.received_qty}/{r.ordered_qty}
                  </td>
                  <td className="num">{formatPaisa(r.grand_total)}</td>
                  <td className={`num ${r.due > 0 ? "font-semibold text-warn" : ""}`}>
                    {r.status === "confirmed" ? formatPaisa(r.due) : "—"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
      {modal === "new" && <DraftModal onClose={() => setModal(null)} onDone={done} />}
      {modal && typeof modal === "object" && "edit" in modal && (
        <DraftModal purchaseId={modal.edit} onClose={() => setModal(null)} onDone={done} />
      )}
      {modal && typeof modal === "object" && "view" in modal && (
        <DetailModal
          purchaseId={modal.view}
          onClose={() => setModal(null)}
          onChanged={list.reload}
        />
      )}
    </div>
  );
}

interface Line {
  pick: VariantPick;
  qty: string;
  cost: number | null;
}
interface Cost {
  label: string;
  amount: number | null;
}

function DraftModal({
  purchaseId,
  onClose,
  onDone,
}: {
  purchaseId?: string;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const suppliers = useLoad(() => api.suppliersList(undefined, false), []);
  const existing = useLoad<PurchaseDetail | null>(
    () => (purchaseId ? api.purchaseGet(purchaseId) : Promise.resolve(null)),
    [purchaseId],
  );
  const loaded = !purchaseId || existing.data !== null;
  if (!loaded || !suppliers.data)
    return (
      <Modal title="Purchase" onClose={onClose}>
        <p className="text-ink-2">Loading…</p>
      </Modal>
    );
  return (
    <DraftForm
      key={purchaseId ?? "new"}
      purchaseId={purchaseId}
      detail={existing.data}
      suppliers={suppliers.data}
      toast={toast}
      onClose={onClose}
      onDone={onDone}
    />
  );
}

function DraftForm({
  purchaseId,
  detail,
  suppliers,
  toast,
  onClose,
  onDone,
}: {
  purchaseId?: string;
  detail: PurchaseDetail | null;
  suppliers: { id: string; name: string }[];
  toast: ReturnType<typeof useToast>;
  onClose: () => void;
  onDone: () => void;
}) {
  const [supplier, setSupplier] = useState(detail?.purchase.supplier_id ?? "");
  const [invoice, setInvoice] = useState(detail?.purchase.supplier_invoice_no ?? "");
  const [date, setDate] = useState(detail?.purchase.order_date ?? todayDhakaIso());
  const [notes, setNotes] = useState(detail?.notes ?? "");
  const [discount, setDiscount] = useState<number | null>(detail?.discount ?? 0);
  const [lines, setLines] = useState<Line[]>(
    detail?.items.map((i) => ({
      pick: {
        variant_id: i.variant_id,
        product_id: "",
        label: i.label,
        sku: i.sku,
        barcode: null,
        is_serialized: i.is_serialized,
        purchase_cost: i.unit_cost,
        retail_price: 0,
        wholesale_price: 0,
        installment_price: null,
        available: 0,
      },
      qty: String(i.qty),
      cost: i.unit_cost,
    })) ?? [],
  );
  const [costs, setCosts] = useState<Cost[]>(
    detail?.costs.map((c) => ({ label: c.label, amount: c.amount })) ?? [],
  );
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const qtyOf = (l: Line) => (/^\d+$/.test(l.qty) ? Number(l.qty) : 0);
  const subtotal = lines.reduce((s, l) => s + qtyOf(l) * (l.cost ?? 0), 0);
  const landed = costs.reduce((s, c) => s + (c.amount ?? 0), 0);
  const payable = subtotal - (discount ?? 0);

  function build(): PurchaseIn | null {
    if (lines.some((l) => qtyOf(l) <= 0 || l.cost === null)) {
      setErr("Every line needs a whole quantity and a unit cost");
      return null;
    }
    if (costs.some((c) => !c.label.trim() || !c.amount)) {
      setErr("Every landed cost needs a label and an amount");
      return null;
    }
    return {
      supplier_id: supplier,
      supplier_invoice_no: invoice.trim() || null,
      order_date: date,
      discount: discount ?? 0,
      notes,
      items: lines.map((l) => ({
        variant_id: l.pick.variant_id,
        qty: qtyOf(l),
        unit_cost: l.cost ?? 0,
      })),
      costs: costs.map((c) => ({ label: c.label, amount: c.amount ?? 0 })),
    };
  }
  async function save(confirm: boolean) {
    setErr(null);
    const body = build();
    if (!body) return;
    setBusy(true);
    try {
      let id = purchaseId;
      if (id) await api.purchaseUpdateDraft(id, body);
      else id = await api.purchaseCreateDraft(body);
      if (confirm) await api.purchaseConfirm(id);
      toast("ok", confirm ? "Purchase confirmed — supplier payable recorded" : "Draft saved");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal
      title={purchaseId ? `Edit draft ${detail?.purchase.number}` : "New purchase"}
      onClose={onClose}
      width={860}
    >
      <div className="grid grid-cols-4 gap-3">
        <div className="col-span-2">
          <Field label="Supplier">
            {(id) => (
              <Select id={id} value={supplier} onChange={(e) => setSupplier(e.target.value)}>
                <option value="">Select…</option>
                {suppliers.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.name}
                  </option>
                ))}
              </Select>
            )}
          </Field>
        </div>
        <Field label="Supplier invoice no.">
          {(id) => <Input id={id} value={invoice} onChange={(e) => setInvoice(e.target.value)} />}
        </Field>
        <Field label="Order date">
          {(id) => (
            <Input id={id} type="date" value={date} onChange={(e) => setDate(e.target.value)} />
          )}
        </Field>
      </div>
      <div className="mt-4">
        <p className="mb-1 text-[12px] font-semibold text-navy">Items</p>
        <VariantPicker
          onPick={(p) =>
            setLines((ls) =>
              ls.some((l) => l.pick.variant_id === p.variant_id)
                ? ls
                : [...ls, { pick: p, qty: "1", cost: p.purchase_cost }],
            )
          }
        />
        <table className="mt-2 w-full text-[12px]">
          <thead>
            <tr className="text-left text-ink-2">
              <th>Product</th>
              <th className="w-24 num">Qty</th>
              <th className="w-40 num">Unit cost (৳)</th>
              <th className="w-32 num">Line total</th>
              <th className="w-8" />
            </tr>
          </thead>
          <tbody>
            {lines.map((l, i) => (
              <tr key={l.pick.variant_id} className="border-t border-line">
                <td className="py-1.5">
                  {l.pick.label} <span className="text-ink-2">{l.pick.sku}</span>
                  {l.pick.is_serialized && (
                    <span className="ml-1">
                      <Badge tone="ok">serial</Badge>
                    </span>
                  )}
                </td>
                <td>
                  <Input
                    className="text-right"
                    value={l.qty}
                    inputMode="numeric"
                    onChange={(e) =>
                      setLines(lines.map((x, j) => (j === i ? { ...x, qty: e.target.value } : x)))
                    }
                    aria-label="Quantity"
                  />
                </td>
                <td>
                  <MoneyInput
                    value={l.cost}
                    onChange={(c) =>
                      setLines(lines.map((x, j) => (j === i ? { ...x, cost: c } : x)))
                    }
                  />
                </td>
                <td className="num">{formatPaisa(qtyOf(l) * (l.cost ?? 0))}</td>
                <td>
                  <button
                    aria-label="Remove line"
                    onClick={() => setLines(lines.filter((_, j) => j !== i))}
                    className="text-ink-2 hover:text-danger"
                  >
                    <Trash2 size={14} />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {lines.length === 0 && (
          <p className="mt-2 text-[12px] text-ink-2">Search above to add products.</p>
        )}
      </div>
      <div className="mt-4 grid grid-cols-2 gap-6">
        <div>
          <p className="mb-1 flex items-center justify-between text-[12px] font-semibold text-navy">
            Landed costs (freight, unloading…)
            <Button
              variant="ghost"
              onClick={() => setCosts([...costs, { label: "", amount: null }])}
            >
              <Plus size={13} />
              Add
            </Button>
          </p>
          {costs.map((c, i) => (
            <div key={i} className="mb-1 flex gap-2">
              <Input
                value={c.label}
                placeholder="Label"
                onChange={(e) =>
                  setCosts(costs.map((x, j) => (j === i ? { ...x, label: e.target.value } : x)))
                }
                aria-label="Cost label"
              />
              <div className="w-36">
                <MoneyInput
                  value={c.amount}
                  onChange={(a) =>
                    setCosts(costs.map((x, j) => (j === i ? { ...x, amount: a } : x)))
                  }
                />
              </div>
              <button
                aria-label="Remove cost"
                onClick={() => setCosts(costs.filter((_, j) => j !== i))}
                className="text-ink-2 hover:text-danger"
              >
                <Trash2 size={14} />
              </button>
            </div>
          ))}
          <p className="mt-1 text-[11px] text-ink-2">
            Landed costs are spread across the items by value and added to stock cost. They are not
            added to what you owe the supplier.
          </p>
          <div className="mt-3">
            <Field label="Notes">
              {(id) => <Input id={id} value={notes} onChange={(e) => setNotes(e.target.value)} />}
            </Field>
          </div>
        </div>
        <dl className="space-y-1 self-start rounded-lg bg-pearl p-3 text-[13px]">
          <div className="flex justify-between">
            <dt>Subtotal</dt>
            <dd className="tabular-nums">{formatPaisa(subtotal)}</dd>
          </div>
          <div className="flex items-center justify-between gap-3">
            <dt>Supplier discount</dt>
            <dd className="w-36">
              <MoneyInput value={discount} onChange={setDiscount} />
            </dd>
          </div>
          <div className="flex justify-between border-t border-line pt-1 font-semibold text-navy">
            <dt>Payable to supplier</dt>
            <dd className="tabular-nums">{formatPaisa(payable)}</dd>
          </div>
          <div className="flex justify-between text-ink-2">
            <dt>Landed costs</dt>
            <dd className="tabular-nums">{formatPaisa(landed)}</dd>
          </div>
          <div className="flex justify-between text-ink-2">
            <dt>Total stock cost</dt>
            <dd className="tabular-nums">{formatPaisa(payable + landed)}</dd>
          </div>
        </dl>
      </div>
      {err && (
        <p role="alert" className="mt-3 text-[12px] text-danger">
          {err}
        </p>
      )}
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Close</Button>
        <Button busy={busy} onClick={() => void save(false)}>
          Save draft
        </Button>
        <Button variant="primary" busy={busy} onClick={() => void save(true)}>
          Confirm invoice
        </Button>
      </div>
    </Modal>
  );
}

function DetailModal({
  purchaseId,
  onClose,
  onChanged,
}: {
  purchaseId: string;
  onClose: () => void;
  onChanged: () => void;
}) {
  const { can } = useAuth();
  const toast = useToast();
  const d = useLoad(() => api.purchaseGet(purchaseId), [purchaseId]);
  const [sub, setSub] = useState<
    null | "receive" | "return" | "pay" | "cancel" | { reverse: string }
  >(null);
  const after = () => {
    setSub(null);
    d.reload();
    onChanged();
  };
  if (!d.data)
    return (
      <Modal title="Purchase" onClose={onClose}>
        {d.error ? (
          <p role="alert" className="text-danger">
            {d.error}
          </p>
        ) : (
          <p className="text-ink-2">Loading…</p>
        )}
      </Modal>
    );
  const p = d.data.purchase;
  const confirmed = p.status === "confirmed";
  const open = d.data.items.some((i) => i.received_qty < i.qty);
  const returnable = d.data.items.some((i) => i.received_qty - i.returned_qty > 0);
  return (
    <Modal title={`${p.number} · ${p.supplier}`} onClose={onClose} width={880}>
      <div className="mb-3 flex items-center gap-3 text-[12px]">
        <Badge tone={STATUS_TONE[p.status] ?? "neutral"}>{p.status}</Badge>
        <span>Invoice: {p.supplier_invoice_no ?? "—"}</span>
        <span>Date: {p.order_date}</span>
        {d.data.cancel_reason && (
          <span className="text-danger">Cancelled: {d.data.cancel_reason}</span>
        )}
      </div>
      <table className="w-full text-[12px]">
        <thead>
          <tr className="text-left text-ink-2">
            <th>Product</th>
            <th className="num">Ordered</th>
            <th className="num">Received</th>
            <th className="num">Returned</th>
            <th className="num">Unit cost</th>
            <th className="num">Landed/disc.</th>
            <th className="num">Stock cost</th>
          </tr>
        </thead>
        <tbody>
          {d.data.items.map((i) => (
            <tr key={i.id} className="border-t border-line">
              <td className="py-1.5">
                {i.label} <span className="text-ink-2">{i.sku}</span>
              </td>
              <td className="num">{i.qty}</td>
              <td className="num">{i.received_qty}</td>
              <td className="num">{i.returned_qty}</td>
              <td className="num">{formatPaisa(i.unit_cost)}</td>
              <td className="num">{formatPaisa(i.landed_alloc)}</td>
              <td className="num">{formatPaisa(i.line_total)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="mt-3 grid grid-cols-2 gap-6 text-[12px]">
        <div>
          <p className="mb-1 font-semibold text-navy">Payments</p>
          {d.data.payments.length === 0 ? (
            <p className="text-ink-2">None</p>
          ) : (
            d.data.payments.map((x) => (
              <p key={x.id} className="flex justify-between border-t border-line py-1">
                <span>
                  {x.paid_on} · {x.method}
                  {x.reference && ` · ${x.reference}`}
                  {x.reverses_id && " (reversal)"}
                </span>
                <span className="flex items-center gap-2 tabular-nums">
                  {formatPaisa(x.amount)}
                  {can("purchases.reverse_payment") &&
                    x.amount > 0 &&
                    !d.data!.payments.some((r) => r.reverses_id === x.id) && (
                      <button
                        className="text-danger underline"
                        onClick={() => setSub({ reverse: x.id })}
                      >
                        reverse
                      </button>
                    )}
                </span>
              </p>
            ))
          )}
          {d.data.returns.length > 0 && (
            <>
              <p className="mb-1 mt-2 font-semibold text-navy">Returns to supplier</p>
              {d.data.returns.map((r) => (
                <p key={r.id} className="flex justify-between border-t border-line py-1">
                  <span>
                    {r.number} · {r.returned_on} · {r.reason}
                  </span>
                  <span className="tabular-nums">{formatPaisa(r.credit_total)}</span>
                </p>
              ))}
            </>
          )}
        </div>
        <dl className="space-y-1 self-start rounded-lg bg-pearl p-3 text-[13px]">
          <div className="flex justify-between">
            <dt>Payable</dt>
            <dd className="tabular-nums">{formatPaisa(p.grand_total)}</dd>
          </div>
          <div className="flex justify-between">
            <dt>Paid</dt>
            <dd className="tabular-nums">{formatPaisa(p.paid)}</dd>
          </div>
          <div className="flex justify-between">
            <dt>Return credits</dt>
            <dd className="tabular-nums">{formatPaisa(p.credits)}</dd>
          </div>
          <div className="flex justify-between border-t border-line pt-1 font-semibold text-navy">
            <dt>Due</dt>
            <dd className="tabular-nums">{confirmed ? formatPaisa(p.due) : "—"}</dd>
          </div>
          <div className="flex justify-between text-ink-2">
            <dt>Landed costs</dt>
            <dd className="tabular-nums">{formatPaisa(p.landed_total)}</dd>
          </div>
        </dl>
      </div>
      <div className="mt-4 flex flex-wrap justify-end gap-2">
        {confirmed && open && can("purchases.manage") && (
          <Button variant="primary" onClick={() => setSub("receive")}>
            Receive goods
          </Button>
        )}
        {confirmed && p.due > 0 && can("purchases.pay") && (
          <Button onClick={() => setSub("pay")}>Record payment</Button>
        )}
        {confirmed && returnable && can("purchases.return") && (
          <Button onClick={() => setSub("return")}>Return to supplier</Button>
        )}
        {p.status !== "cancelled" && can("purchases.manage") && (
          <Button variant="danger" onClick={() => setSub("cancel")}>
            Cancel purchase
          </Button>
        )}
        <Button onClick={onClose}>Close</Button>
      </div>
      {sub === "receive" && (
        <ReceiveModal detail={d.data} onClose={() => setSub(null)} onDone={after} />
      )}
      {sub === "return" && (
        <ReturnModal detail={d.data} onClose={() => setSub(null)} onDone={after} />
      )}
      {sub === "pay" && (
        <PayModal
          supplier={{ id: p.supplier_id, name: p.supplier }}
          purchaseId={p.id}
          maxAmount={p.due}
          onClose={() => setSub(null)}
          onDone={after}
        />
      )}
      {sub === "cancel" && (
        <ReasonModal
          title={`Cancel ${p.number}`}
          action="Cancel purchase"
          onClose={() => setSub(null)}
          run={async (r) => {
            await api.purchaseCancel(p.id, r);
            toast("ok", "Purchase cancelled");
          }}
          onDone={after}
        />
      )}
      {sub && typeof sub === "object" && "reverse" in sub && (
        <ReasonModal
          title="Reverse payment"
          action="Reverse payment"
          onClose={() => setSub(null)}
          run={async (r) => {
            await api.supplierPaymentReverse(sub.reverse, r);
            toast("ok", "Payment reversed");
          }}
          onDone={after}
        />
      )}
    </Modal>
  );
}

function ReasonModal({
  title,
  action,
  run,
  onClose,
  onDone,
}: {
  title: string;
  action: string;
  run: (reason: string) => Promise<void>;
  onClose: () => void;
  onDone: () => void;
}) {
  const [reason, setReason] = useState("");
  const [err, setErr] = useState<string | null>(null);
  return (
    <Modal title={title} onClose={onClose} width={420}>
      <Field label="Reason (recorded in the audit log)" error={err}>
        {(id) => (
          <Input id={id} value={reason} onChange={(e) => setReason(e.target.value)} autoFocus />
        )}
      </Field>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Back</Button>
        <Button
          variant="danger"
          onClick={() =>
            void run(reason)
              .then(onDone)
              .catch((e) => setErr(toApiError(e).message))
          }
        >
          {action}
        </Button>
      </div>
    </Modal>
  );
}

function ReceiveModal({
  detail,
  onClose,
  onDone,
}: {
  detail: PurchaseDetail;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const open = detail.items.filter((i) => i.received_qty < i.qty);
  const [qty, setQty] = useState<Record<string, string>>(
    Object.fromEntries(open.map((i) => [i.id, String(i.qty - i.received_qty)])),
  );
  const [serials, setSerials] = useState<Record<string, string>>({});
  const [date, setDate] = useState(todayDhakaIso());
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const split = (s: string) =>
    s
      .split(/[\n,;]+/)
      .map((x) => x.trim())
      .filter(Boolean);
  async function go() {
    setBusy(true);
    setErr(null);
    try {
      await api.purchaseReceive(detail.purchase.id, {
        received_on: date,
        note: "",
        lines: open.map((i) => ({
          item_id: i.id,
          qty: /^\d+$/.test(qty[i.id] ?? "") ? Number(qty[i.id]) : 0,
          serials: i.is_serialized ? split(serials[i.id] ?? "") : [],
        })),
      });
      toast("ok", "Goods received into stock");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title="Receive goods" onClose={onClose} width={640}>
      <Field label="Received on">
        {(id) => (
          <Input id={id} type="date" value={date} onChange={(e) => setDate(e.target.value)} />
        )}
      </Field>
      {open.map((i) => (
        <div key={i.id} className="mt-3 rounded-lg border border-line p-3">
          <div className="flex items-center justify-between text-[13px]">
            <b className="text-navy">{i.label}</b>
            <span className="text-ink-2">outstanding {i.qty - i.received_qty}</span>
          </div>
          <div className="mt-2 grid grid-cols-[100px_1fr] gap-3">
            <Field label="Receive now">
              {(id) => (
                <Input
                  id={id}
                  className="text-right"
                  value={qty[i.id] ?? ""}
                  inputMode="numeric"
                  onChange={(e) => setQty({ ...qty, [i.id]: e.target.value })}
                />
              )}
            </Field>
            {i.is_serialized && (
              <Field
                label="Serial numbers (one per line)"
                hint="Exactly one serial number per unit received"
              >
                {(id) => (
                  <textarea
                    id={id}
                    rows={3}
                    value={serials[i.id] ?? ""}
                    onChange={(e) => setSerials({ ...serials, [i.id]: e.target.value })}
                    className="w-full rounded-lg border border-line px-3 py-2 text-[13px] outline-none focus:border-teal"
                  />
                )}
              </Field>
            )}
          </div>
        </div>
      ))}
      {err && (
        <p role="alert" className="mt-3 text-[12px] text-danger">
          {err}
        </p>
      )}
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" busy={busy} onClick={() => void go()}>
          Receive into stock
        </Button>
      </div>
    </Modal>
  );
}

function ReturnModal({
  detail,
  onClose,
  onDone,
}: {
  detail: PurchaseDetail;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const items = detail.items.filter((i) => i.received_qty - i.returned_qty > 0);
  const [qty, setQty] = useState<Record<string, string>>({});
  const [serials, setSerials] = useState<Record<string, string>>({});
  const [reason, setReason] = useState("");
  const [date, setDate] = useState(todayDhakaIso());
  const [err, setErr] = useState<string | null>(null);
  const split = (s: string) =>
    s
      .split(/[\n,;]+/)
      .map((x) => x.trim())
      .filter(Boolean);
  async function go() {
    const lines = items
      .filter((i) => Number(qty[i.id] ?? 0) > 0)
      .map((i) => ({
        item_id: i.id,
        qty: Number(qty[i.id]),
        serials: i.is_serialized ? split(serials[i.id] ?? "") : [],
      }));
    try {
      await api.purchaseReturn(detail.purchase.id, { returned_on: date, reason, lines });
      toast("ok", "Return recorded; supplier credited");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal title="Return to supplier" onClose={onClose} width={640}>
      <div className="grid grid-cols-2 gap-3">
        <Field label="Date">
          {(id) => (
            <Input id={id} type="date" value={date} onChange={(e) => setDate(e.target.value)} />
          )}
        </Field>
        <Field label="Reason">
          {(id) => <Input id={id} value={reason} onChange={(e) => setReason(e.target.value)} />}
        </Field>
      </div>
      {items.map((i) => (
        <div key={i.id} className="mt-3 rounded-lg border border-line p-3">
          <div className="flex justify-between text-[13px]">
            <b className="text-navy">{i.label}</b>
            <span className="text-ink-2">returnable {i.received_qty - i.returned_qty}</span>
          </div>
          <div className="mt-2 grid grid-cols-[100px_1fr] gap-3">
            <Field label="Return qty">
              {(id) => (
                <Input
                  id={id}
                  className="text-right"
                  value={qty[i.id] ?? ""}
                  inputMode="numeric"
                  onChange={(e) => setQty({ ...qty, [i.id]: e.target.value })}
                />
              )}
            </Field>
            {i.is_serialized && (
              <Field label="Serial numbers (one per line)">
                {(id) => (
                  <textarea
                    id={id}
                    rows={2}
                    value={serials[i.id] ?? ""}
                    onChange={(e) => setSerials({ ...serials, [i.id]: e.target.value })}
                    className="w-full rounded-lg border border-line px-3 py-2 text-[13px] outline-none focus:border-teal"
                  />
                )}
              </Field>
            )}
          </div>
        </div>
      ))}
      <p className="mt-2 text-[11px] text-ink-2">
        Supplier credit = units × original unit cost. Landed costs are not refunded.
      </p>
      {err && (
        <p role="alert" className="mt-2 text-[12px] text-danger">
          {err}
        </p>
      )}
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="danger" onClick={() => void go()}>
          Record return
        </Button>
      </div>
    </Modal>
  );
}

export type { PurchaseRow };
