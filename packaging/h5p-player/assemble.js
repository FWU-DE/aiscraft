// Part of build.sh (#62): takes the libraries out of the unpacked H5P Hub packages, checks them
// and writes manifest.json, NOTICE.txt and LICENSES/libraries.txt.
//
//   node assemble.js <hub dir> <stage dir> <h5p-standalone version> <bundle version> <npm integrity>
//   HUB_SOURCES='{"H5P.QuestionSet":"<sha256>",…}'
//
// - Editor libraries (H5PEditor.*) are left out: the player only shows content.
// - A library in several packages is taken once, the highest patch version.
// - Every dependency of every library must be in the bundle, and the content types AIS.ChatCraft
//   writes (crates/core/src/h5p.rs) must be there with the same major version.
// - Only file types the app accepts (crates/engine/src/learn/h5p_player.rs, BUNDLE_TYPES), no
//   links, no folder names outside the stage.
'use strict';
const fs = require('fs');
const path = require('path');

const [hub, stage, h5psVersion, bundleVersion, h5psIntegrity] = process.argv.slice(2);
if (!hub || !stage || !h5psVersion || !bundleVersion) {
  console.error('assemble.js <hub dir> <stage dir> <h5p-standalone version> <bundle version> <npm integrity>');
  process.exit(2);
}
const sources = JSON.parse(process.env.HUB_SOURCES || '{}');

// Keep in step with BUNDLE_TYPES in crates/engine/src/learn/h5p_player.rs.
const ALLOWED = new Set(['js', 'css', 'json', 'png', 'jpg', 'jpeg', 'gif', 'svg', 'webp', 'bmp', 'woff', 'woff2', 'ttf', 'eot', 'otf', 'txt', 'md',
  'mp3', 'm4a', 'ogg', 'wav', 'webm', 'mp4', 'vtt', 'xml']);
// Types H5P allows in libraries that playing never needs (patches, office documents): left out.
const DROPPED = new Set(['patch', 'diff', 'pdf', 'rtf', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'odt', 'ods', 'odp', 'csv', 'textile']);
// The content types AIS.ChatCraft writes (crates/core/src/h5p.rs): machine name, major version.
const NEEDED = [['H5P.QuestionSet', 1], ['H5P.MultiChoice', 1], ['H5P.TrueFalse', 1], ['H5P.Blanks', 1], ['H5P.DragText', 1]];

function die(msg) {
  console.error('assemble.js: ' + msg);
  process.exit(1);
}

const libs = new Map(); // folder -> { dir, json, from }
for (const pkg of fs.readdirSync(hub).sort()) {
  const pdir = path.join(hub, pkg);
  if (!fs.lstatSync(pdir).isDirectory()) continue;
  for (const folder of fs.readdirSync(pdir).sort()) {
    const lj = path.join(pdir, folder, 'library.json');
    if (!fs.existsSync(lj)) continue;
    const json = JSON.parse(fs.readFileSync(lj, 'utf8').replace(/^﻿/, ''));
    if (String(json.machineName).startsWith('H5PEditor.')) continue;
    const expected = `${json.machineName}-${json.majorVersion}.${json.minorVersion}`;
    if (folder !== expected || !/^[A-Za-z0-9._-]+-\d+\.\d+$/.test(folder)) die(`${pkg}: Ordner ${folder} passt nicht zu library.json (${expected})`);
    const prev = libs.get(folder);
    if (!prev || Number(json.patchVersion) > Number(prev.json.patchVersion)) libs.set(folder, { dir: path.join(pdir, folder), json, from: pkg });
  }
}
if (libs.size === 0) die('keine Bibliotheken gefunden');

for (const [folder, { json }] of libs) {
  for (const d of json.preloadedDependencies || []) {
    if (String(d.machineName).startsWith('H5PEditor.')) continue;
    const want = `${d.machineName}-${d.majorVersion}.${d.minorVersion}`;
    if (!libs.has(want)) die(`${folder} braucht ${want}, das in keinem Hub-Paket steckt`);
  }
}
for (const [name, major] of NEEDED) {
  const found = [...libs.values()].filter(l => l.json.machineName === name && Number(l.json.majorVersion) === major);
  if (found.length === 0) die(`${name} ${major}.x fehlt`);
}

function check(dir, rel) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const r = rel ? `${rel}/${e.name}` : e.name;
    if (e.name.startsWith('.') || e.name.includes('\\') || e.name.includes(':')) die(`unzulässiger Name: ${r}`);
    if (e.isSymbolicLink()) die(`Verknüpfung: ${r}`);
    if (e.isDirectory()) check(path.join(dir, e.name), r);
    else {
      const ext = path.extname(e.name).slice(1).toLowerCase();
      if (DROPPED.has(ext)) {
        fs.unlinkSync(path.join(dir, e.name));
        console.log(`  weggelassen (nicht für die Wiedergabe): ${r}`);
      } else if (!ALLOWED.has(ext)) {
        die(`Dateityp nicht vorgesehen: ${r} (in assemble.js und h5p_player.rs ergänzen, wenn er gebraucht wird)`);
      }
    }
  }
}

const list = [];
for (const folder of [...libs.keys()].sort()) {
  const { dir, json, from } = libs.get(folder);
  fs.cpSync(dir, path.join(stage, 'libraries', folder), { recursive: true });
  list.push({ folder, json, from });
}
check(stage, '');

const libraries = list.map(({ json }) => `${json.machineName} ${json.majorVersion}.${json.minorVersion}.${json.patchVersion}`);
const manifest = {
  name: 'aiscraft-h5p-player',
  version: Number(bundleVersion),
  h5p_standalone: h5psVersion,
  libraries,
  sources: { npm: `h5p-standalone@${h5psVersion} ${h5psIntegrity || ''}`.trim(), hub: sources },
};
fs.writeFileSync(path.join(stage, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n');

const libLines = list.map(({ folder, json, from }) =>
  `${json.machineName} ${json.majorVersion}.${json.minorVersion}.${json.patchVersion}\n` +
  `  Lizenz laut library.json: ${json.license || 'keine Angabe'}\n` +
  `  Autor: ${json.author || 'keine Angabe'}\n` +
  `  Ordner: libraries/${folder}\n` +
  `  Aus dem H5P-Hub-Paket ${from} (SHA-256 ${sources[from] || '?'}), https://api.h5p.org/v1/content-types/${from}\n`);
fs.writeFileSync(path.join(stage, 'LICENSES', 'libraries.txt'),
  'H5P-Bibliotheken in diesem Paket\n' +
  '================================\n\n' +
  'Quelltext der H5P-Bibliotheken: https://github.com/h5p (je Bibliothek ein Repository)\n\n' +
  libLines.join('\n'));

fs.writeFileSync(path.join(stage, 'NOTICE.txt'),
`AIS.ChatCraft – H5P-Player (optionaler Zusatz), Paketversion ${bundleVersion}
${'='.repeat(60)}

Dieses Paket gehört nicht zu AIS.ChatCraft. AIS.ChatCraft lädt es nur auf ausdrücklichen Wunsch
und zeigt damit H5P-Inhalte im eigenen Vorschaufenster, ohne Netz. Es enthält:

1. h5p-standalone ${h5psVersion} (MIT), Ordner player/
   Quelltext: https://github.com/tunapanda/h5p-standalone/tree/v${h5psVersion}
   npm: https://www.npmjs.com/package/h5p-standalone/v/${h5psVersion} (${h5psIntegrity || ''})
   Lizenz: LICENSES/h5p-standalone-MIT.txt, Hinweise der gebündelten Teile: player/main.bundle.js.LICENSE.txt

   Darin enthalten ist der H5P-Kern (h5p.js, h5p.css und weitere Dateien aus h5p-php-library,
   Joubel AS), lizenziert unter der GNU General Public License Version 3 (GPL-3.0).
   Quelltext: https://github.com/h5p/h5p-php-library
   Lizenztext: LICENSES/GPL-3.0.txt

   Schriften Inter und Open Sans (SIL Open Font License 1.1): Lizenztexte in player/fonts/.

2. H5P-Inhaltstypen und ihre Bibliotheken (Ordner libraries/), unverändert aus den Paketen des
   H5P-Hubs (api.h5p.org), meist MIT. Liste mit Versionen und Lizenzangaben:
   LICENSES/libraries.txt

Das Paket wird unverändert weitergegeben. Wer es weitergibt, gibt diese Hinweise und die
Lizenztexte mit.
`);

console.log(`${libraries.length} Bibliotheken: ${libraries.join(', ')}`);
