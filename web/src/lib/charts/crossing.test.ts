import { describe, expect, it } from 'vitest';

import { crossing } from './crossing.js';

const rows = [
	{ x: 0, y: 10 },
	{ x: 1000, y: 20 },
	{ x: 2000, y: null },
	{ x: 3000, y: 5 },
	{ x: 4000, y: 50 }
];
const y = (r: { y: number | null }) => r.y;

describe('crossing', () => {
	it('interpolates between the points around x', () => {
		expect(crossing(rows, 250, y)).toBe(12.5);
		expect(crossing(rows, 1000, y), 'on a point').toBe(20);
	});

	it('has no value outside the line or next to a gap', () => {
		expect(crossing(rows, -1, y)).toBe(null);
		expect(crossing(rows, 4001, y)).toBe(null);
		expect(crossing(rows, 1500, y)).toBe(null);
		expect(crossing(rows, 2500, y)).toBe(null);
		expect(crossing([], 0, y)).toBe(null);
	});

	it('follows the line as drawn on a log scale', () => {
		expect(crossing(rows, 3500, y, true)).toBeCloseTo(Math.sqrt(5 * 50));
		expect(
			crossing(
				[
					{ x: 0, y: 0 },
					{ x: 2, y: 4 }
				],
				1,
				y,
				true
			),
			'no log of 0'
		).toBe(null);
	});
});
