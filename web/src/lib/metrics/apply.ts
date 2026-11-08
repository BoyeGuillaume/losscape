// Runs as the UI shows them: summaries with renamed metrics, virtual metrics, and best
// values where a metric's summary is "best".

import type { RunSummary } from '#lib/api/types.js';
import { evaluate } from './formula.js';
import type { Registry } from './registry.js';

type RunMetricsData = Pick<RunSummary, 'summary' | 'metric_min' | 'metric_max' | 'renames'>;

/** The summary of a run with its project's custom metrics applied. */
export function effectiveSummary(run: RunMetricsData, registry: Registry): Record<string, unknown> {
	const renames = run.renames ?? {};
	const extremes = (raw: string, direction: 'max' | 'min') =>
		(direction === 'max' ? run.metric_max : run.metric_min)?.[raw];
	const rawKeys = new Set([...Object.keys(run.summary), ...Object.keys(run.metric_min ?? {})]);
	const metrics = registry.forRun(rawKeys, renames);

	const out: Record<string, unknown> = {};
	for (const [raw, value] of Object.entries(run.summary)) out[renames[raw] ?? raw] = value;
	for (const [shown, raw] of metrics.shown) {
		const meta = registry.describe(shown);
		const best =
			meta.summary === 'best' && meta.direction ? extremes(raw, meta.direction) : undefined;
		if (typeof best === 'number') out[shown] = best;
	}
	for (const name of registry.virtualNames) {
		const r = metrics.resolve(name);
		if (!r) continue;
		if (r.kind === 'key') {
			const meta = registry.describe(name);
			// The best of the source (a negative factor turns its best into the worst).
			const which =
				meta.direction && r.scale < 0 ? (meta.direction === 'max' ? 'min' : 'max') : meta.direction;
			const base =
				meta.summary === 'best' && which
					? (extremes(r.raw, which) ?? run.summary[r.raw])
					: run.summary[r.raw];
			if (typeof base === 'number') out[name] = base * r.scale;
		} else {
			const value = evaluate(r.formula, (ref) => {
				const source = r.refs.get(ref)!;
				const v = run.summary[source.raw];
				return typeof v === 'number' ? v * source.scale : null;
			});
			if (value !== null) out[name] = value;
		}
	}
	return out;
}

const applied = new WeakMap<RunSummary, { registry: Registry; run: RunSummary }>();

/** `run` with its effective summary (cached per run and registry). */
export function withMetrics<T extends RunSummary>(run: T, registry: Registry): T {
	const hit = applied.get(run);
	if (hit?.registry === registry) return hit.run as T;
	const out = { ...run, summary: effectiveSummary(run, registry) };
	applied.set(run, { registry, run: out });
	return out;
}
