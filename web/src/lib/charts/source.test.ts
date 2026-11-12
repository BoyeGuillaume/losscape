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

vi.mock('./tiles.svelte.js', () => ({
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

function sourceFor(formula: string) {
	const f = parseFormula(formula);
	const refs = new Map<string, { raw: string; scale: number }>();
	const collect = (node: typeof f): void => {
		if (node.kind === 'ref') refs.set(node.key, { raw: node.key, scale: 1 });
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
