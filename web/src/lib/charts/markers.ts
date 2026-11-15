// Vertical lines at given steps of a chart (a run's checkpoints): hovered, they say what they
// mark; clicked, they open it.

export type ChartMarker = {
	step: number;
	/** First line of what's shown when hovered, e.g. "Checkpoint at step 1,999". */
	title: string;
	/** Second line, e.g. "2 hours ago · 5 files · 45.8 MiB". */
	detail: string;
};

/** Pixels from a line within which the pointer is on it. */
export const MARKER_REACH = 8;

/**
 * The marker nearest to the pointer at `px` (pixels), if within `reach`; `toPx` places a step.
 */
export function nearestMarker<T extends { step: number }>(
	markers: readonly T[],
	px: number,
	toPx: (step: number) => number,
	reach = MARKER_REACH
): T | null {
	let best: T | null = null;
	let bestDistance = reach;
	for (const m of markers) {
		const distance = Math.abs(toPx(m.step) - px);
		if (distance <= bestDistance) {
			best = m;
			bestDistance = distance;
		}
	}
	return best;
}
