import { describe, expect, it } from 'vitest';

import { aggregate, durationTicks, logTicks, smooth } from './settings.js';

describe('smooth', () => {
	it('is the identity at weight 0', () => {
		expect(smooth([1, null, 3], [1, 0, 1], 0)).toEqual([1, null, 3]);
	});

	it('weighs buckets by their point count', () => {
		// One bucket of 2 points decays like two single points of the same value.
		const single = smooth([1, 2, 2], [1, 1, 1], 0.5);
		const bucketed = smooth([1, 2], [1, 2], 0.5);
		expect(bucketed[1]).toBeCloseTo(single[2]!);
	});

	it('is debiased: a constant series stays constant', () => {
		for (const v of smooth([5, 5, 5], [3, 1, 7], 0.9)) expect(v).toBeCloseTo(5, 12);
	});

	it('keeps gaps', () => {
		expect(smooth([1, null, 1], [1, 0, 1], 0.5)[1]).toBeNull();
	});
});

describe('logTicks', () => {
	it('uses 1-2-5 steps over a short range', () => {
		expect(logTicks(0.15, 3)).toEqual([0.2, 0.5, 1, 2]);
	});

	it('keeps powers of ten over a long range', () => {
		expect(logTicks(0.001, 10)).toEqual([0.001, 0.01, 0.1, 1, 10]);
		expect(logTicks(1e-8, 1e2)).toEqual([1e-8, 1e-5, 0.01, 10]);
	});

	it('gives nothing for a domain that is not positive', () => {
		expect(logTicks(0, 1)).toEqual([]);
	});
});

describe('durationTicks', () => {
	it('picks round steps for relative time', () => {
		expect(durationTicks(0, 250)).toEqual([0, 60, 120, 180, 240]);
		expect(durationTicks(12, 47)).toEqual(
			[15, 25, 35, 45].map((v) => v - 5).filter((v) => v >= 12)
		);
	});

	it('works on epoch milliseconds', () => {
		const t0 = Date.UTC(2026, 0, 1, 10, 3);
		const ticks = durationTicks(t0, t0 + 3_600_000, 1000);
		expect(ticks.length).toBeGreaterThanOrEqual(3);
		expect(ticks.every((t) => t % 900_000 === 0)).toBe(true);
	});
});

describe('aggregate', () => {
	it('gives the mean with a standard deviation or min-max band', () => {
		expect(aggregate([1, 2, 3], 'std')).toEqual({ mean: 2, lo: 1, hi: 3 });
		expect(aggregate([1, 2, 6], 'range')).toEqual({ mean: 3, lo: 1, hi: 6 });
		expect(aggregate([4], 'std')).toEqual({ mean: 4, lo: 4, hi: 4 });
	});
});
