// Runs table columns: run fields, plus any config or summary value (`config.<key>`,
// `summary.<key>`, nested keys joined with dots).

import type { RunSummary } from '#lib/api/types.js';
import { fieldValue, flatRun } from '#lib/filter.js';
import { formatValue } from '#lib/format.js';
import { currentRegistry } from '#lib/metrics/store.svelte.js';

export type ColumnKind = 'state' | 'created' | 'runtime' | 'steps' | 'group' | 'tags' | 'value';
export type Column = {
	id: string;
	/** Key shown in the header; `prefix` above it (config, summary). */
	label: string;
	prefix?: string;
	width: number;
	kind: ColumnKind;
	numeric: boolean;
};

const BASE: Record<string, Column> = {
	state: { id: 'state', label: 'State', width: 104, kind: 'state', numeric: false },
	created: { id: 'created', label: 'Created', width: 112, kind: 'created', numeric: true },
	runtime: { id: 'runtime', label: 'Runtime', width: 92, kind: 'runtime', numeric: true },
	steps: { id: 'steps', label: 'Steps', width: 88, kind: 'steps', numeric: true },
	group: { id: 'group', label: 'Group', width: 128, kind: 'group', numeric: false },
	tags: { id: 'tags', label: 'Tags', width: 168, kind: 'tags', numeric: false }
};
export const BASE_COLUMNS = Object.keys(BASE);

export function column(id: string): Column {
	if (BASE[id]) return BASE[id];
	const dot = id.indexOf('.');
	return {
		id,
		label: id.slice(dot + 1),
		prefix: id.slice(0, dot),
		width: 120,
		kind: 'value',
		numeric: true
	};
}

/** Steps logged: the last step + 1 (undefined before the first point). */
export const stepCount = (r: RunSummary) => (r.last_step === null ? undefined : r.last_step + 1);

const endOf = (r: RunSummary, now: number) =>
	r.state === 'running' ? now : (r.finished_at_ms ?? r.heartbeat_at_ms);

/** The value a column sorts by. */
export function sortValue(run: RunSummary, id: string, now = Date.now()): unknown {
	switch (id) {
		case 'name':
			return run.name;
		case 'state':
			return run.state;
		case 'created':
			return run.created_at_ms;
		case 'runtime':
			return endOf(run, now) - run.created_at_ms;
		case 'steps':
			return stepCount(run);
		case 'group':
			return run.group_name ?? undefined;
		case 'tags':
			return run.tags.length ? run.tags.join(', ') : undefined;
		default:
			return fieldValue(run, id);
	}
}

/** Numbers before text, missing values last in both directions. */
export function compareRuns(id: string, desc: boolean) {
	const now = Date.now();
	return (a: RunSummary, b: RunSummary) => {
		const [x, y] = [sortValue(a, id, now), sortValue(b, id, now)];
		const missing = (v: unknown) => v === undefined || v === null;
		if (missing(x) || missing(y)) return missing(x) === missing(y) ? 0 : missing(x) ? 1 : -1;
		let order: number;
		if (typeof x === 'number' && typeof y === 'number') order = x - y;
		else if (typeof x === 'number') order = -1;
		else if (typeof y === 'number') order = 1;
		else order = String(x).localeCompare(String(y), undefined, { numeric: true });
		return desc ? -order : order;
	};
}

/** Whether a value column holds numbers (in most of the first runs that have it). */
export function isNumeric(runs: RunSummary[], id: string): boolean {
	let numbers = 0;
	let others = 0;
	for (const run of runs.slice(0, 500)) {
		const v = fieldValue(run, id);
		if (typeof v === 'number') numbers++;
		else if (v !== undefined && v !== null) others++;
	}
	return numbers >= others;
}

/** Runs scanned to discover config and summary keys. */
const SCAN = 2_000;
const isScalar = (v: unknown) => v === null || ['number', 'string', 'boolean'].includes(typeof v);

/** Config and summary keys present in the runs, most common first. */
export function availableKeys(runs: RunSummary[]) {
	const config = new Map<string, number>();
	const summary = new Map<string, number>();
	for (const run of runs.slice(0, SCAN)) {
		const flat = flatRun(run);
		for (const [k, v] of flat.config) if (isScalar(v)) config.set(k, (config.get(k) ?? 0) + 1);
		for (const [k, v] of flat.summary) if (isScalar(v)) summary.set(k, (summary.get(k) ?? 0) + 1);
	}
	const ranked = (m: Map<string, number>) =>
		[...m].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([k]) => k);
	return { config: ranked(config), summary: ranked(summary) };
}

/**
 * Default columns: run fields, then summary values: the important metrics, then the most
 * common numeric ones (hidden metrics left out). Config keys (there can be many) are added
 * by hand, from the column picker.
 */
export function defaultColumns(runs: RunSummary[]): string[] {
	const sample = runs.slice(0, SCAN);
	const { summary } = availableKeys(sample);
	const registry = currentRegistry();
	const numericSummary = summary.filter(
		(key) =>
			!registry.describe(key).hidden &&
			sample.some((run) => typeof flatRun(run).summary.get(key) === 'number')
	);
	const important = numericSummary.filter((key) => registry.describe(key).important);
	const others = numericSummary.filter((key) => !registry.describe(key).important);
	return [
		'state',
		'created',
		'runtime',
		'steps',
		'tags',
		...[...important, ...others.slice(0, Math.max(0, 6 - important.length))].map(
			(k) => `summary.${k}`
		)
	];
}

/** Runs sharing one value of the group-by column. */
export type RunGroup = { key: string; label: string; runs: RunSummary[] };

const MISSING = '\u0000none';

/** Groups `runs` by the value of column `by`, in order of first appearance. */
export function groupRuns(runs: RunSummary[], by: string): RunGroup[] {
	const groups = new Map<string, RunGroup>();
	const name = column(by).prefix ? column(by).label : column(by).label.toLowerCase();
	for (const run of runs) {
		const value = by === 'name' ? run.name : sortValue(run, by);
		const key =
			value === undefined || value === null || value === ''
				? MISSING
				: typeof value === 'string'
					? value
					: JSON.stringify(value);
		let group = groups.get(key);
		if (!group) {
			const label =
				key === MISSING
					? `no ${name}`
					: `${name} = ${typeof value === 'string' ? value : formatValue(value)}`;
			groups.set(key, (group = { key, label, runs: [] }));
		}
		group.runs.push(run);
	}
	return [...groups.values()];
}
