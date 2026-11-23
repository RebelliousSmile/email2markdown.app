// Captures des fenêtres de l'application pour les guides.
// Les fenêtres sont des gabarits HTML (assets/*.html) remplis par l'hôte Rust : on les
// remplit ici avec des données fictives et on simule window.ipc.
// Usage : pnpm install && pnpm captures [nom-de-capture ...]
import { chromium } from 'playwright';
import { readFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const assets = path.resolve(here, '../../assets');
const outDir = path.resolve(here, '../../aidd_docs/guides/guide-utilisateur/images');
mkdirSync(outDir, { recursive: true });

const template = name => readFileSync(path.join(assets, name), 'utf8');
const json = value => JSON.stringify(value).replace(/</g, '\u003c');

const IPC_STUB = 'window.ipc = { postMessage: m => { (window.__ipc = window.__ipc || []).push(m); } };';

const settings = {
  export_base_dir: 'C:\\Users\\Camille\\Documents\\Export',
  notes_dir: 'C:\\Users\\Camille\\Documents\\Notes',
  destinations_file: 'C:\\Users\\Camille\\AppData\\Roaming\\email-to-markdown\\destinations.yaml',
  ai_routing_enabled: false,
  ai_confidence_threshold: 0.7,
  organize_by_date: true,
  date_layout: 'year_month',
  destination_max_depth: 2,
  defaults: {
    quote_depth: 1, skip_existing: true, collect_contacts: false,
    skip_signature_images: true, delete_after_export: false, cleanup_empty_dirs: false, batch_size: 0,
  },
  accounts: {},
};

const accounts = [
  { name: 'Perso', server: 'imap.exemple.org', port: 993, username: 'camille@exemple.org', ignored_folders: ['Spam', 'Corbeille'] },
  { name: 'Travail', server: 'imap.entreprise.example', port: 993, username: 'c.martin@entreprise.example', ignored_folders: ['Junk'] },
];

function configHtml(view = '') {
  return template('config_window.html')
    .replace('__SETTINGS_JSON__', json(settings))
    .replace('__ACCOUNTS_JSON__', json(accounts))
    .replace('__INITIAL_VIEW__', view)
    .replace('__CONFIG_DIR__', 'C:\\Users\\Camille\\AppData\\Roaming\\email-to-markdown');
}

const progressHtml = (name, warning, cancel) => template('progress_window.html')
  .replace('__ACTION_NAME__', name)
  .replace('__WARNING__', warning)
  .replace('__HAS_CANCEL__', cancel ? '<button id="cancel" onclick="requestCancel(this)">Annuler</button>' : '');

const destinationsHtml = () => template('destinations_window.html')
  .replace('__INITIAL_PATH_JSON__', 'null')
  .replace('__CONTEXTUAL_SETUP_JSON__', 'false');

const destinationsState = {
  type: 'state', max_depth: 2, missing: [], too_deep: [],
  destinations: [
    { path: 'Perso/Banque', note: 'Relevés et courriers de la banque', rules: [{ domain: 'banque-exemple.ch' }] },
    { path: 'Perso/Santé', note: 'Mutuelle et rendez-vous médicaux', rules: [{ domain: 'mutuelle.example' }, { from: 'cabinet@sante.example' }] },
    { path: 'Travail/Projets/Alpha', rules: [{ subject: 'Projet Alpha' }, { account: 'Travail' }] },
    { path: 'Messy/Emails', note: 'Destination par défaut', default: true },
  ],
};

const knownPaths = destinationsState.destinations.map(d => d.path);
const routeDecisions = [
  { file: '2026-03-14_facture.md', subject: 'Facture de mars', subject_from_header: true, sender: 'Banque Exemple', sender_email: 'service@banque-exemple.ch', sender_domain: 'banque-exemple.ch', correspondents: ['service@banque-exemple.ch'], date: '2026-03-14', dest_path: 'Perso/Banque', is_default: false, source: 'rule' },
  { file: '2026-03-12_rdv.md', subject: 'Confirmation de rendez-vous', subject_from_header: true, sender: 'Cabinet du Dr Durand', sender_email: 'cabinet@sante.example', sender_domain: 'sante.example', correspondents: ['cabinet@sante.example'], date: '2026-03-12', dest_path: 'Perso/Santé', is_default: false, source: 'rule' },
  { file: '2026-03-10_bonjour.md', subject: 'Bonjour', subject_from_header: true, sender: 'Inconnu', sender_email: 'contact@nouveau.example', sender_domain: 'nouveau.example', correspondents: ['contact@nouveau.example'], date: '2026-03-10', dest_path: 'Messy/Emails', is_default: true },
  { file: '2026-03-09_planning.md', subject: 'Planning du projet Alpha', subject_from_header: true, sender: 'Marie Leroy', sender_email: 'm.leroy@entreprise.example', sender_domain: 'entreprise.example', correspondents: ['m.leroy@entreprise.example'], date: '2026-03-09', dest_path: 'Travail/Projets/Alpha', is_default: false, source: 'rule' },
];

const thunderbirdScan = {
  action: 'thunderbird_scan', ok: true,
  profile: 'C:\\Users\\Camille\\AppData\\Roaming\\Thunderbird\\Profiles\\x1y2z3.default-release',
  accounts: [
    { server_id: 'server1', name: 'Perso', server: 'imap.exemple.org', username: 'camille@exemple.org', auth: 'password', has_saved_password: true, already_configured: true },
    { server_id: 'server2', name: 'Association', server: 'imap.association.example', username: 'camille@association.example', auth: 'password', has_saved_password: true, already_configured: false },
    { server_id: 'server3', name: 'Messagerie Gmail', server: 'imap.gmail.com', username: 'camille.martin@gmail.com', auth: 'oauth2', has_saved_password: false, already_configured: false },
  ],
};

const contextualLaunch = {
  target: 'C:\\Users\\Camille\\Documents\\Notes\\Perso\\Banque',
  relative_path: 'Perso/Banque',
  rule_labels: ['domaine : banque-exemple.ch'],
  accounts: [{ name: 'Perso', password_missing: false, delete_after_export: false }, { name: 'Travail', password_missing: false, delete_after_export: false }],
};
const contextualResult = {
  preflight: { required: false, supported: true, provider: 'generic', reason: null },
  rows: [
    { key: 'a1', date: '2026-03-14T09:12:00+01:00', correspondent: 'Banque Exemple', sender_email: 'service@banque-exemple.ch', correspondents: ['service@banque-exemple.ch'], subject: 'Facture de mars', folder: 'INBOX', already_present: false },
    { key: 'a2', date: '2026-02-14T09:05:00+01:00', correspondent: 'Banque Exemple', sender_email: 'service@banque-exemple.ch', correspondents: ['service@banque-exemple.ch'], subject: 'Facture de février', folder: 'INBOX', already_present: true },
    { key: 'a3', date: '2026-01-15T08:40:00+01:00', correspondent: 'Banque Exemple', sender_email: 'service@banque-exemple.ch', correspondents: ['service@banque-exemple.ch'], subject: 'Votre relevé annuel', folder: 'Archives/2025', already_present: false },
  ],
};

// Chaque capture : fichier, taille de fenêtre, HTML, script avant chargement, étapes après chargement.
const shots = [
  {
    name: 'parametres-generaux', size: [700, 640], html: () => configHtml(),
    blur: ['#export-base-dir', '#notes-dir', '#destinations-file'],
  },
  {
    name: 'parametres-comptes', size: [700, 520], html: () => configHtml(),
    steps: async page => { await page.click('.tab-btn[data-tab="accounts"]'); },
  },
  {
    name: 'parametres-compte-detail', size: [700, 900], html: () => configHtml(),
    steps: async page => {
      await page.click('.tab-btn[data-tab="accounts"]');
      await page.click('.acct-edit-btn[data-idx="0"]');
    },
    blur: ['#acct-server', '#acct-username', '#acct-password'],
  },
  {
    name: 'import-thunderbird', size: [700, 520], html: () => configHtml('thunderbird'),
    steps: async page => {
      await page.evaluate(result => window.config_result(result), thunderbirdScan);
    },
    blur: ['#tb-profile', '.tb-item-sub'],
  },
  {
    name: 'destinations', size: [820, 560], html: () => destinationsHtml(),
    steps: async page => {
      await page.evaluate(state => window_msg(JSON.stringify(state)), destinationsState);
      await page.click('#destinations-list li:nth-child(2), ul li:nth-child(2)');
    },
    blur: ['.rule-label'],
  },
  {
    name: 'revue-routage', size: [900, 600], html: () => template('route_review.html'),
    blur: ['td.sender'],
    init: `window.__DECISIONS_DATA__=${json(routeDecisions)};window.__KNOWN_PATHS__=${json(knownPaths)};window.__RULE_OWNERS__=${json({})};`,
  },
  {
    name: 'export-contextuel', size: [900, 640], html: () => template('contextual_export.html'),
    init: `window.__CONTEXTUAL_LAUNCH__=${json(contextualLaunch)};`,
    steps: async page => {
      await page.selectOption('#account', 'Perso');
      await page.addStyleTag({ content: '#load-more { display: none !important; }' });
      await page.evaluate(payload => window.contextual_search_done(payload), contextualResult);
    },
    blur: ['#rows td:nth-child(3)', '#target'],
  },
  {
    name: 'progression-export', size: [520, 270],
    html: () => progressHtml('Export — Perso', '', true),
    steps: async page => { await page.evaluate(() => { step(412, 1200, 'Perso — INBOX : message 412 sur 1 200'); statusLine('Facture de mars — banque-exemple.ch'); }); },
  },
  {
    name: 'progression-terminee', size: [520, 270],
    html: () => progressHtml('Export — Perso', '', true),
    steps: async page => { await page.evaluate(() => finish('1 200 messages exportés, 3 ignorés, 0 erreur.', 'Classer les notes')); },
  },
  {
    name: 'mise-a-jour', size: [700, 500],
    html: () => template('update_window.html'),
    steps: async page => {
      await page.evaluate(() => window_msg(JSON.stringify({
        type: 'check_result', current: '0.28.0', latest: '0.29.0',
        body: 'Import des comptes Thunderbird depuis la fenêtre des paramètres.\nTest de connexion d’un compte avant enregistrement.',
      })));
    },
  },
];

// Floute les zones sensibles (adresses, serveurs, chemins personnels) avant la capture.
// Les données sont fictives, mais le flou garde le même rendu si l'on branche des données réelles.
async function blurSensitive(page, selectors) {
  await page.addStyleTag({ content: `${selectors.join(',')} { filter: blur(5px) !important; user-select: none; }` });
}

const only = process.argv.slice(2);
const browser = await chromium.launch({ channel: process.env.CAPTURE_CHANNEL ?? 'chrome' });
for (const shot of shots.filter(s => !only.length || only.includes(s.name))) {
  const context = await browser.newContext({ viewport: { width: shot.size[0], height: shot.size[1] }, deviceScaleFactor: 2, locale: 'fr-FR' });
  const page = await context.newPage();
  page.on('pageerror', e => console.log('  erreur page:', shot.name, e.message));
  // Le script est intégré au HTML (avant les scripts du gabarit) : addInitScript n'est pas
  // rejoué par setContent.
  const html = shot.html().replace('<head>', `<head><script>${IPC_STUB}${shot.init ?? ''}</script>`);
  await page.setContent(html, { waitUntil: 'load' });
  await shot.steps?.(page);
  await page.waitForTimeout(250);
  if (shot.blur?.length) await blurSensitive(page, shot.blur);
  const file = path.join(outDir, `${shot.name}.png`);
  await page.screenshot({ path: file });
  console.log('capture', file);
  await context.close();
}
await browser.close();
