// A project's board (its Overview tab): which runs it shows, and how it charts them. The
// query, sort and limit are saved with the project, and applied here.

import type { RunSummary } from '#lib/api/types.js';
import { flatRun, matches, type Node } from '#lib/filter.js';
import { formatCompact, timeAgo } from '#lib/format.js';
import type { ValueMode } from '#lib/metrics/apply.js';
import { isModelMetric, type Meta } from '#lib/metrics/registry.js';
import { formatUnit } from '#lib/units.js';
import { availableKeys, column, compareRuns, defaultColumns, sortValue } from './columns.js';

export type BoardSort = { id: string; desc: boolean };
export type Board = {
	/** The version this is, or is based on: saved all at once, so it's checked. */
	version?: number;
	query: string;
	sort: BoardSort;
	limit: number;
	/** The run others are measured against. */
	baseline: string | null;
	/** The config key metrics are plotted against. */
	explore: string | null;
	/** The leaderboard's columns, if chosen by hand. */
	columns: string[] | null;
};

export const DEFAULT_BOARD: Board = {
	query: '',
	sort: { id: 'created', desc: true },
	limit: 10,
	baseline: null,
	explore: null,
	columns: null
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

/** The runs ticked among those shown, or else all of them, with the baseline if any. */
export function chartedRuns(
	shown: RunSummary[],
	isPicked: (id: string) => boolean,
	baselineId: string | null = null
) {
	const picked = shown.filter((r) => isPicked(r.id));
	if (!picked.length) return shown.slice(0, MAX_CHARTED);
	const baseline = shown.filter((r) => r.id === baselineId && !isPicked(r.id));
	return [...baseline, ...picked].slice(0, MAX_CHARTED);
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

const OPTIONAL_COLUMNS = new Set(['state', 'created', 'runtime', 'steps', 'tags']);

/** Every run reads the same here, so the column says nothing. */
function isUniform(runs: RunSummary[], id: string): boolean {
	const text = (run: RunSummary) =>
		id === 'created' ? timeAgo(run.created_at_ms) : String(sortValue(run, id) ?? '');
	return runs.length > 1 && runs.every((run) => text(run) === text(runs[0]));
}

/**
 * The leaderboard: its important metrics, what tells the runs apart, and what it's sorted
 * by if that's something else. Run fields that read the same on every row are left out.
 */
export function boardColumns(runs: RunSummary[], describe: Describe, sortId: string): string[] {
	const important = importantKeys(runs, describe).map((key) => `summary.${key}`);
	const config = differingConfig(runs).map((key) => `config.${key}`);
	const metrics = important.length ? ['state', ...important] : defaultColumns(runs);
	const columns = [
		...metrics.filter((c) => c !== 'tags' && c !== 'created'),
		...config,
		'tags',
		'created'
	];
	const useful = columns.filter(
		(c) => c === sortId || !OPTIONAL_COLUMNS.has(c) || !isUniform(runs, c)
	);
	if (sortId === 'name' || useful.includes(sortId)) return useful;
	return [sortId, ...useful];
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

// ---- Config that tells these runs apart ----

const SEED = /(^|[._/-])seed$/i;
const MAX_CATEGORIES = 12;
const SCAN = 2_000;

type ConfigStat = { key: string; values: unknown[]; distinct: number };

/** Config keys with more than one value here. All different text says nothing (paths, ids). */
function differing(runs: RunSummary[]): ConfigStat[] {
	const sample = runs.slice(0, SCAN);
	const byKey = new Map<string, unknown[]>();
	for (const run of sample) {
		for (const [key, value] of flatRun(run).config) {
			if (value === null || typeof value === 'object') continue;
			let values = byKey.get(key);
			if (!values) byKey.set(key, (values = []));
			values.push(value);
		}
	}
	const stats: ConfigStat[] = [];
	for (const [key, values] of byKey) {
		const distinct = new Set(values.map(String)).size;
		if (distinct < 2) continue;
		const allDifferentText =
			values.length > 2 && distinct === values.length && values.every((v) => typeof v === 'string');
		if (!allDifferentText) stats.push({ key, values, distinct });
	}
	// Seeds last: they differ in most sweeps and rarely explain anything.
	return stats.sort(
		(a, b) =>
			Number(SEED.test(a.key)) - Number(SEED.test(b.key)) ||
			b.values.length - a.values.length ||
			a.key.localeCompare(b.key)
	);
}

/** Config keys worth a column: what makes these runs different. */
export function differingConfig(runs: RunSummary[], max = 4): string[] {
	return differing(runs)
		.slice(0, max)
		.map((s) => s.key);
}

export type ExploreKey = { key: string; kind: 'number' | 'category' | 'metric' };

/** Explore's choice of a metric rather than a config value. */
export const metricChoice = (key: string): ExploreKey => ({ key: `metric:${key}`, kind: 'metric' });
export const choiceMetric = (choice: ExploreKey) => choice.key.slice('metric:'.length);

/** Config keys to plot a metric against: numbers, or text with few values. */
export function exploreKeys(runs: RunSummary[]): ExploreKey[] {
	return differing(runs).flatMap((s): ExploreKey[] => {
		if (s.values.every((v) => typeof v === 'number')) return [{ key: s.key, kind: 'number' }];
		return s.distinct <= MAX_CATEGORIES ? [{ key: s.key, kind: 'category' }] : [];
	});
}

// ---- Change between two values ----

export type Change = { text: string; better: boolean | null };

function amount(n: number): string {
	if (n >= 10) return n.toFixed(0);
	return n >= 1 ? n.toFixed(1) : String(Number(n.toPrecision(2)));
}

/** `value` against `base`: points for percentages, otherwise relative. */
export function changeFrom(
	value: number,
	base: number,
	meta: Pick<Meta, 'unit' | 'direction'>
): Change {
	const diff = value - base;
	if (diff === 0) return { text: 'no change', better: null };
	const size = Math.abs(diff);
	const unit = meta.unit;
	const text =
		unit?.steps[0]?.suffix === '%'
			? `${amount(size * (unit.scale ?? 1))} pts`
			: base !== 0
				? `${amount((size / Math.abs(base)) * 100)}%`
				: unit
					? formatUnit(size, unit)
					: formatCompact(size);
	return {
		text: `${diff > 0 ? '↑ +' : '↓ −'}${text}`,
		better: meta.direction ? (meta.direction === 'max') === diff > 0 : null
	};
}

export const metricValue = (run: RunSummary, key: string): number | null => {
	const value = flatRun(run).summary.get(key);
	return typeof value === 'number' && Number.isFinite(value) ? value : null;
};

// ---- Figures: the best run of each important metric ----

export type Headline = {
	key: string;
	run: RunSummary;
	value: number;
	/** How far ahead of (or behind) the baseline or, without one, the runner-up. */
	change: Change | null;
	against: 'baseline' | 'runner-up' | null;
	isBaseline: boolean;
};

/** Without a direction there's no best: the first run, in the board's order. */
export function headlineFor(
	runs: RunSummary[],
	key: string,
	meta: Pick<Meta, 'unit' | 'direction'>,
	baseline: RunSummary | null
): Headline | null {
	const rows = runs.flatMap((run) => {
		const value = metricValue(run, key);
		return value === null ? [] : [{ run, value }];
	});
	if (!rows.length) return null;
	const { direction } = meta;
	if (direction) {
		rows.sort((a, b) => (direction === 'max' ? b.value - a.value : a.value - b.value));
	}
	const [top, second] = rows;
	const base = baseline ? metricValue(baseline, key) : null;
	const isBaseline = baseline?.id === top.run.id;
	const head = { key, run: top.run, value: top.value, isBaseline };
	if (base !== null && !isBaseline) {
		return { ...head, change: changeFrom(top.value, base, meta), against: 'baseline' };
	}
	if (second && direction && !isBaseline) {
		return { ...head, change: changeFrom(top.value, second.value, meta), against: 'runner-up' };
	}
	return { ...head, change: null, against: null };
}

// ---- Dots: one per run, over time or against a config value ----

export type Dot = {
	id: string;
	name: string;
	x: number;
	y: number;
	running: boolean;
	/** On a trajectory: the step it's at. */
	step?: number;
};

/** What a chart of this metric shows, in a few words. */
export function valueNote(
	mode: ValueMode,
	meta: Pick<Meta, 'summary' | 'direction'>,
	computed: boolean
): string {
	if (mode === 'last' || (mode === 'default' && meta.summary !== 'best')) return 'last value';
	if (computed) return 'last value (computed)';
	return meta.direction ? 'best value' : 'last value (no direction)';
}

const dot = (run: RunSummary, x: number, y: number): Dot => ({
	id: run.id,
	name: run.name,
	x,
	y,
	running: run.state === 'running'
});

/** Runs with this metric, by when they started. */
export function progressDots(runs: RunSummary[], key: string): Dot[] {
	return runs
		.flatMap((run) => {
			const y = metricValue(run, key);
			return y === null ? [] : [dot(run, run.start_ms, y)];
		})
		.sort((a, b) => a.x - b.x);
}

/** The line of the best value so far, in steps, up to the last run. Ids of the record runs. */
export function bestSoFar(dots: Dot[], direction: 'max' | 'min' | null) {
	const line: { x: number; y: number }[] = [];
	const records = new Set<string>();
	if (!direction || !dots.length) return { line, records };
	let best = dots[0].y;
	line.push({ x: dots[0].x, y: best });
	records.add(dots[0].id);
	for (const d of dots.slice(1)) {
		if (direction === 'max' ? d.y > best : d.y < best) {
			line.push({ x: d.x, y: best }, { x: d.x, y: d.y });
			best = d.y;
			records.add(d.id);
		}
	}
	line.push({ x: dots[dots.length - 1].x, y: best });
	return { line, records };
}

/** At most `max` dots, evenly spread, always with those in `keep`. */
export function thinDots(dots: Dot[], keep: ReadonlySet<string>, max = 2_000) {
	if (dots.length <= max) return dots;
	const every = dots.length / max;
	return dots.filter(
		(d, i) => keep.has(d.id) || Math.floor(i / every) !== Math.floor((i - 1) / every)
	);
}

/** A stable offset in [-0.25, 0.25], so runs of one category don't all stack up. */
function jitter(id: string): number {
	let h = 0;
	for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) | 0;
	return ((Math.abs(h) % 1000) / 1000 - 0.5) / 2;
}

export type ExplorePlot = {
	dots: Dot[];
	/** Category names, the x of a dot being around the index of its category. */
	categories: string[] | null;
	log: boolean;
};

/** All positive and over two orders of magnitude: a log scale tells them apart. */
export function spansOrders(values: number[]): boolean {
	if (!values.length || values.some((v) => v <= 0)) return false;
	return Math.max(...values) / Math.min(...values) >= 100;
}

/** A metric against a config value or another metric. Numbers that span orders of magnitude
 * get a log scale; among metrics only parameter counts and such, whose size is the point. */
export function explorePlot(runs: RunSummary[], metric: string, explore: ExploreKey): ExplorePlot {
	if (explore.kind === 'metric') {
		const other = choiceMetric(explore);
		const dots = runs.flatMap((run) => {
			const [x, y] = [metricValue(run, other), metricValue(run, metric)];
			return x === null || y === null ? [] : [dot(run, x, y)];
		});
		return {
			categories: null,
			log: isModelMetric(other) && spansOrders(dots.map((d) => d.x)),
			dots
		};
	}
	const rows = runs.flatMap((run) => {
		const value = flatRun(run).config.get(explore.key);
		const y = metricValue(run, metric);
		return y === null || value === undefined || value === null ? [] : [{ run, value, y }];
	});
	if (explore.kind === 'category') {
		const categories = [...new Set(rows.map((r) => String(r.value)))].sort((a, b) =>
			a.localeCompare(b, undefined, { numeric: true })
		);
		return {
			categories,
			log: false,
			dots: rows.map((r) => dot(r.run, categories.indexOf(String(r.value)) + jitter(r.run.id), r.y))
		};
	}
	return {
		categories: null,
		log: spansOrders(rows.map((r) => r.value as number)),
		dots: rows.map((r) => dot(r.run, r.value as number, r.y))
	};
}
