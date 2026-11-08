import { describe, expect, it } from 'vitest';

import { isolatedPoints } from './isolated.js';

const found = (values: (number | null)[]) => isolatedPoints(values, (v) => v !== null);

describe('isolated points', () => {
	it('finds a value logged once', () => {
		expect(found([5])).toEqual([5]);
		expect(found([null, 5, null])).toEqual([5]);
	});

	it('finds values between gaps, also at the ends', () => {
		expect(found([1, null, 2, 3, null, 4])).toEqual([1, 4]);
	});

	it('leaves out what a line reaches', () => {
		expect(found([1, 2])).toEqual([]);
		expect(found([1, 2, 3, null, 4, 5])).toEqual([]);
		expect(found([])).toEqual([]);
		expect(found([null, null])).toEqual([]);
	});
});
