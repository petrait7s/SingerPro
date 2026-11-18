import { useState } from "react";
import { ChevronDown, ChevronRight, Plus, Settings2 } from "lucide-react";
import {
  api,
  toApiError,
  type CategoryRow,
  type Named,
  type NewVariant,
  type ProductRow,
  type VariantRow,
} from "../lib/api";
import { useAuth } from "../lib/auth";
import { useLoad } from "../lib/useLoad";
import { formatPaisa } from "../lib/format";
import { Badge, Card, EmptyState } from "../ui/Card";
import { Button, Field, Input, Modal, Select, Toggle, useToast } from "../ui/controls";
import { MoneyInput } from "../ui/MoneyInput";

export function ProductsPage() {
  const { can } = useAuth();
  const canManage = can("products.manage");
  const toast = useToast();
  const [search, setSearch] = useState("");
  const [brandId, setBrandId] = useState("");
  const [lowOnly, setLowOnly] = useState(false);
  const [inactive, setInactive] = useState(false);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [modal, setModal] = useState<
    | null
    | "new"
    | "lookups"
    | { edit: ProductRow }
    | { addVariant: ProductRow }
    | { variant: VariantRow }
  >(null);

  const brands = useLoad(() => api.brandsList(), []);
  const cats = useLoad(() => api.categoriesList(), []);
  const list = useLoad(
    () =>
      api.productsList({
        search: search || undefined,
        brand_id: brandId || undefined,
        include_inactive: inactive,
        low_stock_only: lowOnly,
        limit: 100,
      }),
    [search, brandId, inactive, lowOnly],
  );
  const refresh = () => {
    list.reload();
    brands.reload();
    cats.reload();
  };

  const toggle = (id: string) =>
    setOpen((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  async function setActive(p: ProductRow, active: boolean) {
    try {
      await api.productSetActive(p.id, active);
      toast("ok", `${p.name}: ${active ? "activated" : "deactivated"}`);
      refresh();
    } catch (e) {
      toast("bad", toApiError(e).message);
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-3">
        <Input
          className="!w-72"
          placeholder="Search name, model, SKU, barcode…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label="Search products"
        />
        <Select
          className="!w-44"
          value={brandId}
          onChange={(e) => setBrandId(e.target.value)}
          aria-label="Brand"
        >
          <option value="">All brands</option>
          {brands.data?.map((b) => (
            <option key={b.id} value={b.id}>
              {b.name}
            </option>
          ))}
        </Select>
        <Toggle label="Low stock only" checked={lowOnly} onChange={setLowOnly} />
        <Toggle label="Show inactive" checked={inactive} onChange={setInactive} />
        <div className="ml-auto flex gap-2">
          {canManage && (
            <Button onClick={() => setModal("lookups")}>
              <Settings2 size={14} />
              Brands & categories
            </Button>
          )}
          {canManage && (
            <Button variant="primary" onClick={() => setModal("new")}>
              <Plus size={14} />
              New product
            </Button>
          )}
        </div>
      </div>
      {list.error && (
        <p role="alert" className="text-[13px] text-danger">
          {list.error}
        </p>
      )}
      <Card>
        {list.data && list.data.rows.length === 0 ? (
          <EmptyState
            title="No products yet"
            hint={
              canManage
                ? "Create brands and categories, then add your first product with its variants and prices."
                : "No products match."
            }
          />
        ) : (
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-left text-ink-2">
                <th className="w-6 pb-2" />
                <th>Product</th>
                <th>Brand</th>
                <th>Category</th>
                <th>Tracking</th>
                <th className="num">Available</th>
                <th>Status</th>
              </tr>
            </thead>
            <tbody>
              {list.data?.rows.map((p) => (
                <ProductRows
                  key={p.id}
                  p={p}
                  expanded={open.has(p.id)}
                  onToggle={() => toggle(p.id)}
                  canManage={canManage}
                  onEdit={() => setModal({ edit: p })}
                  onAddVariant={() => setModal({ addVariant: p })}
                  onVariant={(v) => setModal({ variant: v })}
                  onActive={(a) => void setActive(p, a)}
                />
              ))}
            </tbody>
          </table>
        )}
        {list.data && <p className="mt-3 text-[11px] text-ink-2">{list.data.total} product(s)</p>}
      </Card>
      {modal === "new" && brands.data && cats.data && (
        <ProductModal
          brands={brands.data}
          cats={cats.data}
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            refresh();
          }}
        />
      )}
      {modal && typeof modal === "object" && "edit" in modal && brands.data && cats.data && (
        <ProductModal
          product={modal.edit}
          brands={brands.data}
          cats={cats.data}
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            refresh();
          }}
        />
      )}
      {modal && typeof modal === "object" && "addVariant" in modal && (
        <VariantModal
          productId={modal.addVariant.id}
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            refresh();
          }}
        />
      )}
      {modal && typeof modal === "object" && "variant" in modal && (
        <VariantModal
          variant={modal.variant}
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            refresh();
          }}
        />
      )}
      {modal === "lookups" && brands.data && cats.data && (
        <LookupsModal
          brands={brands.data}
          cats={cats.data}
          onClose={() => setModal(null)}
          onChanged={refresh}
        />
      )}
    </div>
  );
}

function ProductRows({
  p,
  expanded,
  onToggle,
  canManage,
  onEdit,
  onAddVariant,
  onVariant,
  onActive,
}: {
  p: ProductRow;
  expanded: boolean;
  onToggle: () => void;
  canManage: boolean;
  onEdit: () => void;
  onAddVariant: () => void;
  onVariant: (v: VariantRow) => void;
  onActive: (a: boolean) => void;
}) {
  return (
    <>
      <tr className="cursor-pointer border-t border-line hover:bg-mint/30" onClick={onToggle}>
        <td className="py-2">
          {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </td>
        <td className="font-medium text-navy">
          {p.name}
          {p.model && <span className="ml-1 text-ink-2">{p.model}</span>}
        </td>
        <td>{p.brand}</td>
        <td>{p.category}</td>
        <td>
          {p.is_serialized ? (
            <Badge tone="ok">Serial no.</Badge>
          ) : (
            <Badge tone="neutral">Quantity</Badge>
          )}
        </td>
        <td className="num">
          {p.available_total}
          {p.low_stock && (
            <span className="ml-1">
              <Badge tone="bad">low</Badge>
            </span>
          )}
        </td>
        <td className="pl-4">
          {p.is_active ? <Badge tone="ok">Active</Badge> : <Badge tone="neutral">Inactive</Badge>}
        </td>
      </tr>
      {expanded && (
        <tr className="bg-pearl">
          <td />
          <td colSpan={6} className="pb-3">
            <table className="mt-1 w-full text-[12px]">
              <thead>
                <tr className="text-left text-ink-2">
                  <th>SKU</th>
                  <th>Barcode</th>
                  <th>Variant</th>
                  <th className="num">Cost</th>
                  <th className="num">Retail</th>
                  <th className="num">Wholesale</th>
                  <th className="num">Installment</th>
                  <th className="num">Avail.</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {p.variants.map((v) => (
                  <tr key={v.id} className="border-t border-line">
                    <td className="py-1.5 font-medium">
                      {v.sku}
                      {!v.is_active && " (inactive)"}
                    </td>
                    <td>{v.barcode ?? "—"}</td>
                    <td>{[v.colour, v.capacity, v.size].filter(Boolean).join(" · ") || "—"}</td>
                    <td className="num">{formatPaisa(v.purchase_cost)}</td>
                    <td className="num">{formatPaisa(v.retail_price)}</td>
                    <td className="num">{formatPaisa(v.wholesale_price)}</td>
                    <td className="num">
                      {v.installment_price === null ? "—" : formatPaisa(v.installment_price)}
                    </td>
                    <td className="num">{v.available}</td>
                    <td className="text-right">
                      {canManage && (
                        <Button variant="ghost" onClick={() => onVariant(v)}>
                          Edit
                        </Button>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {canManage && (
              <div className="mt-2 flex gap-2">
                <Button onClick={onEdit}>Edit product</Button>
                <Button onClick={onAddVariant}>Add variant</Button>
                <Button variant="ghost" onClick={() => onActive(!p.is_active)}>
                  {p.is_active ? "Deactivate" : "Reactivate"}
                </Button>
              </div>
            )}
            {p.description && <p className="mt-2 text-[12px] text-ink-2">{p.description}</p>}
            {p.warranty_months > 0 && (
              <p className="mt-1 text-[11px] text-ink-2">
                Warranty: {p.warranty_months} month(s) {p.warranty_terms && `— ${p.warranty_terms}`}
              </p>
            )}
          </td>
        </tr>
      )}
    </>
  );
}

const blankVariant = (): VariantDraft => ({
  sku: "",
  barcode: "",
  colour: "",
  capacity: "",
  size: "",
  purchase_cost: null,
  retail_price: null,
  wholesale_price: null,
  installment_price: null,
});
interface VariantDraft {
  sku: string;
  barcode: string;
  colour: string;
  capacity: string;
  size: string;
  purchase_cost: number | null;
  retail_price: number | null;
  wholesale_price: number | null;
  installment_price: number | null;
}

function toNew(d: VariantDraft): NewVariant {
  return {
    sku: d.sku,
    barcode: d.barcode.trim() || null,
    colour: d.colour,
    capacity: d.capacity,
    size: d.size,
    purchase_cost: d.purchase_cost ?? 0,
    retail_price: d.retail_price ?? 0,
    wholesale_price: d.wholesale_price ?? 0,
    installment_price: d.installment_price,
  };
}

function VariantFields({ v, onChange }: { v: VariantDraft; onChange: (v: VariantDraft) => void }) {
  const set = <K extends keyof VariantDraft>(k: K, val: VariantDraft[K]) =>
    onChange({ ...v, [k]: val });
  return (
    <div className="grid grid-cols-3 gap-3">
      <Field label="SKU">
        {(id) => <Input id={id} value={v.sku} onChange={(e) => set("sku", e.target.value)} />}
      </Field>
      <Field label="Barcode (optional)">
        {(id) => (
          <Input id={id} value={v.barcode} onChange={(e) => set("barcode", e.target.value)} />
        )}
      </Field>
      <Field label="Colour">
        {(id) => <Input id={id} value={v.colour} onChange={(e) => set("colour", e.target.value)} />}
      </Field>
      <Field label="Capacity">
        {(id) => (
          <Input id={id} value={v.capacity} onChange={(e) => set("capacity", e.target.value)} />
        )}
      </Field>
      <Field label="Size">
        {(id) => <Input id={id} value={v.size} onChange={(e) => set("size", e.target.value)} />}
      </Field>
      <span />
      <Field label="Purchase cost (৳)">
        {(id) => (
          <MoneyInput id={id} value={v.purchase_cost} onChange={(x) => set("purchase_cost", x)} />
        )}
      </Field>
      <Field label="Retail price (৳)">
        {(id) => (
          <MoneyInput id={id} value={v.retail_price} onChange={(x) => set("retail_price", x)} />
        )}
      </Field>
      <Field label="Wholesale price (৳)">
        {(id) => (
          <MoneyInput
            id={id}
            value={v.wholesale_price}
            onChange={(x) => set("wholesale_price", x)}
          />
        )}
      </Field>
      <Field
        label="Installment price (৳)"
        hint="Optional; markup is shown separately on agreements"
      >
        {(id) => (
          <MoneyInput
            id={id}
            value={v.installment_price}
            onChange={(x) => set("installment_price", x)}
          />
        )}
      </Field>
    </div>
  );
}

function ProductModal({
  product,
  brands,
  cats,
  onClose,
  onDone,
}: {
  product?: ProductRow;
  brands: Named[];
  cats: CategoryRow[];
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const [f, setF] = useState({
    brand_id: product?.brand_id ?? "",
    category_id: product?.category_id ?? "",
    name: product?.name ?? "",
    model: product?.model ?? "",
    description: product?.description ?? "",
    warranty_months: String(product?.warranty_months ?? 0),
    warranty_terms: product?.warranty_terms ?? "",
    min_stock: String(product?.min_stock ?? 0),
    is_serialized: product?.is_serialized ?? false,
  });
  const [v, setV] = useState<VariantDraft>(blankVariant());
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const set = (k: keyof typeof f, val: string | boolean) => setF((x) => ({ ...x, [k]: val }));
  const parent = cats.filter((c) => c.parent_id === null);

  async function save() {
    setBusy(true);
    setErr(null);
    const base = {
      brand_id: f.brand_id,
      category_id: f.category_id,
      name: f.name,
      model: f.model,
      description: f.description,
      warranty_months: Number(f.warranty_months),
      warranty_terms: f.warranty_terms,
      min_stock: Number(f.min_stock),
      is_serialized: f.is_serialized,
    };
    try {
      if (product) await api.productUpdate({ id: product.id, ...base });
      else await api.productCreate({ ...base, variants: [toNew(v)] });
      toast("ok", product ? "Product updated" : "Product created");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title={product ? "Edit product" : "New product"} onClose={onClose} width={720}>
      <div className="grid grid-cols-2 gap-3">
        <Field label="Brand">
          {(id) => (
            <Select id={id} value={f.brand_id} onChange={(e) => set("brand_id", e.target.value)}>
              <option value="">Select…</option>
              {brands
                .filter((b) => b.is_active || b.id === f.brand_id)
                .map((b) => (
                  <option key={b.id} value={b.id}>
                    {b.name}
                  </option>
                ))}
            </Select>
          )}
        </Field>
        <Field label="Category / subcategory">
          {(id) => (
            <Select
              id={id}
              value={f.category_id}
              onChange={(e) => set("category_id", e.target.value)}
            >
              <option value="">Select…</option>
              {parent.map((c) => [
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>,
                ...cats
                  .filter((s) => s.parent_id === c.id)
                  .map((s) => <option key={s.id} value={s.id}>{`${c.name} › ${s.name}`}</option>),
              ])}
            </Select>
          )}
        </Field>
        <Field label="Product name">
          {(id) => (
            <Input id={id} value={f.name} onChange={(e) => set("name", e.target.value)} autoFocus />
          )}
        </Field>
        <Field label="Model number">
          {(id) => <Input id={id} value={f.model} onChange={(e) => set("model", e.target.value)} />}
        </Field>
        <div className="col-span-2">
          <Field label="Description">
            {(id) => (
              <Input
                id={id}
                value={f.description}
                onChange={(e) => set("description", e.target.value)}
              />
            )}
          </Field>
        </div>
        <Field label="Warranty (months)">
          {(id) => (
            <Input
              id={id}
              type="number"
              min={0}
              value={f.warranty_months}
              onChange={(e) => set("warranty_months", e.target.value)}
            />
          )}
        </Field>
        <Field label="Warranty terms">
          {(id) => (
            <Input
              id={id}
              value={f.warranty_terms}
              onChange={(e) => set("warranty_terms", e.target.value)}
            />
          )}
        </Field>
        <Field label="Minimum stock (low-stock alert)">
          {(id) => (
            <Input
              id={id}
              type="number"
              min={0}
              value={f.min_stock}
              onChange={(e) => set("min_stock", e.target.value)}
            />
          )}
        </Field>
        <div className="flex items-end pb-2">
          <Toggle
            label="Track each unit by serial number"
            checked={f.is_serialized}
            onChange={(x) => set("is_serialized", x)}
          />
        </div>
      </div>
      {!product && (
        <div className="mt-4 border-t border-line pt-3">
          <p className="mb-2 text-[12px] font-semibold text-navy">
            First variant (more can be added later)
          </p>
          <VariantFields v={v} onChange={setV} />
        </div>
      )}
      {err && (
        <p role="alert" className="mt-3 text-[12px] text-danger">
          {err}
        </p>
      )}
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" busy={busy} onClick={() => void save()}>
          Save
        </Button>
      </div>
    </Modal>
  );
}

function VariantModal({
  productId,
  variant,
  onClose,
  onDone,
}: {
  productId?: string;
  variant?: VariantRow;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const [v, setV] = useState<VariantDraft>(
    variant
      ? {
          sku: variant.sku,
          barcode: variant.barcode ?? "",
          colour: variant.colour,
          capacity: variant.capacity,
          size: variant.size,
          purchase_cost: variant.purchase_cost,
          retail_price: variant.retail_price,
          wholesale_price: variant.wholesale_price,
          installment_price: variant.installment_price,
        }
      : blankVariant(),
  );
  const [err, setErr] = useState<string | null>(null);
  async function save() {
    try {
      if (variant) await api.variantUpdate({ id: variant.id, ...toNew(v) });
      else await api.variantAdd(productId!, toNew(v));
      toast("ok", "Saved");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  async function setActive(a: boolean) {
    try {
      await api.variantSetActive(variant!.id, a);
      toast("ok", a ? "Variant activated" : "Variant deactivated");
      onDone();
    } catch (e) {
      setErr(toApiError(e).message);
    }
  }
  return (
    <Modal
      title={variant ? `Edit variant ${variant.sku}` : "Add variant"}
      onClose={onClose}
      width={680}
    >
      <VariantFields v={v} onChange={setV} />
      {variant && (
        <p className="mt-3 text-[11px] text-ink-2">
          Price changes are recorded in the audit log with before and after values.
        </p>
      )}
      {err && (
        <p role="alert" className="mt-3 text-[12px] text-danger">
          {err}
        </p>
      )}
      <div className="mt-4 flex justify-between">
        <div>
          {variant && (
            <Button variant="ghost" onClick={() => void setActive(!variant.is_active)}>
              {variant.is_active ? "Deactivate" : "Reactivate"}
            </Button>
          )}
        </div>
        <div className="flex gap-2">
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" onClick={() => void save()}>
            Save
          </Button>
        </div>
      </div>
    </Modal>
  );
}

function LookupsModal({
  brands,
  cats,
  onClose,
  onChanged,
}: {
  brands: Named[];
  cats: CategoryRow[];
  onClose: () => void;
  onChanged: () => void;
}) {
  const toast = useToast();
  const [brand, setBrand] = useState("");
  const [cat, setCat] = useState("");
  const [parent, setParent] = useState("");
  const run = async (fn: () => Promise<unknown>, ok: string) => {
    try {
      await fn();
      toast("ok", ok);
      onChanged();
    } catch (e) {
      toast("bad", toApiError(e).message);
    }
  };
  return (
    <Modal title="Brands & categories" onClose={onClose} width={640}>
      <div className="grid grid-cols-2 gap-6">
        <div>
          <p className="mb-2 text-[12px] font-semibold text-navy">Brands</p>
          <div className="mb-2 flex gap-2">
            <Input
              value={brand}
              onChange={(e) => setBrand(e.target.value)}
              placeholder="New brand"
              aria-label="New brand"
            />
            <Button
              variant="primary"
              onClick={() =>
                void run(async () => {
                  await api.brandCreate(brand);
                  setBrand("");
                }, "Brand added")
              }
            >
              Add
            </Button>
          </div>
          <ul className="space-y-1 text-[13px]">
            {brands.map((b) => (
              <li
                key={b.id}
                className="flex items-center justify-between rounded-lg px-2 py-1 hover:bg-slate-50"
              >
                <span className={b.is_active ? "" : "text-ink-2 line-through"}>{b.name}</span>
                <Button
                  variant="ghost"
                  onClick={() => void run(() => api.brandSetActive(b.id, !b.is_active), "Updated")}
                >
                  {b.is_active ? "Deactivate" : "Reactivate"}
                </Button>
              </li>
            ))}
          </ul>
        </div>
        <div>
          <p className="mb-2 text-[12px] font-semibold text-navy">Categories</p>
          <div className="mb-2 flex gap-2">
            <Input
              value={cat}
              onChange={(e) => setCat(e.target.value)}
              placeholder="New category"
              aria-label="New category"
            />
            <Select
              className="!w-36"
              value={parent}
              onChange={(e) => setParent(e.target.value)}
              aria-label="Parent category"
            >
              <option value="">Top level</option>
              {cats
                .filter((c) => !c.parent_id)
                .map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.name}
                  </option>
                ))}
            </Select>
            <Button
              variant="primary"
              onClick={() =>
                void run(async () => {
                  await api.categoryCreate(cat, parent || null);
                  setCat("");
                }, "Category added")
              }
            >
              Add
            </Button>
          </div>
          <ul className="space-y-1 text-[13px]">
            {cats.map((c) => (
              <li
                key={c.id}
                className="flex items-center justify-between rounded-lg px-2 py-1 hover:bg-slate-50"
              >
                <span
                  className={`${c.parent_id ? "pl-4" : "font-medium"} ${c.is_active ? "" : "text-ink-2 line-through"}`}
                >
                  {c.name}
                </span>
                <Button
                  variant="ghost"
                  onClick={() =>
                    void run(() => api.categorySetActive(c.id, !c.is_active), "Updated")
                  }
                >
                  {c.is_active ? "Deactivate" : "Reactivate"}
                </Button>
              </li>
            ))}
          </ul>
        </div>
      </div>
      <div className="mt-4 flex justify-end">
        <Button onClick={onClose}>Done</Button>
      </div>
    </Modal>
  );
}
