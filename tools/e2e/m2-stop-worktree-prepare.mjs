// Prepare one private M2 F stop-gate fixture through the installed User bridge.
// Importing this module starts nothing; run it only through the explicit CLI entry.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { ActualProduct, readJson, sha256, id } from './product-cdp.mjs';

async function main() {
const here = path.dirname(fileURLToPath(import.meta.url));
const configPath = process.argv[2];
if (!configPath) throw Error('Private M2 stop-worktree preparation config path is required');
const config = readJson(configPath);
const c = config.stopWorktreePrep;
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const inside = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const check = (condition, reason) => { if (!condition) throw Error(reason); };
const required = ['installed', 'version', 'sourceCommit', 'installedSha256', 'registryKey',
  'pwsh', 'python', 'evidenceDirectory', 'result', 'stateRoot', 'testbedSource',
  'domainId', 'repositoryId', 'instanceId', 'stopWorktreePrep'];

if (process.platform !== 'win32' || required.some(key => config[key] === undefined) ||
    !/^\d+\.\d+\.\d+$/.test(config.version) || !/^[a-f0-9]{40}$/.test(config.sourceCommit) ||
    config.repositoryId !== 'gogokeSeatTestbed' || !atom(config.domainId) ||
    !atom(config.instanceId) ||
    c.lifecycleOwnership !== 'EXCLUSIVE_M2_STOP_WORKTREE_PREP' || !atom(c.templateId) ||
    typeof config.installedSha256 !== 'object' || config.installedSha256 === null ||
    !['gogoke.exe', 'gogoke-native-host.exe', 'resource-index.json']
      .every(name => /^[a-f0-9]{64}$/.test(config.installedSha256[name] ?? '')) ||
    !['installed', 'pwsh', 'python', 'stateRoot', 'testbedSource', 'evidenceDirectory', 'result']
      .every(key => typeof config[key] === 'string') ||
    !['installed', 'pwsh', 'python', 'stateRoot', 'testbedSource', 'evidenceDirectory', 'result']
      .every(key => path.isAbsolute(config[key])) ||
    !fs.existsSync(config.evidenceDirectory) || !fs.existsSync(config.testbedSource) ||
    fs.existsSync(config.result) ||
    !inside(path.resolve(config.result), path.resolve(config.evidenceDirectory)) ||
    path.dirname(path.resolve(config.result)) !== path.resolve(config.evidenceDirectory) ||
    [config.installed, config.stateRoot, config.testbedSource, path.resolve(here, '..', '..')]
      .some(root => inside(path.resolve(config.evidenceDirectory), path.resolve(root))) ||
    inside(path.resolve(config.stateRoot), path.resolve(config.evidenceDirectory)) ||
    path.resolve(config.stateRoot) === path.resolve(config.testbedSource) ||
    config.testerArmy === false) {
  throw Error('Fresh private stop-worktree preparation requires an exact installed candidate, testbed source, isolated evidence, and original User bridge');
}

const seatId = id('m2StopUser');
const worktreeId = id('m2StopTree');
const readbackSource = String.raw`import hashlib, json, os, sqlite3, sys
from pathlib import Path

def check(value, reason):
    if not value: raise RuntimeError(reason)
def digest(data): return hashlib.sha256(data).hexdigest()
def rows(db, sql, args=()): return [dict(row) for row in db.execute(sql, args)]
def one(db, sql, args=()):
    found=rows(db,sql,args); check(len(found)==1, f"expected one row, found {len(found)}"); return found[0]
def same_path(a,b): return os.path.normcase(os.path.realpath(str(a))) == os.path.normcase(os.path.realpath(str(b)))
def wire(journal, request_id):
    matches=[item for item in journal["operations"] if item.get("request",{}).get("requestId")==request_id]
    check(len(matches)==1,"original request missing or duplicated")
    item=matches[0]; check(json.loads(item["rawFrame"])==item["request"] and item.get("rawReceiptSha256"),"original request/receipt binding missing")
    check(item["receipt"].get("requestId")==request_id and item["receipt"].get("domainId")==item["request"].get("domainId") and item["receipt"].get("targetId")==item["request"].get("targetId"),"sanitized native receipt identity differs")
    return item
def seat_op(journal, request_id, db, parts):
    item=wire(journal,request_id); request=item["request"]
    pre=bytearray()
    for part in parts:
        data=part.encode("utf-8"); pre.extend(len(data).to_bytes(8,"big")); pre.extend(data)
    raw=item["rawFrame"].encode("utf-8"); pre.extend(len(raw).to_bytes(8,"big")); pre.extend(raw)
    saved=one(db,"SELECT o.fingerprint,o.seat_id,o.incarnation,o.layer,COALESCE(o.parent_seat_id,'') parent_seat_id,o.kind,o.instance_id,o.state,o.revision,o.generation,COALESCE(s.template_id,'') template_id,COALESCE(s.settings_json,'') settings_json FROM gogoke_v37_seat_operations o LEFT JOIN gogoke_v37_seat_operation_snapshots s USING(domain_id,request_id) WHERE o.domain_id=? AND o.request_id=?",(request["domainId"],request_id))
    check(saved["fingerprint"]=="sha256:"+hashlib.sha256(pre).hexdigest() and saved["seat_id"]==request["targetId"],"User seat operation fingerprint/target differs")
    return item,saved
def check_optional_ledger(item, db):
    request=item["request"]
    found=rows(db,"SELECT request_bytes,receipt_bytes FROM v37_ledger_receipt WHERE family=? AND domain_id=? AND request_id=?",(request["family"],request["domainId"],request["requestId"]))
    if request["operation"]=="state-card": check(len(found)==1,"Original K-SEAT state-card ledger receipt is missing")
    if found:
        check(len(found)==1 and bytes(found[0]["request_bytes"]).decode()==item["rawFrame"] and digest(bytes(found[0]["receipt_bytes"]))==item["rawReceiptSha256"],"Original User ledger request/receipt bytes differ")

root=Path(sys.argv[1]).resolve(strict=True); output=Path(sys.argv[2]).resolve(); journal_path=Path(sys.argv[3]).resolve(strict=True); phase=sys.argv[4]
check(phase in ("baseline","final") and not output.exists() and not output.is_relative_to(root),"Fresh read-only proof path and phase required")
journal=json.loads(journal_path.read_text(encoding="utf-8-sig")); case=journal.get("stopWorktreePrepare")
check(case and case["schema"]=="gogoke.37.m2-stop-worktree-prepare.v1" and case["acceptance"] is False and case["sourceCommit"]==journal["sourceCommit"] and Path(case["stateRoot"]).resolve(strict=True)==root,"Original preparation case/root required")
evidence=Path(case["evidenceDirectory"]).resolve(strict=True)
check(output.parent==journal_path.parent==evidence,"Readback output and original journal must share the private evidence directory")
launches,closes=journal.get("launches",[]),journal.get("closes",[]); launch=launches[-1] if launches else {}; close=closes[-1] if closes else {}; endpoint=journal.get("currentEndpoint",{}); boot=launch.get("bootstrap",{})
check(launch.get("pid")==close.get("pid")==endpoint.get("pid") and close.get("exitCode")==0 and close.get("forceKill") is False and launch.get("sourceCommit")==case["sourceCommit"] and boot.get("version")==case["candidateVersion"] and launch.get("setId")==boot.get("setId") and launch.get("generationId")==boot.get("generationId"),"Latest exact candidate launch lacks its normal-close receipt")
installed=case["candidateInstalledSha256"]; check(isinstance(installed,dict) and installed and all(isinstance(v,str) and len(v)==64 for v in installed.values()),"Actual installed byte pins required")
st=root.stat(); dbfile=root/"state.sqlite"; dbst=dbfile.stat(); root_identity={"observer":"python-stat","device":str(st.st_dev),"inode":str(st.st_ino),"databaseDevice":str(dbst.st_dev),"databaseInode":str(dbst.st_ino)}
candidate={"sourceCommit":launch["sourceCommit"],"version":boot["version"],"setId":launch["setId"],"generationId":launch["generationId"]}
if phase=="final":
    ref=case.get("baseline"); check(ref and Path(ref["file"]).name==ref["file"],"Exact baseline reference required")
    baseline_path=evidence/ref["file"]; check(digest(baseline_path.read_bytes())==ref["sha256"],"Original baseline proof bytes changed")
    baseline=json.loads(baseline_path.read_text(encoding="utf-8-sig"))
    check(baseline.get("phase")=="baseline" and baseline.get("caseId")==journal["caseId"] and baseline.get("rootIdentity")==root_identity and baseline.get("candidateIdentity")==candidate and baseline.get("candidateInstalledSha256")==installed,"Final candidate/root differs from baseline")
database=root/"state.sqlite"; wal=Path(str(database)+"-wal"); shm=Path(str(database)+"-shm")
check(database.is_file() and (not wal.exists() or wal.stat().st_size==0),"Normal close and empty/absent WAL required")
def files(): return {p.name:{"length":p.stat().st_size,"sha256":digest(p.read_bytes())} for p in (database,wal,shm) if p.exists()}
before=files(); template_id=case["templateId"]; domain=case["domainId"]; seat_id=case["seatId"]; worktree_id=case["worktreeId"]; repo_id=case["repositoryId"]
with sqlite3.connect(database.as_uri()+"?mode=ro&immutable=1",uri=True) as db:
    db.row_factory=sqlite3.Row; db.execute("PRAGMA query_only=ON")
    template=one(db,"SELECT settings_json,revision FROM gogoke_v37_seat_templates WHERE domain_id=? AND template_id=?",(domain,template_id)); template_hash=digest(template["settings_json"].encode())
    source=one(db,"SELECT source_path,source_identity,common_path,common_identity,baseline_commit,git_digest,git_version,revision FROM gogoke_v37_worktree_sources WHERE repository_id=?",(repo_id,))
    check(same_path(source["source_path"],case["testbedSource"]) and source["revision"]==1,"Actual registered testbed source path/revision differs")
    program=one(db,"SELECT git_path FROM gogoke_v37_worktree_programs WHERE repository_id=?",(repo_id,))["git_path"]
    program_path=Path(program).resolve(strict=True); program_hash=digest(program_path.read_bytes()); pinned=source["git_digest"].removeprefix("sha256:")
    check(program_hash==pinned,"Registered Git executable bytes differ from the exact F pin")
    seat_rows=rows(db,"SELECT * FROM gogoke_v37_seats WHERE domain_id=? AND seat_id=?",(domain,seat_id)); settings_rows=rows(db,"SELECT template_id,settings_json FROM gogoke_v37_seat_settings WHERE domain_id=? AND seat_id=?",(domain,seat_id))
    worktree_ops=rows(db,"SELECT request_id,request_hash,repository_id,domain_id,seat_id,worktree_id,seat_incarnation,seat_generation,seat_revision,instance_id,permission_tier,phase FROM gogoke_v37_worktree_operations WHERE worktree_id=?",(worktree_id,))
    worktree_rows=rows(db,"SELECT * FROM gogoke_v37_worktrees WHERE worktree_id=?",(worktree_id,)); lifecycle=rows(db,"SELECT state,revision,COALESCE(stop_fact_id,'') stop_fact_id FROM gogoke_v37_worktree_lifecycle WHERE worktree_id=?",(worktree_id,))
    if phase=="baseline":
        check(not seat_rows and not settings_rows and not worktree_ops and not worktree_rows and not lifecycle,"Exact generated seat/worktree IDs were not absent at baseline")
        facts={"templateSettingsSha256":template_hash,"templateRevision":template["revision"],"testbedSource":str(Path(source["source_path"]).resolve(strict=True)),"testbedSourceIdentity":source["source_identity"],"commonIdentity":source["common_identity"],"gitPathSha256":program_hash,"gitVersion":source["git_version"],"targetSeatAbsent":True,"targetWorktreeAbsent":True}
    else:
        check(len(seat_rows)==len(settings_rows)==len(worktree_ops)==len(worktree_rows)==len(lifecycle)==1,"Original single created/registered seat and worktree rows required")
        seat=seat_rows[0]; settings=settings_rows[0]; fop=worktree_ops[0]; tree=worktree_rows[0]; life=lifecycle[0]
        create=wire(journal,case["createRequestId"]); bind=wire(journal,case["bindRequestId"]); card=wire(journal,case["cardRequestId"]); create_tree=wire(journal,case["worktreeCreateRequestId"]); register=wire(journal,case["worktreeRegisterRequestId"]); graph=wire(journal,case["graphQueryRequestId"])
        check_optional_ledger(create,db); check_optional_ledger(bind,db); check_optional_ledger(card,db); check_optional_ledger(create_tree,db); check_optional_ledger(register,db); check_optional_ledger(graph,db)
        created,create_row=seat_op(journal,case["createRequestId"],db,["create",domain,seat_id,template_id,"","LONG","USER","","",""])
        bound,bind_row=seat_op(journal,case["bindRequestId"],db,["bind-instance",domain,seat_id,"1","1",case["instanceId"],"","",""])
        check(created["receipt"]["status"]==bound["receipt"]["status"]==card["receipt"]["status"]=="APPLIED" and create_row["layer"]==bind_row["layer"]=="USER" and create_row["template_id"]==bind_row["template_id"]==template_id and digest(create_row["settings_json"].encode())==digest(bind_row["settings_json"].encode())==template_hash,"Original User template copy/bind facts or source template hash differ")
        check(seat["layer"]=="USER" and seat["kind"]=="LONG" and seat["state"]=="IDLE" and seat["instance_id"]==case["instanceId"] and seat["parent_seat_id"] in (None,"") and seat["revision"]==2 and seat["generation"]==2 and settings["template_id"]==template_id and digest(settings["settings_json"].encode())==template_hash,"Final exact User seat is not idle/template-copied/instance-bound")
        check(card["receipt"]["result"]["state"]=="IDLE" and card["receipt"]["result"]["layer"]=="USER" and card["receipt"]["result"]["instanceId"]==case["instanceId"],"Original User state-card differs")
        check(fop["request_id"]==case["worktreeCreateRequestId"] and fop["request_hash"]==digest(create_tree["rawFrame"].encode()) and fop["repository_id"]==repo_id and fop["domain_id"]==domain and fop["seat_id"]==seat_id and fop["worktree_id"]==worktree_id and fop["phase"]=="REGISTERED" and fop["seat_incarnation"]==seat["incarnation"] and fop["seat_generation"]==2 and fop["seat_revision"]==2 and fop["instance_id"]==case["instanceId"],"Original F create request hash/binding differs")
        check(register["request"]["family"]=="K-WORKTREE" and register["request"]["operation"]=="register" and register["request"]["expectedRevision"]=="1" and register["receipt"]["status"]=="APPLIED" and register["receipt"]["result"]["worktreeId"]==worktree_id,"Original F register receipt differs")
        register_op=one(db,"SELECT request_hash,worktree_id,operation,phase FROM gogoke_v37_worktree_lifecycle_ops WHERE request_id=?",(case["worktreeRegisterRequestId"],))
        check(register_op["request_hash"]==digest(register["rawFrame"].encode()) and register_op["worktree_id"]==worktree_id and register_op["operation"]=="REGISTER" and register_op["phase"]=="APPLIED" and life["state"]=="REGISTERED" and life["revision"]==2,"Original F lifecycle registration proof differs")
        check(tree["repository_id"]==repo_id and tree["domain_id"]==domain and tree["seat_id"]==seat_id and tree["instance_id"]==case["instanceId"] and tree["state"]=="REGISTERED" and tree["seat_incarnation"]==seat["incarnation"] and tree["seat_generation"]==2 and tree["seat_revision"]==2 and tree["baseline_commit"]==source["baseline_commit"],"Original F worktree durable identity differs")
        path=Path(tree["worktree_path"]).resolve(strict=True); single_root=(root/"v37-worktrees"/"single").resolve(strict=True); dotgit=path/".git"
        check(single_root in path.parents and path.is_dir() and not path.is_symlink() and dotgit.is_file() and not dotgit.is_symlink(),"Actual single worktree path/pointer is outside candidate-owned tree root")
        graph_result=graph["receipt"]["result"]; members=graph_result.get("members",[])
        check(graph["receipt"]["status"]=="APPLIED" and graph_result["state"]=="REGISTERED" and graph_result["classification"]=="SINGLE" and len(members)==1 and members[0]["worktreeId"]==worktree_id and members[0]["repositoryId"]==repo_id and members[0]["domainId"]==domain and members[0]["seatId"]==seat_id and members[0]["instanceId"]==case["instanceId"],"Original User graph-query is not exactly one registered SINGLE member")
        facts={"templateSettingsSha256":template_hash,"templateRevision":template["revision"],"seat":{"seatId":seat_id,"layer":seat["layer"],"kind":seat["kind"],"state":seat["state"],"revision":str(seat["revision"]),"generation":str(seat["generation"]),"incarnation":seat["incarnation"],"templateId":settings["template_id"],"settingsSha256":digest(settings["settings_json"].encode()),"instanceId":seat["instance_id"]},"worktree":{"worktreeId":worktree_id,"repositoryId":tree["repository_id"],"domainId":tree["domain_id"],"seatId":tree["seat_id"],"instanceId":tree["instance_id"],"state":tree["state"],"lifecycleState":life["state"],"lifecycleRevision":life["revision"],"classification":graph_result["classification"],"path":str(path),"pathDevice":str(path.stat().st_dev),"pathInode":str(path.stat().st_ino),"gitPointerSha256":digest(dotgit.read_bytes()),"baselineCommit":tree["baseline_commit"],"createRequestId":case["worktreeCreateRequestId"],"registerRequestId":case["worktreeRegisterRequestId"],"graphRequestId":case["graphQueryRequestId"]},"testbedSource":str(Path(source["source_path"]).resolve(strict=True)),"testbedSourceIdentity":source["source_identity"],"commonIdentity":source["common_identity"],"gitPathSha256":program_hash,"gitVersion":source["git_version"]}
after=files(); check(before==after and (not wal.exists() or wal.stat().st_size==0),"Immutable readback changed DB/WAL/SHM bytes")
proof={"schema":"gogoke.37.private-m2-stop-worktree-prepare-readback.v1","phase":phase,"caseId":journal["caseId"],"sourceCommit":case["sourceCommit"],"domainId":domain,"stateRoot":str(root),"evidenceDirectory":str(evidence),"readerSha256":case["readerSha256"],"candidateIdentity":candidate,"candidateInstalledSha256":installed,"rootIdentity":root_identity,"launch":{"pid":launch["pid"],"sourceCommit":launch["sourceCommit"]},"normalClose":{"pid":close["pid"],"exitCode":close["exitCode"],"forceKill":close["forceKill"]},"databaseSha256":digest(database.read_bytes()),"filesBefore":before,"filesAfter":after,"measurementPreservedDatabaseBytes":before==after,"directCaseEvidence":phase=="final","acceptance":False,**facts}
output.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
print(json.dumps({"phase":phase,"directCaseEvidence":proof["directCaseEvidence"],"acceptance":False}))
`;

const readerSha256 = createHash('sha256').update(readbackSource).digest('hex');
const journal = { schema: 'gogoke.37.m2-stop-worktree-prepare-win11.v1', caseId: id('m2StopPrep'),
  sourceCommit: config.sourceCommit, domainId: config.domainId, repositoryId: config.repositoryId,
  stateRoot: path.resolve(config.stateRoot), evidenceDirectory: path.resolve(config.evidenceDirectory),
  candidateVersion: config.version, candidateInstalledSha256: config.installedSha256,
  state: 'RUNNING', acceptance: false, operations: [], launches: [], closes: [], readbacks: [], sessions: [] };
const product = new ActualProduct(config, journal);
const record = { schema: 'gogoke.37.m2-stop-worktree-prepare.v1', state: 'RUNNING', acceptance: false,
  sourceCommit: config.sourceCommit, domainId: config.domainId, repositoryId: config.repositoryId,
  stateRoot: path.resolve(config.stateRoot), testbedSource: path.resolve(config.testbedSource),
  evidenceDirectory: path.resolve(config.evidenceDirectory), candidateVersion: config.version,
  candidateInstalledSha256: config.installedSha256, instanceId: config.instanceId,
  templateId: c.templateId,
  seatId, worktreeId, readerSha256, createRequestId: null, bindRequestId: null,
  cardRequestId: null, worktreeCreateRequestId: null, worktreeRegisterRequestId: null,
  graphQueryRequestId: null, baseline: null, final: null };
journal.driverBytes['m2-stop-worktree-prepare.mjs'] = sha256(path.join(here, 'm2-stop-worktree-prepare.mjs'));
const save = () => { journal.stopWorktreePrepare = record; product.save(); };

async function userOperation(family, operation, targetId, payload, expectedRevision = '0',
  allowed = ['APPLIED']) {
  const request = { schema: 'gogoke.37.operations.v1', family, operation,
    requestId: id('m2StopPrepRequest'), domainId: config.domainId, targetId,
    expectedRevision: String(expectedRevision), payload };
  const rawFrame = JSON.stringify(request);
  const entry = { request, rawFrame, rawReceiptSha256: null, receipt: null,
    startedAt: new Date().toISOString() };
  journal.operations.push(entry); save();
  try {
    const rawReceipt = await product.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(rawFrame)}})`);
    entry.rawReceiptSha256 = createHash('sha256').update(rawReceipt).digest('hex');
    const receipt = JSON.parse(rawReceipt);
    if (family === 'K-SEAT' && receipt.result && typeof receipt.result.settings === 'object') {
      receipt.result = { ...receipt.result, settingsSha256: createHash('sha256')
        .update(JSON.stringify(receipt.result.settings)).digest('hex') };
      delete receipt.result.settings;
    }
    entry.receipt = receipt; entry.finishedAt = new Date().toISOString(); save();
  } catch (error) {
    entry.originalError = String(error?.stack ?? error); save(); throw error;
  }
  check(entry.receipt.schema === request.schema && entry.receipt.requestId === request.requestId &&
    entry.receipt.domainId === request.domainId && entry.receipt.targetId === targetId &&
    entry.receipt.family === family && entry.receipt.operation === operation &&
    allowed.includes(entry.receipt.status),
  `Original User ${family}/${operation} returned ${entry.receipt.status}; no replay`);
  return entry;
}

async function stateCard(targetId, allowed = ['APPLIED']) {
  let entry = await userOperation('K-SEAT', 'state-card', targetId, {}, '0', [...allowed, 'STALE']);
  if (entry.receipt.status === 'STALE') {
    entry = await userOperation('K-SEAT', 'state-card', targetId, {}, entry.receipt.revision, allowed);
  }
  return entry;
}

async function graphQuery() {
  let entry = await userOperation('K-WORKTREE', 'graph-query', worktreeId, {}, '0', ['APPLIED', 'STALE']);
  if (entry.receipt.status === 'STALE') {
    entry = await userOperation('K-WORKTREE', 'graph-query', worktreeId, {}, entry.receipt.revision);
  }
  return entry;
}

async function immutableReadback(phase) {
  await product.closeNormally();
  const file = `m2-stop-worktree-prepare-${phase}.json`;
  const output = path.join(config.evidenceDirectory, file);
  check(!fs.existsSync(output), `Original ${phase} private readback output already exists`);
  const stdout = await new Promise((resolve, reject) => {
    const child = spawn(config.python, ['-c', readbackSource, config.stateRoot, output, config.result, phase],
      { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '', err = '';
    child.stdout.on('data', bytes => { out = (out + bytes).slice(-8192); });
    child.stderr.on('data', bytes => { err = (err + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve(out) :
      reject(Error(`Private stop-worktree ${phase} immutable readback exit=${code}; stdout=${out}; stderr=${err}`)));
  });
  const proof = readJson(output);
  check(proof.schema === 'gogoke.37.private-m2-stop-worktree-prepare-readback.v1' &&
    proof.phase === phase && proof.caseId === journal.caseId && proof.sourceCommit === config.sourceCommit &&
    proof.domainId === config.domainId && proof.readerSha256 === readerSha256 &&
    proof.stateRoot === record.stateRoot && proof.evidenceDirectory === record.evidenceDirectory &&
    proof.measurementPreservedDatabaseBytes === true && proof.acceptance === false &&
    proof.normalClose?.exitCode === 0 && proof.normalClose.forceKill === false &&
    proof.launch?.pid === proof.normalClose.pid &&
    JSON.stringify(proof.candidateInstalledSha256) === JSON.stringify(config.installedSha256),
  `Original ${phase} readback is not bound to the normally closed installed candidate`);
  const reference = { file, sha256: sha256(output) };
  journal.readbacks.push({ phase, ...reference, acceptance: false }); save();
  return { reference, proof, stdout: String(stdout).trim() };
}

async function assertLoggedInInstance() {
  const snapshot = await product.instances();
  const instance = snapshot.instances.find(row => row.instanceId === config.instanceId);
  check(instance?.driverId === 'codex' && instance.state === 'LOGGED_IN',
    'Preparation requires the existing already logged-in Codex test instance');
  record.instanceState = { instanceId: instance.instanceId, driverId: instance.driverId,
    version: instance.version, revision: instance.revision, state: instance.state };
  save();
}

try {
  journal.stopWorktreePrepare = record; product.save();
  await product.launch(); await product.custody(); product.verifyBytes();
  const ui = await product.evaluate('({url:location.href,home:!!document.querySelector(".home-product-entry"),tauri:!!window.__TAURI_INTERNALS__})');
  check(ui.url === product.endpoint.url && ui.home && ui.tauri && journal.connectionBackend?.agentActs === 0 &&
    journal.connectionBackend?.telemetryDisabled === true, 'Original installed User bridge and no-agent preview backend required');
  await assertLoggedInInstance();
  await product.custody(); product.verifyBytes();
  const baseline = await immutableReadback('baseline');
  check(baseline.proof.directCaseEvidence === false && baseline.proof.targetSeatAbsent === true &&
    baseline.proof.targetWorktreeAbsent === true && baseline.proof.templateSettingsSha256,
  'Private baseline does not prove an existing template and absent generated targets');
  record.baseline = baseline.reference; record.baselineProof = baseline.proof; save();
  await product.launch(); await product.custody(); product.verifyBytes();
  await assertLoggedInInstance();

  const created = await userOperation('K-SEAT', 'create-from-template', seatId,
    { layer: 'USER', templateId: c.templateId }, '0');
  check(created.receipt.result.layer === 'USER' && created.receipt.result.kind === 'LONG' &&
    created.receipt.result.state === 'IDLE' && created.receipt.result.templateId === c.templateId &&
    created.receipt.revision === '1',
  'Original User create-from-template receipt does not match the private template copy');
  record.createRequestId = created.request.requestId; save();

  const bound = await userOperation('K-SEAT', 'bind-instance', seatId,
    { instanceId: config.instanceId }, created.receipt.revision);
  check(bound.receipt.result.layer === 'USER' && bound.receipt.result.state === 'IDLE' &&
    bound.receipt.result.instanceId === config.instanceId && bound.receipt.result.revision === '2' &&
    bound.receipt.result.generation === '2', 'Original User bind-instance did not bind the idle seat to the existing test instance');
  record.bindRequestId = bound.request.requestId; save();
  const card = await stateCard(seatId);
  check(card.receipt.result.layer === 'USER' && card.receipt.result.kind === 'LONG' &&
    card.receipt.result.state === 'IDLE' && card.receipt.result.instanceId === config.instanceId &&
    card.receipt.result.templateId === c.templateId && card.receipt.revision === '2',
  'Original User state-card does not confirm the created idle seat and binding');
  record.cardRequestId = card.request.requestId; save();

  const treeCreated = await userOperation('K-WORKTREE', 'create', worktreeId,
    { repositoryId: config.repositoryId, seatId, layout: 'single' }, '0');
  check(treeCreated.receipt.result.worktreeId === worktreeId && treeCreated.receipt.status === 'APPLIED',
    'Original User F create did not register the requested logical worktree');
  record.worktreeCreateRequestId = treeCreated.request.requestId; save();
  const registered = await userOperation('K-WORKTREE', 'register', worktreeId, {}, '1');
  check(registered.receipt.result.worktreeId === worktreeId && registered.receipt.revision === '2',
    'Original User F registration receipt differs');
  record.worktreeRegisterRequestId = registered.request.requestId; save();
  const graph = await graphQuery();
  const members = graph.receipt.result.members ?? [];
  check(graph.receipt.status === 'APPLIED' && graph.receipt.result.state === 'REGISTERED' &&
    graph.receipt.result.classification === 'SINGLE' && members.length === 1 &&
    members[0].worktreeId === worktreeId && members[0].repositoryId === config.repositoryId &&
    members[0].domainId === config.domainId && members[0].seatId === seatId &&
    members[0].instanceId === config.instanceId,
  'Original User graph-query did not confirm one registered SINGLE worktree for the new seat');
  record.graphQueryRequestId = graph.request.requestId; record.graphRevision = graph.receipt.revision;
  record.instanceCardAfter = (await stateCard(seatId)).receipt; product.save();

  await product.custody(); product.verifyBytes();
  const final = await immutableReadback('final');
  check(final.proof.directCaseEvidence === true && final.proof.seat?.seatId === seatId &&
    final.proof.worktree?.worktreeId === worktreeId && final.proof.worktree.classification === 'SINGLE' &&
    final.proof.worktree.lifecycleState === 'REGISTERED' && final.proof.seat.state === 'IDLE' &&
    JSON.stringify(final.proof.rootIdentity) === JSON.stringify(baseline.proof.rootIdentity) &&
    JSON.stringify(final.proof.candidateIdentity) === JSON.stringify(baseline.proof.candidateIdentity),
  'Final original F/seat immutable readback differs from baseline or User receipts');
  record.final = final.reference;
  record.fixture = { stopWorktree: { lifecycleOwnership: 'EXCLUSIVE_M2_STOP_WORKTREE',
    seatId, worktreeId } };
  record.state = 'PREPARE_COMPLETE_REVIEW_REQUIRED'; save();
  journal.state = record.state; journal.notRun = [
    { caseId: 'STOP_GATE_DENIED_THEN_CLEANUP', state: 'NOT_RUN', reason: 'Prepared fixture only; invoke the existing stop-worktree entry separately.' },
    { caseId: 'V07_RESIDUAL_CHILD_PROCESSES', state: 'NOT_RUN', reason: 'No descendant-process census was run.' },
    { caseId: 'V07_HOST_RESTART', state: 'NOT_RUN', reason: 'No host restart was performed.' },
  ];
  product.save();
} catch (error) {
  record.state = 'FAIL_ORIGINAL_REQUESTS_RETAINED';
  record.error = String(error?.stack ?? error);
  journal.state = record.state; save();
  process.exitCode = 1;
  try { await product.preserveFailure(); }
  catch (preserveError) { record.preserveError = String(preserveError?.stack ?? preserveError); save(); }
}
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
