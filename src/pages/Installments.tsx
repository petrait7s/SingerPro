import { useState } from "react";
import { AlertTriangle } from "lucide-react";
import { api, toApiError, type AgreementDetail, type AgreementRow } from "../lib/api";
import { useAuth } from "../lib/auth";
import { useLoad } from "../lib/useLoad";
import { formatDhaka, formatPaisa, newKey, todayDhakaIso } from "../lib/format";
import { Badge, Card, EmptyState } from "../ui/Card";
import { Button, Field, Input, Modal, Select, useToast } from "../ui/controls";
import { MoneyInput } from "../ui/MoneyInput";

const STATUS_TONE: Record<string, "ok" | "bad" | "neutral"> = {
  paid: "ok",
  completed: "ok",
  overdue: "bad",
  partial: "neutral",
  pending: "neutral",
  superseded: "neutral",
  active: "neutral",
};

export function InstallmentsPage() {
  const [tab, setTab] = useState<"agreements" | "overdue">("agreements");
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const list = useLoad(
    () =>
      api.agreementsList({
        search: search || undefined,
        status: status || undefined,
        overdue_only: tab === "overdue",
        limit: 200,
      }),
    [search, status, tab],
  );
  const rows = list.data?.rows ?? [];
  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-3">
        <div className="flex gap-1 rounded-lg border border-line p-0.5 text-[12px] font-semibold">
          {(
            [
              ["agreements", "All agreements"],
              ["overdue", "Overdue follow-up"],
            ] as const
          ).map(([k, l]) => (
            <button
              key={k}
              onClick={() => setTab(k)}
              aria-pressed={tab === k}
              className={`rounded-md px-3 py-1 ${tab === k ? "bg-navy text-white" : "text-ink-2"}`}
            >
              {l}
            </button>
          ))}
        </div>
        <Input
          className="!w-64"
          placeholder="Search agreement, customer, phone…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label="Search agreements"
        />
        {tab === "agreements" && (
          <Select
            className="!w-40"
            value={status}
            onChange={(e) => setStatus(e.target.value)}
            aria-label="Status"
          >
            <option value="">All</option>
            <option value="active">Active</option>
            <option value="completed">Completed</option>
          </Select>
        )}
        {list.data && (
          <span className="ml-auto text-[13px] text-ink-2">
            Outstanding{" "}
            <b className="text-navy tabular-nums">{formatPaisa(list.data.outstanding_total)}</b> ·
            Overdue{" "}
            <b className="text-danger tabular-nums">{formatPaisa(list.data.overdue_total)}</b>
          </span>
        )}
      </div>
      {list.error && (
        <p role="alert" className="text-[13px] text-danger">
          {list.error}
        </p>
      )}
      <Card>
        {list.data && rows.length === 0 ? (
          <EmptyState
            title={tab === "overdue" ? "Nothing overdue" : "No installment agreements yet"}
            hint={
              tab === "overdue"
                ? "Customers with unpaid installments past their due date appear here, most overdue first."
                : "Create one from New Sale / POS using the Installment type."
            }
          />
        ) : (
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th className="pb-2">Agreement</th>
                <th>Customer</th>
                <th>Phone</th>
                <th className="num">Outstanding</th>
                <th>Next due</th>
                <th className="num">Overdue</th>
                <th>Status</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr
                  key={r.id}
                  className="cursor-pointer border-t border-line hover:bg-mint/30"
                  onClick={() => setOpen(r.id)}
                >
                  <td className="py-2 font-medium text-navy">
                    {r.number}
                    <span className="ml-1 text-ink-2">{r.sale_number}</span>
                  </td>
                  <td>{r.customer}</td>
                  <td>{r.customer_phone}</td>
                  <td className="num">{formatPaisa(r.outstanding)}</td>
                  <td>
                    {r.next_due_date
                      ? `${r.next_due_date} · ${formatPaisa(r.next_due_amount)}`
                      : "—"}
                  </td>
                  <td className={`num ${r.overdue_amount > 0 ? "font-semibold text-danger" : ""}`}>
                    {r.overdue_amount > 0
                      ? `${formatPaisa(r.overdue_amount)} (${r.max_days_overdue}d)`
                      : "—"}
                  </td>
                  <td>
                    <Badge tone={STATUS_TONE[r.status] ?? "neutral"}>{r.status}</Badge>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
      {open && <AgreementModal id={open} onClose={() => setOpen(null)} onChanged={list.reload} />}
    </div>
  );
}

export function AgreementModal({
  id,
  onClose,
  onChanged,
}: {
  id: string;
  onClose: () => void;
  onChanged?: () => void;
}) {
  const { can } = useAuth();
  const d = useLoad<AgreementDetail>(() => api.agreementGet(id), [id]);
  const [sub, setSub] = useState<
    | null
    | { collect: string | null }
    | "settle"
    | "reschedule"
    | "followup"
    | { fee: string }
    | { reverse: string }
  >(null);
  const after = () => {
    setSub(null);
    d.reload();
    onChanged?.();
  };
  if (!d.data)
    return (
      <Modal title="Installment agreement" onClose={onClose}>
        {d.error ? (
          <p role="alert" className="text-danger">
            {d.error}
          </p>
        ) : (
          <p className="text-ink-2">Loading…</p>
        )}
      </Modal>
    );
  const x = d.data;
  const a = x.agreement;
  const active = a.status === "active";
  const canCollect = can("installments.collect");
  return (
    <Modal title={`${a.number} · ${a.customer}`} onClose={onClose} width={980}>
      <div className="grid grid-cols-[1fr_300px] gap-5">
        <div>
          <div className="mb-3 flex items-center gap-2 text-[12px]">
            <Badge tone={STATUS_TONE[a.status] ?? "neutral"}>{a.status}</Badge>
            <span>Invoice {a.sale_number}</span>
            <span>
              {x.frequency}, {a.installment_count} installments from {x.first_due_date}
            </span>
            {a.overdue_amount > 0 && (
              <span className="flex items-center gap-1 font-semibold text-danger">
                <AlertTriangle size={13} />
                {formatPaisa(a.overdue_amount)} overdue ({a.max_days_overdue} days)
              </span>
            )}
          </div>
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th>#</th>
                <th>Due</th>
                <th className="num">Amount</th>
                <th className="num">Paid</th>
                <th className="num">Left</th>
                <th>Status</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {x.items.map((i) => (
                <tr
                  key={i.id}
                  className={`border-t border-line ${i.superseded ? "text-ink-2 line-through" : ""}`}
                >
                  <td className="py-1.5">{i.seq}</td>
                  <td>{i.due_date}</td>
                  <td className="num">
                    {formatPaisa(i.amount)}
                    {i.late_fee > 0 && (
                      <span className="text-warn"> +{formatPaisa(i.late_fee)} fee</span>
                    )}
                  </td>
                  <td className="num">{formatPaisa(i.paid)}</td>
                  <td className="num">{formatPaisa(i.remaining)}</td>
                  <td>
                    <Badge tone={STATUS_TONE[i.status] ?? "neutral"}>
                      {i.status}
                      {i.days_overdue > 0 ? ` ${i.days_overdue}d` : ""}
                    </Badge>
                  </td>
                  <td className="space-x-1 text-right whitespace-nowrap">
                    {active && canCollect && i.remaining > 0 && !i.superseded && (
                      <button className="underline" onClick={() => setSub({ collect: i.id })}>
                        pay this
                      </button>
                    )}
                    {active &&
                      i.status === "overdue" &&
                      i.late_fee === 0 &&
                      can("installments.reschedule") && (
                        <button
                          className="text-warn underline"
                          onClick={() => setSub({ fee: i.id })}
                        >
                          late fee
                        </button>
                      )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="mb-1 mt-4 text-[12px] font-semibold text-navy">Collections</p>
          <table className="w-full text-[12px]">
            <tbody>
              {x.collections.length === 0 && (
                <tr>
                  <td className="text-ink-2">No collections yet.</td>
                </tr>
              )}
              {x.collections.map((c) => (
                <tr key={c.id} className="border-t border-line">
                  <td className="py-1">{c.receipt_no}</td>
                  <td>{c.paid_on}</td>
                  <td>{c.kind === "rebate" ? "rebate" : c.method}</td>
                  <td className="num">{formatPaisa(c.amount)}</td>
                  <td className="text-ink-2">{c.note}</td>
                  <td className="text-right">
                    {can("installments.reverse_payment") &&
                      c.amount > 0 &&
                      !x.collections.some((r) => r.reverses_id === c.id) && (
                        <button
                          className="text-danger underline"
                          onClick={() => setSub({ reverse: c.id })}
                        >
                          reverse
                        </button>
                      )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {x.followups.length > 0 && (
            <>
              <p className="mb-1 mt-4 text-[12px] font-semibold text-navy">Follow-ups</p>
              {x.followups.map((f, i) => (
                <p key={i} className="border-t border-line py-1 text-[12px]">
                  <span className="text-ink-2">
                    {formatDhaka(f.at)} {f.actor && `· ${f.actor}`}
                  </span>{" "}
                  — {f.note}
                  {f.promise_date && <b> (promised {f.promise_date})</b>}
                </p>
              ))}
            </>
          )}
          {x.reschedules.length > 0 && (
            <>
              <p className="mb-1 mt-4 text-[12px] font-semibold text-navy">Reschedules</p>
              {x.reschedules.map((r, i) => (
                <p key={i} className="border-t border-line py-1 text-[12px]">
                  {formatDhaka(r.at)} — outstanding {formatPaisa(r.outstanding)} re-planned into{" "}
                  {r.new_count} installments. {r.reason}
                </p>
              ))}
            </>
          )}
        </div>
        <dl className="space-y-1 self-start rounded-lg bg-pearl p-3 text-[12px]">
          <div className="flex justify-between">
            <dt>Cash price</dt>
            <dd className="tabular-nums">{formatPaisa(x.cash_price)}</dd>
          </div>
          <div className="flex justify-between">
            <dt>Installment price</dt>
            <dd className="tabular-nums">{formatPaisa(x.price_total)}</dd>
          </div>
          <div className="flex justify-between">
            <dt>Down payment</dt>
            <dd className="tabular-nums">-{formatPaisa(a.down_payment)}</dd>
          </div>
          <div className="flex justify-between border-t border-line pt-1 font-semibold">
            <dt>Financed amount</dt>
            <dd className="tabular-nums">{formatPaisa(x.principal)}</dd>
          </div>
          <div className="flex justify-between">
            <dt>Markup ({x.markup_bp / 100}%)</dt>
            <dd className="tabular-nums">{formatPaisa(x.markup)}</dd>
          </div>
          {x.fees.map((f) => (
            <div key={f.label} className="flex justify-between">
              <dt>{f.label}</dt>
              <dd className="tabular-nums">{formatPaisa(f.amount)}</dd>
            </div>
          ))}
          {x.late_fees_total > 0 && (
            <div className="flex justify-between">
              <dt>Late fees</dt>
              <dd className="tabular-nums">{formatPaisa(x.late_fees_total)}</dd>
            </div>
          )}
          <div className="flex justify-between border-t border-line pt-1 font-semibold text-navy">
            <dt>Total payable</dt>
            <dd className="tabular-nums">{formatPaisa(a.total_payable)}</dd>
          </div>
          {x.rebates > 0 && (
            <div className="flex justify-between">
              <dt>Rebate granted</dt>
              <dd className="tabular-nums">-{formatPaisa(x.rebates)}</dd>
            </div>
          )}
          <div className="flex justify-between">
            <dt>Collected</dt>
            <dd className="tabular-nums">{formatPaisa(a.collected)}</dd>
          </div>
          <div className="metric-display flex justify-between border-t border-line pt-1 text-[16px] font-bold text-navy">
            <dt>Outstanding</dt>
            <dd className="tabular-nums">{formatPaisa(a.outstanding)}</dd>
          </div>
          <div className="flex justify-between text-ink-2">
            <dt>Markup earned / unearned</dt>
            <dd className="tabular-nums">
              {formatPaisa(x.earned_markup)} / {formatPaisa(x.unearned_markup)}
            </dd>
          </div>
          <p className="pt-1 text-[11px] text-ink-2">
            Allocation rule: a payment pays the chosen installment first, then the oldest unpaid
            installment. Nothing is added to this contract unless shown above.
          </p>
        </dl>
      </div>
      <div className="mt-4 flex flex-wrap justify-end gap-2">
        {active && canCollect && (
          <Button variant="primary" onClick={() => setSub({ collect: null })}>
            Collect payment
          </Button>
        )}
        {active && canCollect && <Button onClick={() => setSub("settle")}>Settle early</Button>}
        {active && can("installments.reschedule") && (
          <Button onClick={() => setSub("reschedule")}>Reschedule</Button>
        )}
        {canCollect && <Button onClick={() => setSub("followup")}>Add follow-up</Button>}
        <Button onClick={onClose}>Close</Button>
      </div>
      {sub && typeof sub === "object" && "collect" in sub && (
        <CollectModal detail={x} target={sub.collect} onClose={() => setSub(null)} onDone={after} />
      )}
      {sub === "settle" && <SettleModal detail={x} onClose={() => setSub(null)} onDone={after} />}
      {sub === "reschedule" && (
        <RescheduleModal detail={x} onClose={() => setSub(null)} onDone={after} />
      )}
      {sub === "followup" && (
        <FollowupModal id={a.id} onClose={() => setSub(null)} onDone={after} />
      )}
      {sub && typeof sub === "object" && "fee" in sub && (
        <ReasonModal
          title="Charge a late fee"
          action="Charge late fee"
          hint="Uses the late-fee rate set in Settings; once per installment; shown on the contract."
          onClose={() => setSub(null)}
          run={async (r) => {
            const f = await api.installmentLateFee(a.id, sub.fee, r);
            return `Late fee ${formatPaisa(f)} charged`;
          }}
          onDone={after}
        />
      )}
      {sub && typeof sub === "object" && "reverse" in sub && (
        <ReasonModal
          title="Reverse collection"
          action="Reverse"
          hint="The receipt stays on record; an opposite entry reopens the balance."
          onClose={() => setSub(null)}
          run={async (r) => {
            await api.installmentReverse(sub.reverse, r);
            return "Collection reversed";
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
  hint,
  run,
  onClose,
  onDone,
}: {
  title: string;
  action: string;
  hint: string;
  run: (reason: string) => Promise<string>;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const [reason, setReason] = useState("");
  const [err, setErr] = useState<string | null>(null);
  return (
    <Modal title={title} onClose={onClose} width={440}>
      <p className="mb-2 text-[12px] text-ink-2">{hint}</p>
      <Field label="Reason (audited)" error={err}>
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
              .then((m) => {
                toast("ok", m);
                onDone();
              })
              .catch((e) => setErr(toApiError(e).message))
          }
        >
          {action}
        </Button>
      </div>
    </Modal>
  );
}

function CollectModal({
  detail,
  target,
  onClose,
  onDone,
}: {
  detail: AgreementDetail;
  target: string | null;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const item = detail.items.find((i) => i.id === target);
  const next = detail.items.find((i) => !i.superseded && i.remaining > 0);
  const [amount, setAmount] = useState<number | null>((item ?? next)?.remaining ?? null);
  const [method, setMethod] = useState("cash");
  const [ref, setRef] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [key] = useState(newKey);
  async function go() {
    if (!amount) return setErr("Enter an amount");
    setBusy(true);
    try {
      const r = await api.installmentCollect({
        agreement_id: detail.agreement.id,
        amount,
        method,
        reference: ref,
        paid_on: null,
        target_schedule_id: target,
        client_key: key,
      });
      toast(
        "ok",
        `Receipt ${r.receipt}: ${formatPaisa(amount)} applied to installment ${r.allocations.map(([n, a]) => `#${n} (${formatPaisa(a)})`).join(", ")}${r.completed ? " — agreement completed" : ""}`,
      );
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title={`Collect payment: ${detail.agreement.number}`} onClose={onClose} width={460}>
      <p className="mb-3 text-[12px] text-ink-2">
        {item
          ? `Paid to installment #${item.seq} first, then the oldest unpaid ones.`
          : "Applied to the oldest unpaid installments first."}{" "}
        Outstanding: {formatPaisa(detail.agreement.outstanding)}.
      </p>
      <div className="space-y-3">
        <Field label="Amount (৳)" hint="Part-payments and advance payments are allowed">
          {(id) => <MoneyInput id={id} value={amount} onChange={setAmount} />}
        </Field>
        <Field label="Method">
          {(id) => (
            <Select id={id} value={method} onChange={(e) => setMethod(e.target.value)}>
              <option value="cash">Cash</option>
              <option value="bank">Bank</option>
              <option value="mobile_wallet">Mobile wallet</option>
              <option value="cheque">Cheque</option>
            </Select>
          )}
        </Field>
        <Field label="Reference" error={err}>
          {(id) => <Input id={id} value={ref} onChange={(e) => setRef(e.target.value)} />}
        </Field>
      </div>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" busy={busy} onClick={() => void go()}>
          Receive payment
        </Button>
      </div>
    </Modal>
  );
}

function SettleModal({
  detail,
  onClose,
  onDone,
}: {
  detail: AgreementDetail;
  onClose: () => void;
  onDone: () => void;
}) {
  const { can } = useAuth();
  const toast = useToast();
  const quote = useLoad(
    () => api.installmentSettleQuote(detail.agreement.id),
    [detail.agreement.id],
  );
  const [rebate, setRebate] = useState<number | null>(0);
  const [reason, setReason] = useState("");
  const [method, setMethod] = useState("cash");
  const [err, setErr] = useState<string | null>(null);
  const [key] = useState(newKey);
  const o = quote.data?.outstanding ?? detail.agreement.outstanding;
  const pay = o - (rebate ?? 0);
  async function go() {
    try {
      await api.installmentSettle({
        agreement_id: detail.agreement.id,
        rebate: rebate ?? 0,
        reason,
        method,
        reference: "",
        client_key: key,
      });
      toast("ok", `Agreement settled: ${formatPaisa(pay)} received`);
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal title={`Early settlement: ${detail.agreement.number}`} onClose={onClose} width={480}>
      <dl className="space-y-1 text-[13px]">
        <div className="flex justify-between">
          <dt>Outstanding</dt>
          <dd className="tabular-nums">{formatPaisa(o)}</dd>
        </div>
        {can("installments.reschedule") && (
          <div className="flex items-center justify-between gap-3">
            <dt>Rebate (waived markup/fees)</dt>
            <dd className="w-40">
              <MoneyInput value={rebate} onChange={setRebate} />
            </dd>
          </div>
        )}
        <div className="flex justify-between border-t border-line pt-1 font-semibold text-navy">
          <dt>Customer pays now</dt>
          <dd className="tabular-nums">{formatPaisa(pay)}</dd>
        </div>
      </dl>
      {can("installments.reschedule") && (
        <p className="mt-1 text-[11px] text-ink-2">
          Maximum rebate: {formatPaisa(quote.data?.max_rebate ?? 0)} (only unearned markup and fees
          can be waived).
        </p>
      )}
      <div className="mt-3 space-y-3">
        {(rebate ?? 0) > 0 && (
          <Field label="Reason for the rebate (audited)">
            {(id) => <Input id={id} value={reason} onChange={(e) => setReason(e.target.value)} />}
          </Field>
        )}
        <Field label="Method" error={err}>
          {(id) => (
            <Select id={id} value={method} onChange={(e) => setMethod(e.target.value)}>
              <option value="cash">Cash</option>
              <option value="bank">Bank</option>
              <option value="mobile_wallet">Mobile wallet</option>
              <option value="cheque">Cheque</option>
            </Select>
          )}
        </Field>
      </div>
      <p className="mt-2 text-[11px] text-ink-2">
        The agreement closes only when the balance is exactly zero.
      </p>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" onClick={() => void go()}>
          Settle and close
        </Button>
      </div>
    </Modal>
  );
}

function RescheduleModal({
  detail,
  onClose,
  onDone,
}: {
  detail: AgreementDetail;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const [count, setCount] = useState("4");
  const [freq, setFreq] = useState("monthly");
  const [first, setFirst] = useState(todayDhakaIso());
  const [reason, setReason] = useState("");
  const [err, setErr] = useState<string | null>(null);
  async function go() {
    try {
      await api.installmentReschedule({
        agreement_id: detail.agreement.id,
        count: Number(count),
        frequency: freq,
        first_due_date: first,
        custom: [],
        reason,
      });
      toast("ok", "Agreement rescheduled");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal title={`Reschedule ${detail.agreement.number}`} onClose={onClose} width={460}>
      <p className="mb-3 text-[12px] text-ink-2">
        The outstanding {formatPaisa(detail.agreement.outstanding)} is re-planned. Payments already
        made and past schedule rows stay on record.
      </p>
      <div className="grid grid-cols-2 gap-3">
        <Field label="Installments">
          {(id) => (
            <Input
              id={id}
              className="text-right"
              inputMode="numeric"
              value={count}
              onChange={(e) => setCount(e.target.value)}
            />
          )}
        </Field>
        <Field label="Frequency">
          {(id) => (
            <Select id={id} value={freq} onChange={(e) => setFreq(e.target.value)}>
              <option value="monthly">Monthly</option>
              <option value="weekly">Weekly</option>
            </Select>
          )}
        </Field>
        <Field label="First due date">
          {(id) => (
            <Input id={id} type="date" value={first} onChange={(e) => setFirst(e.target.value)} />
          )}
        </Field>
        <Field label="Reason (audited)" error={err}>
          {(id) => <Input id={id} value={reason} onChange={(e) => setReason(e.target.value)} />}
        </Field>
      </div>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" onClick={() => void go()}>
          Reschedule
        </Button>
      </div>
    </Modal>
  );
}

function FollowupModal({
  id,
  onClose,
  onDone,
}: {
  id: string;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const [note, setNote] = useState("");
  const [promise, setPromise] = useState("");
  const [err, setErr] = useState<string | null>(null);
  async function go() {
    try {
      await api.installmentFollowup(id, note, promise || null);
      toast("ok", "Follow-up recorded");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal title="Add follow-up" onClose={onClose} width={440}>
      <div className="space-y-3">
        <Field label="Note">
          {(fid) => (
            <Input id={fid} value={note} onChange={(e) => setNote(e.target.value)} autoFocus />
          )}
        </Field>
        <Field label="Customer promised to pay by (optional)" error={err}>
          {(fid) => (
            <Input
              id={fid}
              type="date"
              value={promise}
              onChange={(e) => setPromise(e.target.value)}
            />
          )}
        </Field>
      </div>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" onClick={() => void go()}>
          Save
        </Button>
      </div>
    </Modal>
  );
}

export type { AgreementRow };
