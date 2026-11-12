import { describe, expect, it, vi } from 'vitest';

import { parseFormula } from '#lib/metrics/formula.js';
import type { Registry, Resolution, RunMetrics } from '#lib/metrics/registry.js';
import { MetricSource } from './source.js';
import type { SeriesPoints } from './tiles.svelte.js';

// Logged points per metric, one bucket per step.
const logged: Record<string, [number, number][]> = {
	'train/flops': [
		[0, 10],
		[1, 20],
		[2, 30]
	],
	'model/params': [[0, 5]],
	'model/layers': [[0, 2]]
};

// Statistics over all points, as the server would give them; `loading` hasn't arrived.
const totals: Record<
	string,
	{ n: number; min: number; max: number; mean: number; median: number }
> = {
	'train/flops': { n: 3, min: 10, max: 30, mean: 20, median: 20 }
};
let loading = false;

vi.mock('./tiles.svelte.js', () => ({
	tileStore: {
		stats: (_run: string, key: string) => (loading ? undefined : (totals[key] ?? null))
	},
	seriesInRange: (_run: string, key: string, from: number, to: number): SeriesPoints => {
		const points = logged[key].filter(([step]) => step >= from && step <= to);
		const values = points.map(([, v]) => v);
		return {
			x: points.map(([step]) => step),
			n: points.map(() => 1),
			time: points.map(([step]) => step),
			mean: values,
			min: values,
			max: values,
			complete: true
		};
	}
}));

function sourceFor(formula: string, scales: Record<string, number> = {}) {
	const f = parseFormula(formula);
	const refs = new Map<string, { raw: string; scale: number }>();
	const collect = (node: typeof f): void => {
		if (node.kind === 'ref' || node.kind === 'agg')
			refs.set(node.key, { raw: node.key, scale: scales[node.key] ?? 1 });
		else if (node.kind === 'neg') collect(node.arg);
		else if (node.kind === 'op') [node.left, node.right].forEach(collect);
		else if (node.kind === 'call') node.args.forEach(collect);
	};
	collect(f);
	const resolution: Resolution = { kind: 'formula', formula: f, refs };
	const run = { resolve: () => resolution } as unknown as RunMetrics;
	return new MetricSource({} as Registry, new Map([['r', run]]), (_run, raw) => {
		const steps = logged[raw].map(([step]) => step);
		return { min: Math.min(...steps), max: Math.max(...steps) };
	});
}

describe('formulas over a metric logged once', () => {
	it('hold it at every step', () => {
		const source = sourceFor('[train/flops] / [model/params]');
		const s = source.points('r', 'm', 0, 2, 100)!;
		expect(s.x).toEqual([0, 1, 2]);
		expect(s.mean).toEqual([2, 4, 6]);
		expect(source.extentOf('r', 'm')).toEqual({ min: 0, max: 2 });
	});

	it('still give a single point when every input was logged once', () => {
		const source = sourceFor('[model/params] * [model/layers]');
		const s = source.points('r', 'm', 0, 2, 100)!;
		expect(s.x).toEqual([0]);
		expect(s.mean).toEqual([10]);
	});

	it('show the whole run when zoomed away from the step it was logged at', () => {
		const s = sourceFor('[train/flops] / [model/params]').points('r', 'm', 1, 2, 100)!;
		expect(s.mean).toEqual([4, 6]);
	});
});

describe('formulas over a whole run', () => {
	it('compare each step with an aggregate', () => {
		const s = sourceFor('[train/flops] / highest([train/flops])').points('r', 'm', 0, 2, 100)!;
		expect(s.mean).toEqual([1 / 3, 2 / 3, 1]);
	});

	it('span the steps of what they aggregate, when that is all they read', () => {
		const source = sourceFor('mean([train/flops])');
		expect(source.extentOf('r', 'm')).toEqual({ min: 0, max: 2 });
		expect(source.points('r', 'm', 0, 2, 100)!.mean).toEqual([20, 20, 20]);
	});

	it('swap lowest and highest through a negative factor', () => {
		const s = sourceFor('lowest([train/flops])', { 'train/flops': -1 }).points(
			'r',
			'm',
			0,
			2,
			100
		)!;
		expect(s.mean[0]).toBe(-30);
	});

	it('wait for the statistics', () => {
		loading = true;
		try {
			const s = sourceFor('mean([train/flops])').points('r', 'm', 0, 2, 100)!;
			expect(s.x).toEqual([]);
			expect(s.complete).toBe(false);
		} finally {
			loading = false;
		}
	});
});
