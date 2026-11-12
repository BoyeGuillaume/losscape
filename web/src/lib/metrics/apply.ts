// Runs as the UI shows them: summaries with renamed (and rescaled) metrics, computed
// metrics, and best values where a metric's summary is "best".

import type { RunSummary } from '#lib/api/types.js';
import { evaluate } from './formula.js';
import type { Registry } from './registry.js';
import { own } from '#lib/group.js';

type RunMetricsData = Pick<
	RunSummary,
	'summary' | 'metric_min' | 'metric_max' | 'renames' | 'rename_scales'
>;

/** Which value of a metric: what its settings say, or the last or best one regardless. */
export type ValueMode = 'default' | 'last' | 'best';

/** The summary of a run with its project's custom metrics applied. */
export function effectiveSummary(
	run: RunMetricsData,
	registry: Registry,
	mode: ValueMode = 'default'
): Record<string, unknown> {
	const extremes = (raw: string, direction: 'max' | 'min') =>
		own(direction === 'max' ? run.metric_max : run.metric_min, raw);
	const rawKeys = new Set([...Object.keys(run.summary), ...Object.keys(run.metric_min ?? {})]);
	const metrics = registry.forRun(rawKeys, run);

	const out: Record<string, unknown> = {};
	for (const [shown, { raw, scale }] of metrics.shown) {
		const last = own(run.summary, raw);
		if (last === undefined) continue;
		out[shown] = typeof last === 'number' ? last * scale : last;
	}
	for (const [shown, { raw, scale }] of metrics.shown) {
		const meta = registry.describe(shown);
		const wantsBest = mode === 'best' || (mode === 'default' && meta.summary === 'best');
		if (!wantsBest || !meta.direction) continue;
		// A negative scale turns the highest logged value into the lowest shown.
		const best = extremes(raw, (meta.direction === 'max') === scale >= 0 ? 'max' : 'min');
		if (typeof best === 'number') out[shown] = best * scale;
	}
	for (const name of registry.computedNames) {
		const r = metrics.resolve(name);
		if (r?.kind !== 'formula') continue;
		const value = evaluate(
			r.formula,
			(ref) => {
				const source = r.refs.get(ref)!;
				const v = own(run.summary, source.raw);
				return typeof v === 'number' ? v * source.scale : null;
			},
			(ref, fn) => {
				// Mean and median need every point, which charts load and tables don't.
				if (fn === 'mean' || fn === 'median') return null;
				const { raw, scale } = r.refs.get(ref)!;
				const v = extremes(raw, (fn === 'highest') === scale >= 0 ? 'max' : 'min');
				return typeof v === 'number' ? v * scale : null;
			}
		);
		if (value !== null) out[name] = value;
	}
	return out;
}

type Applied = WeakMap<RunSummary, { registry: Registry; run: RunSummary }>;
const applied: Record<ValueMode, Applied> = {
	default: new WeakMap(),
	last: new WeakMap(),
	best: new WeakMap()
};

/** `run` with its effective summary (cached per run, registry and mode). */
export function withMetrics<T extends RunSummary>(
	run: T,
	registry: Registry,
	mode: ValueMode = 'default'
): T {
	const hit = applied[mode].get(run);
	if (hit?.registry === registry) return hit.run as T;
	const out = { ...run, summary: effectiveSummary(run, registry, mode) };
	applied[mode].set(run, { registry, run: out });
	return out;
}
