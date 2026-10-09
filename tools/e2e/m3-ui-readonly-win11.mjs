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

if (process.platform !== 'win32' || !expected ||
    expected.expectedSecretaryState !== 'UNSET' ||
    !nonempty(expected.instanceLabel) || !nonempty(expected.instanceSummaryText) ||
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
    (expected.workspaceName !== undefined && !nonempty(expected.workspaceName)) ||
    (expected.workspaceName !== undefined &&
      (!Array.isArray(expected.panelLabels) || expected.panelLabels.length !== 2 ||
       expected.panelLabels.some(label => !nonempty(label)))) ||
    config.testerArmy === false) {
  throw Error('M3 V15 requires exact installed bytes, signed custody, a real User UI bridge and explicit expected read facts');
}

const journal = {
  schema: 'gogoke.37.private-m3-v15-ui-readonly.v1',
  caseId: id('m3V15Ui'), sourceCommit: config.sourceCommit,
  candidateVersion: config.version, candidateInstalledSha256: config.installedSha256,
  evidenceDirectory: path.resolve(config.evidenceDirectory), state: 'RUNNING', acceptance: false,
  coverage: ['SECRETARY_UNSET_ENTRY', 'INSTANCE_READ_ONLY', 'PROJECT_PANEL_LABELS_READ_ONLY'],
  notRun: [
    'V15_FULL_25_STATES', 'SECRETARY_CONFIGURED', 'SECRETARY_CONVERSATION',
    'SECRETARY_SETTINGS_WRITE', 'INSTANCE_LOGIN_OR_CHECK', 'SEAT_OR_SIDECHAT_OPERATION',
    'MODEL_OR_QUOTA', 'CREDENTIAL_OR_ACCOUNT_CONTENT', 'V15_ACCEPTANCE',
  ],
  navigation: [], ui: [], actionsInvoked: [], launches: [], closes: [],
};
const product = new ActualProduct(config, journal);
journal.driverBytes['m3-ui-readonly-win11.mjs'] = sha256(path.join(here, 'm3-ui-readonly-win11.mjs'));
const record = (name, value) => { journal.ui.push({ name, value, at: new Date().toISOString() }); product.save(); };

async function readSecretaryEntry(page) {
  const entry = page.locator('.sidebar > .sec-entry');
  await entry.waitFor({ state: 'visible', timeout: 30000 });
  check(await entry.count() === 1, 'Actual Home has exactly one fixed Secretary entry');
  const name = entry.locator('.sec-entry-name');
  const subtitle = entry.locator('.sec-entry-sub');
  check(await name.innerText() === '秘书长', 'Secretary entry label is the actual product label');
  check(await subtitle.innerText() === '还没设置：选一个实例它才能干活',
    'Secretary entry does not read the expected native UNSET state');
  check(await entry.isDisabled(), 'UNSET Secretary entry is disabled and cannot open a conversation');
  check(await page.locator('.sidebar > .sidebar-body').count() === 1,
    'Secretary entry is outside the scrollable project/sidebar body');
  record('secretaryUnsetEntry', { disabled: true, fixedEntry: true, state: 'UNSET' });
}

async function readInstance(page) {
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

  const card = page.locator('.instances-row').filter({ hasText: expected.instanceLabel });
  await card.first().waitFor({ state: 'visible', timeout: 15000 });
  check(await card.count() === 1, 'Actual Instances page has exactly one configured target row');
  check(await card.locator('.settings-toggle-title').innerText() === expected.instanceLabel,
    'Actual Instances row label differs from the expected private fixture');
  check(await card.locator('.instances-row-sub').innerText() === expected.instanceSummaryText,
    'Actual Instances row summary differs from the expected read-only fixture');
  record('instanceReadOnly', { rowCount: 1, summaryMatched: true, actionClicks: 0 });

  const close = page.locator('.settings-close');
  check(await close.count() === 1, 'Actual Settings close control is unavailable');
  await close.click();
  journal.navigation.push('close-settings');
  await page.locator('.home-product-entry').waitFor({ state: 'visible', timeout: 15000 });
  product.save();
}

async function readProjectPanelLabels(page) {
  if (expected.workspaceName === undefined) {
    journal.notRun.push('PROJECT_PANEL_LABELS_NO_CONFIGURED_WORKSPACE');
    record('projectPanelLabels', { state: 'NOT_RUN_NO_CONFIGURED_WORKSPACE' });
    return;
  }
  const row = page.locator('.workspace-row').filter({ hasText: expected.workspaceName });
  await row.waitFor({ state: 'visible', timeout: 15000 });
  check(await row.count() === 1, 'Configured project row is not uniquely visible');
  await row.click();
  journal.navigation.push('select-existing-workspace');
  product.save();

  const labels = expected.panelLabels;
  const tablist = page.locator('[role="tablist"]').filter({
    has: page.locator(`[role="tab"][aria-label="${labels[0]}"]`),
  });
  await tablist.waitFor({ state: 'visible', timeout: 15000 });
  check(await tablist.count() === 1, 'Actual project panel tablist is not unique');
  for (const label of labels) {
    const tab = tablist.locator(`[role="tab"][aria-label="${label}"]`);
    check(await tab.count() === 1 && await tab.isVisible(), `Actual project panel tab is missing: ${label}`);
  }
  record('projectPanelLabels', { state: 'OBSERVED', labels, clickedTabs: false });
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
