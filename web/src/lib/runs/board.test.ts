import { describe, expect, it } from 'vitest';

import type { RunSummary } from '#lib/api/types.js';
import { parseFilter } from '#lib/filter.js';
import { Registry } from '#lib/metrics/registry.js';
import {
	assignSlots,
	bestSoFar,
	boardColumns,
	changeFrom,
	chartedRuns,
	clampLimit,
	differingConfig,
	explorePlot,
	exploreKeys,
	headlineFor,
	importantKeys,
	matchingRuns,
	MAX_CHARTED,
	naturalSort,
	metricChoice,
	progressDots,
	sortChoices,
	sortLabel,
	thinDots,
	valueNote
} from './board.js';

const run = (
	name: string,
	summary: Record<string, unknown>,
	tags: string[] = [],
	config: Record<string, unknown> = {},
	start_ms = 0
) =>
	({
		id: name,
		name,
		summary,
		config,
		tags,
		aliases: [],
		state: 'finished',
		group_name: null,
		created_at_ms: Number(name.replace(/\D/g, '')) || 0,
		start_ms,
		finished_at_ms: 0,
		heartbeat_at_ms: 0,
		last_step: 1
	}) as unknown as RunSummary;

const registry = new Registry([
	{ name: 'val/acc', kind: 'describe', spec: { important: true, direction: 'max', label: 'Acc' } },
	{ name: 'val/loss', kind: 'describe', spec: { important: true, direction: 'min' } },
	{ name: 'noise', kind: 'describe', spec: { important: true, hidden: true } }
]);
const d = (key: string) => registry.describe(key);

const RUNS = [
	run('r1', { 'val/acc': 0.5, 'val/loss': 3, lr: 1, noise: 1 }, ['good']),
	run('r2', { 'val/acc': 0.9, 'val/loss': 1, lr: 2 }, ['good']),
	run('r3', { 'val/acc': 0.7, 'val/loss': 2, lr: 3 }),
	run('r4', { lr: 4 }, ['good'])
];

describe('board', () => {
	it('keeps what matches, the best first, runs without the metric last', () => {
		const names = (query: string, id: string, desc: boolean) =>
			matchingRuns(RUNS, parseFilter(query), { id, desc }).map((r) => r.name);
		expect(names('', 'summary.val/acc', true)).toEqual(['r2', 'r3', 'r1', 'r4']);
		expect(names('', 'summary.val/acc', false)).toEqual(['r1', 'r3', 'r2', 'r4']);
		expect(names('tag:good', 'summary.val/acc', true)).toEqual(['r2', 'r1', 'r4']);
		expect(names('', 'created', true)).toEqual(['r4', 'r3', 'r2', 'r1']);
	});

	it('does not reorder the list it was given', () => {
		const before = RUNS.map((r) => r.name);
		matchingRuns(RUNS, null, { id: 'created', desc: true });
		expect(RUNS.map((r) => r.name)).toEqual(before);
	});

	it('limits between 1 and 40', () => {
		expect([clampLimit(0), clampLimit(7.4), clampLimit(500)]).toEqual([1, 7, 40]);
	});

	it('charts what is ticked among the runs shown, or else the first ones', () => {
		const many = Array.from({ length: 12 }, (_, i) => run(`r${i}`, {}));
		expect(chartedRuns(many, () => false)).toHaveLength(MAX_CHARTED);
		expect(chartedRuns(many, (id) => id === 'r9').map((r) => r.name)).toEqual(['r9']);
		expect(chartedRuns(many, (id) => id === 'elsewhere')).toHaveLength(MAX_CHARTED);
		// The baseline comes along with what's ticked.
		expect(chartedRuns(many, (id) => id === 'r9', 'r0').map((r) => r.name)).toEqual(['r0', 'r9']);
	});

	it('keeps a run in its color as the order changes', () => {
		const first = assignSlots(['a', 'b', 'c'], new Map());
		expect([...first.values()]).toEqual([0, 1, 2]);
		const moved = assignSlots(['c', 'a', 'b'], first);
		expect(moved).toEqual(first);
		const replaced = assignSlots(['c', 'd', 'a'], moved);
		expect(replaced.get('c')).toBe(2);
		expect(replaced.get('a')).toBe(0);
		expect(replaced.get('d')).toBe(1); // the slot b left
		expect(new Set(replaced.values()).size).toBe(3);
	});

	it('finds important metrics, leaving hidden ones out', () => {
		expect(importantKeys(RUNS, d).sort()).toEqual(['val/acc', 'val/loss']);
		expect(importantKeys([run('x', { lr: 1 })], d)).toEqual([]);
	});

	it('shows important metrics, and the sorted one whatever it is', () => {
		expect(boardColumns(RUNS, d, 'created')).toEqual(
			expect.arrayContaining(['summary.val/acc', 'summary.val/loss', 'created', 'tags'])
		);
		expect(boardColumns(RUNS, d, 'created')).not.toContain('summary.lr');
		expect(boardColumns(RUNS, d, 'summary.lr')[0]).toBe('summary.lr');
		expect(boardColumns(RUNS, d, 'name')).not.toContain('name');
		// Nothing important: the usual columns.
		expect(boardColumns([run('x', { lr: 1 })], d, 'created')).toContain('summary.lr');
	});

	it('leaves out run fields that read the same, unless sorted by them', () => {
		expect(boardColumns(RUNS, d, 'created')).not.toContain('state');
		expect(boardColumns(RUNS, d, 'state')).toContain('state');
		const mixed = [...RUNS, { ...run('r5', {}), state: 'running' } as RunSummary];
		expect(boardColumns(mixed, d, 'created')).toContain('state');
		const untagged = [run('a', { 'val/acc': 1 }), run('b', { 'val/acc': 2 })];
		expect(boardColumns(untagged, d, 'created')).not.toContain('tags');
	});

	it('finds the config that tells runs apart', () => {
		const runs = ['a', 'b', 'c'].map((name, i) =>
			run(name, {}, [], {
				seed: i,
				lr: i ? 0.1 : 0.01,
				same: 'x',
				dir: `/runs/${name}`,
				model: { name: i ? 'vit' : 'emb' }
			})
		);
		expect(differingConfig(runs)).toEqual(['lr', 'model.name', 'seed']);
		expect(differingConfig(runs, 1)).toEqual(['lr']);
		expect(boardColumns(runs, d, 'created')).toEqual(
			expect.arrayContaining(['config.lr', 'config.model.name'])
		);
		expect(exploreKeys(runs)).toEqual([
			{ key: 'lr', kind: 'number' },
			{ key: 'model.name', kind: 'category' },
			{ key: 'seed', kind: 'number' }
		]);
	});

	it('words a change for its unit, better following the direction', () => {
		const fraction = new Registry([
			{ name: 'acc', kind: 'describe', spec: { unit: { preset: 'fraction' }, direction: 'max' } }
		]).describe('acc');
		expect(changeFrom(0.902, 0.892, fraction)).toEqual({ text: '↑ +1.0 pts', better: true });
		expect(changeFrom(0.8, 0.9, fraction).better).toBe(false);
		const loss = d('val/loss');
		expect(changeFrom(1, 2, loss)).toEqual({ text: '↓ −50%', better: true });
		expect(changeFrom(3, 2, loss)).toEqual({ text: '↑ +50%', better: false });
		expect(changeFrom(2, 2, loss)).toEqual({ text: 'no change', better: null });
		expect(changeFrom(3, 2, { unit: null, direction: null }).better).toBe(null);
		expect(changeFrom(2, 0, { unit: null, direction: null }).text).toBe('↑ +2');
		expect(changeFrom(-1, -2, { unit: null, direction: null }).text).toBe('↑ +50%');
	});

	it('names the best run and its lead', () => {
		const acc = d('val/acc');
		const lead = headlineFor(RUNS, 'val/acc', acc, null)!;
		expect([lead.run.name, lead.value, lead.against]).toEqual(['r2', 0.9, 'runner-up']);
		const loss = headlineFor(RUNS, 'val/loss', d('val/loss'), null)!;
		expect(loss.run.name).toBe('r2'); // lower is better
		const vs = headlineFor(RUNS, 'val/acc', acc, RUNS[0])!;
		expect([vs.against, vs.change?.better]).toEqual(['baseline', true]);
		const itself = headlineFor(RUNS, 'val/acc', acc, RUNS[1])!;
		expect([itself.isBaseline, itself.change]).toEqual([true, null]);
		// A baseline without the metric: as if there were none.
		expect(headlineFor(RUNS, 'val/acc', acc, RUNS[3])!.against).toBe('runner-up');
		expect(headlineFor(RUNS, 'absent', acc, null)).toBe(null);
		// No direction: no best to speak of, so the board's first run and no lead.
		const plain = headlineFor(RUNS, 'lr', d('lr'), null)!;
		expect([plain.run.name, plain.change]).toEqual(['r1', null]);
	});

	it('draws the best so far in steps', () => {
		const runs = [
			run('c', { m: 3 }, [], {}, 30),
			run('a', { m: 1 }, [], {}, 10),
			run('b', { m: 2 }, [], {}, 20),
			run('d', { m: 2.5 }, [], {}, 40),
			run('x', {}, [], {}, 50)
		];
		const dots = progressDots(runs, 'm');
		expect(dots.map((p) => p.name)).toEqual(['a', 'b', 'c', 'd']);
		const up = bestSoFar(dots, 'max');
		expect([...up.records]).toEqual(['a', 'b', 'c']);
		expect(up.line.map((p) => [p.x, p.y])).toEqual([
			[10, 1],
			[20, 1],
			[20, 2],
			[30, 2],
			[30, 3],
			[40, 3]
		]);
		expect([...bestSoFar(dots, 'min').records]).toEqual(['a']);
		expect(bestSoFar(dots, null).line).toEqual([]);
		expect(bestSoFar([], 'max').line).toEqual([]);
	});

	it('thins out dots, keeping the ones that matter', () => {
		const dots = Array.from({ length: 10_000 }, (_, i) => ({
			id: `r${i}`,
			name: `r${i}`,
			x: i,
			y: i,
			running: false
		}));
		const kept = thinDots(dots, new Set(['r7']));
		expect(kept.length).toBeLessThanOrEqual(2_001);
		expect(kept.length).toBeGreaterThan(1_900);
		expect(kept.some((p) => p.id === 'r7')).toBe(true);
		expect(thinDots(dots.slice(0, 50), new Set())).toHaveLength(50);
	});

	it('plots a metric against numbers or categories', () => {
		const runs = [
			run('a', { m: 1 }, [], { lr: 0.001, model: 'vit' }),
			run('b', { m: 2 }, [], { lr: 0.1, model: 'emb' }),
			run('c', { m: 3 }, [], { model: 'emb' }),
			run('d', {}, [], { lr: 0.01, model: 'vit' })
		];
		const num = explorePlot(runs, 'm', { key: 'lr', kind: 'number' });
		expect(num.dots.map((p) => p.x)).toEqual([0.001, 0.1]);
		expect(num.log).toBe(true);
		expect(explorePlot(runs.slice(1), 'm', { key: 'lr', kind: 'number' }).log).toBe(false);
		const cat = explorePlot(runs, 'm', { key: 'model', kind: 'category' });
		expect(cat.categories).toEqual(['emb', 'vit']);
		expect(cat.dots.map((p) => Math.round(p.x) + 0)).toEqual([1, 0, 0]);
		// Jitter stays put from one render to the next, inside its category.
		const again = explorePlot(runs, 'm', { key: 'model', kind: 'category' });
		expect(again.dots.map((p) => p.x)).toEqual(cat.dots.map((p) => p.x));
		expect(cat.dots.every((p) => Math.abs(p.x - Math.round(p.x)) <= 0.25)).toBe(true);
	});

	it('plots a metric against another', () => {
		const runs = [run('a', { m: 1, n: 10 }), run('b', { m: 2 }), run('c', { m: 3, n: 30 })];
		const plot = explorePlot(runs, 'm', metricChoice('n'));
		expect(plot.dots.map((p) => [p.name, p.x, p.y])).toEqual([
			['a', 10, 1],
			['c', 30, 3]
		]);
		expect([plot.categories, plot.log]).toEqual([null, false]);
	});

	it('says which value a chart shows', () => {
		const best = { summary: 'best', direction: 'max' } as const;
		const last = { summary: 'last', direction: 'max' } as const;
		const plain = { summary: 'last', direction: null } as const;
		expect(valueNote('default', best, false)).toBe('best value');
		expect(valueNote('default', last, false)).toBe('last value');
		expect(valueNote('last', best, false)).toBe('last value');
		expect(valueNote('best', last, false)).toBe('best value');
		expect(valueNote('best', plain, false)).toBe('last value (no direction)');
		expect(valueNote('best', last, true)).toBe('last value (computed)');
		expect(valueNote('default', best, true)).toBe('last value (computed)');
		expect(valueNote('last', best, true)).toBe('last value');
	});

	it('offers important metrics apart, never hidden ones', () => {
		const choices = sortChoices(RUNS, d);
		expect(choices.important.sort()).toEqual(['summary.val/acc', 'summary.val/loss']);
		expect(choices.metrics).toEqual(['summary.lr']);
	});

	it('names sorts, and takes the best values first', () => {
		expect(sortLabel('summary.val/acc', d)).toBe('Acc');
		expect(sortLabel('summary.lr', d)).toBe('lr');
		expect(sortLabel('created', d)).toBe('Created');
		expect(sortLabel('name', d)).toBe('Name');
		expect(sortLabel('config.lr', d)).toBe('config.lr');
		expect(naturalSort('summary.val/acc', d).desc).toBe(true);
		expect(naturalSort('summary.val/loss', d).desc).toBe(false);
		expect(naturalSort('created', d).desc).toBe(true);
		expect(naturalSort('name', d).desc).toBe(false);
	});
});
