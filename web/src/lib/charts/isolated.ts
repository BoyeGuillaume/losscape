/**
 * Points with no neighbour to draw a line to: a value logged once, or one between gaps. A line
 * through them has no length, so they'd be invisible without a mark of their own.
 */
export function isolatedPoints<Row>(rows: readonly Row[], defined: (row: Row) => boolean): Row[] {
	return rows.filter(
		(row, i) =>
			defined(row) &&
			(i === 0 || !defined(rows[i - 1])) &&
			(i === rows.length - 1 || !defined(rows[i + 1]))
	);
}
