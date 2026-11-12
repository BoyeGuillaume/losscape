// Where a line crosses a vertical: for fitting the y range to a view that falls between two
// points, where the line is all there is to see.

/**
 * The value at `x` of the line through `rows` (sorted by x): interpolated between the two
 * points around it, in log space on a log scale (as the line is drawn). Null where there's no
 * line: outside the rows, or next to a point that isn't drawn (a gap).
 */
export function crossing<Row extends { x: number }>(
	rows: readonly Row[],
	x: number,
	value: (row: Row) => number | null,
	log = false
): number | null {
	const usable = (v: number | null): v is number =>
		v !== null && Number.isFinite(v) && (!log || v > 0);
	// Last row at or before x.
	let lo = 0;
	let hi = rows.length - 1;
	if (hi < 0 || x < rows[0].x || x > rows[hi].x) return null;
	while (lo < hi) {
		const mid = Math.ceil((lo + hi) / 2);
		if (rows[mid].x <= x) lo = mid;
		else hi = mid - 1;
	}
	const a = value(rows[lo]);
	if (rows[lo].x === x) return usable(a) ? a : null;
	const next = rows[lo + 1];
	const b = next ? value(next) : null;
	if (!usable(a) || !usable(b)) return null;
	const t = (x - rows[lo].x) / (next.x - rows[lo].x);
	return log ? Math.exp(Math.log(a) + t * (Math.log(b) - Math.log(a))) : a + t * (b - a);
}
