// What a set of runs contains, for the config explorer and filter suggestions: every config
// key with how many runs set it and its values by count, plus tags, aliases, groups.

import type { RunSummary } from '#lib/api/types.js';
import { flatRun } from '#lib/filter.js';

export type ValueCount = { value: unknown; count: number };
export type KeyStat = {
	key: string;
	/** Runs that set the key. */
	runs: number;
	/** Distinct values, most common first (at most `MAX_VALUES`). */
	values: ValueCount[];
	distinct: number;
};

export type Catalog = {
	config: KeyStat[];
	summaryKeys: string[];
	states: ValueCount[];
	tags: ValueCount[];
	aliases: ValueCount[];
	groups: ValueCount[];
};

/** Values kept per key; enough to pick from, bounded for keys like seeds. */
const MAX_VALUES = 200;

/** Counts values; numbers equal to each other count once (1e-3 and 0.001). */
class Counter {
	private counts = new Map<string, ValueCount>();
	add(value: unknown) {
		const id = typeof value === 'string' ? `s${value}` : JSON.stringify(value);
		const entry = this.counts.get(id);
		if (entry) entry.count++;
		else this.counts.set(id, { value, count: 1 });
	}
	get distinct() {
		return this.counts.size;
	}
	/** Most common first; ties: numbers ascending, then text. */
	top(limit = MAX_VALUES): ValueCount[] {
		return [...this.counts.values()]
			.sort((a, b) => {
				if (b.count !== a.count) return b.count - a.count;
				if (typeof a.value === 'number' && typeof b.value === 'number') return a.value - b.value;
				return String(a.value).localeCompare(String(b.value));
			})
			.slice(0, limit);
	}
}

export function buildCatalog(runs: RunSummary[]): Catalog {
	const config = new Map<string, { runs: number; values: Counter }>();
	const summary = new Set<string>();
	const [states, tags, aliases, groups] = [
		new Counter(),
		new Counter(),
		new Counter(),
		new Counter()
	];
	for (const run of runs) {
		const flat = flatRun(run);
		for (const [key, value] of flat.config) {
			let stat = config.get(key);
			if (!stat) config.set(key, (stat = { runs: 0, values: new Counter() }));
			stat.runs++;
			stat.values.add(value);
		}
		for (const key of flat.summary.keys()) summary.add(key);
		states.add(run.state);
		run.tags.forEach((t) => tags.add(t));
		run.aliases.forEach((a) => aliases.add(a));
		if (run.group_name) groups.add(run.group_name);
	}
	return {
		config: [...config]
			.map(([key, s]) => ({
				key,
				runs: s.runs,
				values: s.values.top(),
				distinct: s.values.distinct
			}))
			.sort((a, b) => a.key.localeCompare(b.key)),
		summaryKeys: [...summary].sort(),
		states: states.top(),
		tags: tags.top(),
		aliases: aliases.top(),
		groups: groups.top()
	};
}

/** How a value reads in the explorer and suggestions: numbers and text as they are. */
export function valueLabel(value: unknown): string {
	if (typeof value === 'string') return value;
	return JSON.stringify(value);
}
