import { describe, expect, it } from 'vitest';

import {
	aggregate,
	distinctTicks,
	durationTicks,
	isFlat,
	logTicks,
	smooth,
	stepTicks,
	withoutOutliers
} from './settings.js';

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

describe('stepTicks', () => {
	it('shows only whole steps when few fit', () => {
		expect(stepTicks(-0.5, 0.5)).toEqual([0]);
		expect(stepTicks(0, 3)).toEqual([0, 1, 2, 3]);
		expect(stepTicks(2.5, 4.5)).toEqual([3, 4]);
	});

	it('leaves wide ranges to the chart', () => {
		expect(stepTicks(0, 1000)).toBe(5);
	});
});

describe('withoutOutliers', () => {
	const steady = Array.from({ length: 200 }, (_, i) => 1 + i / 200);

	it('leaves a spike out of the range', () => {
		const range = withoutOutliers([...steady, 1e6, 1e6])!;
		expect(range[0]).toBeGreaterThanOrEqual(1);
		expect(range[1]).toBeLessThan(3);
	});

	it('leaves out extremes on both sides', () => {
		const range = withoutOutliers([-1e6, -1e6, ...steady, 1e6, 1e6])!;
		expect(range[0]).toBeGreaterThan(0);
		expect(range[1]).toBeLessThan(3);
	});

	it("doesn't go beyond the data", () => {
		const range = withoutOutliers(steady)!;
		expect(range[0]).toBeGreaterThanOrEqual(1);
		expect(range[1]).toBeLessThanOrEqual(2);
	});

	it('gives up without enough values, or on a flat line', () => {
		expect(withoutOutliers([1, 2, 3])).toBe(null);
		expect(withoutOutliers(Array(50).fill(3))).toBe(null);
	});
});

describe('near-identical values', () => {
	// What buckets of a constant learning rate really average to.
	const means = [3e-6 * (1 - 2 ** -52), 3e-6, 3e-6 * (1 + 2 ** -52)];
	const [lo, hi] = [means[0], means[2]];

	it('count as flat', () => {
		expect(isFlat(lo, hi)).toBe(true);
		expect(isFlat(0, 0)).toBe(true);
		expect(isFlat(5, 5)).toBe(true);
	});

	it("aren't confused with a small but real range", () => {
		expect(isFlat(3e-6, 3.1e-6)).toBe(false);
		expect(isFlat(0, 1e-300)).toBe(false);
		expect(isFlat(-1, 1)).toBe(false);
	});

	it('give no outlier-free range', () => {
		expect(withoutOutliers(Array.from({ length: 50 }, (_, i) => means[i % 3]))).toBe(null);
	});

	it('never give the grid the same tick twice', () => {
		const scale = { ticks: () => [3e-6, 3e-6, hi] };
		expect(distinctTicks()(scale)).toEqual([3e-6, hi]);
	});
});
