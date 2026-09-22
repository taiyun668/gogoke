import { useCallback } from "react";
import { useDictation } from "../../dictation/hooks/useDictation";
import { useDictationModel } from "../../dictation/hooks/useDictationModel";
import { useHoldToDictate } from "../../dictation/hooks/useHoldToDictate";
import type { AppSettings } from "../../../types";

type DictationController = {
  dictationModel: ReturnType<typeof useDictationModel>;
  dictationState: ReturnType<typeof useDictation>["state"];
  dictationLevel: ReturnType<typeof useDictation>["level"];
  dictationTranscript: ReturnType<typeof useDictation>["transcript"];
  dictationError: ReturnType<typeof useDictation>["error"];
  dictationHint: ReturnType<typeof useDictation>["hint"];
  dictationReady: boolean;
  handleToggleDictation: () => Promise<void>;
  clearDictationTranscript: ReturnType<typeof useDictation>["clearTranscript"];
  clearDictationError: ReturnType<typeof useDictation>["clearError"];
  clearDictationHint: ReturnType<typeof useDictation>["clearHint"];
  startDictation: ReturnType<typeof useDictation>["start"];
  stopDictation: ReturnType<typeof useDictation>["stop"];
  cancelDictation: ReturnType<typeof useDictation>["cancel"];
};

export function useDictationController(appSettings: AppSettings): DictationController {
  const dictationModel = useDictationModel(appSettings.dictationModelId);
  const {
    state: dictationState,
    level: dictationLevel,
    transcript: dictationTranscript,
    error: dictationError,
    hint: dictationHint,
    start: startDictation,
    stop: stopDictation,
    cancel: cancelDictation,
    clearTranscript: clearDictationTranscript,
    clearError: clearDictationError,
    clearHint: clearDictationHint,
  } = useDictation();
  // Keep the model status/path visible for preservation checks, but do not
  // allow a ready legacy model to reopen capture during the sealed phase.
  const dictationReady = false;
  const holdDictationKey = (appSettings.dictationHoldKey ?? "").toLowerCase();

  // Keep the existing controller surface for callers, but make every entry a
  // sealed no-op until voice reactivation is separately authorized.
  const handleToggleDictation = useCallback(async () => {}, []);

  useHoldToDictate({
    enabled: false,
    ready: dictationReady,
    state: dictationState,
    preferredLanguage: appSettings.dictationPreferredLanguage,
    holdKey: holdDictationKey,
    startDictation,
    stopDictation,
    cancelDictation,
  });

  return {
    dictationModel,
    dictationState,
    dictationLevel,
    dictationTranscript,
    dictationError,
    dictationHint,
    dictationReady,
    handleToggleDictation,
    clearDictationTranscript,
    clearDictationError,
    clearDictationHint,
    startDictation,
    stopDictation,
    cancelDictation,
  };
}
