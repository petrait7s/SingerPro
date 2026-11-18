import { useAuth } from "../../lib/auth";
import { useI18n } from "../../lib/i18n";
import { bpToPercent, percentToBp } from "../../lib/format";
import { Card } from "../../ui/Card";
import { Button, Field, Input, Select, Toggle } from "../../ui/controls";
import { useSettings } from "./useSettings";

const CUR = ["currency.code", "currency.symbol"];
const TAX = ["tax.enabled", "tax.label", "tax.default_rate_bp", "tax.prices_include_tax"];

export function FinanceTab() {
  const { t } = useI18n();
  const { can } = useAuth();
  const s = useSettings();
  if (!s.loaded) return <p className="text-ink-2">{t("common.loading", "Loading…")}</p>;
  const canCur = can("settings.manage");
  const canTax = can("tax.manage");
  const rate = bpToPercent(s.get("tax.default_rate_bp"));
  const validate = () =>
    percentToBp(rate) === null ? "Tax rate must be 0–100 with up to 2 decimals" : null;
  return (
    <div className="grid grid-cols-2 gap-4">
      <Card title={t("fin.code", "Currency")}>
        <div className="space-y-3">
          <Field label={t("fin.code", "Currency code")} hint="ISO code, e.g. BDT">
            {(id) => (
              <Input
                id={id}
                value={s.get("currency.code")}
                onChange={(e) => s.set("currency.code", e.target.value.toUpperCase())}
                disabled={!canCur}
                maxLength={3}
              />
            )}
          </Field>
          <Field label={t("fin.symbol", "Currency symbol")}>
            {(id) => (
              <Input
                id={id}
                value={s.get("currency.symbol")}
                onChange={(e) => s.set("currency.symbol", e.target.value)}
                disabled={!canCur}
                maxLength={4}
              />
            )}
          </Field>
          <Field label="Time zone" hint="Fixed to Asia/Dhaka in this release">
            {(id) => (
              <Select id={id} value={s.get("timezone")} disabled>
                <option>Asia/Dhaka</option>
              </Select>
            )}
          </Field>
        </div>
        {canCur && (
          <div className="mt-4 flex justify-end">
            <Button variant="primary" busy={s.busy} onClick={() => void s.save(CUR)}>
              {t("common.save", "Save")}
            </Button>
          </div>
        )}
      </Card>
      <Card title={t("fin.taxEnabled", "Tax / VAT")}>
        <div className="space-y-3">
          <Toggle
            label={t("fin.taxEnabled", "Enable tax")}
            checked={s.get("tax.enabled") === "true"}
            disabled={!canTax}
            onChange={(v) => s.set("tax.enabled", String(v))}
          />
          <Field label={t("fin.taxLabel", "Tax label")}>
            {(id) => (
              <Input
                id={id}
                value={s.get("tax.label")}
                onChange={(e) => s.set("tax.label", e.target.value)}
                disabled={!canTax}
              />
            )}
          </Field>
          <Field
            label={t("fin.taxRate", "Default tax rate (%)")}
            hint="Disabled by default (0%). Changes are audited."
          >
            {(id) => (
              <Input
                id={id}
                inputMode="decimal"
                value={rate}
                onChange={(e) => {
                  const bp = percentToBp(e.target.value);
                  s.set("tax.default_rate_bp", bp === null ? "-1" : String(bp));
                }}
                onBlur={() => undefined}
                disabled={!canTax}
              />
            )}
          </Field>
          <Toggle
            label={t("fin.taxIncl", "Prices include tax")}
            checked={s.get("tax.prices_include_tax") === "true"}
            disabled={!canTax}
            onChange={(v) => s.set("tax.prices_include_tax", String(v))}
          />
        </div>
        {s.error && (
          <p role="alert" className="mt-3 text-[12px] text-danger">
            {s.error}
          </p>
        )}
        {canTax && (
          <div className="mt-4 flex justify-end">
            <Button variant="primary" busy={s.busy} onClick={() => void s.save(TAX, validate)}>
              {t("common.save", "Save")}
            </Button>
          </div>
        )}
      </Card>
    </div>
  );
}
