import { describe, expect, it } from 'vitest';

import type { RunSummary } from '#lib/api/types.js';
import { parseFilter } from '#lib/filter.js';
import { Registry } from '#lib/metrics/registry.js';
import {
	assignSlots,
	boardColumns,
	chartedRuns,
	clampLimit,
	importantKeys,
	matchingRuns,
	MAX_CHARTED,
	naturalSort,
	sortChoices,
	sortLabel
} from './board.js';

const run = (name: string, summary: Record<string, unknown>, tags: string[] = []) =>
	({
		id: name,
		name,
		summary,
		config: {},
		tags,
		aliases: [],
		state: 'finished',
		group_name: null,
		created_at_ms: Number(name.replace(/\D/g, '')) || 0,
		start_ms: 0,
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
		expect(boardColumns(RUNS, d, 'summary.lr').slice(0, 2)).toEqual(['state', 'summary.lr']);
		expect(boardColumns(RUNS, d, 'name')).not.toContain('name');
		// Nothing important: the usual columns.
		expect(boardColumns([run('x', { lr: 1 })], d, 'created')).toContain('summary.lr');
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
