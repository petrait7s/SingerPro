import { NAV, type PageId } from "../shell/nav";
import { EmptyState } from "../ui/Card";

/** Honest placeholder: states the build phase instead of faking functionality. */
export function ModulePage({ id }: { id: PageId }) {
  const n = NAV.find((x) => x.id === id)!;
  return <EmptyState title={`${n.label} — scheduled for ${n.phase}`} hint={n.summary} />;
}
