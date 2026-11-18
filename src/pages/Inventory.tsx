import { useState } from "react";
import { AlertTriangle, CheckCircle2 } from "lucide-react";
import { api, toApiError, type SerialRow, type StockRow } from "../lib/api";
import { useAuth } from "../lib/auth";
import { useLoad } from "../lib/useLoad";
import { formatDhaka, formatPaisa } from "../lib/format";
import { Badge, Card, EmptyState } from "../ui/Card";
import { Button, Field, Input, Modal, Select, Toggle, useToast } from "../ui/controls";
import { MoneyInput } from "../ui/MoneyInput";

type Tab = "levels" | "serials" | "reconcile";
const BUCKETS = [
  ["available", "Available"],
  ["reserved", "Reserved"],
  ["damaged", "Damaged"],
  ["warranty", "Warranty / service"],
  ["pending_inspection", "Awaiting inspection"],
] as const;
const MANUAL_TARGETS: Record<string, string[]> = {
  available: ["reserved", "damaged", "warranty_service", "written_off"],
  reserved: ["available"],
  damaged: ["available", "warranty_service", "written_off"],
  warranty_service: ["available", "damaged", "written_off"],
};

export function InventoryPage() {
  const [tab, setTab] = useState<Tab>("levels");
  return (
    <div className="space-y-4">
      <div className="flex gap-1 border-b border-line">
        {(
          [
            ["levels", "Stock levels"],
            ["serials", "Serial numbers"],
            ["reconcile", "Reconciliation"],
          ] as [Tab, string][]
        ).map(([k, l]) => (
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
      {tab === "levels" && <Levels />}
      {tab === "serials" && <Serials />}
      {tab === "reconcile" && <Reconcile />}
    </div>
  );
}

function Levels() {
  const { can } = useAuth();
  const canAdjust = can("inventory.adjust");
  const [search, setSearch] = useState("");
  const [low, setLow] = useState(false);
  const [inStock, setInStock] = useState(false);
  const [modal, setModal] = useState<
    null | { moves: StockRow } | { adjust: StockRow } | { move: StockRow } | { opening: StockRow }
  >(null);
  const list = useLoad(
    () =>
      api.stockLevels({
        search: search || undefined,
        low_only: low,
        in_stock_only: inStock,
        limit: 200,
      }),
    [search, low, inStock],
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
          placeholder="Search product, SKU, barcode…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label="Search stock"
        />
        <Toggle label="Low stock only" checked={low} onChange={setLow} />
        <Toggle label="Only items on hand" checked={inStock} onChange={setInStock} />
        {list.data && (
          <span className="ml-auto text-[13px] text-ink-2">
            Stock value:{" "}
            <b className="text-navy tabular-nums">{formatPaisa(list.data.total_value)}</b>
          </span>
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
            title="No stock to show"
            hint="Receive a purchase or record opening stock for a product variant."
          />
        ) : (
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th className="pb-2">Product</th>
                <th>SKU</th>
                <th className="num">Available</th>
                <th className="num">Reserved</th>
                <th className="num">Damaged</th>
                <th className="num">Service</th>
                <th className="num">Inspect</th>
                <th className="num">Avg cost</th>
                <th className="num">Value</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {list.data?.rows.map((r) => (
                <tr key={r.variant_id} className="border-t border-line">
                  <td className="py-2 font-medium text-navy">
                    {r.label}
                    {r.is_serialized && (
                      <span className="ml-1">
                        <Badge tone="ok">serial</Badge>
                      </span>
                    )}
                    {r.low_stock && (
                      <span className="ml-1">
                        <Badge tone="bad">low</Badge>
                      </span>
                    )}
                  </td>
                  <td>{r.sku}</td>
                  <td className="num font-semibold">{r.available}</td>
                  <td className="num">{r.reserved}</td>
                  <td className="num">{r.damaged}</td>
                  <td className="num">{r.warranty}</td>
                  <td className="num">{r.pending_inspection}</td>
                  <td className="num">{formatPaisa(r.avg_cost)}</td>
                  <td className="num">{formatPaisa(r.value)}</td>
                  <td className="space-x-1 text-right whitespace-nowrap">
                    <Button variant="ghost" onClick={() => setModal({ moves: r })}>
                      History
                    </Button>
                    {canAdjust && (
                      <Button variant="ghost" onClick={() => setModal({ opening: r })}>
                        Opening
                      </Button>
                    )}
                    {canAdjust && !r.is_serialized && (
                      <>
                        <Button variant="ghost" onClick={() => setModal({ adjust: r })}>
                          Adjust
                        </Button>
                        <Button variant="ghost" onClick={() => setModal({ move: r })}>
                          Move
                        </Button>
                      </>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
      {modal && "moves" in modal && <MovesModal row={modal.moves} onClose={() => setModal(null)} />}
      {modal && "adjust" in modal && (
        <AdjustModal row={modal.adjust} onClose={() => setModal(null)} onDone={done} />
      )}
      {modal && "move" in modal && (
        <MoveModal row={modal.move} onClose={() => setModal(null)} onDone={done} />
      )}
      {modal && "opening" in modal && (
        <OpeningModal row={modal.opening} onClose={() => setModal(null)} onDone={done} />
      )}
    </div>
  );
}

function MovesModal({ row, onClose }: { row: StockRow; onClose: () => void }) {
  const m = useLoad(() => api.stockMovements(row.variant_id, 200, 0), [row.variant_id]);
  return (
    <Modal title={`Stock history: ${row.label}`} onClose={onClose} width={900}>
      {m.error && (
        <p role="alert" className="text-danger">
          {m.error}
        </p>
      )}
      <table className="w-full text-[12px]">
        <thead>
          <tr className="text-left text-ink-2">
            <th className="pb-2">When (Dhaka)</th>
            <th>Bucket</th>
            <th className="num">Qty</th>
            <th className="num">Balance</th>
            <th className="num">Cost</th>
            <th>Document</th>
            <th>Serial</th>
            <th>Reason</th>
            <th>By</th>
          </tr>
        </thead>
        <tbody>
          {m.data?.map((x) => (
            <tr key={x.id} className="border-t border-line">
              <td className="py-1.5 whitespace-nowrap">{formatDhaka(x.at)}</td>
              <td>{x.bucket}</td>
              <td className={`num ${x.qty_delta < 0 ? "text-danger" : "text-teal-deep"}`}>
                {x.qty_delta > 0 ? "+" : ""}
                {x.qty_delta}
              </td>
              <td className="num">{x.balance_after}</td>
              <td className="num">{formatPaisa(x.cost_total)}</td>
              <td>{x.doc_type.replace("_", " ")}</td>
              <td>{x.serial_no ?? ""}</td>
              <td>{x.reason}</td>
              <td>{x.actor}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {m.data?.length === 0 && <p className="text-[13px] text-ink-2">No movements yet.</p>}
      <div className="mt-4 flex justify-end">
        <Button onClick={onClose}>Close</Button>
      </div>
    </Modal>
  );
}

function AdjustModal({
  row,
  onClose,
  onDone,
}: {
  row: StockRow;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const [bucket, setBucket] = useState("available");
  const [dir, setDir] = useState<"add" | "remove">("remove");
  const [qty, setQty] = useState("");
  const [cost, setCost] = useState<number | null>(row.avg_cost);
  const [reason, setReason] = useState("");
  const [err, setErr] = useState<string | null>(null);
  async function go() {
    const n = /^\d+$/.test(qty) ? Number(qty) : 0;
    if (n <= 0) return setErr("Enter a whole quantity");
    try {
      await api.stockAdjust({
        variant_id: row.variant_id,
        bucket,
        qty_delta: dir === "add" ? n : -n,
        unit_cost: dir === "add" ? cost : null,
        reason,
      });
      toast("ok", "Adjustment recorded");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal title={`Adjust stock: ${row.label}`} onClose={onClose} width={460}>
      <div className="space-y-3">
        <div className="grid grid-cols-2 gap-3">
          <Field label="Bucket">
            {(id) => (
              <Select id={id} value={bucket} onChange={(e) => setBucket(e.target.value)}>
                {BUCKETS.map(([k, l]) => (
                  <option key={k} value={k}>
                    {l}
                  </option>
                ))}
              </Select>
            )}
          </Field>
          <Field label="Direction">
            {(id) => (
              <Select
                id={id}
                value={dir}
                onChange={(e) => setDir(e.target.value as "add" | "remove")}
              >
                <option value="remove">Remove (write-off / count loss)</option>
                <option value="add">Add (count gain)</option>
              </Select>
            )}
          </Field>
        </div>
        <div className="grid grid-cols-2 gap-3">
          <Field label="Quantity">
            {(id) => (
              <Input
                id={id}
                className="text-right"
                inputMode="numeric"
                value={qty}
                onChange={(e) => setQty(e.target.value)}
              />
            )}
          </Field>
          {dir === "add" ? (
            <Field label="Unit cost (৳)">
              {(id) => <MoneyInput id={id} value={cost} onChange={setCost} />}
            </Field>
          ) : (
            <p className="self-end pb-2 text-[11px] text-ink-2">
              Removed at average cost {formatPaisa(row.avg_cost)}
            </p>
          )}
        </div>
        <Field label="Reason (required)" error={err}>
          {(id) => <Input id={id} value={reason} onChange={(e) => setReason(e.target.value)} />}
        </Field>
      </div>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" onClick={() => void go()}>
          Record adjustment
        </Button>
      </div>
    </Modal>
  );
}

function MoveModal({
  row,
  onClose,
  onDone,
}: {
  row: StockRow;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const [from, setFrom] = useState("available");
  const [to, setTo] = useState("damaged");
  const [qty, setQty] = useState("");
  const [reason, setReason] = useState("");
  const [err, setErr] = useState<string | null>(null);
  async function go() {
    try {
      await api.stockMove({
        variant_id: row.variant_id,
        from_bucket: from,
        to_bucket: to,
        qty: /^\d+$/.test(qty) ? Number(qty) : 0,
        reason,
      });
      toast("ok", "Stock moved");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal title={`Move stock: ${row.label}`} onClose={onClose} width={460}>
      <div className="grid grid-cols-3 gap-3">
        <Field label="From">
          {(id) => (
            <Select id={id} value={from} onChange={(e) => setFrom(e.target.value)}>
              {BUCKETS.map(([k, l]) => (
                <option key={k} value={k}>
                  {l}
                </option>
              ))}
            </Select>
          )}
        </Field>
        <Field label="To">
          {(id) => (
            <Select id={id} value={to} onChange={(e) => setTo(e.target.value)}>
              {BUCKETS.map(([k, l]) => (
                <option key={k} value={k}>
                  {l}
                </option>
              ))}
            </Select>
          )}
        </Field>
        <Field label="Quantity">
          {(id) => (
            <Input
              id={id}
              className="text-right"
              inputMode="numeric"
              value={qty}
              onChange={(e) => setQty(e.target.value)}
            />
          )}
        </Field>
      </div>
      <div className="mt-3">
        <Field label="Reason (required)" error={err}>
          {(id) => <Input id={id} value={reason} onChange={(e) => setReason(e.target.value)} />}
        </Field>
      </div>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" onClick={() => void go()}>
          Move
        </Button>
      </div>
    </Modal>
  );
}

function OpeningModal({
  row,
  onClose,
  onDone,
}: {
  row: StockRow;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const [qty, setQty] = useState("");
  const [cost, setCost] = useState<number | null>(null);
  const [serials, setSerials] = useState("");
  const [err, setErr] = useState<string | null>(null);
  async function go() {
    try {
      await api.stockOpening({
        variant_id: row.variant_id,
        qty: /^\d+$/.test(qty) ? Number(qty) : 0,
        unit_cost: cost ?? 0,
        serials: row.is_serialized
          ? serials
              .split(/[\n,;]+/)
              .map((s) => s.trim())
              .filter(Boolean)
          : [],
        reason: "Opening balance",
      });
      toast("ok", "Opening stock recorded");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal title={`Opening stock: ${row.label}`} onClose={onClose} width={480}>
      <p className="mb-3 text-[12px] text-ink-2">
        Only allowed before this item has any stock movement. For later changes use a purchase or an
        adjustment.
      </p>
      <div className="grid grid-cols-2 gap-3">
        {!row.is_serialized && (
          <Field label="Quantity">
            {(id) => (
              <Input
                id={id}
                className="text-right"
                inputMode="numeric"
                value={qty}
                onChange={(e) => setQty(e.target.value)}
              />
            )}
          </Field>
        )}
        <Field label="Unit cost (৳)">
          {(id) => <MoneyInput id={id} value={cost} onChange={setCost} />}
        </Field>
      </div>
      {row.is_serialized && (
        <div className="mt-3">
          <Field label="Serial numbers (one per line)">
            {(id) => (
              <textarea
                id={id}
                rows={4}
                value={serials}
                onChange={(e) => setSerials(e.target.value)}
                className="w-full rounded-lg border border-line px-3 py-2 text-[13px] outline-none focus:border-teal"
              />
            )}
          </Field>
        </div>
      )}
      {err && (
        <p role="alert" className="mt-3 text-[12px] text-danger">
          {err}
        </p>
      )}
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" onClick={() => void go()}>
          Record opening stock
        </Button>
      </div>
    </Modal>
  );
}

function Serials() {
  const { can } = useAuth();
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState("");
  const [modal, setModal] = useState<null | { history: SerialRow } | { status: SerialRow }>(null);
  const list = useLoad(
    () => api.serialsList({ search: search || undefined, status: status || undefined, limit: 200 }),
    [search, status],
  );
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <Input
          className="!w-72"
          placeholder="Find serial number…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label="Search serial numbers"
        />
        <Select
          className="!w-56"
          value={status}
          onChange={(e) => setStatus(e.target.value)}
          aria-label="Status"
        >
          <option value="">All statuses</option>
          {[
            "available",
            "reserved",
            "sold",
            "returned_pending_inspection",
            "damaged",
            "warranty_service",
            "written_off",
            "returned_to_supplier",
          ].map((s) => (
            <option key={s} value={s}>
              {s.replace(/_/g, " ")}
            </option>
          ))}
        </Select>
      </div>
      {list.error && (
        <p role="alert" className="text-[13px] text-danger">
          {list.error}
        </p>
      )}
      <Card>
        {list.data && list.data.length === 0 ? (
          <EmptyState
            title="No serial numbers"
            hint="Serial-tracked products get one record per unit when received."
          />
        ) : (
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th className="pb-2">Serial</th>
                <th>Product</th>
                <th>SKU</th>
                <th>Status</th>
                <th className="num">Cost</th>
                <th>Received</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {list.data?.map((s) => (
                <tr key={s.id} className="border-t border-line">
                  <td className="py-1.5 font-medium text-navy">{s.serial_no}</td>
                  <td>{s.label}</td>
                  <td>{s.sku}</td>
                  <td>
                    <Badge
                      tone={
                        s.status === "available"
                          ? "ok"
                          : s.status === "damaged" || s.status === "written_off"
                            ? "bad"
                            : "neutral"
                      }
                    >
                      {s.status.replace(/_/g, " ")}
                    </Badge>
                  </td>
                  <td className="num">{formatPaisa(s.cost)}</td>
                  <td>{formatDhaka(s.received_at)}</td>
                  <td className="space-x-1 text-right">
                    <Button variant="ghost" onClick={() => setModal({ history: s })}>
                      History
                    </Button>
                    {can("inventory.adjust") && MANUAL_TARGETS[s.status] && (
                      <Button variant="ghost" onClick={() => setModal({ status: s })}>
                        Change status
                      </Button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
      {modal && "history" in modal && (
        <SerialHistory row={modal.history} onClose={() => setModal(null)} />
      )}
      {modal && "status" in modal && (
        <SerialStatusModal
          row={modal.status}
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

function SerialHistory({ row, onClose }: { row: SerialRow; onClose: () => void }) {
  const h = useLoad(() => api.serialHistory(row.id), [row.id]);
  return (
    <Modal title={`Serial ${row.serial_no}`} onClose={onClose} width={640}>
      <table className="w-full text-[12px]">
        <thead>
          <tr className="text-left text-ink-2">
            <th className="pb-2">When (Dhaka)</th>
            <th>Change</th>
            <th>Reason</th>
            <th>By</th>
          </tr>
        </thead>
        <tbody>
          {h.data?.map((e, i) => (
            <tr key={i} className="border-t border-line">
              <td className="py-1.5">{formatDhaka(e.at)}</td>
              <td>
                {e.from_status ? `${e.from_status.replace(/_/g, " ")} → ` : "received as "}
                {e.to_status.replace(/_/g, " ")}
              </td>
              <td>{e.reason}</td>
              <td>{e.actor}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="mt-4 flex justify-end">
        <Button onClick={onClose}>Close</Button>
      </div>
    </Modal>
  );
}

function SerialStatusModal({
  row,
  onClose,
  onDone,
}: {
  row: SerialRow;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const targets = MANUAL_TARGETS[row.status] ?? [];
  const [to, setTo] = useState(targets[0] ?? "");
  const [reason, setReason] = useState("");
  const [err, setErr] = useState<string | null>(null);
  async function go() {
    try {
      await api.serialSetStatus(row.id, to, reason);
      toast("ok", "Status changed");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal title={`Change status: ${row.serial_no}`} onClose={onClose} width={440}>
      <p className="mb-3 text-[12px] text-ink-2">
        Currently <b>{row.status.replace(/_/g, " ")}</b>. Sales, returns and inspection moves are
        made by those workflows.
      </p>
      <Field label="New status">
        {(id) => (
          <Select id={id} value={to} onChange={(e) => setTo(e.target.value)}>
            {targets.map((t) => (
              <option key={t} value={t}>
                {t.replace(/_/g, " ")}
              </option>
            ))}
          </Select>
        )}
      </Field>
      <div className="mt-3">
        <Field label="Reason (required)" error={err}>
          {(id) => <Input id={id} value={reason} onChange={(e) => setReason(e.target.value)} />}
        </Field>
      </div>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" onClick={() => void go()}>
          Change status
        </Button>
      </div>
    </Modal>
  );
}

function Reconcile() {
  const r = useLoad(() => api.stockReconcile(), []);
  return (
    <Card title="Stock reconciliation" action={<Button onClick={r.reload}>Run again</Button>}>
      {r.error && (
        <p role="alert" className="text-danger">
          {r.error}
        </p>
      )}
      {r.data && r.data.length === 0 && (
        <p className="flex items-center gap-2 text-[13px] text-teal-deep">
          <CheckCircle2 size={16} />
          Stock ledger, cached balances and serial numbers all agree.
        </p>
      )}
      {r.data && r.data.length > 0 && (
        <>
          <p className="mb-2 flex items-center gap-2 text-[13px] text-danger">
            <AlertTriangle size={16} />
            {r.data.length} discrepancy(ies) found. Do not trade these items until resolved; contact
            support with this list.
          </p>
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th>SKU</th>
                <th>Bucket</th>
                <th>Check</th>
                <th className="num">Expected</th>
                <th className="num">Actual</th>
              </tr>
            </thead>
            <tbody>
              {r.data.map((d, i) => (
                <tr key={i} className="border-t border-line">
                  <td className="py-1.5">{d.sku}</td>
                  <td>{d.bucket}</td>
                  <td>{d.check.replace(/_/g, " ")}</td>
                  <td className="num">{d.expected}</td>
                  <td className="num">{d.actual}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </Card>
  );
}
