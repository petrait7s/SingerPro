import { useState } from "react";
import { AlertTriangle, CheckCircle2 } from "lucide-react";
import { api, toApiError, type ManualLine } from "../lib/api";
import { useAuth } from "../lib/auth";
import { useLoad } from "../lib/useLoad";
import { formatPaisa, todayDhakaIso } from "../lib/format";
import { Badge, Card } from "../ui/Card";
import { Button, Field, Input, Modal, Select, useToast } from "../ui/controls";
import { MoneyInput } from "../ui/MoneyInput";

type Tab = "accounts" | "cashbook" | "journal" | "trial" | "check";
const TABS: [Tab, string][] = [
  ["accounts", "Chart of accounts"],
  ["cashbook", "Cashbook & ledgers"],
  ["journal", "Journal"],
  ["trial", "Trial balance"],
  ["check", "Reconciliation"],
];

export function AccountsPage() {
  const [tab, setTab] = useState<Tab>("accounts");
  const [ledgerFor, setLedgerFor] = useState("1000");
  return (
    <div className="space-y-4">
      <div className="flex gap-1 border-b border-line">
        {TABS.map(([k, l]) => (
          <button
            key={k}
            onClick={() => setTab(k)}
            aria-current={tab === k}
            className={`-mb-px border-b-2 px-4 py-2 text-[13px] font-semibold ${tab === k ? "border-teal text-navy" : "border-transparent text-ink-2 hover:text-navy"}`}
          >
            {l}
          </button>
        ))}
      </div>
      {tab === "accounts" && (
        <Chart
          onLedger={(c) => {
            setLedgerFor(c);
            setTab("cashbook");
          }}
        />
      )}
      {tab === "cashbook" && <LedgerView code={ledgerFor} onCode={setLedgerFor} />}
      {tab === "journal" && <Journal />}
      {tab === "trial" && <Trial />}
      {tab === "check" && <Check />}
    </div>
  );
}

const TYPE_ORDER = ["asset", "liability", "equity", "income", "expense"];

function Chart({ onLedger }: { onLedger: (code: string) => void }) {
  const { can } = useAuth();
  const toast = useToast();
  const list = useLoad(() => api.accountsList(), []);
  const [opening, setOpening] = useState<string | null>(null);
  return (
    <Card>
      {list.error && (
        <p role="alert" className="text-danger">
          {list.error}
        </p>
      )}
      {TYPE_ORDER.map((t) => (
        <div key={t} className="mb-4">
          <p className="mb-1 text-[11px] font-semibold uppercase tracking-wider text-ink-2">{t}</p>
          <table className="w-full text-[12px]">
            <tbody>
              {list.data
                ?.filter((a) => a.account_type === t)
                .map((a) => (
                  <tr key={a.id} className="border-t border-line">
                    <td className="w-16 py-1.5 text-ink-2">{a.code}</td>
                    <td className="font-medium text-navy">{a.name}</td>
                    <td className="num w-40">{formatPaisa(a.balance)}</td>
                    <td className="w-48 text-right">
                      <Button variant="ghost" onClick={() => onLedger(a.code)}>
                        Ledger
                      </Button>
                      {can("accounts.manage") && ["1000", "1010", "1020"].includes(a.code) && (
                        <Button variant="ghost" onClick={() => setOpening(a.code)}>
                          Opening balance
                        </Button>
                      )}
                    </td>
                  </tr>
                ))}
            </tbody>
          </table>
        </div>
      ))}
      {opening && (
        <OpeningModal
          code={opening}
          onClose={() => setOpening(null)}
          onDone={() => {
            setOpening(null);
            list.reload();
            toast("ok", "Opening balance recorded");
          }}
        />
      )}
    </Card>
  );
}

function OpeningModal({
  code,
  onClose,
  onDone,
}: {
  code: string;
  onClose: () => void;
  onDone: () => void;
}) {
  const [amount, setAmount] = useState<number | null>(null);
  const [date, setDate] = useState(todayDhakaIso());
  const [err, setErr] = useState<string | null>(null);
  async function go() {
    try {
      await api.accountOpeningBalance(code, amount ?? 0, date);
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal title={`Opening balance: ${code}`} onClose={onClose} width={420}>
      <div className="space-y-3">
        <Field label="Amount (৳)">
          {(id) => <MoneyInput id={id} value={amount} onChange={setAmount} />}
        </Field>
        <Field label="As of" error={err}>
          {(id) => (
            <Input id={id} type="date" value={date} onChange={(e) => setDate(e.target.value)} />
          )}
        </Field>
      </div>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" onClick={() => void go()}>
          Record
        </Button>
      </div>
    </Modal>
  );
}

function LedgerView({ code, onCode }: { code: string; onCode: (c: string) => void }) {
  const accounts = useLoad(() => api.accountsList(), []);
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const led = useLoad(
    () => api.accountLedger(code, from || undefined, to || undefined),
    [code, from, to],
  );
  return (
    <Card
      title={led.data ? `${led.data.account} · ${led.data.name}` : "Ledger"}
      action={
        <div className="flex gap-2">
          <Select
            className="!w-56"
            value={code}
            onChange={(e) => onCode(e.target.value)}
            aria-label="Account"
          >
            {accounts.data?.map((a) => (
              <option key={a.code} value={a.code}>
                {a.code} {a.name}
              </option>
            ))}
          </Select>
          <Input
            className="!w-36"
            type="date"
            value={from}
            onChange={(e) => setFrom(e.target.value)}
            aria-label="From"
          />
          <Input
            className="!w-36"
            type="date"
            value={to}
            onChange={(e) => setTo(e.target.value)}
            aria-label="To"
          />
        </div>
      }
    >
      {led.error && (
        <p role="alert" className="text-danger">
          {led.error}
        </p>
      )}
      {led.data && (
        <table className="w-full text-[12px]">
          <thead>
            <tr className="text-left text-ink-2">
              <th className="pb-2">Date</th>
              <th>Entry</th>
              <th>Description</th>
              <th className="num">Debit</th>
              <th className="num">Credit</th>
              <th className="num">Balance</th>
            </tr>
          </thead>
          <tbody>
            <tr className="border-t border-line text-ink-2">
              <td colSpan={5} className="py-1.5">
                Opening balance
              </td>
              <td className="num">{formatPaisa(led.data.opening)}</td>
            </tr>
            {led.data.lines.map((l, i) => (
              <tr key={i} className="border-t border-line">
                <td className="py-1.5">{l.date}</td>
                <td>{l.number}</td>
                <td>{l.memo}</td>
                <td className="num">{l.debit ? formatPaisa(l.debit) : ""}</td>
                <td className="num">{l.credit ? formatPaisa(l.credit) : ""}</td>
                <td className="num">{formatPaisa(l.balance)}</td>
              </tr>
            ))}
            <tr className="border-t-2 border-navy font-semibold">
              <td colSpan={5} className="py-1.5">
                Closing balance
              </td>
              <td className="num">{formatPaisa(led.data.closing)}</td>
            </tr>
          </tbody>
        </table>
      )}
    </Card>
  );
}

function Journal() {
  const { can } = useAuth();
  const [search, setSearch] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const [manual, setManual] = useState(false);
  const list = useLoad(
    () => api.journalList({ search: search || undefined, limit: 100 }),
    [search],
  );
  return (
    <div className="space-y-3">
      <div className="flex gap-3">
        <Input
          className="!w-72"
          placeholder="Search description or JE no.…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label="Search journal"
        />
        {can("accounts.manage") && (
          <Button className="ml-auto" variant="primary" onClick={() => setManual(true)}>
            Manual journal
          </Button>
        )}
      </div>
      <Card>
        {list.error && (
          <p role="alert" className="text-danger">
            {list.error}
          </p>
        )}
        <table className="w-full text-[12px]">
          <thead>
            <tr className="text-left text-ink-2">
              <th className="pb-2">Entry</th>
              <th>Date</th>
              <th>Description</th>
              <th>Source</th>
              <th className="num">Amount</th>
            </tr>
          </thead>
          <tbody>
            {list.data?.rows.map((r) => (
              <tr
                key={r.id}
                className="cursor-pointer border-t border-line hover:bg-mint/30"
                onClick={() => setOpen(r.id)}
              >
                <td className="py-1.5 font-medium text-navy">{r.number}</td>
                <td>{r.entry_date}</td>
                <td>
                  {r.memo}
                  {r.reversed && (
                    <span className="ml-1">
                      <Badge tone="bad">reversed</Badge>
                    </span>
                  )}
                  {r.reverses_id && (
                    <span className="ml-1">
                      <Badge tone="neutral">reversal</Badge>
                    </span>
                  )}
                </td>
                <td>{r.source_type.replace("_", " ")}</td>
                <td className="num">{formatPaisa(r.total)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {list.data && (
          <p className="mt-2 text-[11px] text-ink-2">
            {list.data.total} entries. Posted entries are never edited; corrections are reversals.
          </p>
        )}
      </Card>
      {open && <EntryModal id={open} onClose={() => setOpen(null)} onChanged={list.reload} />}
      {manual && (
        <ManualModal
          onClose={() => setManual(false)}
          onDone={() => {
            setManual(false);
            list.reload();
          }}
        />
      )}
    </div>
  );
}

function EntryModal({
  id,
  onClose,
  onChanged,
}: {
  id: string;
  onClose: () => void;
  onChanged: () => void;
}) {
  const { can } = useAuth();
  const toast = useToast();
  const d = useLoad(() => api.journalGet(id), [id]);
  const [reason, setReason] = useState("");
  const [asking, setAsking] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  async function reverse() {
    try {
      await api.journalReverse(id, reason);
      toast("ok", "Entry reversed");
      setAsking(false);
      d.reload();
      onChanged();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  if (!d.data)
    return (
      <Modal title="Journal entry" onClose={onClose}>
        <p className="text-ink-2">{d.error ?? "Loading…"}</p>
      </Modal>
    );
  const e = d.data.entry;
  return (
    <Modal title={`${e.number} · ${e.entry_date}`} onClose={onClose} width={640}>
      <p className="mb-2 text-[13px]">{e.memo}</p>
      <table className="w-full text-[12px]">
        <thead>
          <tr className="text-left text-ink-2">
            <th>Account</th>
            <th>Memo</th>
            <th className="num">Debit</th>
            <th className="num">Credit</th>
          </tr>
        </thead>
        <tbody>
          {d.data.lines.map((l, i) => (
            <tr key={i} className="border-t border-line">
              <td className="py-1.5">
                {l.code} {l.name}
              </td>
              <td>{l.memo}</td>
              <td className="num">{l.debit ? formatPaisa(l.debit) : ""}</td>
              <td className="num">{l.credit ? formatPaisa(l.credit) : ""}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {asking && (
        <div className="mt-3">
          <Field label="Reason for reversal" error={err}>
            {(fid) => (
              <Input
                id={fid}
                value={reason}
                onChange={(x) => setReason(x.target.value)}
                autoFocus
              />
            )}
          </Field>
        </div>
      )}
      <div className="mt-4 flex justify-end gap-2">
        {can("accounts.manage") &&
          ["manual", "opening"].includes(e.source_type) &&
          !e.reversed &&
          !e.reverses_id &&
          (asking ? (
            <Button variant="danger" onClick={() => void reverse()}>
              Confirm reversal
            </Button>
          ) : (
            <Button variant="danger" onClick={() => setAsking(true)}>
              Reverse entry
            </Button>
          ))}
        <Button onClick={onClose}>Close</Button>
      </div>
    </Modal>
  );
}

function ManualModal({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const toast = useToast();
  const accounts = useLoad(() => api.accountsList(), []);
  const [date, setDate] = useState(todayDhakaIso());
  const [memo, setMemo] = useState("");
  const [lines, setLines] = useState<
    { code: string; debit: number | null; credit: number | null }[]
  >([
    { code: "1000", debit: null, credit: null },
    { code: "4900", debit: null, credit: null },
  ]);
  const [err, setErr] = useState<string | null>(null);
  const d = lines.reduce((a, l) => a + (l.debit ?? 0), 0);
  const c = lines.reduce((a, l) => a + (l.credit ?? 0), 0);
  async function go() {
    const body: ManualLine[] = lines
      .filter((l) => (l.debit ?? 0) > 0 || (l.credit ?? 0) > 0)
      .map((l) => ({ account_code: l.code, debit: l.debit ?? 0, credit: l.credit ?? 0, memo: "" }));
    try {
      await api.journalManual(date, memo, body);
      toast("ok", "Journal posted");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal title="Manual journal" onClose={onClose} width={700}>
      <div className="grid grid-cols-[160px_1fr] gap-3">
        <Field label="Date">
          {(id) => (
            <Input id={id} type="date" value={date} onChange={(e) => setDate(e.target.value)} />
          )}
        </Field>
        <Field label="Description">
          {(id) => <Input id={id} value={memo} onChange={(e) => setMemo(e.target.value)} />}
        </Field>
      </div>
      <table className="mt-3 w-full text-[12px]">
        <thead>
          <tr className="text-left text-ink-2">
            <th>Account</th>
            <th className="w-36 num">Debit</th>
            <th className="w-36 num">Credit</th>
          </tr>
        </thead>
        <tbody>
          {lines.map((l, i) => (
            <tr key={i}>
              <td>
                <Select
                  aria-label="Account"
                  value={l.code}
                  onChange={(e) =>
                    setLines(lines.map((x, j) => (j === i ? { ...x, code: e.target.value } : x)))
                  }
                >
                  {accounts.data
                    ?.filter((a) => a.is_active)
                    .map((a) => (
                      <option key={a.code} value={a.code}>
                        {a.code} {a.name}
                      </option>
                    ))}
                </Select>
              </td>
              <td>
                <MoneyInput
                  value={l.debit}
                  onChange={(v) =>
                    setLines(lines.map((x, j) => (j === i ? { ...x, debit: v } : x)))
                  }
                />
              </td>
              <td>
                <MoneyInput
                  value={l.credit}
                  onChange={(v) =>
                    setLines(lines.map((x, j) => (j === i ? { ...x, credit: v } : x)))
                  }
                />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="mt-2 flex items-center justify-between">
        <Button onClick={() => setLines([...lines, { code: "1000", debit: null, credit: null }])}>
          Add line
        </Button>
        <p className={`text-[12px] ${d === c && d > 0 ? "text-teal-deep" : "text-danger"}`}>
          Debits {formatPaisa(d)} · Credits {formatPaisa(c)}
          {d !== c && " — must balance"}
        </p>
      </div>
      {err && (
        <p role="alert" className="mt-2 text-[12px] text-danger">
          {err}
        </p>
      )}
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" disabled={d !== c || d === 0} onClick={() => void go()}>
          Post journal
        </Button>
      </div>
    </Modal>
  );
}

function Trial() {
  const [asOf, setAsOf] = useState("");
  const tb = useLoad(() => api.trialBalance(asOf || undefined), [asOf]);
  return (
    <Card
      title="Trial balance"
      action={
        <Input
          className="!w-40"
          type="date"
          value={asOf}
          onChange={(e) => setAsOf(e.target.value)}
          aria-label="As of date"
        />
      }
    >
      {tb.error && (
        <p role="alert" className="text-danger">
          {tb.error}
        </p>
      )}
      {tb.data && (
        <>
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th>Account</th>
                <th className="num">Debit</th>
                <th className="num">Credit</th>
              </tr>
            </thead>
            <tbody>
              {tb.data.rows.map((r) => {
                const net = r.debit - r.credit;
                return (
                  <tr key={r.code} className="border-t border-line">
                    <td className="py-1.5">
                      {r.code} {r.name}
                    </td>
                    <td className="num">{net > 0 ? formatPaisa(net) : ""}</td>
                    <td className="num">{net < 0 ? formatPaisa(-net) : ""}</td>
                  </tr>
                );
              })}
              <tr className="border-t-2 border-navy font-semibold">
                <td className="py-1.5">Total</td>
                <td className="num">{formatPaisa(tb.data.total_debit)}</td>
                <td className="num">{formatPaisa(tb.data.total_credit)}</td>
              </tr>
            </tbody>
          </table>
          <p
            className={`mt-2 flex items-center gap-2 text-[12px] ${tb.data.balanced ? "text-teal-deep" : "text-danger"}`}
          >
            {tb.data.balanced ? (
              <>
                <CheckCircle2 size={14} />
                Balanced
              </>
            ) : (
              <>
                <AlertTriangle size={14} />
                Out of balance
              </>
            )}
          </p>
        </>
      )}
    </Card>
  );
}

function Check() {
  const r = useLoad(() => api.accountingReconcile(), []);
  return (
    <Card title="Books reconciliation" action={<Button onClick={r.reload}>Run again</Button>}>
      {r.error && (
        <p role="alert" className="text-danger">
          {r.error}
        </p>
      )}
      {r.data && r.data.length === 0 && (
        <p className="flex items-center gap-2 text-[13px] text-teal-deep">
          <CheckCircle2 size={16} />
          Journals balance; customer, supplier, stock and tax accounts agree with their sub-ledgers.
        </p>
      )}
      {r.data && r.data.length > 0 && (
        <>
          <p className="mb-2 flex items-center gap-2 text-[13px] text-danger">
            <AlertTriangle size={16} />
            {r.data.length} issue(s) found.
          </p>
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th>Check</th>
                <th>Detail</th>
                <th className="num">Expected</th>
                <th className="num">Actual</th>
              </tr>
            </thead>
            <tbody>
              {r.data.map((m, i) => (
                <tr key={i} className="border-t border-line">
                  <td className="py-1.5">{m.check.replace(/_/g, " ")}</td>
                  <td>{m.detail}</td>
                  <td className="num">{formatPaisa(m.expected)}</td>
                  <td className="num">{formatPaisa(m.actual)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </Card>
  );
}
