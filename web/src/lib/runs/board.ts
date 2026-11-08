// A project's board (its Overview tab): which runs it shows, and how it charts them. The
// query, sort and limit are saved with the project, and applied here.

import type { RunSummary } from '#lib/api/types.js';
import { matches, type Node } from '#lib/filter.js';
import type { Meta } from '#lib/metrics/registry.js';
import { availableKeys, column, compareRuns, defaultColumns } from './columns.js';

export type BoardSort = { id: string; desc: boolean };
export type Board = { query: string; sort: BoardSort; limit: number };

export const DEFAULT_BOARD: Board = {
	query: '',
	sort: { id: 'created', desc: true },
	limit: 10
};
/** Runs a board lists at most: as many as Compare takes. */
export const MAX_LIMIT = 40;
/** Runs charted at most: one color each, never the same one twice. */
export const MAX_CHARTED = 8;

type Describe = (key: string) => Meta;

const RUN_FIELDS = ['created', 'runtime', 'steps', 'name'];
const MAX_SORT_CHOICES = 100;

export const clampLimit = (n: number) => Math.min(MAX_LIMIT, Math.max(1, Math.round(n)));

/** The matching runs, sorted (the board shows the first `limit`). */
export function matchingRuns(runs: RunSummary[], query: Node | null, sort: BoardSort) {
	return runs.filter((r) => matches(query, r)).sort(compareRuns(sort.id, sort.desc));
}

/** The runs ticked among those shown, or else all of them: the first few are charted. */
export function chartedRuns(shown: RunSummary[], isPicked: (id: string) => boolean) {
	const picked = shown.filter((r) => isPicked(r.id));
	return (picked.length ? picked : shown).slice(0, MAX_CHARTED);
}

/**
 * A color slot per run. Rows of a leaderboard move as values change, and a line shouldn't
 * change color with them: a run keeps its slot while it stays, a new one takes a free one.
 */
export function assignSlots(ids: string[], previous: ReadonlyMap<string, number>) {
	const slots = new Map<string, number>();
	for (const id of ids) {
		const slot = previous.get(id);
		if (slot !== undefined) slots.set(id, slot);
	}
	const taken = new Set(slots.values());
	let free = 0;
	for (const id of ids) {
		if (slots.has(id)) continue;
		while (taken.has(free)) free++;
		slots.set(id, free);
		taken.add(free);
	}
	return slots;
}

/** Summary keys marked important, among these runs. */
export function importantKeys(runs: RunSummary[], describe: Describe): string[] {
	return availableKeys(runs).summary.filter((key) => {
		const meta = describe(key);
		return meta.important && !meta.hidden;
	});
}

/** The leaderboard: its important metrics, and what it's sorted by if that's something else. */
export function boardColumns(runs: RunSummary[], describe: Describe, sortId: string): string[] {
	const important = importantKeys(runs, describe).map((key) => `summary.${key}`);
	const columns = important.length
		? ['state', ...important, 'tags', 'created']
		: defaultColumns(runs);
	if (sortId === 'name' || columns.includes(sortId)) return columns;
	return ['state', sortId, ...columns.filter((c) => c !== 'state')];
}

/** What a board can be sorted by. */
export function sortChoices(runs: RunSummary[], describe: Describe) {
	const summary = availableKeys(runs).summary.filter((key) => !describe(key).hidden);
	const ids = (keys: string[]) => keys.map((key) => `summary.${key}`);
	return {
		run: RUN_FIELDS,
		important: ids(summary.filter((key) => describe(key).important)),
		metrics: ids(summary.filter((key) => !describe(key).important)).slice(0, MAX_SORT_CHOICES)
	};
}

export function sortLabel(id: string, describe: Describe): string {
	if (id === 'name') return 'Name';
	const c = column(id);
	if (c.kind !== 'value') return c.label;
	return c.prefix === 'summary' ? describe(c.label).label || c.label : id;
}

/** The useful way round when choosing a sort: the best values first, the newest runs first. */
export function naturalSort(id: string, describe: Describe): BoardSort {
	if (id === 'name') return { id, desc: false };
	const c = column(id);
	const lowerIsBetter = c.prefix === 'summary' && describe(c.label).direction === 'min';
	return { id, desc: !lowerIsBetter };
}
