# S1-R4 public object contract

`object-model.v1.json` is the checked-in public vocabulary for S1-01-C. The
server codec in `third_party/t3code/apps/server/src/gogoke/contracts/` is the
executable byte boundary.

Wire records are UTF-8 JSON objects with `schema`, `objectType`, and `object`.
V1 fields are required. Unknown minor-version fields are preserved separately
from the known object and never grant authority. Duplicate keys, unknown major
versions, unsafe JSON integer numbers, non-string counters, and values above
`uint64` are rejected before a record reaches product logic.

The vocabulary is exactly the 16 normative objects in the fixed OBJECT_MODEL.
Driver and instance IDs are syntactically open; native model, role, and seat
identities are separately branded, opaque references. An account is represented
only by a non-secret `RuntimeAccountRef` outside this v1 object vocabulary, so
it cannot be confused with a driver, instance, model, role, or seat. Raw
credentials are not part of this contract.
