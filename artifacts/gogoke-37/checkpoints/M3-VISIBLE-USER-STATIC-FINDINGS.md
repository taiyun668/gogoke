# M3 USER conversation integration: static findings

## Changed

The fresh all-axis static review of `a279e610` found five implementation defects: the new USER route accepted an ordinary CLI model path; the stop cache lacked association identity; the live-state producer and consumer schemas differed; later input could invalidate a fixed history page snapshot; and recovery of an original resume receipt depended on current cache eligibility.

The integration patch closes the ordinary model path, consumes the original UNKNOWN live object, stores the original stop fact with its association under one lock, clears it on association advance, and checks the original session Arc and association when the awaited stop returns. Legacy worktree mutation commands now consult the native route before reaching the compatibility writer, including each child of a removed parent.

The producer worker owns the history snapshot and original-receipt/cache separation fixes. The G feature directories remain owned by Claude.

## Result

This is WIP in a separate integration worktree. The five findings belong to the reviewed commit; neither the current edits nor the new USER conversation have passed native behavior tests. Browser build passed on the reviewed commit. Its Windows run is still collecting the direct compilation/loader result. Stable candidate 0.1.39 remains frozen at `d2267b28` and does not contain this integration patch.

The route defect concerns the product's authorized USER API and the Owner decision that all model sessions run in LPAC. It is not evidence of an LPAC escape. Ordinary-user execution remains limited to fixed CLI login. No capabilities, CLI bytes, credentials or Owner touchpoints are changed.

## Next

Integrate the producer patch, obtain the current Windows result before starting the next source run, and review the five corrections on the exact new commit. Complete stable caller intents, original event/history projection and actual USER conversation wiring before claiming a working conversation.

## Reference

Reviewed the historical gogo-party account lifecycle (`stillMine` checks before asynchronous completion), seat-runtime close handling, the repository reuse blueprint and substrate teardown, and the existing host original-journal/association CAS. The Arc check follows the existing identity rule; the original StopFact and H generation are retained because this host requires physical custody evidence rather than a generic process exit. A fixed source high-water mark and recovery from original receipts remain the host facts; a current cache does not replace either.

## Non-claims

Native execution, production mutation, installed Win11 USER conversation, acceptance and publication: NOT_RUN.
