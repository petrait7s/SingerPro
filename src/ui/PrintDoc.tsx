import { createPortal } from "react-dom";
import { useEffect, useState } from "react";
import { api, type Branding, type SaleDetail } from "../lib/api";
import { formatDhaka, formatPaisa } from "../lib/format";

export type PrintKind = "a4" | "receipt";

/** Loads shop identity + logo once for documents. */
export function useShopIdentity() {
  const [b, setB] = useState<Branding | null>(null);
  const [logo, setLogo] = useState<string | null>(null);
  useEffect(() => {
    api
      .branding()
      .then(setB)
      .catch(() => undefined);
    api
      .logoGet()
      .then(setLogo)
      .catch(() => undefined);
  }, []);
  return { branding: b, logo };
}

const METHOD: Record<string, string> = {
  cash: "Cash",
  bank: "Bank",
  mobile_wallet: "Mobile wallet",
  cheque: "Cheque",
  other: "Other",
};

/**
 * Renders an invoice into a print-only portal and opens the system print dialog.
 * A4 invoice or 80mm receipt; everything is local (no network), fonts are bundled.
 */
export function PrintDoc({
  sale,
  kind,
  branding,
  logo,
  onDone,
}: {
  sale: SaleDetail;
  kind: PrintKind;
  branding: Branding | null;
  logo: string | null;
  onDone: () => void;
}) {
  useEffect(() => {
    const style = document.createElement("style");
    style.textContent =
      kind === "receipt"
        ? "@page { size: 80mm auto; margin: 3mm; }"
        : "@page { size: A4; margin: 14mm; }";
    document.head.append(style);
    const t = setTimeout(() => window.print(), 150);
    const after = () => onDone();
    window.addEventListener("afterprint", after);
    return () => {
      clearTimeout(t);
      window.removeEventListener("afterprint", after);
      style.remove();
    };
  }, [kind, onDone]);

  const narrow = kind === "receipt";
  const paid = sale.sale.paid;
  const doc = (
    <div id="print-root" className={narrow ? "print-receipt" : "print-a4"}>
      <header className={narrow ? "text-center" : "flex items-start justify-between"}>
        <div>
          {logo && (
            <img src={logo} alt="" className={narrow ? "mx-auto mb-1 max-h-12" : "mb-2 max-h-16"} />
          )}
          <h1 className="text-[16px] font-bold">{branding?.shop_name ?? "Singer Pro"}</h1>
          {branding?.shop_address && <p>{branding.shop_address}</p>}
          {(branding?.shop_phone || branding?.shop_email) && (
            <p>{[branding.shop_phone, branding.shop_email].filter(Boolean).join(" · ")}</p>
          )}
        </div>
        <div className={narrow ? "mt-2" : "text-right"}>
          <h2 className="text-[14px] font-bold uppercase">{narrow ? "Receipt" : "Invoice"}</h2>
          <p>No: {sale.sale.number}</p>
          <p>Date: {sale.sale.sale_date}</p>
          {sale.cashier && <p>Served by: {sale.cashier}</p>}
        </div>
      </header>
      {sale.sale.customer && (
        <section className="mt-2">
          <p>
            <b>Customer:</b> {sale.sale.customer}
            {sale.customer_phone ? ` · ${sale.customer_phone}` : ""}
          </p>
          {sale.customer_address && <p>{sale.customer_address}</p>}
        </section>
      )}
      <table className="mt-3 w-full border-collapse text-[11px]">
        <thead>
          <tr className="border-y border-black text-left">
            <th className="py-1">Item</th>
            <th className="text-right">Qty</th>
            <th className="text-right">Price</th>
            <th className="text-right">Amount</th>
          </tr>
        </thead>
        <tbody>
          {sale.items.map((i, n) => (
            <tr key={n} className="align-top">
              <td className="py-0.5">
                {i.label}
                <br />
                <span className="text-[10px]">
                  {i.sku}
                  {i.serial_no ? ` · S/N ${i.serial_no}` : ""}
                  {i.discount ? ` · disc. ${formatPaisa(i.discount)}` : ""}
                </span>
              </td>
              <td className="text-right">{i.qty}</td>
              <td className="text-right">{formatPaisa(i.unit_price)}</td>
              <td className="text-right">{formatPaisa(i.line_total - i.tax)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <dl
        className={`mt-2 border-t border-black pt-1 text-[11px] ${narrow ? "" : "ml-auto w-[260px]"}`}
      >
        <div className="flex justify-between">
          <dt>Subtotal</dt>
          <dd>{formatPaisa(sale.subtotal)}</dd>
        </div>
        {sale.discount_total > 0 && (
          <div className="flex justify-between">
            <dt>Discount</dt>
            <dd>-{formatPaisa(sale.discount_total)}</dd>
          </div>
        )}
        {sale.financing_charges > 0 && (
          <div className="flex justify-between">
            <dt>Installment markup &amp; fees</dt>
            <dd>{formatPaisa(sale.financing_charges)}</dd>
          </div>
        )}
        {sale.tax_total > 0 && (
          <div className="flex justify-between">
            <dt>Tax</dt>
            <dd>{formatPaisa(sale.tax_total)}</dd>
          </div>
        )}
        <div className="flex justify-between text-[13px] font-bold">
          <dt>Total</dt>
          <dd>{formatPaisa(sale.sale.grand_total)}</dd>
        </div>
        {sale.payments.map((p) => (
          <div key={p.id} className="flex justify-between">
            <dt>
              {p.amount < 0 ? "Reversed" : "Paid"} · {METHOD[p.method] ?? p.method} ({p.receipt_no})
            </dt>
            <dd>{formatPaisa(p.amount)}</dd>
          </div>
        ))}
        <div className="flex justify-between font-bold">
          <dt>Paid</dt>
          <dd>{formatPaisa(paid)}</dd>
        </div>
        <div className="flex justify-between font-bold">
          <dt>Balance due</dt>
          <dd>{formatPaisa(sale.sale.due)}</dd>
        </div>
      </dl>
      {!narrow && (
        <div className="mt-10 flex justify-between text-[11px]">
          <span className="border-t border-black pt-1">Customer signature</span>
          <span className="border-t border-black pt-1">Authorised signature</span>
        </div>
      )}
      {branding?.invoice_footer && (
        <p className="mt-3 text-center text-[11px]">{branding.invoice_footer}</p>
      )}
      <p className="mt-3 text-center text-[9px] text-neutral-500">
        Printed {formatDhaka(new Date().toISOString())} · Developed by PETRA Software Solutions
      </p>
    </div>
  );
  return createPortal(doc, document.body);
}
