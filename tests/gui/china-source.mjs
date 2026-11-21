// Run: node tests/gui/china-source.mjs
// China Nationwide and China Province are complete selectable OSM sources. Only the chosen
// source's flag is sent, and switching away leaves no China parameter behind.
import assert from 'node:assert';
import fs from 'node:fs';
import { meldFlags, chinaProvinceMissing } from '../../src/gui/js/meld-features.js';

const html = fs.readFileSync(new URL('../../src/gui/index.html', import.meta.url), 'utf8');
const state = new Map();
let activeSource = 'default';

function fromHtml(id) {
  const tag = html.match(new RegExp(`<(input|select)[^>]* id="${id}"[^>]*>`));
  if (!tag) return null;
  const attr = (name) => (tag[0].match(new RegExp(` ${name}="([^"]*)"`)) || [])[1];
  let value = attr('value') ?? '';
  if (tag[1] === 'select') {
    const block = html.slice(tag.index, html.indexOf('</select>', tag.index));
    const chosen = block.match(/<option value="([^"]*)" selected/) || block.match(/<option value="([^"]*)"/);
    value = chosen ? chosen[1] : '';
  }
  return { value, checked: /\bchecked\b/.test(tag[0]), closest: () => null };
}

globalThis.window = {};
globalThis.document = {
  getElementById(id) {
    if (!state.has(id)) state.set(id, fromHtml(id));
    return state.get(id);
  },
  querySelector(selector) {
    if (selector === '#meld-osm-source-group .segment.active') {
      return { dataset: { v: activeSource } };
    }
    const id = selector.match(/^#([\w-]+) \.segment\.active$/)?.[1];
    if (!id) return null;
    const start = html.indexOf(`id="${id}"`);
    const block = html.slice(start, html.indexOf('</div>', start));
    const active = block.match(/class="segment active"[^>]*data-v="([^"]*)"/);
    return active ? { dataset: { v: active[1] } } : null;
  },
};

const province = document.getElementById('meld-china-province-select');
const toggle = document.getElementById('advanced-meld-features-toggle');
assert.ok(html.includes('data-v="china"'), 'China Province is offered in Settings');
assert.ok(html.includes('data-v="china-national"'), 'China Nationwide is offered in Settings');
const options = html
  .slice(html.indexOf('id="meld-china-province-select"'), html.indexOf('</select>', html.indexOf('id="meld-china-province-select"')))
  .match(/<option value="[^"]*"/g);
assert.strictEqual(options.length, 34, '33 places plus the empty prompt');
assert.ok(html.includes('value="hong-kong"') && html.includes('value="inner-mongolia"'));
assert.ok(province && toggle);

const ctx = { oneWorld: false, java: true, caves: false, extendHeight: false, lods: false };
toggle.checked = true;
activeSource = 'china';
province.value = '';
assert.strictEqual(chinaProvinceMissing(), true);
assert.ok(!meldFlags(ctx).some((f) => f.startsWith('--china-province=')));

province.value = 'beijing';
assert.strictEqual(chinaProvinceMissing(), false);
assert.deepStrictEqual(meldFlags(ctx), ['--china-province=beijing']);

activeSource = 'china-national';
assert.strictEqual(chinaProvinceMissing(), false, 'the national source needs no province');
assert.deepStrictEqual(meldFlags(ctx), ['--china-national']);

activeSource = 'default';
assert.strictEqual(chinaProvinceMissing(), false);
assert.deepStrictEqual(meldFlags(ctx), [], 'the stored province is not sent by another source');

activeSource = 'region';
assert.ok(meldFlags(ctx).includes('--osm-pbf=geofabrik'));
assert.ok(!meldFlags(ctx).some((f) => f.includes('china')), 'switching source leaves no China flag');

toggle.checked = false;
activeSource = 'china';
assert.strictEqual(chinaProvinceMissing(), false, 'Advanced off makes every Advanced source inert');

const store = fs.readFileSync(new URL('../../src/gui/js/settings-store.js', import.meta.url), 'utf8');
assert.ok(store.includes("{ id: 'meld-china-province-select', kind: 'select', store: OWN }"));
const layout = fs.readFileSync(new URL('../../src/gui/js/settings-layout.js', import.meta.url), 'utf8');
assert.ok(layout.includes("'meld-china-province-select': 'china province"));

for (const f of fs.readdirSync(new URL('../../src/gui/locales/', import.meta.url)).filter((f) => f.endsWith('.json'))) {
  const loc = JSON.parse(fs.readFileSync(new URL(`../../src/gui/locales/${f}`, import.meta.url), 'utf8'));
  for (const key of [
    'meld_osm_source_china_national',
    'meld_osm_source_china',
    'meld_china_province',
    'meld_china_province_desc',
    'meld_china_province_tip',
    'placeholder_meld_china_province',
    'reason_china',
    'meld_china_province_required',
  ]) {
    assert.ok(loc[key], `${f} lacks ${key}`);
  }
}

console.log('china-source: ok');
