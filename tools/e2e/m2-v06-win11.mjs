// Strict V06 installed-product adapter. HostRoot owns the actual H session,
// normal close, and immutable readback; this module never invents those facts.
import { runSeatManagementCases } from './m2-seat-management.mjs';

const atom = value => typeof value === 'string' &&
  /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const check = (value, reason) => { if (!value) throw Error(reason); };

/**
 * HostRoot is the caller's original installed-product controller:
 * { product, journal, originalModelAttempt, normalCloseReadbackRestart }.
 * It must send one turn through the already open original H session and return
 * that turn's original IDs. The immutable reader verifies its raw producer.
 */
export async function runV06InstalledCases(product, config, journal, hostRoot) {
  check(process.platform === 'win32' && hostRoot?.product === product &&
    hostRoot.journal === journal &&
    typeof hostRoot.originalModelAttempt === 'function' &&
    typeof hostRoot.normalCloseReadbackRestart === 'function' &&
    atom(config?.seatManagement?.originalSessionId) &&
    config.seatManagement.leadSeat &&
    Array.isArray(config.seatManagement.projects) &&
    config.seatManagement.projects.some(row => row.domainId === config.domainId),
  'V06 requires the original installed HostRoot, its live H session, registered LEAD seat, and immutable readback');

  const seatManagement = {
    ...config.seatManagement,
    originalModelAttempt: details => hostRoot.originalModelAttempt(details),
    normalCloseReadbackRestart: phase => hostRoot.normalCloseReadbackRestart(phase),
  };
  const result = await runSeatManagementCases(product, { ...config, seatManagement }, journal);
  const recorded = journal.seatManagement;
  check(result.acceptance === false && recorded?.acceptance === false &&
    result.state === 'READBACK_COMPLETE_REVIEW_REQUIRED' &&
    recorded.modelAttempt?.sendRequestId && recorded.modelAttempt?.turnId &&
    recorded.leadSeat?.tuneReceipt && recorded.leadSeat?.reclaimReceipt &&
    !recorded.notRun?.some(row =>
      row.caseId === 'MODEL_DENIAL' || row.caseId === 'USER_LEAD_TUNE_AND_RECLAIM'),
  'Original V06 template/model/LEAD subcases did not produce complete native evidence');
  return { ...result, v06Coverage: 'TEMPLATE_COPY_TUNE_MODEL_DENIAL_USER_LEAD_TUNE_RECLAIM',
    remainingV06: ['SET_BOUNDS', 'LEAD_BOUND_REFUSALS', 'BUSY_CHANGE_AND_STOP_RECLAIM',
      'SHORT_TO_LONG', 'LOAD_CAP', 'HOST_CHOICES'], acceptance: false };
}
