import { useEffect, useRef, useState } from "react";
import { toApiError } from "./api";

/** Loads async data when `deps` change; `reload()` re-fetches. Errors are surfaced, never swallowed. */
export function useLoad<T>(fn: () => Promise<T>, deps: unknown[]) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  const fnRef = useRef(fn);
  useEffect(() => {
    fnRef.current = fn;
  });
  const key = JSON.stringify(deps);
  useEffect(() => {
    let live = true;
    fnRef
      .current()
      .then((d) => {
        if (live) {
          setData(d);
          setError(null);
        }
      })
      .catch((e) => {
        if (live) setError(toApiError(e).message);
      });
    return () => {
      live = false;
    };
  }, [key, tick]);
  return { data, error, reload: () => setTick((t) => t + 1) };
}
