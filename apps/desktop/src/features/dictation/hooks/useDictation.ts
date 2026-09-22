import { useCallback, useEffect, useRef, useState } from "react";
import type {
  DictationSessionState,
  DictationTranscript,
} from "../../../types";

type UseDictationResult = {
  state: DictationSessionState;
  level: number;
  transcript: DictationTranscript | null;
  error: string | null;
  hint: string | null;
  start: (preferredLanguage: string | null) => Promise<void>;
  stop: () => Promise<void>;
  cancel: () => Promise<void>;
  clearTranscript: (id: string) => void;
  clearError: () => void;
  clearHint: () => void;
};

export function useDictation(): UseDictationResult {
  const state: DictationSessionState = "idle";
  const level = 0;
  const [transcript, setTranscript] = useState<DictationTranscript | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [hint, setHint] = useState<string | null>(null);
  const hintTimeoutRef = useRef<number | null>(null);

  // Voice is intentionally sealed for this release. Do not subscribe to the
  // native event stream: a stale event must not make the UI look active or
  // reintroduce a transcript from an already-running legacy capture.
  useEffect(() => {
    return () => {
      if (hintTimeoutRef.current) {
        window.clearTimeout(hintTimeoutRef.current);
        hintTimeoutRef.current = null;
      }
    };
  }, []);

  const start = useCallback(async (_preferredLanguage: string | null) => {
    setError(null);
    setHint(null);
  }, []);

  const stop = useCallback(async () => {}, []);

  const cancel = useCallback(async () => {}, []);

  const clearTranscript = useCallback(
    (id: string) => {
      setTranscript((prev) => (prev?.id === id ? null : prev));
    },
    [],
  );

  const clearError = useCallback(() => {
    setError(null);
  }, []);

  const clearHint = useCallback(() => {
    setHint(null);
    if (hintTimeoutRef.current) {
      window.clearTimeout(hintTimeoutRef.current);
      hintTimeoutRef.current = null;
    }
  }, []);

  return {
    state,
    level,
    transcript,
    error,
    hint,
    start,
    stop,
    cancel,
    clearTranscript,
    clearError,
    clearHint,
  };
}
