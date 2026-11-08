// Chart settings shared by every chart of a page, and the x-axis transforms.

import { preciseAxis } from '#lib/format.js';
import { formatMetric, formatSteps, metricAxis } from '#lib/metric-units.js';

/** `metric:<key>` plots against another metric logged at the same steps. */
export type XAxis = 'step' | 'time' | 'relative' | `metric:${string}`;

export type ChartSettings = {
	x: XAxis;
	/** EMA weight, 0 (off) to 0.99. */
	smoothing: number;
	logY: boolean;
	/** Min/max band of each downsampled bucket, or the spread of a group of runs. */
	band: boolean;
	/** A group's band: mean ± standard deviation, or min-max across its runs. */
	spread: 'std' | 'range';
	/** Best value so far, for metrics where higher or lower is better. */
	best: boolean;
};

export const DEFAULT_SETTINGS: ChartSettings = {
	x: 'step',
	smoothing: 0,
	logY: false,
	band: true,
	spread: 'std',
	best: false
};

/** What one chart changes from the page's settings. */
export type ChartOverride = Partial<Pick<ChartSettings, 'x' | 'smoothing' | 'band' | 'best'>>;

/** A run: hyphenated id, and its start on the client's clock (for relative time). */
export type ChartRun = { id: string; startMs: number };

/** One line on a chart: a run, or a group of runs drawn as their mean and spread. */
export type ChartSeries = { id: string; name: string; color: string; runs: ChartRun[] };

export const xMetric = (x: XAxis) => (x.startsWith('metric:') ? x.slice(7) : null);

export function xLabel(x: XAxis): string {
	if (x === 'step') return 'Step';
	if (x === 'time') return 'Wall time';
	if (x === 'relative') return 'Relative time';
	return xMetric(x)!;
}

const clock = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' });
const day = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' });

/** Seconds as a short duration: 45s, 12m, 3.5h, 2.1d. */
export function shortDuration(seconds: number): string {
	const abs = Math.abs(seconds);
	if (abs < 60) return `${Number(seconds.toPrecision(3))}s`;
	if (abs < 3600) return `${Number((seconds / 60).toPrecision(3))}m`;
	if (abs < 86400) return `${Number((seconds / 3600).toPrecision(3))}h`;
	return `${Number((seconds / 86400).toPrecision(3))}d`;
}

/** Axis tick formatter for an x-axis over `[lo, hi]`. */
export function xFormatter(x: XAxis, lo: number, hi: number): (value: number) => string {
	if (x === 'time') {
		const multiDay = hi - lo > 36 * 3600_000;
		return (v) => (multiDay ? day.format(v) : clock.format(v));
	}
	if (x === 'relative') return shortDuration;
	const key = xMetric(x);
	return key ? metricAxis(key, [lo, hi]) : (preciseAxis(lo, hi) ?? formatSteps);
}

/** Tooltip header for the hovered x. */
export function xTitle(x: XAxis, value: number): string {
	if (x === 'step') return `step ${Math.round(value).toLocaleString('en')}`;
	if (x === 'time') return new Date(value).toLocaleString();
	if (x === 'relative') return shortDuration(value);
	const key = xMetric(x)!;
	return `${key} ${formatMetric(key, value)}`;
}

/**
 * Debiased exponential moving average over downsampled buckets: a bucket of `n` points
 * decays the average as much as `n` single points would, so smoothing looks the same at
 * every zoom level. Empty buckets stay gaps.
 */
export function smooth(mean: (number | null)[], n: number[], weight: number): (number | null)[] {
	if (weight <= 0) return mean;
	let average = 0;
	let norm = 0;
	return mean.map((value, i) => {
		if (value === null || n[i] === 0) return null;
		const decay = weight ** n[i];
		average = average * decay + (1 - decay) * value;
		norm = norm * decay + (1 - decay);
		return average / norm;
	});
}

/**
 * Ticks for a log axis over `[lo, hi]`: 1-2-5 steps, thinned to powers of ten (then to
 * every n-th) when there would be more than `max`. d3's own log ticks label every minor tick.
 */
export function logTicks(lo: number, hi: number, max = 5): number[] {
	if (!(lo > 0) || !(hi > lo)) return [];
	const ticks: number[] = [];
	for (let e = Math.floor(Math.log10(lo)); e <= Math.ceil(Math.log10(hi)); e++) {
		for (const m of [1, 2, 5]) {
			const v = Number((m * 10 ** e).toPrecision(12));
			if (v >= lo * (1 - 1e-9) && v <= hi * (1 + 1e-9)) ticks.push(v);
		}
	}
	if (ticks.length <= max) return ticks;
	const powers = ticks.filter((v) => Math.abs(Math.log10(v) - Math.round(Math.log10(v))) < 1e-9);
	const stride = Math.ceil(powers.length / max);
	return powers.filter((_, i) => i % stride === 0);
}

/**
 * Too close to tell apart. Averages of a constant differ in their last digits, and a range that
 * narrow has no ticks to show.
 */
export function isFlat(lo: number, hi: number): boolean {
	return hi - lo <= Math.max(Math.abs(lo), Math.abs(hi)) * 1e-9;
}

/** Ticks for a chart's grid, which can't draw the same tick twice (as narrow ranges may give). */
export function distinctTicks(count = 4) {
	return (scale: { ticks?: (count: number) => number[] }) => [
		...new Set(scale.ticks?.(count) ?? [])
	];
}

/**
 * Ticks for a step axis: whole steps when few fit in the range (there's no step 0.2), otherwise
 * however many the chart likes.
 */
export function stepTicks(lo: number, hi: number, count = 5): number[] | number {
	if (hi - lo > count) return count;
	const ticks: number[] = [];
	for (let step = Math.ceil(lo) || 0; step <= hi; step++) ticks.push(step);
	return ticks;
}

/**
 * A y-range ignoring the most extreme values: from the 1st to the 99th percentile, with a little
 * room. One spike to 1e6 shouldn't flatten a whole curve. Null without enough values to tell.
 */
export function withoutOutliers(values: readonly number[]): [number, number] | null {
	if (values.length < 20) return null;
	const sorted = [...values].sort((a, b) => a - b);
	const at = (q: number) => sorted[Math.round((sorted.length - 1) * q)];
	const [lo, hi] = [at(0.01), at(0.99)];
	if (isFlat(lo, hi)) return null;
	const room = (hi - lo) * 0.05;
	return [Math.max(lo - room, sorted[0]), Math.min(hi + room, sorted[sorted.length - 1])];
}

/** Tick steps for time axes, in seconds. */
const DURATION_STEPS = [
	1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200, 10800, 21600, 43200, 86400, 172800,
	604800, 1209600, 2592000
];

/**
 * About `count` ticks on round durations over `[lo, hi]`, in units of `unit` seconds
 * (1 for relative time in seconds, 1000 for epoch milliseconds).
 */
export function durationTicks(lo: number, hi: number, unit = 1, count = 5): number[] {
	const span = (hi - lo) / unit;
	if (!(span > 0)) return [];
	const step = (DURATION_STEPS.find((s) => span / s <= count) ?? DURATION_STEPS.at(-1)!) * unit;
	// Steps of a day or more are aligned to local midnight; shorter ones to the round value.
	const offset = step >= 86400_000 && unit === 1000 ? new Date(0).getTimezoneOffset() * 60_000 : 0;
	const ticks: number[] = [];
	for (let t = Math.ceil((lo + offset) / step) * step - offset; t <= hi; t += step) ticks.push(t);
	return ticks;
}

/** Smoothing weights offered per chart. */
export const SMOOTHING_PRESETS = [0, 0.6, 0.8, 0.9, 0.95, 0.99];

/** Mean, and the band for `spread`, of the values of a group at one step. */
export function aggregate(values: number[], spread: ChartSettings['spread']) {
	const n = values.length;
	const mean = values.reduce((a, b) => a + b, 0) / n;
	if (spread === 'range') return { mean, lo: Math.min(...values), hi: Math.max(...values) };
	const std = n > 1 ? Math.sqrt(values.reduce((a, v) => a + (v - mean) ** 2, 0) / (n - 1)) : 0;
	return { mean, lo: mean - std, hi: mean + std };
}
