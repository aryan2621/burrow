import { useEffect, useState } from "react";
import { onEvent } from "./rpc";

/** Live model download progress (id → percent) and the last error per model. */
export default function useDownloads(onFinished: () => void) {
  const [progress, setProgress] = useState<Record<string, number>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  useEffect(() => {
    const un = onEvent(({ method, params }) => {
      const id = params.id;
      if (!id) return;
      if (method === "models.progress" && params.total)
        setProgress((p) => ({ ...p, [id]: Math.floor(((params.done ?? 0) * 100) / params.total!) }));
      if (method === "models.downloaded" || method === "models.download_failed") {
        setProgress(({ [id]: _, ...rest }) => rest);
        if (method === "models.download_failed") setErrors((e) => ({ ...e, [id]: params.error ?? "failed" }));
        onFinished();
      }
    });
    return () => void un.then((f) => f());
  }, []);
  /** Drop progress for downloads the daemon no longer runs (e.g. it restarted mid-download). */
  const settle = (active: string[]) => setProgress((p) => Object.fromEntries(Object.entries(p).filter(([id]) => active.includes(id))));
  return { progress, errors, settle };
}
