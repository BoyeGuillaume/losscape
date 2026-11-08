// A run's path through "phase space": one metric against another, step by step.

import type { SeriesPoints } from './tiles.svelte.js';

export type PhasePoint = { step: number; x: number; y: number };

type Series = Pick<SeriesPoints, 'x' | 'mean'>;

function known(s: Series) {
	return s.x.flatMap((step, i) => {
		const value = s.mean[i];
		return value !== null && Number.isFinite(value) ? [{ step, value }] : [];
	});
}

/**
 * Pairs two metrics of a run. They're often logged at different rates (training every step,
 * validation every few): one point per step of the sparser one, with the other's latest
 * value at that step. Nothing before both have started. `follow` picks the one to follow
 * instead: a value only logged when it changes shouldn't make a path a single point.
 */
export function phasePath(xs: Series, ys: Series, follow?: 'x' | 'y'): PhasePoint[] {
	const [a, b] = [known(xs), known(ys)];
	const xIsBase = follow ? follow === 'x' : a.length <= b.length;
	const [base, other] = xIsBase ? [a, b] : [b, a];
	const path: PhasePoint[] = [];
	let j = 0;
	let latest: number | null = null;
	for (const p of base) {
		while (j < other.length && other[j].step <= p.step) latest = other[j++].value;
		if (latest === null) continue;
		path.push({
			step: p.step,
			x: xIsBase ? p.value : latest,
			y: xIsBase ? latest : p.value
		});
	}
	return path;
}
