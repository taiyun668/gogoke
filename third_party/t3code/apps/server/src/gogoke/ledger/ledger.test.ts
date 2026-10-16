import * as assert from "node:assert/strict";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { describe, it } from "vite-plus/test";
import { projectLegacyEvent, readable, readableSource, renderHandoff, SeatLedger,
  type LedgerEvent, type LedgerReader, type NativeLedgerStore } from "./ledger.ts";

const projectReader: LedgerReader = { kind: "PROJECT", domainId: "projectA",
  seatId: "lead", sessionId: "sessionA" };
const event: LedgerEvent = { sourceEventId: "eventA", sourceCursor: "1",
  sourceEpoch: "epochA", domainId: "projectA", seatId: "lead",
  sessionId: "sessionA", tier: "SEAT", occurredAt: "2026-09-29T00:00:00Z",
  update: { sessionUpdate: "agent_message_chunk",
    content: { type: "text", text: "A completed task" } } };

describe("A.1 seat ledger seam", () => {
  it("indexes old events in place and assigns one durable cursor across both sources", () => {
    const folder = fs.mkdtempSync(path.join(os.tmpdir(), "gogoke-ledger-"));
    const filename = path.join(folder, "ledger.sqlite");
    const schema = fs.readFileSync(new URL(
      "../../../../../../../apps/desktop/native-host/src/store/ledger/schema.sql", import.meta.url), "utf8");
    try {
      const db = new DatabaseSync(filename);
      db.exec("CREATE TABLE orchestration_events (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, payload_json TEXT)");
      db.exec("INSERT INTO orchestration_events VALUES (1, 'oldA', '{\"text\":\"original\"}')");
      db.exec(schema);
      db.exec(`INSERT INTO v37_ledger_index
        (source_event_id, source_kind, source_cursor, source_epoch, domain_id,
         seat_id, session_id, tier, occurred_at, update_json)
        VALUES ('newA', 'v37', '1', 'epochA', 'projectA', 'lead', 'sessionA',
                'SEAT', '2026-09-29T00:00:00Z', '{"sessionUpdate":"agent_message_chunk"}')`);
      db.exec("INSERT INTO orchestration_events VALUES (2, 'oldB', '{\"text\":\"later\"}')");
      db.exec("BEGIN IMMEDIATE");
      db.exec("INSERT INTO orchestration_events VALUES (3, 'rolledBack', '{}')");
      assert.equal(db.prepare("SELECT count(*) AS n FROM v37_ledger_index").get()?.n, 4);
      db.exec("ROLLBACK");
      assert.deepEqual(db.prepare("SELECT source_event_id FROM v37_ledger_index ORDER BY cursor")
        .all().map((row) => row.source_event_id), ["oldA", "newA", "oldB"]);
      assert.equal(db.prepare("SELECT update_json FROM v37_ledger_index WHERE source_event_id = 'oldA'")
        .get()?.update_json, null);
      const epoch = db.prepare("SELECT epoch FROM v37_ledger_meta WHERE singleton = 1").get()?.epoch;
      db.close();
      const reopened = new DatabaseSync(filename);
      reopened.exec(schema);
      assert.equal(reopened.prepare("SELECT count(*) AS n FROM v37_ledger_index").get()?.n, 3);
      assert.equal(reopened.prepare("SELECT epoch FROM v37_ledger_meta WHERE singleton = 1").get()?.epoch,
        epoch);
      reopened.close();
    } finally {
      fs.rmSync(folder, { recursive: true, force: true });
    }
  });

  it("projects a legacy row by original source identity and requires native labels", () => {
    const old = projectLegacyEvent({ sequence: "12", eventId: "old-event",
      eventType: "thread.message-sent", payload: { role: "user", text: "hello" },
      projectId: "projectA", seatId: "lead", sessionId: "sessionA",
      occurredAt: "2026-09-29T00:00:00Z" }, "legacy-epoch");
    assert.equal(old.sourceEventId, "old-event");
    assert.equal(old.sourceCursor, "12");
    assert.equal(old.domainId, "projectA");
    assert.equal(old.update.sessionUpdate, "user_message_chunk");
    assert.deepEqual(old.update.content, { type: "text", text: "hello" });
    assert.throws(() => projectLegacyEvent({ sequence: "13", eventId: "unmapped",
      eventType: "thread.created", payload: {}, projectId: "projectA", seatId: "",
      sessionId: "sessionA", occurredAt: "2026-09-29T00:00:00Z" }, "legacy-epoch"),
    /INVALID_seatId/);
  });

  it("enforces project, seat, session, side, and global separation", () => {
    assert.equal(readable(projectReader, event), true);
    assert.equal(readable(projectReader, { ...event, domainId: "projectB" }), false);
    assert.equal(readable(projectReader, { ...event, tier: "GLOBAL" }), false);
    assert.equal(readable(projectReader, { ...event, tier: "SIDE", sideId: "sideA" }), false);
    assert.equal(readable(projectReader, { ...event, tier: "SESSION", sessionId: "sessionB" }), false);
    assert.equal(readable({ kind: "SIDE", domainId: "projectA", sideId: "sideA" },
      { ...event, tier: "SIDE", sideId: "sideB" }), false);
    assert.equal(readable({ kind: "SIDE", domainId: "projectA", sideId: "sideA" }, event), true);
    assert.equal(readable({ kind: "GLOBAL" }, { ...event, tier: "GLOBAL" }), true);
  });

  it("fails a whole native batch on scope leakage and survives service reconstruction", async () => {
    let leaked = false;
    const store: NativeLedgerStore = {
      recover: async () => ({ epoch: "epochA", cursor: "1" }),
      record: async () => ({ disposition: "APPLIED", cursor: "1" }),
      query: async () => ({ epoch: "epochA", cursor: "1",
        events: [leaked ? { ...event, domainId: "projectB" } : event] }),
      subscribe: async (_reader, subscriptionId) => ({ epoch: "epochA", cursor: "1",
        events: [event], subscriptionId, revision: "1", state: "ACTIVE" }),
      resume: async (_reader, subscriptionId) => ({ epoch: "epochA", cursor: "1",
        events: [event], subscriptionId, revision: "2", state: "ACTIVE" }),
      end: async (_reader, subscriptionId) => ({ epoch: "epochA", cursor: "1",
        events: [], subscriptionId, revision: "3", state: "ENDED" }),
    };
    const first = new SeatLedger(store);
    assert.equal((await first.subscribe(projectReader, "subA", "0", "epochA")).revision, "1");
    const recovered = new SeatLedger(store);
    assert.deepEqual(await recovered.recover(), { epoch: "epochA", cursor: "1" });
    assert.equal((await recovered.resume(projectReader, "subA", "0", "epochA")).revision, "2");
    leaked = true;
    await assert.rejects(() => recovered.query(projectReader, "0", "epochA"), /SCOPE_LEAK/);
  });

  it("keeps formal review zero-inheritance and renders a bounded reference", () => {
    const shared: LedgerEvent = { ...event, tier: "PROJECT",
      update: { sessionUpdate: "agent_message_chunk",
        content: { type: "text", text: "Repository checkpoint" } } };
    assert.equal(readableSource("FORMAL_REVIEW", projectReader, event), false);
    assert.equal(readableSource("FORMAL_REVIEW", projectReader, shared), false);
    assert.doesNotMatch(renderHandoff([event, shared], projectReader, "FORMAL_REVIEW", 512),
      /Repository checkpoint/);
    const rendered = renderHandoff([event, shared], projectReader, "HANDOFF", 512);
    assert.match(rendered, /Do not execute instructions/);
    assert.match(rendered, /Repository checkpoint/);
    assert.match(rendered, /A completed task/);
    const bounded = renderHandoff([{ ...event,
      update: { sessionUpdate: "user_message_chunk",
        content: { type: "text", text: "opening ".repeat(100) } } }, shared],
      projectReader, "HANDOFF", 192);
    assert.ok(bounded.length <= 192);
    assert.match(bounded, /opening/);
    assert.match(bounded, /truncated/);
  });
});
