import { describe, expect, it } from 'vitest';

import type { RunSummary } from '#lib/api/types.js';
import { Registry } from '#lib/metrics/registry.js';
import { compareMetrics, configDiff } from './compare.js';

describe('configDiff', () => {
	const configs = [
		{ lr: 0.001, model: { name: 'emb1', depth: 12 }, tags: ['a', 'b'] },
		{ lr: 0.003, model: { name: 'emb1', depth: 12 }, tags: ['a', 'b'] },
		{ lr: 0.001, model: { name: 'emb2' }, seed: 1 }
	];

	it('compares nested keys, missing ones included', () => {
		const rows = configDiff(configs, 0);
		expect(rows.map((r) => r.key)).toEqual(['lr', 'model.depth', 'model.name', 'seed', 'tags']);
		const byKey = Object.fromEntries(rows.map((r) => [r.key, r]));
		expect(byKey['lr'].differs).toBe(true);
		expect(byKey['lr'].changed).toEqual([false, true, false]);
		expect(byKey['model.depth'].values).toEqual([12, 12, undefined]);
		expect(byKey['model.depth'].changed).toEqual([false, false, true]);
		expect(byKey['seed'].changed, 'set only in run 3').toEqual([false, false, true]);
	});

	it('compares lists by value and follows the reference', () => {
		const rows = configDiff(configs, 2);
		const tags = rows.find((r) => r.key === 'tags')!;
		expect(tags.differs).toBe(true);
		expect(tags.changed).toEqual([true, true, false]);
		const same = configDiff([{ a: [1, 2] }, { a: [1, 2] }], 0)[0];
		expect([same.differs, same.changed]).toEqual([false, [false, false]]);
	});
});

describe('compareMetrics', () => {
	const run = (summary: Record<string, unknown>) => ({ summary }) as RunSummary;
	const runs = [run({ loss: 1, acc: 0.5, name: 'x' }), run({ loss: 2, f1: 0.3 })];

	it('picks the important metrics, else the most common ones', () => {
		expect(compareMetrics(runs, new Registry([]))).toEqual(['loss', 'acc', 'f1']);
		const registry = new Registry([
			{ name: 'f1', kind: 'describe', spec: { important: true } },
			{ name: 'acc', kind: 'describe', spec: { hidden: true } }
		]);
		expect(compareMetrics(runs, registry)).toEqual(['f1']);
		expect(
			compareMetrics(
				runs,
				new Registry([{ name: 'acc', kind: 'describe', spec: { hidden: true } }])
			)
		).toEqual(['loss', 'f1']);
	});
});
