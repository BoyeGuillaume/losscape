import { describe, expect, it } from 'vitest';

import { effectiveSummary } from './apply.js';
import { detectFamilies } from './families.js';
import { evaluate, parseFormula, references } from './formula.js';
import { match } from './pattern.js';
import { Registry, problems, unitOf, type MetricDef } from './registry.js';
import { formatUnit } from '#lib/units.js';

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
	{ name: 'val/*', kind: 'describe', spec: { direction: 'max', unit: { preset: 'fraction' } } },
	{ name: 'val/pck-{k}', kind: 'family', spec: { bounds: [0, 1], summary: 'best' } },
	{
		name: 'val/pck-8',
		kind: 'describe',
		spec: { label: 'PCK at 8 px', important: true }
	},
	{ name: 'val/loss', kind: 'describe', spec: { direction: 'min', hidden: true } },
	{
		name: 'val/f1',
		kind: 'formula',
		spec: { expression: '2 * [val/p] * [val/r] / ([val/p] + [val/r])', summary: 'last' }
	},
	{ name: 'broken', kind: 'formula', spec: { expression: '1 +' } }
];
const registry = new Registry(defs);

describe('registry', () => {
	it('describes field by field: own, then the most specific pattern, then built-in', () => {
		const pck8 = registry.describe('val/pck-8');
		expect([pck8.label, pck8.important, pck8.direction, pck8.summary]).toEqual([
			'PCK at 8 px',
			true,
			'max',
			'best'
		]);
		expect(pck8.def?.name, 'its own definition').toBe('val/pck-8');
		expect(pck8.unit?.scale, 'unit from val/*').toBe(100);
		expect(registry.describe('val/pck-16').bounds).toEqual([0, 1]);
		expect(registry.describe('val/loss').direction, 'own beats the pattern').toBe('min');
		expect(registry.describe('val/f1').summary).toBe('last');
		expect(registry.describe('system/cpu/util_pct').bounds).toEqual([0, 100]);
		const process = registry.describe('system/cpu/process_pct');
		expect(process.bounds, 'many threads: above 100%').toEqual([0, null]);
		expect(process.unit?.steps[0].suffix).toBe('%');
		const read = registry.describe('system/disk/read').unit!;
		expect(formatUnit(3.5 * 2 ** 20, read), 'scaled').toBe('3.5 MiB/s');
		expect(formatUnit(512, read)).toBe('512 B/s');
		expect(registry.describe('train/loss').def).toBeNull();
		expect(registry.inherited('val/pck-8').label, 'without its own').toBeNull();
	});

	it('skips invalid definitions and explains them', () => {
		expect(registry.computedNames).toEqual(['val/f1']);
		expect(problems(defs[5])[0]).toMatch(/^Formula:/);
		expect(problems({ name: 'blank', kind: 'formula', spec: { expression: '' } })).toEqual([]);
		expect(problems({ name: 'val/{a}-{b}', kind: 'family', spec: {} })).toHaveLength(1);
	});

	it('shows renamed and rescaled metrics, leaving hidden ones out', () => {
		const run = registry.forRun(['val/pck-8-fine', 'val/loss'], {
			renames: { 'val/pck-8-fine': 'val/pck-8' },
			rename_scales: { 'val/pck-8-fine': 0.01 }
		});
		expect(run.resolve('val/pck-8')).toEqual({ kind: 'key', raw: 'val/pck-8-fine', scale: 0.01 });
		expect(run.source('val/pck-8')).toBe('val/pck-8-fine');
		expect(run.keys()).toEqual(['val/pck-8']);
		expect(run.keys(true)).toEqual(['val/pck-8', 'val/loss']);
	});

	it('resolves formulas only when every input exists', () => {
		expect(registry.forRun(['val/p', 'val/r']).resolve('val/f1')?.kind).toBe('formula');
		expect(registry.forRun(['val/p']).resolve('val/f1')).toBeNull();
	});
});

describe('effective summaries', () => {
	it('apply renames with their scale, best values and formulas', () => {
		const summary = effectiveSummary(
			{
				summary: { 'val/pck-8-fine': 70, 'val/p': 0.5, 'val/r': 1, old: 3 },
				metric_min: { 'val/pck-8-fine': 10, 'val/p': 0.1, 'val/r': 0.2, old: 1 },
				metric_max: { 'val/pck-8-fine': 80, 'val/p': 0.6, 'val/r': 1, old: 4 },
				renames: { old: 'new', 'val/pck-8-fine': 'val/pck-8' },
				rename_scales: { 'val/pck-8-fine': 0.01 }
			},
			registry
		);
		expect(summary['new']).toBe(3);
		expect(summary['old']).toBeUndefined();
		expect(summary['val/pck-8'], 'best of the logged metric, scaled').toBeCloseTo(0.8);
		expect(summary['val/f1']).toBeCloseTo(2 / 3);
		expect(summary['val/p'], 'last unless "best"').toBe(0.5);
	});
});

describe('family detection', () => {
	it('groups names that differ by a number in their last part', () => {
		const keys = [
			'val/pck-16',
			'val/pck-4',
			'val/pck-8',
			'val/pck-8-fine',
			'train/loss',
			'layer2/w',
			'system/gpu0/util_pct',
			'system/gpu1/util_pct',
			'system/gpu2/util_pct'
		];
		expect(detectFamilies(keys)).toEqual([
			{ pattern: 'val/pck-{k}', members: ['val/pck-4', 'val/pck-8', 'val/pck-16'] }
		]);
		expect(detectFamilies(['top1', 'top5'], 2)[0].pattern).toBe('top{k}');
		expect(detectFamilies(['val/pck-16px', 'val/pck-4px', 'val/pck-8px'])).toEqual([
			{ pattern: 'val/pck-{k}px', members: ['val/pck-4px', 'val/pck-8px', 'val/pck-16px'] }
		]);
		expect(match('val/pck-{k}px', 'val/pck-16px'), 'a suffix after the number').toEqual({
			k: '16'
		});
	});
});

describe('custom units', () => {
	it('show a suffix, alone or after k, M, B, times a factor', () => {
		expect(formatUnit(950, unitOf({ preset: 'custom', suffix: 'px' })!)).toBe('950 px');
		const compact = unitOf({ preset: 'custom', suffix: 'px', compact: true })!;
		expect(formatUnit(12_345, compact)).toBe('12.3k px');
		expect(formatUnit(950, compact)).toBe('950 px');
		const scaled = unitOf({ preset: 'custom', suffix: 'tok', compact: true, scale: 1000 })!;
		expect(formatUnit(1.5, scaled), 'logged in thousands').toBe('1.5k tok');
	});
});
