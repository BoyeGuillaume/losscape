import { describe, expect, it } from 'vitest';

import { formatMetric, formatSteps, unitFor, unitTicks } from './metric-units.js';
import { BITS_PER_SECOND, BYTES, COUNT, PERCENT, formatUnit, scaled, type Unit } from './units.js';

describe('formatUnit', () => {
	it('scales counts by thousands', () => {
		expect([0, 950, 1000, 50_000, 51_234, 1_234_567, 3e9].map((v) => formatUnit(v, COUNT))).toEqual(
			['0', '950', '1k', '50k', '51.2k', '1.23M', '3B']
		);
		expect(formatSteps(24_314)).toBe('24.3k');
	});

	it('moves up a step when rounding reaches it', () => {
		expect(formatUnit(999_960, COUNT)).toBe('1M');
		expect(formatUnit(1023.9, BYTES), '1020 is still below 1024').toBe('1020 B');
		expect(formatUnit(1_048_570, BYTES)).toBe('1020 KiB');
		expect(formatUnit(1_048_576, BYTES)).toBe('1 MiB');
	});

	it('scales bytes by 1024, with values stored in another unit', () => {
		expect(formatUnit(512, BYTES)).toBe('512 B');
		expect(formatUnit(1536, BYTES)).toBe('1.5 KiB');
		expect(formatUnit(0.5, scaled(BYTES, 1024 ** 3))).toBe('512 MiB');
		expect(formatUnit(3.2, scaled(BYTES, 1024 ** 3))).toBe('3.2 GiB');
	});

	it('shows percent with fixed decimals, trailing zeros dropped', () => {
		expect(formatUnit(45.27, PERCENT)).toBe('45.3%');
		expect(formatUnit(100, PERCENT)).toBe('100%');
		expect(formatUnit(0.04, PERCENT)).toBe('0%');
	});

	it('takes any ladder and precision', () => {
		const duration: Unit = {
			steps: [
				{ factor: 1, suffix: 's' },
				{ factor: 60, suffix: 'min' },
				{ factor: 3600, suffix: 'h' }
			],
			digits: 2
		};
		expect([42, 90, 5400, 7200].map((v) => formatUnit(v, duration))).toEqual([
			'42 s',
			'1.5 min',
			'1.5 h',
			'2 h'
		]);
	});

	it('keeps signs, tiny values and non-finite values', () => {
		expect(formatUnit(-1536, BYTES)).toBe('-1.5 KiB');
		expect(formatUnit(0.000123, COUNT)).toBe('0.000123');
		expect(formatUnit(NaN, COUNT)).toBe('NaN');
	});
});

describe('metric units', () => {
	it('applies to the system metrics listed, and nothing else', () => {
		expect(unitFor('system/cpu/util')).toEqual(PERCENT);
		expect(unitFor('system/disk/usage')).toEqual(PERCENT);
		expect(formatMetric('system/memory/process', 1.25 * 2 ** 30)).toBe('1.25 GiB');
		expect(formatMetric('system/gpu0/memory', 2 ** 28)).toBe('256 MiB');
		expect(formatMetric('system/gpu0/power', 312.5)).toBe('313 W');
		expect(formatMetric('system/gpu0/temp', 67)).toBe('67 °C');
		expect(unitFor('system/cpu/util_pct'), 'old names: plain').toBeNull();
		expect(formatMetric('system/network/recv', 12_500_000)).toBe('12.5 Mbps');
		expect(formatUnit(800, BITS_PER_SECOND)).toBe('800 bps');
		expect(unitFor('system/network/recv_mbps'), 'old key: plain').toBeNull();
		expect(unitFor('train/loss')).toBeNull();
		expect(formatMetric('train/loss', 0.123456)).toBe('0.1235');
	});
});

describe('unitTicks', () => {
	it('puts ticks on round values of the unit shown', () => {
		const gib = scaled(BYTES, 1024 ** 3);
		// 2-29 MiB stored in GiB: ticks every 5 MiB, not every 0.005 GiB (5.12 MiB).
		const ticks = unitTicks(gib, 2 / 1024, 29 / 1024);
		expect(ticks.map((t) => formatUnit(t, gib))).toEqual([
			'5 MiB',
			'10 MiB',
			'15 MiB',
			'20 MiB',
			'25 MiB'
		]);
		expect(unitTicks(PERCENT, 0, 100)).toEqual([0, 20, 40, 60, 80, 100]);
	});
});
