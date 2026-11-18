import { useState } from "react";
import { Plus } from "lucide-react";
import { api, toApiError, type Statement, type SupplierIn, type SupplierRow } from "../lib/api";
import { useAuth } from "../lib/auth";
import { useLoad } from "../lib/useLoad";
import { formatDhaka, formatPaisa, newKey, todayDhakaIso } from "../lib/format";
import { Badge, Card, EmptyState } from "../ui/Card";
import { Button, Field, Input, Modal, Select, Toggle, useToast } from "../ui/controls";
import { MoneyInput } from "../ui/MoneyInput";

const blank: SupplierIn = {
  name: "",
  contact_person: "",
  phone: "",
  alt_phone: "",
  email: "",
  address: "",
  notes: "",
};

export function SuppliersPage() {
  const { can } = useAuth();
  const canManage = can("suppliers.manage");
  const canPay = can("purchases.pay");
  const toast = useToast();
  const [search, setSearch] = useState("");
  const [inactive, setInactive] = useState(false);
  const [modal, setModal] = useState<
    null | "new" | { edit: SupplierRow } | { statement: SupplierRow } | { pay: SupplierRow }
  >(null);
  const list = useLoad(() => api.suppliersList(search || undefined, inactive), [search, inactive]);

  async function setActive(s: SupplierRow, a: boolean) {
    try {
      await api.supplierSetActive(s.id, a);
      toast("ok", `${s.name}: ${a ? "activated" : "deactivated"}`);
      list.reload();
    } catch (e) {
      toast("bad", toApiError(e).message);
    }
  }
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <Input
          className="!w-72"
          placeholder="Search suppliers…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label="Search suppliers"
        />
        <Toggle label="Show inactive" checked={inactive} onChange={setInactive} />
        {canManage && (
          <Button className="ml-auto" variant="primary" onClick={() => setModal("new")}>
            <Plus size={14} />
            New supplier
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
            title="No suppliers yet"
            hint="Add the companies you buy from, then record purchases against them."
          />
        ) : (
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th className="pb-2">Supplier</th>
                <th>Contact</th>
                <th>Phone</th>
                <th className="num">We owe</th>
                <th>Status</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {list.data?.map((s) => (
                <tr key={s.id} className="border-t border-line">
                  <td className="py-2 font-medium text-navy">{s.name}</td>
                  <td>{s.contact_person}</td>
                  <td>{s.phone}</td>
                  <td className={`num ${s.balance > 0 ? "font-semibold text-warn" : ""}`}>
                    {formatPaisa(s.balance)}
                  </td>
                  <td>
                    {s.is_active ? (
                      <Badge tone="ok">Active</Badge>
                    ) : (
                      <Badge tone="neutral">Inactive</Badge>
                    )}
                  </td>
                  <td className="space-x-1 text-right">
                    <Button variant="ghost" onClick={() => setModal({ statement: s })}>
                      Statement
                    </Button>
                    {canPay && s.is_active && (
                      <Button variant="ghost" onClick={() => setModal({ pay: s })}>
                        Pay
                      </Button>
                    )}
                    {canManage && (
                      <Button variant="ghost" onClick={() => setModal({ edit: s })}>
                        Edit
                      </Button>
                    )}
                    {canManage && (
                      <Button variant="ghost" onClick={() => void setActive(s, !s.is_active)}>
                        {s.is_active ? "Deactivate" : "Reactivate"}
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
        <SupplierModal
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            list.reload();
          }}
        />
      )}
      {modal && typeof modal === "object" && "edit" in modal && (
        <SupplierModal
          supplier={modal.edit}
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            list.reload();
          }}
        />
      )}
      {modal && typeof modal === "object" && "statement" in modal && (
        <StatementModal supplier={modal.statement} onClose={() => setModal(null)} />
      )}
      {modal && typeof modal === "object" && "pay" in modal && (
        <PayModal
          supplier={modal.pay}
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

function SupplierModal({
  supplier,
  onClose,
  onDone,
}: {
  supplier?: SupplierRow;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const [f, setF] = useState<SupplierIn>(
    supplier
      ? {
          name: supplier.name,
          contact_person: supplier.contact_person,
          phone: supplier.phone,
          alt_phone: supplier.alt_phone,
          email: supplier.email,
          address: supplier.address,
          notes: supplier.notes,
        }
      : blank,
  );
  const [err, setErr] = useState<string | null>(null);
  const set = (k: keyof SupplierIn, v: string) => setF((x) => ({ ...x, [k]: v }));
  async function save() {
    try {
      if (supplier) await api.supplierUpdate(supplier.id, f);
      else await api.supplierCreate(f);
      toast("ok", "Supplier saved");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  const field = (k: keyof SupplierIn, label: string) => (
    <Field label={label}>
      {(id) => <Input id={id} value={f[k]} onChange={(e) => set(k, e.target.value)} />}
    </Field>
  );
  return (
    <Modal title={supplier ? "Edit supplier" : "New supplier"} onClose={onClose} width={560}>
      <div className="grid grid-cols-2 gap-3">
        <div className="col-span-2">{field("name", "Supplier name")}</div>
        {field("contact_person", "Contact person")}
        {field("phone", "Phone")}
        {field("alt_phone", "Alternative phone")}
        {field("email", "Email")}
        <div className="col-span-2">{field("address", "Address")}</div>
        <div className="col-span-2">{field("notes", "Notes")}</div>
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

function StatementModal({ supplier, onClose }: { supplier: SupplierRow; onClose: () => void }) {
  const st = useLoad<Statement>(() => api.supplierStatement(supplier.id), [supplier.id]);
  return (
    <Modal title={`Statement: ${supplier.name}`} onClose={onClose} width={720}>
      {st.error && (
        <p role="alert" className="text-[12px] text-danger">
          {st.error}
        </p>
      )}
      {st.data && (
        <>
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th className="pb-2">When (Dhaka)</th>
                <th>Type</th>
                <th>Note</th>
                <th className="num">Amount</th>
                <th className="num">Balance</th>
              </tr>
            </thead>
            <tbody>
              {st.data.lines.map((l, i) => (
                <tr key={i} className="border-t border-line">
                  <td className="py-1.5">{formatDhaka(l.at)}</td>
                  <td>{l.kind.replace("_", " ")}</td>
                  <td>{l.note}</td>
                  <td className="num">{formatPaisa(l.amount)}</td>
                  <td className="num">{formatPaisa(l.balance)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {st.data.lines.length === 0 && (
            <p className="text-[13px] text-ink-2">No transactions yet.</p>
          )}
          <p className="mt-3 text-right text-[13px] font-semibold text-navy">
            Closing balance (we owe): {formatPaisa(st.data.closing_balance)}
          </p>
        </>
      )}
      <div className="mt-4 flex justify-end">
        <Button onClick={onClose}>Close</Button>
      </div>
    </Modal>
  );
}

export function PayModal({
  supplier,
  purchaseId,
  maxAmount,
  onClose,
  onDone,
}: {
  supplier: { id: string; name: string };
  purchaseId?: string;
  maxAmount?: number;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const [amount, setAmount] = useState<number | null>(maxAmount ?? null);
  const [method, setMethod] = useState("cash");
  const [date, setDate] = useState(todayDhakaIso());
  const [ref, setRef] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [key] = useState(newKey); // one key per opened form: a double click can never post twice
  async function pay() {
    if (!amount || amount <= 0) return setErr("Enter an amount");
    setBusy(true);
    try {
      await api.supplierPay({
        supplier_id: supplier.id,
        purchase_id: purchaseId ?? null,
        amount,
        method,
        paid_on: date,
        reference: ref,
        note: "",
        client_key: key,
      });
      toast("ok", `Payment of ${formatPaisa(amount)} recorded`);
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title={`Pay ${supplier.name}`} onClose={onClose} width={440}>
      <div className="space-y-3">
        <Field
          label="Amount (৳)"
          hint={
            maxAmount !== undefined
              ? `Amount due: ${formatPaisa(maxAmount)}`
              : "Payment on account (reduces what we owe)"
          }
        >
          {(id) => <MoneyInput id={id} value={amount} onChange={setAmount} />}
        </Field>
        <Field label="Method">
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
        <Field label="Date">
          {(id) => (
            <Input id={id} type="date" value={date} onChange={(e) => setDate(e.target.value)} />
          )}
        </Field>
        <Field label="Reference (cheque / transaction no.)" error={err}>
          {(id) => <Input id={id} value={ref} onChange={(e) => setRef(e.target.value)} />}
        </Field>
      </div>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" busy={busy} onClick={() => void pay()}>
          Record payment
        </Button>
      </div>
    </Modal>
  );
}
