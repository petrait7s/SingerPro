import { useState } from "react";
import { Plus } from "lucide-react";
import {
  api,
  toApiError,
  type Coverage,
  type ReturnableItem,
  type ReturnDetail,
  type ReturnRow,
  type WarrantyCaseDetail,
  type WarrantyCaseRow,
} from "../lib/api";
import { useAuth } from "../lib/auth";
import { useLoad } from "../lib/useLoad";
import { formatDhaka, formatPaisa, newKey } from "../lib/format";
import { Badge, Card, EmptyState } from "../ui/Card";
import { Button, Field, Input, Modal, Select, useToast } from "../ui/controls";
import { MoneyInput } from "../ui/MoneyInput";

const RETURN_TONE: Record<string, "ok" | "bad" | "neutral"> = {
  requested: "neutral",
  completed: "ok",
  rejected: "bad",
};
const CASE_TONE: Record<string, "ok" | "bad" | "neutral"> = {
  open: "neutral",
  in_service: "neutral",
  resolved: "ok",
  rejected: "bad",
};

export function ReturnsPage() {
  const { can } = useAuth();
  const [tab, setTab] = useState<"returns" | "warranty">("returns");
  const tabs = [
    ["returns", "Returns"],
    ["warranty", "Warranty"],
  ] as const;
  return (
    <div className="space-y-4">
      <div role="tablist" className="flex gap-1 border-b border-line">
        {tabs.map(([id, label]) => (
          <button
            key={id}
            role="tab"
            aria-selected={tab === id}
            onClick={() => setTab(id)}
            className={`px-4 py-2 text-[13px] font-semibold ${tab === id ? "border-b-2 border-teal text-navy" : "text-ink-2"}`}
          >
            {label}
          </button>
        ))}
      </div>
      {tab === "returns" ? (
        <ReturnsTab canProcess={can("returns.process")} canApprove={can("returns.approve")} />
      ) : (
        <WarrantyTab canManage={can("warranty.manage")} />
      )}
    </div>
  );
}

// ------------------------------------------------------------------ returns

function ReturnsTab({ canProcess, canApprove }: { canProcess: boolean; canApprove: boolean }) {
  const [status, setStatus] = useState("");
  const [search, setSearch] = useState("");
  const [modal, setModal] = useState<null | "new" | { open: ReturnRow }>(null);
  const list = useLoad(
    () => api.returnsList(status || undefined, search || undefined),
    [status, search],
  );
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <Input
          className="!w-64"
          placeholder="Search return, invoice or customer…"
          aria-label="Search returns"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <Select
          className="!w-44"
          aria-label="Return status"
          value={status}
          onChange={(e) => setStatus(e.target.value)}
        >
          <option value="">All statuses</option>
          <option value="requested">Awaiting inspection</option>
          <option value="completed">Completed</option>
          <option value="rejected">Rejected</option>
        </Select>
        {canProcess && (
          <Button className="ml-auto" variant="primary" onClick={() => setModal("new")}>
            <Plus size={14} />
            New return
          </Button>
        )}
      </div>
      {list.error && (
        <p role="alert" className="text-[13px] text-danger">
          {list.error}
        </p>
      )}
      <Card>
        {list.data && list.data.length === 0 ? (
          <EmptyState
            title="No returns"
            hint="Record a return against an invoice. Returned goods wait for inspection before they can be sold again or refunded."
          />
        ) : (
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th className="pb-2">Return</th>
                <th>Date</th>
                <th>Invoice</th>
                <th>Customer</th>
                <th>Reason</th>
                <th className="num">Value</th>
                <th>Status</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {list.data?.map((r) => (
                <tr key={r.id} className="border-t border-line">
                  <td className="py-2 font-semibold">{r.number}</td>
                  <td>{r.return_date}</td>
                  <td>{r.sale_number}</td>
                  <td>{r.customer ?? "Walk-in"}</td>
                  <td className="max-w-[240px] truncate">{r.reason}</td>
                  <td className="num">{formatPaisa(r.value)}</td>
                  <td>
                    <Badge tone={RETURN_TONE[r.status] ?? "neutral"}>
                      {r.status === "requested" ? "awaiting inspection" : r.status}
                    </Badge>
                  </td>
                  <td className="text-right">
                    <Button variant="ghost" onClick={() => setModal({ open: r })}>
                      {r.status === "requested" && canApprove ? "Inspect" : "View"}
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
      {modal === "new" && (
        <NewReturnModal
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            list.reload();
          }}
        />
      )}
      {modal && modal !== "new" && (
        <ReturnModal
          row={modal.open}
          canApprove={canApprove}
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            list.reload();
          }}
        />
      )}
    </div>
  );
}

function NewReturnModal({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const toast = useToast();
  const [query, setQuery] = useState("");
  const [sale, setSale] = useState<{ id: string; number: string } | null>(null);
  const [items, setItems] = useState<ReturnableItem[]>([]);
  const [qty, setQty] = useState<Record<string, number>>({});
  const [reason, setReason] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [key] = useState(newKey);

  async function find() {
    setErr(null);
    setSale(null);
    setItems([]);
    const q = query.trim();
    if (!q) return;
    try {
      let found: { id: string; number: string } | null = null;
      const bySerial = await api.returnFindSaleBySerial(q);
      if (bySerial) found = { id: bySerial[0], number: bySerial[1] };
      else {
        const page = await api.salesList({ search: q, limit: 5 });
        const hit = page.rows.find((r) => r.number.toLowerCase() === q.toLowerCase());
        if (hit) found = { id: hit.id, number: hit.number };
      }
      if (!found)
        return setErr(
          "No invoice or serial number matches. Enter an exact invoice number or a serial number.",
        );
      setSale(found);
      setItems(await api.returnableItems(found.id));
      setQty({});
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  async function submit() {
    if (!sale) return;
    const lines = Object.entries(qty)
      .filter(([, q]) => q > 0)
      .map(([sale_item_id, q]) => ({ sale_item_id, qty: q }));
    if (lines.length === 0) return setErr("Choose at least one item to return");
    if (!reason.trim()) return setErr("Enter the reason for the return");
    setBusy(true);
    try {
      await api.returnRecord({ sale_id: sale.id, reason, lines, client_key: key });
      toast("ok", "Return recorded. Goods are held for inspection.");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title="New return" onClose={onClose} width={640}>
      <div className="space-y-3">
        <Field label="Invoice number or serial number">
          {(id) => (
            <div className="flex gap-2">
              <Input
                id={id}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && void find()}
              />
              <Button onClick={() => void find()}>Find</Button>
            </div>
          )}
        </Field>
        {sale && (
          <>
            <p className="text-[12px] text-ink-2">
              Invoice <b>{sale.number}</b>
            </p>
            <table className="w-full text-[12px]">
              <thead>
                <tr className="text-left text-ink-2">
                  <th>Item</th>
                  <th>Serial</th>
                  <th className="num">Sold</th>
                  <th className="num">Returnable</th>
                  <th className="num">Unit value</th>
                  <th className="num">Return qty</th>
                </tr>
              </thead>
              <tbody>
                {items.map((i) => (
                  <tr key={i.sale_item_id} className="border-t border-line">
                    <td className="py-1.5">{i.label}</td>
                    <td>{i.serial_no ?? "—"}</td>
                    <td className="num">{i.sold_qty}</td>
                    <td className="num">{i.returnable_qty}</td>
                    <td className="num">{formatPaisa(i.unit_value)}</td>
                    <td className="num">
                      <Input
                        className="!w-16 text-right"
                        type="number"
                        min={0}
                        max={i.returnable_qty}
                        aria-label={`Return quantity for ${i.label}`}
                        disabled={i.returnable_qty === 0}
                        value={qty[i.sale_item_id] ?? 0}
                        onChange={(e) =>
                          setQty({ ...qty, [i.sale_item_id]: Math.max(0, Number(e.target.value)) })
                        }
                      />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            <Field label="Reason" error={err}>
              {(id) => <Input id={id} value={reason} onChange={(e) => setReason(e.target.value)} />}
            </Field>
          </>
        )}
        {!sale && err && (
          <p role="alert" className="text-[12px] text-danger">
            {err}
          </p>
        )}
      </div>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" busy={busy} disabled={!sale} onClick={() => void submit()}>
          Record return
        </Button>
      </div>
    </Modal>
  );
}

function ReturnModal({
  row,
  canApprove,
  onClose,
  onDone,
}: {
  row: ReturnRow;
  canApprove: boolean;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const detail = useLoad<ReturnDetail>(() => api.returnGet(row.id), [row.id]);
  const [outcome, setOutcome] = useState<Record<string, string>>({});
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [refund, setRefund] = useState<number | null>(0);
  const [method, setMethod] = useState("cash");
  const [note, setNote] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [key] = useState(newKey);
  const items = detail.data?.items ?? [];
  const pending = row.status === "requested" && canApprove;

  async function approve() {
    if (items.some((i) => !outcome[i.id]))
      return setErr("Choose an inspection outcome for every item");
    setBusy(true);
    try {
      const res = await api.returnApprove(row.id, {
        decisions: items.map((i) => ({
          item_id: i.id,
          outcome: outcome[i.id] ?? "",
          note: notes[i.id] ?? "",
        })),
        refund_amount: refund ?? 0,
        refund_method: method,
        note,
        client_key: key,
      });
      toast(
        "ok",
        `Return approved (${formatPaisa(res.value)})${res.refund_receipt ? `, refund ${res.refund_receipt}` : ""}`,
      );
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  async function reject() {
    if (!note.trim()) return setErr("Enter the reason for rejecting this return in the note field");
    setBusy(true);
    try {
      await api.returnReject(row.id, note);
      toast("ok", "Return rejected");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title={`Return ${row.number}`} onClose={onClose} width={680}>
      <p className="mb-2 text-[12px] text-ink-2">
        Invoice {row.sale_number} · {row.customer ?? "Walk-in"} · {row.return_date} · {row.reason}
      </p>
      <table className="w-full text-[12px]">
        <thead>
          <tr className="text-left text-ink-2">
            <th>Item</th>
            <th>Serial</th>
            <th className="num">Qty</th>
            <th className="num">Value</th>
            <th>Outcome</th>
          </tr>
        </thead>
        <tbody>
          {items.map((i) => (
            <tr key={i.id} className="border-t border-line align-top">
              <td className="py-1.5">{i.label}</td>
              <td>{i.serial_no ?? "—"}</td>
              <td className="num">{i.qty}</td>
              <td className="num">{formatPaisa(i.value)}</td>
              <td>
                {pending ? (
                  <div className="space-y-1">
                    <Select
                      aria-label={`Outcome for ${i.label}`}
                      value={outcome[i.id] ?? ""}
                      onChange={(e) => setOutcome({ ...outcome, [i.id]: e.target.value })}
                    >
                      <option value="">Choose…</option>
                      <option value="restock">Good: back to stock</option>
                      <option value="damaged">Damaged: hold</option>
                      <option value="write_off">Write off</option>
                    </Select>
                    <Input
                      placeholder="Inspection note"
                      aria-label={`Note for ${i.label}`}
                      value={notes[i.id] ?? ""}
                      onChange={(e) => setNotes({ ...notes, [i.id]: e.target.value })}
                    />
                  </div>
                ) : (
                  <span>
                    {i.outcome ?? "awaiting inspection"}
                    {i.note ? ` — ${i.note}` : ""}
                  </span>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {detail.data?.decision_note && (
        <p className="mt-2 text-[12px] text-ink-2">Decision note: {detail.data.decision_note}</p>
      )}
      {detail.data?.refunds.map(([no, amount, m]) => (
        <p key={no} className="mt-1 text-[12px]">
          Refund {no}: {formatPaisa(amount)} ({m})
        </p>
      ))}
      {pending && (
        <div className="mt-4 grid grid-cols-2 gap-3">
          <Field
            label="Refund now (৳)"
            hint="0 = credit only. Limited to what the customer has overpaid."
          >
            {(id) => <MoneyInput id={id} value={refund} onChange={setRefund} />}
          </Field>
          <Field label="Refund method">
            {(id) => (
              <Select id={id} value={method} onChange={(e) => setMethod(e.target.value)}>
                <option value="cash">Cash</option>
                <option value="bank">Bank transfer</option>
                <option value="mobile_wallet">Mobile wallet</option>
                <option value="cheque">Cheque</option>
                <option value="other">Other</option>
              </Select>
            )}
          </Field>
          <div className="col-span-2">
            <Field label="Note (required to reject)" error={err}>
              {(id) => <Input id={id} value={note} onChange={(e) => setNote(e.target.value)} />}
            </Field>
          </div>
        </div>
      )}
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Close</Button>
        {pending && (
          <>
            <Button variant="danger" busy={busy} onClick={() => void reject()}>
              Reject return
            </Button>
            <Button variant="primary" busy={busy} onClick={() => void approve()}>
              Approve
            </Button>
          </>
        )}
      </div>
    </Modal>
  );
}

// ------------------------------------------------------------------ warranty

function WarrantyTab({ canManage }: { canManage: boolean }) {
  const [status, setStatus] = useState("");
  const [search, setSearch] = useState("");
  const [modal, setModal] = useState<null | "new" | { open: WarrantyCaseRow }>(null);
  const list = useLoad(
    () => api.warrantyList(status || undefined, search || undefined),
    [status, search],
  );
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <Input
          className="!w-64"
          placeholder="Search case, serial or customer…"
          aria-label="Search warranty cases"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <Select
          className="!w-40"
          aria-label="Case status"
          value={status}
          onChange={(e) => setStatus(e.target.value)}
        >
          <option value="">All statuses</option>
          <option value="open">Open</option>
          <option value="in_service">In service</option>
          <option value="resolved">Resolved</option>
          <option value="rejected">Rejected</option>
        </Select>
        <Button className="ml-auto" onClick={() => setModal("new")}>
          Check serial / open case
        </Button>
      </div>
      <Card>
        {list.data && list.data.length === 0 ? (
          <EmptyState
            title="No warranty cases"
            hint="Look up a serial number to see its coverage and open a case."
          />
        ) : (
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th className="pb-2">Case</th>
                <th>Serial</th>
                <th>Product</th>
                <th>Customer</th>
                <th>Opened</th>
                <th>Coverage</th>
                <th>Status</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {list.data?.map((c) => (
                <tr key={c.id} className="border-t border-line">
                  <td className="py-2 font-semibold">{c.number}</td>
                  <td>{c.serial_no}</td>
                  <td>{c.label}</td>
                  <td>{c.customer ?? "—"}</td>
                  <td>{c.opened_on}</td>
                  <td>
                    <Badge tone={c.in_warranty ? "ok" : "bad"}>
                      {c.in_warranty ? "in warranty" : "out of warranty"}
                    </Badge>
                  </td>
                  <td>
                    <Badge tone={CASE_TONE[c.status] ?? "neutral"}>
                      {c.status.replace("_", " ")}
                    </Badge>
                  </td>
                  <td className="text-right">
                    <Button variant="ghost" onClick={() => setModal({ open: c })}>
                      Open
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
      {modal === "new" && (
        <LookupModal
          canManage={canManage}
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            list.reload();
          }}
        />
      )}
      {modal && modal !== "new" && (
        <CaseModal
          row={modal.open}
          canManage={canManage}
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            list.reload();
          }}
        />
      )}
    </div>
  );
}

function LookupModal({
  canManage,
  onClose,
  onDone,
}: {
  canManage: boolean;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const [serial, setSerial] = useState("");
  const [cov, setCov] = useState<Coverage | null | undefined>(undefined);
  const [issue, setIssue] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function look() {
    setErr(null);
    try {
      setCov(await api.warrantyLookup(serial));
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  async function open() {
    setBusy(true);
    try {
      await api.warrantyOpen(serial, issue);
      toast("ok", "Warranty case opened");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title="Warranty lookup" onClose={onClose} width={520}>
      <Field label="Serial number">
        {(id) => (
          <div className="flex gap-2">
            <Input
              id={id}
              value={serial}
              onChange={(e) => setSerial(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void look()}
            />
            <Button onClick={() => void look()}>Check</Button>
          </div>
        )}
      </Field>
      {cov === null && (
        <p role="alert" className="mt-3 text-[13px] text-danger">
          No unit with that serial number.
        </p>
      )}
      {cov && (
        <div className="mt-3 space-y-2 text-[13px]">
          <p className="font-semibold text-navy">{cov.label}</p>
          {cov.sale_number ? (
            <>
              <p>
                Sold on {cov.purchased_on} · invoice {cov.sale_number} · {cov.customer ?? "Walk-in"}
              </p>
              <p>
                <Badge tone={cov.in_warranty ? "ok" : "bad"}>
                  {cov.in_warranty ? "In warranty" : "Out of warranty"}
                </Badge>{" "}
                {cov.warranty_months} months, until {cov.expires_on ?? "—"}
              </p>
              {cov.warranty_terms && <p className="text-ink-2">Terms: {cov.warranty_terms}</p>}
            </>
          ) : (
            <p className="text-ink-2">
              This unit has not been sold ({cov.status}); no claim possible.
            </p>
          )}
          {canManage && cov.sale_number && (
            <>
              <Field label="Reported issue" error={err}>
                {(id) => <Input id={id} value={issue} onChange={(e) => setIssue(e.target.value)} />}
              </Field>
              <div className="flex justify-end">
                <Button variant="primary" busy={busy} onClick={() => void open()}>
                  Open warranty case
                </Button>
              </div>
            </>
          )}
        </div>
      )}
      {cov === undefined && err && (
        <p role="alert" className="mt-2 text-[12px] text-danger">
          {err}
        </p>
      )}
    </Modal>
  );
}

function CaseModal({
  row,
  canManage,
  onClose,
  onDone,
}: {
  row: WarrantyCaseRow;
  canManage: boolean;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const d = useLoad<WarrantyCaseDetail>(() => api.warrantyGet(row.id), [row.id]);
  const [status, setStatus] = useState("in_service");
  const [note, setNote] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const closed = row.status === "resolved" || row.status === "rejected";
  async function save() {
    setBusy(true);
    try {
      await api.warrantyUpdate(row.id, status, note);
      toast("ok", "Case updated");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title={`Case ${row.number}`} onClose={onClose} width={560}>
      <p className="text-[13px]">
        <b>{row.label}</b> · serial {row.serial_no} · {row.customer ?? "Walk-in"}
      </p>
      <p className="mb-3 text-[12px] text-ink-2">Issue: {row.issue}</p>
      <ol className="space-y-1.5 text-[12px]">
        {d.data?.events.map((e, i) => (
          <li key={i} className="rounded-md bg-slate-50 px-3 py-1.5">
            <span className="font-semibold">{e.status.replace("_", " ")}</span> ·{" "}
            {formatDhaka(e.at)} · {e.actor ?? "system"}
            <br />
            {e.note}
          </li>
        ))}
      </ol>
      {canManage && !closed && (
        <div className="mt-4 space-y-3">
          <Field label="Move to">
            {(id) => (
              <Select id={id} value={status} onChange={(e) => setStatus(e.target.value)}>
                <option value="in_service">In service</option>
                <option value="resolved">Resolved</option>
                <option value="rejected">Rejected</option>
              </Select>
            )}
          </Field>
          <Field label="Note" error={err}>
            {(id) => <Input id={id} value={note} onChange={(e) => setNote(e.target.value)} />}
          </Field>
        </div>
      )}
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Close</Button>
        {canManage && !closed && (
          <Button variant="primary" busy={busy} onClick={() => void save()}>
            Save
          </Button>
        )}
      </div>
    </Modal>
  );
}
