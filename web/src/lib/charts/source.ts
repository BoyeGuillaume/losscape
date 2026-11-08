// Reading metrics as shown (renamed, merged, computed by a formula) from the logged ones:
// charts ask a MetricSource for a run's metric and get downsampled points back.

import { evaluate } from '#lib/metrics/formula.js';
import type { Registry, Resolution, RunMetrics } from '#lib/metrics/registry.js';
import { seriesInRange, type SeriesPoints } from './tiles.svelte.js';

export type Extent = { min: number; max: number };

function scaledSeries(
	run: string,
	raw: string,
	scale: number,
	from: number,
	to: number,
	width: number
): SeriesPoints {
	const s = seriesInRange(run, raw, from, to, width);
	if (scale === 1) return s;
	const times = (v: number | null) => (v === null ? null : v * scale);
	const [min, max] = scale < 0 ? [s.max, s.min] : [s.min, s.max];
	return { ...s, mean: s.mean.map(times), min: min.map(times), max: max.map(times) };
}

export class MetricSource {
	constructor(
		readonly registry: Registry,
		private runs: Map<string, RunMetrics>,
		/** Step range of a logged metric of a run. */
		private extent: (run: string, raw: string) => Extent | undefined
	) {}

	private resolve(run: string, key: string): Resolution | null {
		return this.runs.get(run)?.resolve(key) ?? null;
	}

	/** Every metric shown for these runs (and the hidden sources of merges and formulas). */
	keys(withHidden = false): string[] {
		const all = new Set<string>();
		for (const metrics of this.runs.values()) metrics.keys(withHidden).forEach((k) => all.add(k));
		return [...all].sort();
	}

	/** Steps a run has the metric for (a formula: where all its inputs exist). */
	extentOf(run: string, key: string): Extent | undefined {
		const r = this.resolve(run, key);
		if (!r) return undefined;
		if (r.kind === 'key') return this.extent(run, r.raw);
		let out: Extent | undefined;
		for (const { raw } of r.refs.values()) {
			const e = this.extent(run, raw);
			if (!e) return undefined;
			out = out ? { min: Math.max(out.min, e.min), max: Math.min(out.max, e.max) } : e;
		}
		return out && out.max >= out.min ? out : undefined;
	}

	/** Downsampled points of a run's metric over steps `from..=to`, or null if it has none. */
	points(run: string, key: string, from: number, to: number, width: number): SeriesPoints | null {
		const r = this.resolve(run, key);
		if (!r) return null;
		if (r.kind === 'key') return scaledSeries(run, r.raw, r.scale, from, to, width);
		// A formula, bucket by bucket: the same steps and width give the same buckets.
		const inputs = [...r.refs].map(([ref, { raw, scale }]) => {
			const s = scaledSeries(run, raw, scale, from, to, width);
			return { ref, s, at: new Map(s.x.map((x, i) => [x, i])) };
		});
		const [first] = inputs;
		const out: SeriesPoints = {
			x: [],
			n: [],
			time: [],
			mean: [],
			min: [],
			max: [],
			complete: inputs.every((i) => i.s.complete)
		};
		first.s.x.forEach((x, i) => {
			const idx = inputs.map((input) => input.at.get(x));
			if (idx.some((j) => j === undefined)) return;
			const value = evaluate(r.formula, (ref) => {
				const k = inputs.findIndex((input) => input.ref === ref);
				return inputs[k].s.mean[idx[k]!];
			});
			out.x.push(x);
			out.n.push(Math.min(...inputs.map((input, k) => input.s.n[idx[k]!])));
			out.time.push(first.s.time[i]);
			out.mean.push(value);
			out.min.push(value);
			out.max.push(value);
		});
		return out;
	}

	/** The logged metric a shown one reads, when it isn't itself (for hints). */
	sourceOf(run: string, key: string): string | null {
		return this.runs.get(run)?.source(key) ?? null;
	}
}
