# Native child control

References: design 37 sections 2a, 2b and its seat state table allow the lead
to manage its own direct children. Historical gogo-party seat-runtime has
separate delivery, terminal and close controls. The existing H stop intent,
physical Job proof and admission-release transaction remain the implementation;
no scheduler, permission or receipt store is added.

A later sealed model call uses the same current Dispatch grant and copied
Owner scope. It cannot impersonate the earlier reservation call. The existing
stop-intent transaction rechecks that call, exact child/session/generation and
held process custody before recording a new stop. An unresolved generation
change is fenced instead of entering the User-only compound stop path.
The host still records the actual stopped process tail and proof through the
old journal after the stop; it does not require a continuing parent turn to
record a physical fact. Release rechecks the current caller and exact stopped
claim in its existing transaction; only then does the child become IDLE.
Original release replay verifies its old bytes and matching released claim.
E increments the child generation again on BUSY-to-IDLE release. Replay
therefore binds the original H generation to exactly the new Idle generation
minus one, with unchanged incarnation and instance; it cannot follow a later
child generation or a new dispatch. The control receipt labels the current
generation separately from the stopped generation.
The private stop adapter returns the existing K-SESSION admission-release
receipt plus child control metadata. It does not add stop to the closed public
K-SEAT wire; the original H stop failure receipt remains unchanged.
Tool descriptions list the actual admitted payloads. A model's self state card
also exposes references to its own already-answered current-turn native C cards,
reusing C's original source rows. No child answer text, new permission or store
is added. The existing takeover callback still verifies the physical H written
receipt inside E; displaying a reference does not authorize an answer.

Dispatch continues to return submission ACK. It does not block the parent
model from issuing later stop/control calls. State-card/control receipts do
not copy a child's private SESSION ledger. Body reporting still requires the
existing MESSAGE permission and a source-bound communication path; Dispatch
does not grant private transcript access or automatically create MESSAGE rights.
The private dispatch ACK includes the real registered logical worktree ID and
bound seat/generation so the parent can select a later graph/merge operation.
The original H send receipt bytes remain unchanged in their native journal;
the model response retains its identity/status/revisions and adds selectors.
This follows the historical seat-runtime separation of delivery identity and
terminal status, with F's existing registration supplying the worktree ID.
The authority pump also rechecks its snapshot of held sessions before each
drain: a parent's successful stop can remove its child within the same pass.
No missing session is treated as a permission error or recreated, while actual
drain errors on a still-held session remain failures.
Model caller recovery and every transaction revalidation require the existing
H episode to have no committed stop request. The physical stop routine still
captures its original tail before committing STOPPED, but those captured model
calls cannot create effects after the stop intent. This reuses the same H
fence and does not make fact capture depend on a live model authorization.
The state-card adapter follows E's existing readable not-ready result when a
seat has no copied takeover questions. It reports takeoverQuestionsConfigured=false
without inventing questions or completing takeover; core effect authorization
and answer validation retain their strict configured-question checks.

F's BUSY readback is only for the composite seat dispatch's exact original
reservation. Standalone worktree create retains its existing Idle-only check:
its tool and request identity cannot match that original reservation.

Cloud and installed Win11 behavior for these new bytes remain NOT_RUN until
their exact runs complete. A source review or synthetic control is not product
acceptance. Antigravity account isolation and memory-off remain unresolved;
no change to its unsupported login, CLI bytes or LPAC boundary is made here.
