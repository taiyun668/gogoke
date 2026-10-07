import { test } from "node:test";
import { strict as assert } from "node:assert";
import { createDesign37SideChatSource } from "./design37SideChats.ts";

test("side source uses only A text and D receipt states, replaying an unknown question with its original bytes", async () => {
  const asks:string[]=[];
  let attempts=0;
  const source=createDesign37SideChatSource("projectA",async frame=>{
    const request=frame as Record<string,unknown>;
    if (request.schema==="gogoke.37.owner-side-list.v1") return {
      schema:"gogoke.37.side-list.v1",domainId:"projectA",ledgerEpoch:"epochA",ledgerCursor:"1",lead:null,chats:[{
        sideId:"sideA",state:"ACTIVE",seatId:"seatA",seatIncarnation:"incA",
        sourceSeatId:"leadA",sourceSeatIncarnation:"leadIncA",sourceEpoch:"epochA",
        sourceCursor:"1",syncedCursor:"1",revision:"1",
        host:{sessionId:"sessionA",generation:"1",expectedRevision:"2",instanceId:"instanceA",
          driverId:"codex",model:"m",effort:"",answering:false,canAsk:true,questionUnresolved:false},
        transfers:[{id:"sendA",direction:"SIDE_TO_LEAD",sourceSeatId:"seatA",targetSeatId:"leadA",
          body:"tell lead",createdAt:"2026-10-07T00:00:00Z",state:"unknown",reason:"H timeout",nativeReceiptId:""}],
      }],
    };
    if (request.schema==="gogoke.37.owner-side-thread.v1") return {
      schema:"gogoke.37.side-thread.v1",sideId:"sideA",ledgerEpoch:"epochA",
      afterCursor:request.afterCursor,cursor:"2",
      events:request.afterCursor === "0" ? [{cursor:"1",sourceEventId:"eventA",sourceEpoch:"epochA",
        sourceCursor:"1",occurredAt:"2026-10-07T00:00:00Z",
        update:{sessionUpdate:"agent_message_chunk",content:{type:"text",text:"actual answer"}}},
        {cursor:"2",sourceEventId:"eventLate",sourceEpoch:"epochA",sourceCursor:"2",
          occurredAt:"2026-10-07T00:00:01Z",
          update:{sessionUpdate:"agent_message_chunk",content:{type:"text",text:"late answer"}}}] : [],
    };
    if (request.schema==="gogoke.37.owner-side-question.v1") {
      asks.push(request.questionRequest as string);
      return {schema:"gogoke.37.side-sync.v1",sideId:"sideA",
        state:attempts++ === 0 ? "UNKNOWN" : "DELIVERED"};
    }
    throw new Error("unexpected USER operation");
  },()=>"fixedAsk");
  const page=await source.read();
  assert.equal(page.chats[0].messages[0].text,"actual answer");
  assert.equal(page.chats[0].messages.length,1,"late A events are beyond the frozen list head");
  assert.equal(page.chats[0].host?.effort,undefined,"an empty H effort is not fabricated");
  assert.equal(page.chats[0].transfers[0].state,"unknown");
  assert.equal(page.chats[0].transfers[0].reason,"H timeout");
  assert.equal("leadRound" in page,false);
  await assert.rejects(source.actions.ask("sideA","question"),/UNCONFIRMED/);
  await source.actions.ask("sideA","question");
  assert.deepEqual(asks,[asks[0],asks[0]]);
});
