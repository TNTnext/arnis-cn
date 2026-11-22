// Run: node tests/gui/detail-source.mjs
// The standard map and three increasing open-data packages are separate choices. The package
// is off by default, survives the job-flag round trip, and appears in the Download Plan.
import assert from 'node:assert';
import fs from 'node:fs';
import { applyFlags, meldFlags } from '../../src/gui/js/meld-features.js';

const html = fs.readFileSync(new URL('../../src/gui/index.html', import.meta.url), 'utf8');
const css = fs.readFileSync(new URL('../../src/gui/css/settings.css', import.meta.url), 'utf8');
const state = new Map();
let activeDetail = 'standard';

function fromHtml(id) {
  const tag = html.match(new RegExp(`<(input|select)[^>]* id="${id}"[^>]*>`));
  if (!tag) return null;
  const attr = (name) => (tag[0].match(new RegExp(` ${name}="([^"]*)"`)) || [])[1];
  return {
    value: attr('value') ?? '',
    checked: /\bchecked\b/.test(tag[0]),
    closest: () => null,
    dispatchEvent() {},
  };
}

globalThis.window = {};
globalThis.document = {
  getElementById(id) {
    if (!state.has(id)) state.set(id, fromHtml(id));
    return state.get(id);
  },
  querySelector(selector) {
    if (selector === '#meld-osm-source-group .segment.active') {
      return { dataset: { v: 'default' } };
    }
    if (selector === '#meld-detail-data-group .segment.active') {
      return { dataset: { v: activeDetail } };
    }
    const match = selector.match(/^#meld-detail-data-group \.segment\[data-v="([^"]+)"\]$/);
    return match ? { click: () => { activeDetail = match[1]; } } : null;
  },
  querySelectorAll(selector) {
    if (selector !== '#meld-detail-data-group .segment') return [];
    return ['standard', 'overture', 'china-plus', 'maximum'].map((v) => ({
      dataset: { v },
      disabled: false,
      click() {
        activeDetail = v;
      },
    }));
  },
};

assert.ok(html.includes('id="meld-detail-data-group"'));
for (const profile of ['standard', 'overture', 'china-plus', 'maximum']) {
  assert.ok(html.includes(`data-v="${profile}"`), `the window offers ${profile}`);
}
assert.ok(
  css.includes('#meld-detail-data-group {\n  flex-wrap: wrap;'),
  'long localized profile labels wrap instead of overflowing',
);
const ctx = { oneWorld: false, java: true, caves: false, extendHeight: false, lods: false };

const advanced = document.getElementById('advanced-meld-features-toggle');
advanced.checked = true;
assert.deepStrictEqual(meldFlags(ctx), [], 'standard remains the default');
activeDetail = 'overture';
assert.deepStrictEqual(meldFlags(ctx), ['--map-data-profile=overture']);
activeDetail = 'china-plus';
assert.deepStrictEqual(meldFlags(ctx), ['--map-data-profile=china-plus']);
activeDetail = 'maximum';
assert.deepStrictEqual(meldFlags(ctx), ['--map-data-profile=maximum']);
activeDetail = 'standard';
assert.deepStrictEqual(meldFlags(ctx), [], 'switching back leaves no detail flag behind');

applyFlags(['--overture-detail=true']);
assert.strictEqual(activeDetail, 'overture', 'a legacy job restores Overture Complete');
applyFlags(['--map-data-profile=china-plus']);
assert.strictEqual(activeDetail, 'china-plus', 'a resumed job restores China Enhanced');
applyFlags([]);
assert.strictEqual(activeDetail, 'standard', 'a job without the flag restores the standard layer');

const store = fs.readFileSync(new URL('../../src/gui/js/settings-store.js', import.meta.url), 'utf8');
assert.ok(store.includes("{ id: 'meld-detail-data-group', kind: 'segmented', store: OWN, valueAttr: 'data-v' }"));
const layout = fs.readFileSync(new URL('../../src/gui/js/settings-layout.js', import.meta.url), 'utf8');
assert.ok(layout.includes("'meld-detail-data-group': 'detailed complete detail overture china"));
const plan = fs.readFileSync(new URL('../../src/gui/js/meld-features.js', import.meta.url), 'utf8');
assert.ok(plan.includes("overture_detail: 'Buildings, roads and places'"));
assert.ok(plan.includes("open_buildings_vida: 'China Open Buildings'"));
assert.ok(plan.includes("cnbh_heights: 'CNBH building heights'"));
assert.ok(plan.includes("overture_base: 'Water and land use'"));

for (const file of fs.readdirSync(new URL('../../src/gui/locales/', import.meta.url)).filter((f) => f.endsWith('.json'))) {
  const locale = JSON.parse(fs.readFileSync(new URL(`../../src/gui/locales/${file}`, import.meta.url), 'utf8'));
  for (const key of [
    'detail_data',
    'detail_data_desc',
    'detail_data_standard',
    'detail_data_overture',
    'detail_data_china_plus',
    'detail_data_maximum',
    'detail_data_tip',
    'data_source_overture_detail',
    'data_source_open_buildings_vida',
    'data_source_cnbh_heights',
    'data_source_overture_base',
  ]) {
    assert.ok(locale[key], `${file} lacks ${key}`);
  }
}

console.log('detail-source: ok');
