import { describe, expect, it } from 'vitest';

import type { RunSummary } from '#lib/api/types.js';
import { matches, parseFilter } from './filter.js';
import { literal, suggest } from './filter-suggest.js';
import { buildCatalog } from './runs/catalog.js';

function run(
	name: string,
	config: Record<string, unknown>,
	extra: Partial<RunSummary> = {}
): RunSummary {
	return {
		id: name,
		name,
		state: 'finished',
		group_name: null,
		tags: [],
		aliases: [],
		created_at_ms: 0,
		start_ms: 0,
		archived_at_ms: null,
		last_step: null,
		metric_min: {},
		metric_max: {},
		abort_requested_at_ms: null,
		renames: {},
		rename_scales: {},
		heartbeat_at_ms: 0,
		finished_at_ms: null,
		summary: { 'val/acc': 0.9 },
		config,
		...extra
	};
}

const runs = [
	run('a', { lr: 1e-3, optimizer: { name: 'AdamW' }, seed: 1 }, { tags: ['baseline'] }),
	run('b', { lr: 1e-3, optimizer: { name: 'SGD' }, seed: 2 }),
	run(
		'c',
		{ lr: 3e-4, optimizer: { name: 'AdamW' } },
		{ state: 'crashed', tags: ['baseline', 'v2'] }
	)
];
const catalog = buildCatalog(runs);
const at = (text: string) => suggest(text, text.length, catalog);
const labels = (text: string) => at(text).items.map((s) => s.label);

describe('buildCatalog', () => {
	it('counts keys and values', () => {
		const lr = catalog.config.find((s) => s.key === 'lr')!;
		expect(lr).toMatchObject({ runs: 3, distinct: 2 });
		expect(lr.values).toEqual([
			{ value: 0.001, count: 2 },
			{ value: 0.0003, count: 1 }
		]);
		expect(catalog.config.find((s) => s.key === 'seed')!.runs).toBe(2);
		expect(catalog.config.map((s) => s.key)).toEqual(['lr', 'optimizer.name', 'seed']);
		expect(catalog.tags).toEqual([
			{ value: 'baseline', count: 2 },
			{ value: 'v2', count: 1 }
		]);
	});
});

describe('suggest', () => {
	it('completes fields, prefix matches first', () => {
		expect(labels('conf')).toEqual(['config.lr', 'config.optimizer.name', 'config.seed']);
		expect(labels('opt')).toEqual(['config.optimizer.name']);
		expect(labels('st').slice(0, 2)).toEqual(['state:', 'steps']);
		expect(labels(''), 'nothing typed').toEqual([]);
		expect(labels('tag:baseline and'), 'keywords').toEqual([]);
	});

	it('completes values of the field being compared', () => {
		expect(labels('config.lr = ')).toEqual(['0.001', '0.0003']);
		expect(labels('config.lr=0.0003')).toEqual(['0.0003']);
		expect(labels('optimizer.name:sg')).toEqual(['SGD']);
		expect(labels('tag:')).toEqual(['baseline', 'v2']);
		expect(labels('state:cr')).toEqual(['crashed']);
		expect(labels('config.unknown = '), 'unknown field').toEqual([]);
	});

	it('replaces just the word being typed', () => {
		const text = 'state:finished and config.lr = 0.00';
		const s = at(text);
		const done = text.slice(0, s.from) + s.items[0].insert + text.slice(s.to);
		expect(done).toBe('state:finished and config.lr = 0.001 ');
	});
});

describe('literal', () => {
	it('writes values that parse back to themselves', () => {
		for (const value of ['AdamW', 'two words', 'say "hi"', '1e-3', 'and', 0.001, true, null]) {
			const node = parseFilter(`config.v = ${literal(value)}`);
			const r = run('x', { v: value });
			expect(matches(node, r), `${String(value)} -> ${literal(value)}`).toBe(true);
		}
		expect(literal('AdamW')).toBe('AdamW');
		expect(literal('two words')).toBe('"two words"');
		expect(literal('1e-3'), 'text that looks like a number').toBe('"1e-3"');
	});
});
