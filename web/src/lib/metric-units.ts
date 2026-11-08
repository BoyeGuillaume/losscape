// Formatting metric values in their unit. Units come from the project's metric
// definitions (its Metrics tab), with built-in defaults for system metrics.

import { scaleLinear } from 'd3-scale';

import { formatCompact, formatValue } from '#lib/format.js';
import { currentRegistry } from '#lib/metrics/store.svelte.js';
import { COUNT, formatUnit, stepFor, type Unit } from '#lib/units.js';

/** A metric's unit, from the project's metric definitions (or built-in defaults). */
export function unitFor(key: string): Unit | null {
	return currentRegistry().describe(key).unit;
}

/** A metric value for reading (tooltips, tables): its unit, or 4 significant digits. */
export function formatMetric(key: string, value: unknown): string {
	const unit = typeof value === 'number' ? unitFor(key) : null;
	return unit ? formatUnit(value as number, unit) : formatValue(value);
}

/** Axis labels for a metric: its unit, or compact numbers (1.5k, 0.002). */
export function metricAxis(key: string): (value: number) => string {
	const unit = unitFor(key);
	return unit ? (v) => formatUnit(v, unit) : formatCompact;
}

/** Step counts: 50k, 1.2M. */
export const formatSteps = (steps: number) => formatUnit(steps, COUNT);

/**
 * About `count` axis ticks over `[lo, hi]` on round values of the unit shown (5, 10,
 * 15 MiB rather than round GiB values that read 4.88, 9.77 MiB).
 */
export function unitTicks(unit: Unit, lo: number, hi: number, count = 4): number[] {
	const scale = unit.scale ?? 1;
	const { factor } = stepFor(Math.max(Math.abs(lo), Math.abs(hi)) * scale, unit);
	const per = scale / factor; // stored value -> shown number
	return scaleLinear()
		.domain([lo * per, hi * per])
		.ticks(count)
		.map((t) => t / per);
}
