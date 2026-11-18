import { useAuth } from "../../lib/auth";
import { useI18n } from "../../lib/i18n";
import { bpToPercent, percentToBp } from "../../lib/format";
import { Card } from "../../ui/Card";
import { Button, Field, Input, Select, Toggle } from "../../ui/controls";
import { useSettings } from "./useSettings";

const KEYS = [
  "installment.default_markup_bp",
  "installment.markup_recognition",
  "installment.grace_days",
  "installment.late_fee_enabled",
  "installment.late_fee_bp",
  "installment.rounding_remainder",
];

export function InstallmentTab() {
  const { t } = useI18n();
  const { can } = useAuth();
  const s = useSettings();
  if (!s.loaded) return <p className="text-ink-2">{t("common.loading", "Loading…")}</p>;
  const editable = can("settings.manage");
  const pct = (k: string) => (
    <Input
      inputMode="decimal"
      value={bpToPercent(s.get(k))}
      disabled={!editable}
      onChange={(e) => {
        const bp = percentToBp(e.target.value);
        s.set(k, bp === null ? "-1" : String(bp));
      }}
    />
  );
  const validate = () =>
    ["installment.default_markup_bp", "installment.late_fee_bp"].some((k) => Number(s.get(k)) < 0)
      ? "Percentages must be 0–100 with up to 2 decimals"
      : null;
  return (
    <Card title={t("settings.installments", "Installment defaults")}>
      <p className="mb-4 text-[12px] text-ink-2">
        Markup is 0% unless you set it. It is always shown separately from the cash price on every
        agreement; no hidden interest or fees are ever added.
      </p>
      <div className="grid max-w-3xl grid-cols-2 gap-4">
        <Field label={t("inst.markup", "Default markup (%)")}>
          {() => pct("installment.default_markup_bp")}
        </Field>
        <Field
          label={t("inst.recog", "Markup recognition")}
          hint="When the markup counts as earned income"
        >
          {(id) => (
            <Select
              id={id}
              value={s.get("installment.markup_recognition")}
              disabled={!editable}
              onChange={(e) => s.set("installment.markup_recognition", e.target.value)}
            >
              <option value="proportional_to_collections">
                As collections are received (recommended)
              </option>
              <option value="on_agreement">At agreement start</option>
            </Select>
          )}
        </Field>
        <Field label={t("inst.grace", "Grace days")}>
          {(id) => (
            <Input
              id={id}
              type="number"
              min={0}
              max={365}
              value={s.get("installment.grace_days")}
              disabled={!editable}
              onChange={(e) => s.set("installment.grace_days", e.target.value)}
            />
          )}
        </Field>
        <Field
          label="Rounding remainder"
          hint="Fixed rule: any remainder goes to the final installment"
        >
          {(id) => (
            <Select id={id} disabled value="last_installment">
              <option value="last_installment">Final installment</option>
            </Select>
          )}
        </Field>
        <div className="col-span-2 space-y-3 rounded-lg bg-pearl p-3">
          <Toggle
            label={t("inst.lateEnabled", "Enable late fee")}
            checked={s.get("installment.late_fee_enabled") === "true"}
            disabled={!editable}
            onChange={(v) => s.set("installment.late_fee_enabled", String(v))}
          />
          <Field
            label={t("inst.lateFee", "Late fee (%)")}
            hint="Applied only when enabled; shown as an explicit line"
          >
            {() => pct("installment.late_fee_bp")}
          </Field>
        </div>
      </div>
      {s.error && (
        <p role="alert" className="mt-3 text-[12px] text-danger">
          {s.error}
        </p>
      )}
      {editable && (
        <div className="mt-4 flex justify-end">
          <Button variant="primary" busy={s.busy} onClick={() => void s.save(KEYS, validate)}>
            {t("common.save", "Save")}
          </Button>
        </div>
      )}
    </Card>
  );
}
