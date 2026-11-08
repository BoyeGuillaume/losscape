// Comparing runs: their configs key by key, and which metrics to put side by side.

import type { RunSummary } from '#lib/api/types.js';
import { flatten } from '#lib/format.js';
import type { Registry } from '#lib/metrics/registry.js';

export type DiffRow = {
	/** Dotted config key (`optimizer.lr`). */
	key: string;
	/** One per run; undefined when the run doesn't set it. */
	values: unknown[];
	/** Not the same in every run. */
	differs: boolean;
	/** Per run: differs from the reference run. */
	changed: boolean[];
};

const same = (a: unknown, b: unknown) =>
	a === b || (a !== undefined && b !== undefined && JSON.stringify(a) === JSON.stringify(b));

/** Every config key of these runs (nested keys dotted), compared to the run at `ref`. */
export function configDiff(configs: Record<string, unknown>[], ref: number): DiffRow[] {
	const flat = configs.map((c) => new Map(flatten(c)));
	const keys = [...new Set(flat.flatMap((f) => [...f.keys()]))].sort();
	return keys.map((key) => {
		const values = flat.map((f) => f.get(key));
		const changed = values.map((v, i) => i !== ref && !same(v, values[ref]));
		return { key, values, differs: values.some((v) => !same(v, values[0])), changed };
	});
}

/**
 * Metrics to compare: the important ones these runs have; without any, the numeric ones
 * most of them have (at most `max`). Hidden metrics are left out.
 */
export function compareMetrics(runs: RunSummary[], registry: Registry, max = 6): string[] {
	const counts = new Map<string, number>();
	for (const run of runs) {
		for (const [key, value] of Object.entries(run.summary)) {
			if (typeof value === 'number') counts.set(key, (counts.get(key) ?? 0) + 1);
		}
	}
	const keys = [...counts.keys()].filter((k) => !registry.describe(k).hidden);
	const important = keys.filter((k) => registry.describe(k).important).sort();
	if (important.length) return important;
	return keys.sort((a, b) => counts.get(b)! - counts.get(a)! || a.localeCompare(b)).slice(0, max);
}
