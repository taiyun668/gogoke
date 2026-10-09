// M3 V15 narrow UI slice: real installed Home/Settings/project DOM reads only.
// It never logs in, checks, saves, creates, deletes, sends or dispatches a model.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { ActualProduct, id, readJson, sha256 } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const configPath = process.argv[2];
if (!configPath) throw Error('Private M3 V15 UI read-only config path required');
const config = readJson(configPath);
const expected = config.m3UiReadonly;
const absolute = value => typeof value === 'string' && path.isAbsolute(value);
const nonempty = value => typeof value === 'string' && value.length > 0;
const inside = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const check = (ok, reason) => { if (!ok) throw Error(reason); };
const PANEL_LABELS = ['席位', '旁聊'];
const VENDOR_LABELS = {
  codex: 'Codex', claude: 'Claude Code', opencode: 'OpenCode', grok: 'Grok Build',
  antigravity: 'Antigravity',
};

if (process.platform !== 'win32' || !expected ||
    expected.expectedSecretaryState !== 'UNSET' ||
    !nonempty(expected.instanceId) ||
    !['installed', 'pwsh', 'stateRoot', 'testbedSource', 'evidenceDirectory', 'result']
      .every(key => absolute(config[key])) ||
    !/^\d+\.\d+\.\d+$/.test(config.version ?? '') ||
    !/^[a-f0-9]{40}$/.test(config.sourceCommit ?? '') ||
    !['gogoke.exe', 'gogoke-native-host.exe', 'resource-index.json']
      .every(name => /^[a-f0-9]{64}$/.test(config.installedSha256?.[name] ?? '')) ||
    typeof config.registryKey !== 'string' ||
    !fs.existsSync(config.evidenceDirectory) || !fs.existsSync(config.testbedSource) ||
    fs.existsSync(config.result) ||
    path.dirname(path.resolve(config.result)) !== path.resolve(config.evidenceDirectory) ||
    [config.installed, config.stateRoot, config.testbedSource, path.resolve(here, '..', '..')]
      .some(root => inside(path.resolve(config.evidenceDirectory), path.resolve(root))) ||
    inside(path.resolve(config.stateRoot), path.resolve(config.evidenceDirectory)) ||
    path.resolve(config.stateRoot) === path.resolve(config.testbedSource) ||
    config.testerArmy === false) {
  throw Error('M3 V15 requires exact installed bytes, signed custody, a real User UI bridge and explicit expected read facts');
}

const journal = {
  schema: 'gogoke.37.private-m3-v15-ui-readonly.v1',
  caseId: id('m3V15Ui'), sourceCommit: config.sourceCommit,
  candidateVersion: config.version, candidateInstalledSha256: config.installedSha256,
  evidenceDirectory: path.resolve(config.evidenceDirectory), state: 'RUNNING', acceptance: false,
  coverage: [],
  notRun: [
    'SECRETARY_UNSET_NATIVE_READ', 'SECRETARY_UNSET_ENTRY', 'INSTANCE_READ_ONLY',
    'PROJECT_PANEL_LABELS_READ_ONLY',
    'V15_FULL_25_STATES', 'SECRETARY_CONFIGURED', 'SECRETARY_CONVERSATION',
    'SECRETARY_SETTINGS_WRITE', 'INSTANCE_LOGIN_OR_CHECK', 'SEAT_OR_SIDECHAT_OPERATION',
    'MODEL_OR_QUOTA', 'CREDENTIAL_OR_ACCOUNT_CONTENT', 'V15_ACCEPTANCE',
  ],
  native: {}, navigation: [], ui: [], actionsInvoked: [], launches: [], closes: [],
};
const product = new ActualProduct(config, journal);
journal.driverBytes['m3-ui-readonly-win11.mjs'] = sha256(path.join(here, 'm3-ui-readonly-win11.mjs'));
const record = (name, value) => { journal.ui.push({ name, value, at: new Date().toISOString() }); product.save(); };
const markCoverage = name => {
  if (!journal.coverage.includes(name)) journal.coverage.push(name);
  journal.notRun = journal.notRun.filter(item => item !== name);
  product.save();
};

async function readUserFrame(page, frame, project) {
  const rawFrame = JSON.stringify(frame);
  const value = await page.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(rawFrame)}}).then(raw=>{
    if (typeof raw !== 'string') throw Error('Native USER reply is not a JSON frame');
    const value = JSON.parse(raw);
    return (${project.toString()})(value, raw);
  })`);
  return { rawFrame, ...value };
}

async function readSecretaryConfiguration(page) {
  const observed = await readUserFrame(page,
    { schema: 'gogoke.37.owner-configuration.v1', command: 'secretary-configuration-read' },
    (value, rawReply) => ({ rawReply, reply: value }));
  const reply = observed.reply;
  check(reply?.schema === 'gogoke.37.secretary-configuration.v1' && reply.state === 'UNSET' &&
    reply.conversation?.state === 'NONE',
  'Actual USER secretary read is not the expected UNSET/NONE fact');
  check(JSON.stringify(Object.keys(reply).sort()) === JSON.stringify(['conversation', 'schema', 'state']),
    'UNSET USER secretary read contains unexpected non-minimal fields');
  check(JSON.stringify(Object.keys(reply.conversation).sort()) === JSON.stringify(['state']),
    'UNSET USER secretary conversation fact contains unexpected fields');
  journal.native.secretaryConfiguration = {
    rawFrame: observed.rawFrame, rawReply: observed.rawReply,
    reply: { schema: reply.schema, state: reply.state, conversation: { state: reply.conversation.state } },
  };
  markCoverage('SECRETARY_UNSET_NATIVE_READ');
}

async function readInstanceFacts(page) {
  const instances = await page.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_instances').then(value => ({
    schema: value?.schema,
    instances: Array.isArray(value?.instances) ? value.instances.map(item => ({
      instanceId: item?.instanceId, driverId: item?.driverId, version: item?.version,
      state: item?.state, loginState: item?.login?.state, loginSettled: item?.login?.settled,
    })) : null,
  }))`);
  check(instances?.schema === 'gogoke.37.instance-page.v1' && Array.isArray(instances.instances),
    'Actual instance snapshot schema is unavailable');
  const nativeInstanceRows = instances.instances.filter(item => item.instanceId === expected.instanceId);
  check(nativeInstanceRows.length === 1, 'Expected instanceId is not uniquely present in the actual instance snapshot');
  const instance = nativeInstanceRows[0];
  check(['NOT_INSTALLED', 'NOT_LOGGED_IN', 'LOGGED_IN', 'ERROR'].includes(instance.state) &&
    (instance.loginState === undefined ||
      ['PENDING', 'LOGGED_IN', 'LOGGED_OUT', 'UNKNOWN', 'CANCELLED', 'ERROR'].includes(instance.loginState)) &&
    (instance.loginSettled === undefined || typeof instance.loginSettled === 'boolean') &&
    typeof instance.version === 'string',
  'Actual instance snapshot contains an unknown status shape');

  const management = await readUserFrame(page,
    { schema: 'gogoke.37.owner-configuration.v1', command: 'instance-management-read' },
    value => ({ reply: {
      schema: value?.schema,
      profiles: Array.isArray(value?.profiles) ? value.profiles.map(profile => ({
        instanceId: profile?.instanceId, driverId: profile?.driverId,
        name: profile?.name ?? null, enabled: profile?.enabled ?? null,
        nameValid: profile?.name === undefined || profile?.name === null || typeof profile.name === 'string',
        capPresent: profile?.cap !== undefined && profile?.cap !== null,
        capValid: profile?.cap === undefined || profile?.cap === null ||
          (typeof profile.cap === 'number' && Number.isSafeInteger(profile.cap) && profile.cap >= 1),
        programSourceErrorValid: profile?.programSourceError === undefined ||
          profile?.programSourceError === null || typeof profile.programSourceError === 'string',
        programSourceErrorPresent: typeof profile?.programSourceError === 'string' &&
          profile.programSourceError.length > 0,
      })) : null,
    } }));
  const profiles = management.reply;
  check(profiles?.schema === 'gogoke.37.instance-management.v1' && Array.isArray(profiles.profiles),
    'Actual instance management schema is unavailable');
  const matching = profiles.profiles.filter(profile => profile.instanceId === expected.instanceId);
  check(matching.length === 1, 'Expected instanceId is not uniquely present in actual instance management');
  const profile = matching[0];
  check(['codex', 'claude', 'opencode', 'grok', 'antigravity'].includes(profile.driverId) &&
    profile.driverId === instance.driverId && nonempty(profile.driverId) &&
    (profile.enabled === null || typeof profile.enabled === 'boolean') && profile.nameValid &&
    profile.capValid && profile.programSourceErrorValid,
    'Actual instance snapshot/profile identity is inconsistent');
  const ordinal = profiles.profiles.filter(item => item.driverId === profile.driverId)
    .findIndex(item => item.instanceId === profile.instanceId) + 1;
  const name = nonempty(profile.name) ? profile.name :
    `${VENDOR_LABELS[profile.driverId] ?? profile.driverId} 实例${ordinal > 1 ? ` ${ordinal}` : ''}`;
  const statePrefix = profile.enabled === false ? '已停用，主控不会派活给它' :
    instance.state === 'NOT_INSTALLED' ? '它要用的 CLI 还没装' :
    instance.loginState === 'PENDING' && instance.loginSettled !== true ? '正在登录' :
    instance.loginState === 'ERROR' ? '这次没登上' :
    instance.loginState === 'UNKNOWN' ? '没能确认登没登上' :
    profile.programSourceErrorPresent || instance.state === 'ERROR' ? '出错了' :
    instance.state === 'LOGGED_IN' && profile.enabled === true && profile.capPresent ? '可以用' :
    instance.state === 'LOGGED_IN' ? '登上了，但名字、启用或并发上限还没设好' :
    instance.loginState === 'CANCELLED' ? '上次登录已取消' : '还没登录';
  journal.native.instance = {
    snapshotSchema: instances.schema, managementFrame: management.rawFrame,
    managementSchema: profiles.schema,
    instanceId: instance.instanceId, driverId: instance.driverId, version: instance.version,
    state: instance.state, loginState: instance.loginState ?? null, profileName: name,
    profileEnabled: profile.enabled, profileCapPresent: profile.capPresent,
    programSourceErrorPresent: profile.programSourceErrorPresent,
  };
  return { name, statePrefix };
}

async function readSecretaryEntry(page) {
  await readSecretaryConfiguration(page);
  const entry = page.locator('.sidebar > .sec-entry');
  await entry.waitFor({ state: 'visible', timeout: 30000 });
  check(await entry.count() === 1, 'Actual Home has exactly one fixed Secretary entry');
  const name = entry.locator('.sec-entry-name');
  const subtitle = entry.locator('.sec-entry-sub');
  check(await name.innerText() === '秘书长', 'Secretary entry label is the actual product label');
  check(await subtitle.innerText() === '还没设置：选一个实例它才能干活',
    'Secretary entry does not read the expected native UNSET state');
  check(await page.locator('.sidebar > .sidebar-body').count() === 1,
    'Secretary entry is outside the scrollable project/sidebar body');
  markCoverage('SECRETARY_UNSET_ENTRY');
  record('secretaryUnsetEntry', { fixedEntry: true, state: 'UNSET', clicked: false });
}

async function readInstance(page) {
  const actual = await readInstanceFacts(page);
  const openSettings = page.getByRole('button', { name: /^(Open settings|打开设置)$/ });
  check(await openSettings.count() === 1, 'Actual Home settings entry is unavailable');
  await openSettings.click();
  journal.navigation.push('open-settings');
  product.save();

  const sidebar = page.locator('.settings-sidebar');
  await sidebar.waitFor({ state: 'visible', timeout: 15000 });
  const instances = sidebar.getByRole('button', { name: /^(Instances|实例)$/ });
  check(await instances.count() === 1, 'Actual Settings has one Instances navigation entry');
  await instances.click();
  journal.navigation.push('settings-instances');
  product.save();

  const card = page.locator('.instances-row').filter({ hasText: actual.name });
  await card.first().waitFor({ state: 'visible', timeout: 15000 });
  check(await card.count() === 1, 'Actual Instances page has exactly one configured target row');
  check(await card.locator('.settings-toggle-title').innerText() === actual.name,
    'Actual Instances row is not bound to the native profile name');
  const summary = await card.locator('.instances-row-sub').innerText();
  check(summary.startsWith(actual.statePrefix), 'Actual Instances row state differs from native status facts');
  markCoverage('INSTANCE_READ_ONLY');
  record('instanceReadOnly', { rowCount: 1, state: journal.native.instance.state,
    version: journal.native.instance.version, name: actual.name, actionClicks: 0 });

  const close = page.locator('.settings-close');
  check(await close.count() === 1, 'Actual Settings close control is unavailable');
  await close.click();
  journal.navigation.push('close-settings');
  const mainViews = page.locator(
    '.home-product-entry, .main .content, .tablet-main .tablet-content',
  );
  await mainViews.first().waitFor({ state: 'visible', timeout: 15000 });
  check(await page.locator(
    '.home-product-entry:visible, .main .content:visible, .tablet-main .tablet-content:visible',
  ).count() === 1, 'Actual Home or selected-project main view is not uniquely visible');
  product.save();
}

async function readProjectPanelLabels(page) {
  const activeWorkspace = page.locator('.workspace-row.active:visible');
  if (await activeWorkspace.count() !== 1) {
    record('projectPanelLabels', { state: 'NOT_RUN_CURRENT_PROJECT_NOT_SELECTED' });
    return;
  }
  const tablist = page.locator('[role="tablist"]:visible').filter({
    has: page.locator(`[role="tab"][aria-label="${PANEL_LABELS[0]}"]`),
  });
  if (await tablist.count() === 0) {
    record('projectPanelLabels', { state: 'NOT_RUN_CURRENT_PROJECT_NOT_VISIBLE' });
    return;
  }
  check(await tablist.count() === 1, 'Actual project panel tablist is not unique');
  for (const label of PANEL_LABELS) {
    const tab = tablist.locator(`[role="tab"][aria-label="${label}"]`);
    check(await tab.count() === 1 && await tab.isVisible(), `Actual project panel tab is missing: ${label}`);
  }
  markCoverage('PROJECT_PANEL_LABELS_READ_ONLY');
  record('projectPanelLabels', { state: 'OBSERVED', labels: PANEL_LABELS, clickedTabs: false });
}

try {
  await product.launch();
  const page = product.tester?.page;
  check(page, 'Actual tester/e2e connection is required for UI readback');
  await readSecretaryEntry(page);
  await readInstance(page);
  await readProjectPanelLabels(page);
  await product.closeNormally();
  journal.state = 'READBACK_COMPLETE_REQUIRES_REVIEW';
  product.save();
} catch (error) {
  journal.state = 'FAIL_ORIGINAL_EVIDENCE_RETAINED';
  journal.error = String(error?.stack ?? error);
  product.save();
  process.exitCode = 1;
  try { if (product.child?.exitCode === null) await product.preserveFailure(); }
  catch (preserveError) { journal.preserveError = String(preserveError?.stack ?? preserveError); product.save(); }
}
