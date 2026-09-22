import { useCallback, useEffect, useState } from "react";
import type { DictationModelStatus } from "../../../types";
import {
  getDictationModelStatus,
} from "../../../services/tauri";
import { subscribeDictationDownload } from "../../../services/events";

type UseDictationModelResult = {
  status: DictationModelStatus | null;
  refresh: () => Promise<void>;
  download: () => Promise<void>;
  cancel: () => Promise<void>;
  remove: () => Promise<void>;
};

export function useDictationModel(modelId: string | null): UseDictationModelResult {
  const [status, setStatus] = useState<DictationModelStatus | null>(null);

  const refresh = useCallback(async () => {
    const next = await getDictationModelStatus(modelId);
    setStatus(next);
  }, [modelId]);

  useEffect(() => {
    let active = true;

    void (async () => {
      try {
        const next = await getDictationModelStatus(modelId);
        if (active) {
          setStatus(next);
        }
      } catch {
        // Ignore dictation status errors during startup.
      }
    })();

    const unlisten = subscribeDictationDownload((event) => {
      if (!active) {
        return;
      }
      if (!modelId || event.modelId === modelId) {
        setStatus(event);
      }
    });

    return () => {
      active = false;
      unlisten();
    };
  }, [modelId]);

  // Model bytes and their configured path remain observable, but no UI action
  // may download, cancel, or remove them while voice is sealed.
  const download = useCallback(async () => {}, []);

  const cancel = useCallback(async () => {}, []);

  const remove = useCallback(async () => {}, []);

  return {
    status,
    refresh,
    download,
    cancel,
    remove,
  };
}
