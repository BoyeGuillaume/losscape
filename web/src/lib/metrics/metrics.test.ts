import { describe, expect, it } from 'vitest';

import { effectiveSummary } from './apply.js';
import { evaluate, parseFormula, references } from './formula.js';
import { match } from './pattern.js';
import { Registry, problems, type MetricDef } from './registry.js';

describe('patterns', () => {
	it('capture parameters and match wildcards', () => {
		expect(match('val/pck-{k}', 'val/pck-16')).toEqual({ k: '16' });
		expect(match('val/pck-{k}', 'val/pck-8/extra')).toBeNull();
		expect(match('system/*_pct', 'system/gpu0/util_pct')).toEqual({});
		expect(match('a.b', 'aXb'), 'dots are literal').toBeNull();
	});
});

describe('formulas', () => {
	it('parse with precedence and evaluate', () => {
		const f = parseFormula('2 * [val/p] * [val/r] / ([val/p] + [val/r])');
		expect([...references(f)]).toEqual(['val/p', 'val/r']);
		const env: Record<string, number> = { 'val/p': 0.5, 'val/r': 1 };
		expect(evaluate(f, (k) => env[k])).toBeCloseTo(2 / 3);
		expect(evaluate(parseFormula('-2 ^ 2 + max(1, [x], 3)'), () => 5)).toBe(1);
		expect(
			evaluate(parseFormula('1 / [x]'), () => 0),
			'not finite'
		).toBeNull();
		expect(
			evaluate(parseFormula('[x] + 1'), () => null),
			'missing'
		).toBeNull();
	});

	it('report mistakes', () => {
		for (const bad of ['', '[a', '1 +', 'foo(1)', '2 ** 3', 'val/p + 1']) {
			expect(() => parseFormula(bad), bad).toThrow();
		}
	});
});

const defs: MetricDef[] = [
	{
		name: 'val/pck@8',
		kind: 'merge',
		spec: {
			sources: [{ key: 'val/pck-8' }, { key: 'val/pck-8-fine', scale: 0.01 }],
			direction: 'max',
			summary: 'best',
			hideSources: true
		}
	},
	{
		name: 'val/pck-{k}',
		kind: 'family',
		spec: { direction: 'max', bounds: [0, 1], unit: { preset: 'fraction' } }
	},
	{
		name: 'val/pck-8',
		kind: 'describe',
		spec: { label: 'PCK at 8 px', summary: 'best', direction: 'max' }
	},
	{
		name: 'val/f1',
		kind: 'formula',
		spec: { expression: '2 * [val/p] * [val/r] / ([val/p] + [val/r])' }
	},
	{ name: 'broken', kind: 'formula', spec: { expression: '1 +' } }
];
const registry = new Registry(defs);

describe('registry', () => {
	it('describes: exact, then virtual, then the most specific pattern, then built-in', () => {
		expect(registry.describe('val/pck-8').label).toBe('PCK at 8 px');
		expect(registry.describe('val/pck-16').def?.name).toBe('val/pck-{k}');
		expect(registry.describe('val/pck-16').unit?.scale).toBe(100);
		expect(registry.describe('val/pck@8').summary).toBe('best');
		expect(registry.describe('system/cpu/util_pct').bounds).toEqual([0, 100]);
		expect(registry.describe('train/loss').def).toBeNull();
	});

	it('skips invalid definitions and explains them', () => {
		expect(registry.virtualNames).toEqual(['val/pck@8', 'val/f1']);
		expect(problems(defs[4])[0]).toMatch(/^Formula:/);
		expect(problems({ name: 'val/{a}-{b}', kind: 'family', spec: {} })).toHaveLength(1);
	});

	it('resolves merges per run, in order, through renames', () => {
		const fine = registry.forRun(['val/pck-8-fine', 'loss']);
		expect(fine.resolve('val/pck@8')).toEqual({ kind: 'key', raw: 'val/pck-8-fine', scale: 0.01 });
		const renamed = registry.forRun(['val/pck-8px'], { 'val/pck-8px': 'val/pck-8' });
		expect(renamed.resolve('val/pck@8')).toEqual({ kind: 'key', raw: 'val/pck-8px', scale: 1 });
		expect(renamed.keys(), 'merged sources hidden').toEqual(['val/pck@8']);
		expect(registry.forRun(['loss']).resolve('val/pck@8')).toBeNull();
	});

	it('resolves formulas only when every input exists', () => {
		expect(registry.forRun(['val/p', 'val/r']).resolve('val/f1')?.kind).toBe('formula');
		expect(registry.forRun(['val/p']).resolve('val/f1')).toBeNull();
	});
});

describe('effective summaries', () => {
	it('apply renames, best values, merges and formulas', () => {
		const summary = effectiveSummary(
			{
				summary: { 'val/pck-8-fine': 70, 'val/p': 0.5, 'val/r': 1, old: 3 },
				metric_min: { 'val/pck-8-fine': 10, 'val/p': 0.1, 'val/r': 0.2, old: 1 },
				metric_max: { 'val/pck-8-fine': 80, 'val/p': 0.6, 'val/r': 1, old: 4 },
				renames: { old: 'new' }
			},
			registry
		);
		expect(summary['new']).toBe(3);
		expect(summary['old']).toBeUndefined();
		expect(summary['val/pck@8'], 'best of the source, scaled').toBeCloseTo(0.8);
		expect(summary['val/f1']).toBeCloseTo(2 / 3);
		expect(summary['val/p'], 'last by default').toBe(0.5);
		expect(summary['val/pck-8-fine'], 'hidden sources stay filterable').toBe(70);
	});
});
