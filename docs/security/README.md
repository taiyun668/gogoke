# P26 Assurance Baseline

This directory holds the machine-readable P26 assurance baseline.  It is a
design and conformance-structure artifact, not product-security evidence.

- `threat-register.json` is the authoritative threat register.  A threat must
  have a stable ID, asset, trust boundary, control, evidence requirement, gate,
  accountable owner, risk status, and product-assurance status.
- `audit-vocabulary.json` defines stable audit-event names and the minimum
  fields that implementations must retain without recording secrets.
- `tools/assurance/validate_assurance_baseline.py` validates the two documents.

## Status vocabulary

`PASS`, `FAIL`, `WARN`, `INCONCLUSIVE`, `NOT_RUN`, and `BLOCKED` are distinct.
In this baseline, a structural validator may report `PASS` only for the
baseline document itself.  Product-security, isolation, credential, Adapter,
installation, upgrade, rollback, and RC claims remain `INCONCLUSIVE` until
their independent product gates have reproducible evidence.

`NOT_RUN` is never evidence of success.  A gate that depends on `NOT_RUN`
evidence must not report product `PASS`; the validator rejects that shape.

## Ownership and evidence boundary

`P26-W10-INDEPENDENT` owns register completeness and independent assurance
review.  It does not own implementation results or an Owner release decision.
The implementation work package named in each record supplies witnesses; the
named product gate and independent reviewer determine any future verdict.
