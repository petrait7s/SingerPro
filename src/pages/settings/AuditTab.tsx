import { useEffect, useState } from "react";
import { api, toApiError, type AuditPage } from "../../lib/api";
import { formatDhaka } from "../../lib/format";
import { Card } from "../../ui/Card";
import { Button, Input } from "../../ui/controls";

const PAGE = 25;

export function AuditTab() {
  const [page, setPage] = useState<AuditPage | null>(null);
  const [offset, setOffset] = useState(0);
  const [prefix, setPrefix] = useState("");
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    api
      .auditList(PAGE, offset, prefix || undefined)
      .then(setPage)
      .catch((e) => setErr(toApiError(e).message));
  }, [offset, prefix]);
  return (
    <Card
      title="Audit log"
      action={
        <Input
          className="!w-48"
          placeholder="Filter e.g. auth. / user. / backup."
          value={prefix}
          onChange={(e) => {
            setOffset(0);
            setPrefix(e.target.value);
          }}
        />
      }
    >
      {err && (
        <p role="alert" className="text-[12px] text-danger">
          {err}
        </p>
      )}
      <table className="w-full text-[12px]">
        <thead>
          <tr className="text-left text-ink-2">
            <th className="pb-2">When (Dhaka)</th>
            <th>Action</th>
            <th>Entity</th>
            <th>Reason</th>
          </tr>
        </thead>
        <tbody>
          {page?.rows.map((r) => (
            <tr key={r.id} className="border-t border-line">
              <td className="py-1.5">{formatDhaka(r.at)}</td>
              <td>{r.action}</td>
              <td>
                {r.entity_type}
                {r.entity_id ? ` · ${r.entity_id.slice(0, 8)}` : ""}
              </td>
              <td>{r.reason ?? ""}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="mt-3 flex items-center justify-between text-[12px] text-ink-2">
        <span>{page ? `${page.total} events` : ""}</span>
        <div className="flex gap-2">
          <Button disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - PAGE))}>
            Newer
          </Button>
          <Button
            disabled={!page || offset + PAGE >= page.total}
            onClick={() => setOffset(offset + PAGE)}
          >
            Older
          </Button>
        </div>
      </div>
    </Card>
  );
}
