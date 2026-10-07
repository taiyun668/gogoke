/** USER pipe source for the native D/A/H side-chat facts. The UI may render
 * only fields that are present; ledger cursors are not lead round counts. */
const OPERATIONS = "gogoke.37.operations.v1";
const LIST = "gogoke.37.owner-side-list.v1";
const THREAD = "gogoke.37.owner-side-thread.v1";
const QUESTION = "gogoke.37.owner-side-question.v1";

export type UserSideOperation = (frame: object) => Promise<unknown>;
export type SideDeliveryView = {
  id: string; direction: "SIDE_TO_LEAD" | "LEAD_TO_SIDE";
  sourceSeatId: string; targetSeatId: string; body: string; createdAt: string;
  state: "steered" | "new-turn" | "failed" | "unknown";
  reason: string; nativeReceiptId: string;
};
export type SideMessageView = {
  id: string; role: "user" | "assistant"; text: string; occurredAt: string;
  sourceEpoch: string; sourceCursor: string;
};
export type SideHostView = {
  sessionId: string; generation: string; expectedRevision: string; instanceId: string;
  driverId: string; model: string; effort: string;
  answering: boolean; canAsk: boolean; questionUnresolved: boolean;
};
export type SideChatView = {
  id: string; title: string; state: "ACTIVE" | "ARCHIVED";
  seatId: string; seatIncarnation: string; sourceSeatId: string; sourceSeatIncarnation: string;
  sourceEpoch: string; sourceCursor: string; syncedCursor: string; revision: string;
  host?: SideHostView; messages: SideMessageView[]; transfers: SideDeliveryView[];
};
export type Design37SideChatPage = { domainId: string; chats: SideChatView[] };

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("SIDE_INVALID_RECORD");
  return value as Record<string, unknown>;
}
function string(row: Record<string, unknown>, field: string): string {
  const value = row[field];
  if (typeof value !== "string" || value.length === 0) throw new Error(`SIDE_INVALID_${field}`);
  return value;
}
function flag(row: Record<string, unknown>, field: string): boolean {
  if (typeof row[field] !== "boolean") throw new Error(`SIDE_INVALID_${field}`);
  return row[field] as boolean;
}
function rows(row: Record<string, unknown>, field: string): unknown[] {
  if (!Array.isArray(row[field])) throw new Error(`SIDE_INVALID_${field}`);
  return row[field] as unknown[];
}
function optionalHost(value: unknown): SideHostView | undefined {
  if (value === null) return undefined;
  const row = record(value);
  return { sessionId: string(row,"sessionId"), generation: string(row,"generation"),
    expectedRevision: string(row,"expectedRevision"), instanceId: string(row,"instanceId"),
    driverId: string(row,"driverId"), model: string(row,"model"), effort: string(row,"effort"),
    answering: flag(row,"answering"), canAsk: flag(row,"canAsk"),
    questionUnresolved: flag(row,"questionUnresolved") };
}
function transfer(value: unknown): SideDeliveryView {
  const row = record(value);
  const direction = string(row,"direction");
  const state = string(row,"state");
  if (direction !== "SIDE_TO_LEAD" && direction !== "LEAD_TO_SIDE") throw new Error("SIDE_INVALID_DIRECTION");
  if (!["steered","new-turn","failed","unknown"].includes(state)) throw new Error("SIDE_INVALID_DELIVERY_STATE");
  return { id:string(row,"id"), direction, sourceSeatId:string(row,"sourceSeatId"),
    targetSeatId:string(row,"targetSeatId"), body:string(row,"body"),
    createdAt:string(row,"createdAt"), state:state as SideDeliveryView["state"],
    reason:typeof row.reason === "string" ? row.reason : "",
    nativeReceiptId:typeof row.nativeReceiptId === "string" ? row.nativeReceiptId : "" };
}
function list(value: unknown, domainId: string): SideChatView[] {
  const page = record(value);
  if (page.schema !== "gogoke.37.side-list.v1" || page.domainId !== domainId) throw new Error("SIDE_INVALID_LIST");
  return rows(page,"chats").map(value => {
    const row=record(value), state=string(row,"state");
    if (state !== "ACTIVE" && state !== "ARCHIVED") throw new Error("SIDE_INVALID_CHAT_STATE");
    return { id:string(row,"sideId"), title:"旁聊", state,
      seatId:string(row,"seatId"), seatIncarnation:string(row,"seatIncarnation"),
      sourceSeatId:string(row,"sourceSeatId"), sourceSeatIncarnation:string(row,"sourceSeatIncarnation"),
      sourceEpoch:string(row,"sourceEpoch"), sourceCursor:string(row,"sourceCursor"),
      syncedCursor:string(row,"syncedCursor"), revision:string(row,"revision"),
      host:optionalHost(row.host), messages:[], transfers:rows(row,"transfers").map(transfer) };
  });
}
function messages(value: unknown, sideId: string, epoch: string, after: string): {cursor:string;events:SideMessageView[]} {
  const page=record(value);
  if (page.schema !== "gogoke.37.side-thread.v1" || page.sideId !== sideId ||
      page.ledgerEpoch !== epoch || page.afterCursor !== after) throw new Error("SIDE_INVALID_THREAD");
  const events:SideMessageView[]=[];
  for (const value of rows(page,"events")) {
    const row=record(value), update=record(row.update), kind=string(update,"sessionUpdate");
    if (kind !== "user_message_chunk" && kind !== "agent_message_chunk") continue;
    const content=record(update.content);
    if (content.type !== "text" || typeof content.text !== "string" || !content.text) continue;
    events.push({id:string(row,"sourceEventId"), role:kind === "user_message_chunk" ? "user" : "assistant",
      text:content.text, occurredAt:string(row,"occurredAt"),
      sourceEpoch:string(row,"sourceEpoch"), sourceCursor:string(row,"sourceCursor")});
  }
  return {cursor:string(page,"cursor"),events};
}

/** Only operations with a current native USER route are exposed. Creation
 * needs its separate current lead/instance/model choice seam, so it is absent. */
export function createDesign37SideChatSource(domainId:string, execute:UserSideOperation,
    requestId:()=>string = () => `side_${crypto.randomUUID()}`) {
  const pendingAsk=new Map<string,{body:string;questionRequest:string}>();
  const readList=async()=>list(await execute({schema:LIST,domainId}),domainId);
  const current=async(sideId:string)=>{
    const side=(await readList()).find(side=>side.id===sideId);
    if (!side) throw new Error("SIDE_NOT_FOUND");
    return side;
  };
  const operate=async(sideId:string,operation:"archive"|"restore"|"delete")=>{
    const side=await current(sideId);
    const id=requestId();
    const receipt=record(await execute({schema:OPERATIONS,family:"K-SIDE",operation,
      requestId:id,targetId:sideId,domainId,expectedRevision:side.revision,payload:{}}));
    if (receipt.schema!==OPERATIONS || receipt.requestId!==id || receipt.operation!==operation ||
        receipt.targetId!==sideId || !["APPLIED","REPLAYED"].includes(String(receipt.status))) {
      throw new Error(`SIDE_${operation.toUpperCase()}_UNCONFIRMED: ${JSON.stringify(receipt)}`);
    }
  };
  return {
    read:async():Promise<Design37SideChatPage>=>{
      const chats=await readList();
      for (const chat of chats) {
        let after="0";
        for (;;) {
          const page=messages(await execute({schema:THREAD,domainId,sideId:chat.id,
            ledgerEpoch:chat.sourceEpoch,afterCursor:after}),chat.id,chat.sourceEpoch,after);
          chat.messages.push(...page.events);
          if (page.cursor===after) break;
          if (BigInt(page.cursor)<BigInt(after)) throw new Error("SIDE_THREAD_CURSOR_REGRESSION");
          after=page.cursor;
        }
        const first=chat.messages.find(item=>item.role==="user")?.text.trim();
        if (first) chat.title=first.slice(0,48);
      }
      return {domainId,chats};
    },
    actions:{
      ask:async(sideId:string,body:string)=>{
        const side=await current(sideId), host=side.host;
        const old=pendingAsk.get(sideId);
        if (old && old.body!==body) throw new Error("SIDE_PREVIOUS_QUESTION_UNCONFIRMED");
        if (!old && !host?.canAsk) throw new Error(host?.questionUnresolved ? "SIDE_QUESTION_UNCONFIRMED" : "SIDE_HOST_UNAVAILABLE");
        const questionRequest=old ? old.questionRequest : JSON.stringify({schema:OPERATIONS,family:"K-SESSION",
          operation:"send",requestId:requestId(),targetId:host!.sessionId,domainId,
          expectedRevision:host!.expectedRevision,payload:{body,generation:host!.generation}});
        pendingAsk.set(sideId,{body,questionRequest});
        const receipt=record(await execute({schema:QUESTION,sideId,questionRequest}));
        if (receipt.schema!=="gogoke.37.side-sync.v1" || receipt.sideId!==sideId || receipt.state!=="DELIVERED")
          throw new Error(`SIDE_QUESTION_UNCONFIRMED: ${JSON.stringify(receipt)}`);
        pendingAsk.delete(sideId);
      },
      stop:async(sideId:string)=>{
        const side=await current(sideId), host=side.host;
        if (!host) throw new Error("SIDE_HOST_UNAVAILABLE");
        const id=requestId();
        const receipt=record(await execute({schema:OPERATIONS,family:"K-SESSION",operation:"stop",
          requestId:id,targetId:host.sessionId,domainId,expectedRevision:host.expectedRevision,
          payload:{seatId:side.seatId,generation:host.generation}}));
        if (receipt.schema!==OPERATIONS || receipt.requestId!==id || receipt.operation!=="stop" ||
            !["APPLIED","REPLAYED"].includes(String(receipt.status)))
          throw new Error(`SIDE_STOP_UNCONFIRMED: ${JSON.stringify(receipt)}`);
      },
      archive:(sideId:string)=>operate(sideId,"archive"),
      restore:(sideId:string)=>operate(sideId,"restore"),
      remove:(sideId:string)=>operate(sideId,"delete"),
    },
  };
}
