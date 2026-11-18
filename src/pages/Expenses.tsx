import { useState } from "react";
import { Plus } from "lucide-react";
import { api, toApiError, type CashSession, type CashSummary, type ExpenseRow } from "../lib/api";
import { useAuth } from "../lib/auth";
import { useLoad } from "../lib/useLoad";
import { formatDhaka, formatPaisa, newKey, todayDhakaIso } from "../lib/format";
import { Badge, Card, EmptyState } from "../ui/Card";
import { Button, Field, Input, Modal, Select, useToast } from "../ui/controls";
import { MoneyInput } from "../ui/MoneyInput";

const TONE: Record<string, "ok" | "bad" | "neutral"> = {
  pending: "neutral",
  approved: "ok",
  rejected: "bad",
  voided: "bad",
};

export function ExpensesPage() {
  const { can } = useAuth();
  const [tab, setTab] = useState<"expenses" | "cash">(
    can("expenses.create") || can("expenses.approve") ? "expenses" : "cash",
  );
  const tabs: [typeof tab, string][] = [];
  if (can("expenses.create") || can("expenses.approve")) tabs.push(["expenses", "Expenses"]);
  if (can("cash.session")) tabs.push(["cash", "Cash session"]);
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
      {tab === "expenses" ? (
        <ExpensesTab canCreate={can("expenses.create")} canApprove={can("expenses.approve")} />
      ) : (
        <CashTab />
      )}
    </div>
  );
}

function ExpensesTab({ canCreate, canApprove }: { canCreate: boolean; canApprove: boolean }) {
  const toast = useToast();
  const [status, setStatus] = useState("");
  const [search, setSearch] = useState("");
  const [modal, setModal] = useState<
    null | "new" | "categories" | { act: "reject" | "void"; row: ExpenseRow }
  >(null);
  const list = useLoad(
    () => api.expensesList({ status: status || null, search: search || null }),
    [status, search],
  );
  async function approve(r: ExpenseRow) {
    try {
      await api.expenseApprove(r.id);
      toast("ok", `${r.number} approved`);
      list.reload();
    } catch (e) {
      toast("bad", toApiError(e).message);
    }
  }
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <Input
          className="!w-64"
          placeholder="Search payee or description…"
          aria-label="Search expenses"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <Select
          className="!w-40"
          aria-label="Expense status"
          value={status}
          onChange={(e) => setStatus(e.target.value)}
        >
          <option value="">All statuses</option>
          <option value="pending">Pending</option>
          <option value="approved">Approved</option>
          <option value="rejected">Rejected</option>
          <option value="voided">Voided</option>
        </Select>
        <div className="ml-auto flex gap-2">
          {canApprove && <Button onClick={() => setModal("categories")}>Categories</Button>}
          {canCreate && (
            <Button variant="primary" onClick={() => setModal("new")}>
              <Plus size={14} />
              New expense
            </Button>
          )}
        </div>
      </div>
      {list.data && (
        <p className="text-[12px] text-ink-2">
          Approved total: <b>{formatPaisa(list.data.approved_total)}</b> · Awaiting approval:{" "}
          <b>{list.data.pending_count}</b>
        </p>
      )}
      {list.error && (
        <p role="alert" className="text-[13px] text-danger">
          {list.error}
        </p>
      )}
      <Card>
        {list.data && list.data.rows.length === 0 ? (
          <EmptyState
            title="No expenses"
            hint="Record rent, utilities, salaries and other running costs here."
          />
        ) : (
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th className="pb-2">No.</th>
                <th>Date</th>
                <th>Category</th>
                <th>Payee</th>
                <th>Description</th>
                <th>Method</th>
                <th className="num">Amount</th>
                <th>Status</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {list.data?.rows.map((r) => (
                <tr key={r.id} className="border-t border-line">
                  <td className="py-2 font-semibold">{r.number}</td>
                  <td>{r.expense_date}</td>
                  <td>{r.category}</td>
                  <td>{r.payee}</td>
                  <td className="max-w-[220px] truncate" title={r.decision_note ?? r.description}>
                    {r.description}
                  </td>
                  <td>{r.method}</td>
                  <td className="num">{formatPaisa(r.amount)}</td>
                  <td>
                    <Badge tone={TONE[r.status] ?? "neutral"}>{r.status}</Badge>
                  </td>
                  <td className="space-x-1 text-right">
                    {canApprove && r.status === "pending" && (
                      <>
                        <Button variant="ghost" onClick={() => void approve(r)}>
                          Approve
                        </Button>
                        <Button variant="ghost" onClick={() => setModal({ act: "reject", row: r })}>
                          Reject
                        </Button>
                      </>
                    )}
                    {canApprove && r.status === "approved" && (
                      <Button variant="ghost" onClick={() => setModal({ act: "void", row: r })}>
                        Void
                      </Button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
      {modal === "new" && (
        <NewExpenseModal
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            list.reload();
          }}
        />
      )}
      {modal === "categories" && <CategoriesModal onClose={() => setModal(null)} />}
      {modal && typeof modal === "object" && (
        <ReasonModal
          act={modal.act}
          row={modal.row}
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

function NewExpenseModal({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const toast = useToast();
  const cats = useLoad(() => api.expenseCategories(), []);
  const [category, setCategory] = useState("");
  const [date, setDate] = useState(todayDhakaIso());
  const [amount, setAmount] = useState<number | null>(null);
  const [method, setMethod] = useState("cash");
  const [payee, setPayee] = useState("");
  const [desc, setDesc] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [key] = useState(newKey);
  async function save() {
    if (!category) return setErr("Choose a category");
    if (!amount || amount <= 0) return setErr("Enter an amount");
    if (!desc.trim()) return setErr("Describe the expense");
    setBusy(true);
    try {
      await api.expenseCreate({
        category_id: category,
        expense_date: date,
        amount,
        method,
        payee,
        description: desc,
        client_key: key,
      });
      toast("ok", "Expense saved");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title="New expense" onClose={onClose} width={480}>
      <div className="space-y-3">
        <Field label="Category">
          {(id) => (
            <Select id={id} value={category} onChange={(e) => setCategory(e.target.value)}>
              <option value="">Choose…</option>
              {cats.data
                ?.filter((c) => c.is_active)
                .map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.name}
                  </option>
                ))}
            </Select>
          )}
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label="Date">
            {(id) => (
              <Input
                id={id}
                type="date"
                value={date}
                max={todayDhakaIso()}
                onChange={(e) => setDate(e.target.value)}
              />
            )}
          </Field>
          <Field label="Amount (৳)">
            {(id) => <MoneyInput id={id} value={amount} onChange={setAmount} />}
          </Field>
        </div>
        <Field label="Paid by">
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
        <Field label="Payee">
          {(id) => <Input id={id} value={payee} onChange={(e) => setPayee(e.target.value)} />}
        </Field>
        <Field label="Description" error={err}>
          {(id) => <Input id={id} value={desc} onChange={(e) => setDesc(e.target.value)} />}
        </Field>
      </div>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" busy={busy} onClick={() => void save()}>
          Save expense
        </Button>
      </div>
    </Modal>
  );
}

function ReasonModal({
  act,
  row,
  onClose,
  onDone,
}: {
  act: "reject" | "void";
  row: ExpenseRow;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const [reason, setReason] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function go() {
    if (!reason.trim()) return setErr("A reason is required");
    setBusy(true);
    try {
      if (act === "reject") await api.expenseReject(row.id, reason);
      else await api.expenseVoid(row.id, reason);
      toast("ok", `${row.number} ${act === "reject" ? "rejected" : "voided"}`);
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal
      title={`${act === "reject" ? "Reject" : "Void"} ${row.number}`}
      onClose={onClose}
      width={420}
    >
      <p className="mb-3 text-[12px] text-ink-2">
        {formatPaisa(row.amount)} · {row.category} · {row.description}.{" "}
        {act === "void" && "The accounting entry is reversed; the record stays visible."}
      </p>
      <Field label="Reason" error={err}>
        {(id) => <Input id={id} value={reason} onChange={(e) => setReason(e.target.value)} />}
      </Field>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="danger" busy={busy} onClick={() => void go()}>
          {act === "reject" ? "Reject" : "Void expense"}
        </Button>
      </div>
    </Modal>
  );
}

function CategoriesModal({ onClose }: { onClose: () => void }) {
  const toast = useToast();
  const cats = useLoad(() => api.expenseCategories(), []);
  const [name, setName] = useState("");
  const [code, setCode] = useState("6900");
  const [err, setErr] = useState<string | null>(null);
  async function add() {
    try {
      await api.expenseCategoryCreate(name, code);
      setName("");
      setErr(null);
      cats.reload();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  async function toggle(id: string, active: boolean) {
    try {
      await api.expenseCategorySetActive(id, active);
      cats.reload();
    } catch (e) {
      toast("bad", toApiError(e).message);
    }
  }
  return (
    <Modal title="Expense categories" onClose={onClose} width={500}>
      <ul className="mb-4 divide-y divide-line text-[13px]">
        {cats.data?.map((c) => (
          <li key={c.id} className="flex items-center justify-between py-1.5">
            <span className={c.is_active ? "" : "text-ink-2 line-through"}>
              {c.name} <span className="text-[11px] text-ink-2">({c.account_code})</span>
            </span>
            <Button variant="ghost" onClick={() => void toggle(c.id, !c.is_active)}>
              {c.is_active ? "Deactivate" : "Activate"}
            </Button>
          </li>
        ))}
      </ul>
      <div className="grid grid-cols-[1fr_96px_auto] items-end gap-2">
        <Field label="New category" error={err}>
          {(id) => <Input id={id} value={name} onChange={(e) => setName(e.target.value)} />}
        </Field>
        <Field label="Account">
          {(id) => <Input id={id} value={code} onChange={(e) => setCode(e.target.value)} />}
        </Field>
        <Button onClick={() => void add()}>Add</Button>
      </div>
    </Modal>
  );
}

// ------------------------------------------------------------------ cash session

function CashTab() {
  const toast = useToast();
  const current = useLoad(() => api.cashCurrent(), []);
  const history = useLoad(() => api.cashSessions(), []);
  const [modal, setModal] = useState<null | "open" | "close" | { view: CashSession }>(null);
  const cur = current.data;
  const summary = useLoad<CashSummary | null>(
    () => (cur ? api.cashSummary(cur.id) : Promise.resolve(null)),
    [cur?.id ?? "none"],
  );
  function refresh() {
    setModal(null);
    current.reload();
    history.reload();
    summary.reload();
  }
  return (
    <div className="space-y-4">
      <Card
        title={cur ? `Open session ${cur.number}` : "No open session"}
        action={
          cur ? (
            <Button variant="primary" onClick={() => setModal("close")}>
              Count and close
            </Button>
          ) : (
            <Button variant="primary" onClick={() => setModal("open")}>
              Open today&apos;s session
            </Button>
          )
        }
      >
        {cur && summary.data ? (
          <SummaryTable s={summary.data} />
        ) : (
          <p className="text-[13px] text-ink-2">
            Open a session with the cash in the drawer. At close, the counted cash is compared with
            what the books expect and any difference is recorded.
          </p>
        )}
      </Card>
      <Card title="Previous sessions">
        {history.data && history.data.filter((s) => s.status === "closed").length === 0 ? (
          <p className="text-[13px] text-ink-2">No closed sessions yet.</p>
        ) : (
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th className="pb-2">Session</th>
                <th>Date</th>
                <th className="num">Opening</th>
                <th className="num">Expected</th>
                <th className="num">Counted</th>
                <th className="num">Variance</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {history.data
                ?.filter((s) => s.status === "closed")
                .map((s) => (
                  <tr key={s.id} className="border-t border-line">
                    <td className="py-2 font-semibold">{s.number}</td>
                    <td>{s.business_date}</td>
                    <td className="num">{formatPaisa(s.opening_cash)}</td>
                    <td className="num">{formatPaisa(s.expected_cash ?? 0)}</td>
                    <td className="num">{formatPaisa(s.counted_cash ?? 0)}</td>
                    <td className={`num ${(s.variance ?? 0) < 0 ? "text-danger" : ""}`}>
                      {formatPaisa(s.variance ?? 0)}
                    </td>
                    <td className="text-right">
                      <Button variant="ghost" onClick={() => setModal({ view: s })}>
                        Details
                      </Button>
                    </td>
                  </tr>
                ))}
            </tbody>
          </table>
        )}
      </Card>
      {modal === "open" && (
        <OpenModal
          lastCounted={history.data?.find((s) => s.status === "closed")?.counted_cash ?? null}
          onClose={() => setModal(null)}
          onDone={() => {
            toast("ok", "Session opened");
            refresh();
          }}
        />
      )}
      {modal === "close" && cur && summary.data && (
        <CloseModal
          s={summary.data}
          onClose={() => setModal(null)}
          onDone={() => {
            toast("ok", "Session closed");
            refresh();
          }}
        />
      )}
      {modal && typeof modal === "object" && (
        <ViewModal s={modal.view} onClose={() => setModal(null)} />
      )}
    </div>
  );
}

function SummaryTable({ s }: { s: CashSummary }) {
  const rows: [string, number, boolean?][] = [
    ["Opening cash", s.session.opening_cash],
    ["Cash sales received", s.cash_sale_receipts],
    ["Cash installment collections", s.cash_installment_receipts],
    ["Cash refunds paid", -s.cash_refunds],
    ["Cash expenses paid", -s.cash_expenses],
    ["Cash paid to suppliers", -s.cash_supplier_payments],
    ["Expected cash in drawer", s.expected_cash, true],
  ];
  return (
    <table className="w-full max-w-md text-[13px]">
      <tbody>
        {rows.map(([label, v, bold]) => (
          <tr key={label} className={bold ? "border-t border-line font-bold" : ""}>
            <td className="py-1">{label}</td>
            <td className="num">{formatPaisa(v)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function OpenModal({
  lastCounted,
  onClose,
  onDone,
}: {
  lastCounted: number | null;
  onClose: () => void;
  onDone: () => void;
}) {
  const [amount, setAmount] = useState<number | null>(lastCounted ?? 0);
  const [note, setNote] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function go() {
    setBusy(true);
    try {
      await api.cashOpen(amount ?? 0, note);
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title="Open cash session" onClose={onClose} width={420}>
      <div className="space-y-3">
        <Field
          label="Cash in drawer (৳)"
          hint={lastCounted !== null ? `Last close counted ${formatPaisa(lastCounted)}` : undefined}
        >
          {(id) => <MoneyInput id={id} value={amount} onChange={setAmount} />}
        </Field>
        <Field label="Note (required if different from last count)" error={err}>
          {(id) => <Input id={id} value={note} onChange={(e) => setNote(e.target.value)} />}
        </Field>
      </div>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" busy={busy} onClick={() => void go()}>
          Open session
        </Button>
      </div>
    </Modal>
  );
}

function CloseModal({
  s,
  onClose,
  onDone,
}: {
  s: CashSummary;
  onClose: () => void;
  onDone: () => void;
}) {
  const [counted, setCounted] = useState<number | null>(null);
  const [note, setNote] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const variance = counted === null ? null : counted - s.expected_cash;
  async function go() {
    if (counted === null) return setErr("Enter the counted cash");
    setBusy(true);
    try {
      await api.cashClose(s.session.id, counted, note);
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title="Count and close" onClose={onClose} width={460}>
      <SummaryTable s={s} />
      <div className="mt-4 space-y-3">
        <Field label="Counted cash (৳)">
          {(id) => <MoneyInput id={id} value={counted} onChange={setCounted} />}
        </Field>
        {variance !== null && (
          <p
            role="status"
            className={`text-[13px] font-semibold ${variance === 0 ? "text-teal-deep" : "text-danger"}`}
          >
            {variance === 0
              ? "Matches the books."
              : variance < 0
                ? `Shortage of ${formatPaisa(-variance)}`
                : `Overage of ${formatPaisa(variance)}`}
          </p>
        )}
        <Field label="Explanation (required if there is a difference)" error={err}>
          {(id) => <Input id={id} value={note} onChange={(e) => setNote(e.target.value)} />}
        </Field>
      </div>
      <p className="mt-2 text-[11px] text-ink-2">
        Closing locks the session. It cannot be edited afterwards.
      </p>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" busy={busy} onClick={() => void go()}>
          Close session
        </Button>
      </div>
    </Modal>
  );
}

function ViewModal({ s, onClose }: { s: CashSession; onClose: () => void }) {
  const d = useLoad(() => api.cashSummary(s.id), [s.id]);
  return (
    <Modal title={`Session ${s.number}`} onClose={onClose} width={460}>
      <p className="mb-2 text-[12px] text-ink-2">
        Opened {formatDhaka(s.opened_at)} by {s.opened_by ?? "—"}
        {s.closed_at && ` · closed ${formatDhaka(s.closed_at)} by ${s.closed_by ?? "—"}`}
      </p>
      {d.data && <SummaryTable s={d.data} />}
      <p className="mt-3 text-[13px]">
        Counted {formatPaisa(s.counted_cash ?? 0)} · variance {formatPaisa(s.variance ?? 0)}
      </p>
      {s.close_note && <p className="text-[12px] text-ink-2">Note: {s.close_note}</p>}
      <div className="mt-4 flex justify-end">
        <Button onClick={onClose}>Close</Button>
      </div>
    </Modal>
  );
}
