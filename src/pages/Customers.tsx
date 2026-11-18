import { useState } from "react";
import { Plus } from "lucide-react";
import { api, toApiError, type CustomerIn, type CustomerRow } from "../lib/api";
import { useAuth } from "../lib/auth";
import { useLoad } from "../lib/useLoad";
import { formatDhaka, formatPaisa, newKey, todayDhakaIso } from "../lib/format";
import { Badge, Card, EmptyState } from "../ui/Card";
import { Button, Field, Input, Modal, Select, Toggle, useToast } from "../ui/controls";
import { MoneyInput } from "../ui/MoneyInput";
import { SaleModal } from "./Sales";

const blank: CustomerIn = {
  name: "",
  phone: "",
  alt_phone: "",
  address: "",
  customer_type: "retail",
  credit_limit: 0,
  nid: "",
  notes: "",
};

export function CustomersPage() {
  const { can } = useAuth();
  const canManage = can("customers.manage");
  const [search, setSearch] = useState("");
  const [type, setType] = useState("");
  const [owing, setOwing] = useState(false);
  const [inactive, setInactive] = useState(false);
  const [modal, setModal] = useState<null | "new" | { edit: CustomerRow } | { view: CustomerRow }>(
    null,
  );
  const list = useLoad(
    () =>
      api.customersList({
        search: search || undefined,
        customerType: type || undefined,
        includeInactive: inactive,
        withBalanceOnly: owing,
      }),
    [search, type, owing, inactive],
  );
  const done = () => {
    setModal(null);
    list.reload();
  };
  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-3">
        <Input
          className="!w-72"
          placeholder="Search name, phone, customer no.…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label="Search customers"
        />
        <Select
          className="!w-40"
          value={type}
          onChange={(e) => setType(e.target.value)}
          aria-label="Customer type"
        >
          <option value="">All types</option>
          <option value="retail">Retail</option>
          <option value="wholesale">Wholesale</option>
          <option value="dealer">Dealer</option>
        </Select>
        <Toggle label="Owing money" checked={owing} onChange={setOwing} />
        <Toggle label="Show inactive" checked={inactive} onChange={setInactive} />
        {canManage && (
          <Button className="ml-auto" variant="primary" onClick={() => setModal("new")}>
            <Plus size={14} />
            New customer
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
            title="No customers yet"
            hint="Add customers so sales, payments and installments build a complete history."
          />
        ) : (
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th className="pb-2">No.</th>
                <th>Name</th>
                <th>Phone</th>
                <th>Type</th>
                <th className="num">Credit limit</th>
                <th className="num">Owes us</th>
                <th>Status</th>
              </tr>
            </thead>
            <tbody>
              {list.data?.map((c) => (
                <tr
                  key={c.id}
                  className="cursor-pointer border-t border-line hover:bg-mint/30"
                  onClick={() => setModal({ view: c })}
                >
                  <td className="py-2">{c.code}</td>
                  <td className="font-medium text-navy">{c.name}</td>
                  <td>{c.phone}</td>
                  <td>{c.customer_type}</td>
                  <td className="num">{c.credit_limit ? formatPaisa(c.credit_limit) : "—"}</td>
                  <td className={`num ${c.balance > 0 ? "font-semibold text-warn" : ""}`}>
                    {formatPaisa(c.balance)}
                  </td>
                  <td>
                    {c.is_active ? (
                      <Badge tone="ok">Active</Badge>
                    ) : (
                      <Badge tone="neutral">Inactive</Badge>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
      {modal === "new" && <CustomerModal onClose={() => setModal(null)} onDone={done} />}
      {modal && typeof modal === "object" && "edit" in modal && (
        <CustomerModal customer={modal.edit} onClose={() => setModal(null)} onDone={done} />
      )}
      {modal && typeof modal === "object" && "view" in modal && (
        <Customer360
          customer={modal.view}
          onClose={() => setModal(null)}
          onEdit={() => setModal({ edit: (modal as { view: CustomerRow }).view })}
          onChanged={list.reload}
        />
      )}
    </div>
  );
}

function CustomerModal({
  customer,
  onClose,
  onDone,
}: {
  customer?: CustomerRow;
  onClose: () => void;
  onDone: () => void;
}) {
  const { can } = useAuth();
  const toast = useToast();
  const [f, setF] = useState<CustomerIn>(
    customer
      ? {
          name: customer.name,
          phone: customer.phone,
          alt_phone: customer.alt_phone,
          address: customer.address,
          customer_type: customer.customer_type,
          credit_limit: customer.credit_limit,
          nid: customer.nid,
          notes: customer.notes,
        }
      : blank,
  );
  const [err, setErr] = useState<string | null>(null);
  const set = <K extends keyof CustomerIn>(k: K, v: CustomerIn[K]) =>
    setF((x) => ({ ...x, [k]: v }));
  async function save() {
    try {
      if (customer) await api.customerUpdate(customer.id, f);
      else await api.customerCreate(f);
      toast("ok", "Customer saved");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  const t = (k: "name" | "phone" | "alt_phone" | "address" | "notes", label: string) => (
    <Field label={label}>
      {(id) => <Input id={id} value={f[k]} onChange={(e) => set(k, e.target.value)} />}
    </Field>
  );
  return (
    <Modal
      title={customer ? `Edit ${customer.name}` : "New customer"}
      onClose={onClose}
      width={560}
    >
      <div className="grid grid-cols-2 gap-3">
        <div className="col-span-2">{t("name", "Full name")}</div>
        {t("phone", "Phone")}
        {t("alt_phone", "Alternative phone")}
        <div className="col-span-2">{t("address", "Address")}</div>
        <Field label="Customer type">
          {(id) => (
            <Select
              id={id}
              value={f.customer_type}
              onChange={(e) => set("customer_type", e.target.value)}
            >
              <option value="retail">Retail</option>
              <option value="wholesale">Wholesale</option>
              <option value="dealer">Dealer / business</option>
            </Select>
          )}
        </Field>
        <Field
          label="Credit limit (৳)"
          hint={
            can("sales.credit_override")
              ? "0 = no credit"
              : "Changing the limit needs the credit-override permission"
          }
        >
          {(id) => (
            <MoneyInput
              id={id}
              value={f.credit_limit}
              onChange={(v) => set("credit_limit", v ?? 0)}
              disabled={!can("sales.credit_override") && !!customer}
            />
          )}
        </Field>
        <Field label="National ID (optional)" hint="Only if enabled in Settings">
          {(id) => <Input id={id} value={f.nid} onChange={(e) => set("nid", e.target.value)} />}
        </Field>
        <div className="col-span-2">{t("notes", "Notes")}</div>
      </div>
      {err && (
        <p role="alert" className="mt-3 text-[12px] text-danger">
          {err}
        </p>
      )}
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" onClick={() => void save()}>
          Save
        </Button>
      </div>
    </Modal>
  );
}

function Customer360({
  customer,
  onClose,
  onEdit,
  onChanged,
}: {
  customer: CustomerRow;
  onClose: () => void;
  onEdit: () => void;
  onChanged: () => void;
}) {
  const { can } = useAuth();
  const toast = useToast();
  const st = useLoad(() => api.customerStatement(customer.id), [customer.id]);
  const sales = useLoad(
    () => api.salesList({ customer_id: customer.id, limit: 100 }),
    [customer.id],
  );
  const [sub, setSub] = useState<null | "collect" | "opening" | { sale: string }>(null);
  const refresh = () => {
    st.reload();
    sales.reload();
    onChanged();
  };
  const bal = st.data?.closing_balance ?? customer.balance;
  return (
    <Modal title={`${customer.name} · ${customer.code}`} onClose={onClose} width={900}>
      <div className="grid grid-cols-4 gap-3 text-[12px]">
        <div>
          <p className="text-ink-2">Phone</p>
          <p className="font-medium">{customer.phone || "—"}</p>
        </div>
        <div>
          <p className="text-ink-2">Type</p>
          <p className="font-medium">{customer.customer_type}</p>
        </div>
        <div>
          <p className="text-ink-2">Credit limit</p>
          <p className="font-medium">
            {customer.credit_limit ? formatPaisa(customer.credit_limit) : "None"}
          </p>
        </div>
        <div>
          <p className="text-ink-2">Owes us</p>
          <p
            className={`metric-display text-[18px] font-bold ${bal > 0 ? "text-warn" : "text-navy"}`}
          >
            {formatPaisa(bal)}
          </p>
        </div>
      </div>
      {customer.address && <p className="mt-2 text-[12px] text-ink-2">{customer.address}</p>}
      <div className="mt-3 flex gap-2">
        {can("customers.collect_payment") && bal > 0 && (
          <Button variant="primary" onClick={() => setSub("collect")}>
            Collect payment
          </Button>
        )}
        {can("accounts.manage") && (
          <Button onClick={() => setSub("opening")}>Opening balance</Button>
        )}
        {can("customers.manage") && <Button onClick={onEdit}>Edit</Button>}
      </div>
      <div className="mt-4 grid grid-cols-2 gap-4">
        <div>
          <p className="mb-1 text-[12px] font-semibold text-navy">Invoices</p>
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th>No.</th>
                <th>Date</th>
                <th className="num">Total</th>
                <th className="num">Due</th>
              </tr>
            </thead>
            <tbody>
              {sales.data?.rows.map((r) => (
                <tr
                  key={r.id}
                  className="cursor-pointer border-t border-line hover:bg-mint/30"
                  onClick={() => setSub({ sale: r.id })}
                >
                  <td className="py-1.5 text-navy">{r.number}</td>
                  <td>{r.sale_date}</td>
                  <td className="num">{formatPaisa(r.grand_total)}</td>
                  <td className="num">{r.due > 0 ? formatPaisa(r.due) : "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {sales.data?.rows.length === 0 && (
            <p className="text-[12px] text-ink-2">No purchases yet.</p>
          )}
        </div>
        <div>
          <p className="mb-1 text-[12px] font-semibold text-navy">Ledger</p>
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th>When</th>
                <th>Type</th>
                <th className="num">Amount</th>
                <th className="num">Balance</th>
              </tr>
            </thead>
            <tbody>
              {st.data?.lines.map((l, i) => (
                <tr key={i} className="border-t border-line">
                  <td className="py-1.5">{formatDhaka(l.at)}</td>
                  <td>{l.kind.replace("_", " ")}</td>
                  <td className="num">{formatPaisa(l.amount)}</td>
                  <td className="num">{formatPaisa(l.balance)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
      <div className="mt-4 flex justify-end">
        <Button onClick={onClose}>Close</Button>
      </div>
      {sub === "collect" && (
        <CollectModal
          customer={customer}
          due={bal}
          onClose={() => setSub(null)}
          onDone={() => {
            setSub(null);
            refresh();
          }}
        />
      )}
      {sub === "opening" && (
        <OpeningModal
          customer={customer}
          onClose={() => setSub(null)}
          onDone={() => {
            setSub(null);
            refresh();
            toast("ok", "Opening balance recorded");
          }}
        />
      )}
      {sub && typeof sub === "object" && (
        <SaleModal saleId={sub.sale} onClose={() => setSub(null)} onChanged={refresh} />
      )}
    </Modal>
  );
}

function CollectModal({
  customer,
  due,
  onClose,
  onDone,
}: {
  customer: CustomerRow;
  due: number;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const [amount, setAmount] = useState<number | null>(due);
  const [method, setMethod] = useState("cash");
  const [ref, setRef] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [key] = useState(newKey);
  async function go() {
    if (!amount) return setErr("Enter an amount");
    setBusy(true);
    try {
      const r = await api.customerCollect({
        customer_id: customer.id,
        sale_id: null,
        amount,
        method,
        reference: ref,
        paid_on: null,
        client_key: key,
      });
      toast("ok", `Received ${formatPaisa(amount)} — receipt ${r.receipts.join(", ")}`);
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title={`Collect payment: ${customer.name}`} onClose={onClose} width={440}>
      <p className="mb-3 text-[12px] text-ink-2">
        Applied to the oldest unpaid invoices first. A receipt is issued per invoice.
      </p>
      <div className="space-y-3">
        <Field label="Amount (৳)" hint={`Outstanding: ${formatPaisa(due)}`}>
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

function OpeningModal({
  customer,
  onClose,
  onDone,
}: {
  customer: CustomerRow;
  onClose: () => void;
  onDone: () => void;
}) {
  const [amount, setAmount] = useState<number | null>(null);
  const [date, setDate] = useState(todayDhakaIso());
  const [err, setErr] = useState<string | null>(null);
  async function go() {
    try {
      await api.customerOpeningBalance(customer.id, amount ?? 0, date);
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal title={`Opening balance: ${customer.name}`} onClose={onClose} width={420}>
      <p className="mb-3 text-[12px] text-ink-2">
        What this customer already owed before using Singer Pro. Can be recorded once.
      </p>
      <div className="space-y-3">
        <Field label="Amount owed (৳)">
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
