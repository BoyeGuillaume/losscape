// Reading metrics as shown (renamed, merged, computed by a formula) from the logged ones:
// charts ask a MetricSource for a run's metric and get downsampled points back.

import { aggregated, evaluate, references, type Aggregate } from '#lib/metrics/formula.js';
import type { Registry, Resolution, RunMetrics } from '#lib/metrics/registry.js';
import {
	levelFor,
	seriesInRange,
	tileSpan,
	tileStore,
	type MetricStats,
	type SeriesPoints
} from './tiles.svelte.js';

type Refs = Map<string, { raw: string; scale: number }>;

/** An aggregate of a metric shown with a factor (a negative one swaps lowest and highest). */
function scaledAggregate(stats: MetricStats, fn: Aggregate, scale: number): number | null {
	const pick =
		fn === 'lowest'
			? scale >= 0
				? stats.min
				: stats.max
			: fn === 'highest'
				? scale >= 0
					? stats.max
					: stats.min
				: stats[fn];
	return pick === null ? null : pick * scale;
}

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

/**
 * `s` with the nearest point on each side beyond the tiles it was read from, where it has
 * none beyond the view and the run logged some: zoomed in between two points, the line
 * between them still crosses the view.
 */
function withNeighbors(
	run: string,
	raw: string,
	scale: number,
	from: number,
	to: number,
	width: number,
	extent: Extent | undefined,
	s: SeriesPoints
): SeriesPoints {
	if (!s.complete || !extent) return s;
	const span = tileSpan(levelFor(to - from + 1, width));
	const before =
		extent.min < from && !(s.x.length && s.x[0] < from)
			? tileStore.neighbor(run, raw, Math.floor(from / span) * span, 'before')
			: null;
	const after =
		extent.max > to && !(s.x.length && s.x[s.x.length - 1] > to)
			? tileStore.neighbor(run, raw, (Math.floor(to / span) + 1) * span - 1, 'after')
			: null;
	if (!before && !after) {
		return before === undefined || after === undefined ? { ...s, complete: false } : s;
	}
	const out: SeriesPoints = {
		x: [...s.x],
		n: [...s.n],
		time: [...s.time],
		mean: [...s.mean],
		min: [...s.min],
		max: [...s.max],
		complete: before !== undefined && after !== undefined
	};
	const add = (p: { step: number; value: number; time: number }, at: 'start' | 'end') => {
		const v = p.value * scale;
		const put = at === 'start' ? 'unshift' : 'push';
		out.x[put](p.step);
		out.n[put](1);
		out.time[put](p.time);
		out.mean[put](v);
		out.min[put](v);
		out.max[put](v);
	};
	if (before) add(before, 'start');
	if (after) add(after, 'end');
	return out;
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

	/** Every metric shown for these runs (and the hidden ones, if asked). */
	keys(withHidden = false): string[] {
		const all = new Set<string>();
		for (const metrics of this.runs.values()) metrics.keys(withHidden).forEach((k) => all.add(k));
		return [...all].sort();
	}

	/**
	 * A formula's inputs logged at a single step (a model's size): constants, which hold at every
	 * step. Unless all of them are: then the formula is itself a single point.
	 */
	private constants(run: string, refs: Map<string, { raw: string }>): Set<string> {
		const single = new Set<string>();
		for (const [ref, { raw }] of refs) {
			const e = this.extent(run, raw);
			if (e && e.min === e.max) single.add(ref);
		}
		return single.size < refs.size ? single : new Set();
	}

	/**
	 * The inputs a formula is drawn along: those it reads step by step. With only aggregates
	 * (`mean([loss])`), the metrics it aggregates, so that it spans their steps.
	 */
	private stepping(r: Extract<Resolution, { kind: 'formula' }>): Refs {
		const stepped = references(r.formula);
		const along = new Map([...r.refs].filter(([ref]) => stepped.has(ref)));
		return along.size ? along : r.refs;
	}

	/** Steps a run has the metric for (a formula: where all its inputs exist). */
	extentOf(run: string, key: string): Extent | undefined {
		const r = this.resolve(run, key);
		if (!r) return undefined;
		if (r.kind === 'key') return this.extent(run, r.raw);
		for (const { raw } of r.refs.values()) if (!this.extent(run, raw)) return undefined;
		const along = this.stepping(r);
		const constants = this.constants(run, along);
		let out: Extent | undefined;
		for (const [ref, { raw }] of along) {
			if (constants.has(ref)) continue;
			const e = this.extent(run, raw)!;
			out = out ? { min: Math.max(out.min, e.min), max: Math.min(out.max, e.max) } : e;
		}
		return out && out.max >= out.min ? out : undefined;
	}

	/** Downsampled points of a run's metric over steps `from..=to`, or null if it has none. */
	points(run: string, key: string, from: number, to: number, width: number): SeriesPoints | null {
		const r = this.resolve(run, key);
		if (!r) return null;
		if (r.kind === 'key') {
			const s = scaledSeries(run, r.raw, r.scale, from, to, width);
			return withNeighbors(run, r.raw, r.scale, from, to, width, this.extent(run, r.raw), s);
		}
		// A formula, bucket by bucket: the same steps and width give the same buckets.
		const along = this.stepping(r);
		const constants = this.constants(run, along);
		const fixed = new Map<string, number | null>();
		let complete = true;
		for (const ref of constants) {
			const { raw, scale } = r.refs.get(ref)!;
			const at = this.extent(run, raw)!.min;
			const s = scaledSeries(run, raw, scale, at, at, width);
			complete &&= s.complete;
			fixed.set(ref, s.mean[0] ?? null);
		}
		// Over the whole run, from the server: every point, not the ones loaded.
		const totals = new Map<string, MetricStats | null | undefined>();
		for (const ref of aggregated(r.formula)) {
			const stats = tileStore.stats(run, r.refs.get(ref)!.raw);
			complete &&= stats !== undefined;
			totals.set(ref, stats);
		}
		const inputs = [...along]
			.filter(([ref]) => !constants.has(ref))
			.map(([ref, { raw, scale }]) => {
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
			complete: complete && inputs.every((i) => i.s.complete)
		};
		// A constant or an aggregate still loading: nothing to show yet.
		if ([...fixed.values()].includes(null) || [...totals.values()].includes(undefined)) return out;
		first.s.x.forEach((x, i) => {
			const idx = inputs.map((input) => input.at.get(x));
			if (idx.some((j) => j === undefined)) return;
			const value = evaluate(
				r.formula,
				(ref) => {
					if (fixed.has(ref)) return fixed.get(ref)!;
					const k = inputs.findIndex((input) => input.ref === ref);
					return inputs[k].s.mean[idx[k]!];
				},
				(ref, fn) => {
					const stats = totals.get(ref);
					return stats ? scaledAggregate(stats, fn, r.refs.get(ref)!.scale) : null;
				}
			);
			out.x.push(x);
			out.n.push(Math.min(...inputs.map((input, k) => input.s.n[idx[k]!])));
			out.time.push(first.s.time[i]);
			out.mean.push(value);
			out.min.push(value);
			out.max.push(value);
		});
		return out;
	}

	/** The logged metric a shown one reads, and by what factor; null for a computed one. */
	logged(run: string, key: string): { raw: string; scale: number } | null {
		const r = this.resolve(run, key);
		return r?.kind === 'key' ? { raw: r.raw, scale: r.scale } : null;
	}

	/** The logged metric a shown one reads, when it isn't itself (for hints). */
	sourceOf(run: string, key: string): string | null {
		return this.runs.get(run)?.source(key) ?? null;
	}
}
