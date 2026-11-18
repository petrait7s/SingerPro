import { useEffect, useRef, useState } from "react";
import { api, toApiError } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { useI18n } from "../../lib/i18n";
import { Card } from "../../ui/Card";
import { Button, Field, Input, useToast } from "../../ui/controls";
import { useSettings } from "./useSettings";

const KEYS = [
  "shop.name",
  "shop.address",
  "shop.phone",
  "shop.email",
  "shop.invoice_footer",
  "invoice.prefix",
];

function toBase64(buf: ArrayBuffer): string {
  let s = "";
  const b = new Uint8Array(buf);
  for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode(...b.subarray(i, i + 0x8000));
  return btoa(s);
}

export function ShopTab() {
  const { t } = useI18n();
  const { can } = useAuth();
  const toast = useToast();
  const s = useSettings();
  const editable = can("settings.manage");
  const [logo, setLogo] = useState<string | null>(null);
  const file = useRef<HTMLInputElement>(null);
  const loadLogo = () =>
    api
      .logoGet()
      .then(setLogo)
      .catch(() => undefined);
  useEffect(() => {
    void loadLogo();
  }, []);

  async function upload(f: File | undefined) {
    if (!f) return;
    try {
      await api.logoSet(toBase64(await f.arrayBuffer()));
      toast("ok", t("common.saved", "Saved"));
      await loadLogo();
    } catch (e) {
      toast("bad", toApiError(e).message);
    }
    if (file.current) file.current.value = "";
  }
  async function remove() {
    try {
      await api.logoClear();
      setLogo(null);
      toast("ok", t("common.saved", "Saved"));
    } catch (e) {
      toast("bad", toApiError(e).message);
    }
  }
  if (!s.loaded) return <p className="text-ink-2">{t("common.loading", "Loading…")}</p>;
  const f = (k: string, label: string, hint?: string) => (
    <Field label={label} hint={hint}>
      {(id) => (
        <Input
          id={id}
          value={s.get(k)}
          onChange={(e) => s.set(k, e.target.value)}
          disabled={!editable}
        />
      )}
    </Field>
  );
  return (
    <div className="grid grid-cols-[1fr_320px] gap-4">
      <Card title={t("settings.shop", "Shop identity")}>
        <div className="grid grid-cols-2 gap-4">
          {f("shop.name", t("shop.name", "Shop name"))}
          {f("invoice.prefix", t("shop.prefix", "Invoice prefix"), "A–Z, 0–9, '-' (max 8)")}
          <div className="col-span-2">{f("shop.address", t("shop.address", "Address"))}</div>
          {f("shop.phone", t("shop.phone", "Phone"))}
          {f("shop.email", t("shop.email", "Email"))}
          <div className="col-span-2">
            {f("shop.invoice_footer", t("shop.footer", "Invoice footer text"))}
          </div>
        </div>
        {s.error && (
          <p role="alert" className="mt-3 text-[12px] text-danger">
            {s.error}
          </p>
        )}
        {editable && (
          <div className="mt-4 flex justify-end">
            <Button
              variant="primary"
              busy={s.busy}
              disabled={!s.dirty}
              onClick={() => void s.save(KEYS)}
            >
              {t("common.save", "Save")}
            </Button>
          </div>
        )}
      </Card>
      <Card title={t("shop.logo", "Shop logo")}>
        <div className="grid h-40 place-items-center rounded-lg border border-dashed border-line bg-pearl">
          {logo ? (
            <img src={logo} alt="Shop logo" className="max-h-36 max-w-full object-contain" />
          ) : (
            <p className="px-4 text-center text-[12px] text-ink-2">
              No logo set. Invoices use the shop name only. PNG or JPEG, up to 512 KB.
            </p>
          )}
        </div>
        {editable && (
          <div className="mt-3 flex gap-2">
            <input
              ref={file}
              type="file"
              accept="image/png,image/jpeg"
              hidden
              onChange={(e) => void upload(e.target.files?.[0])}
            />
            <Button onClick={() => file.current?.click()}>
              {t("shop.logo.upload", "Upload logo")}
            </Button>
            {logo && (
              <Button variant="ghost" onClick={() => void remove()}>
                {t("shop.logo.remove", "Remove")}
              </Button>
            )}
          </div>
        )}
        <p className="mt-3 text-[11px] text-ink-2">
          Use only artwork you are authorised to use. Singer/Beko marks must not be added without
          supplied authorised assets.
        </p>
      </Card>
    </div>
  );
}
