import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { clockText } from './clockText.ts';
const cases = JSON.parse(readFileSync(new URL('./clockText.golden.json', import.meta.url), 'utf8'));
for (const c of cases) test(`LocalClock: ${c.name}`, () => assert.equal(clockText(c.ms, c.hour12, c.seconds, c.zone), c.expected));
