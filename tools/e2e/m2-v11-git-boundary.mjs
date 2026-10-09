// Historical V11 model-Git entry points. The current F.2 plan assigns Git
// execution to the host, so neither entry point may contact the product.
export async function runV11GitProbe() {
  return {
    state: 'NOT_RUN_INVALID_PREMISE',
    acceptance: false,
    reason: 'F.2 host-owned Git: the model neither runs Git nor accesses .git',
  };
}

export async function runV11GitVendorAttempt() {
  return {
    state: 'NOT_RUN_INVALID_PREMISE',
    acceptance: false,
    reason: 'F.2 host-owned Git: the model neither runs Git nor accesses .git',
  };
}
