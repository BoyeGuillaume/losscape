import { describe, expect, it } from 'vitest';

import { preciseAxis } from './format.js';

describe('preciseAxis', () => {
	it('leaves wide ranges to compact labels', () => {
		expect(preciseAxis(0, 3_000_000)).toBe(null);
		expect(preciseAxis(0, 1)).toBe(null);
		expect(preciseAxis(5, 5)).toBe(null);
	});

	it('tells steps apart deep in a long run', () => {
		const label = preciseAxis(2_717_848, 2_718_567)!;
		expect(label(2_718_000)).toBe('2,718,000');
		expect(label(2_718_200)).toBe('2,718,200');
	});

	it('tells close values apart', () => {
		const label = preciseAxis(0.98312, 0.98319)!;
		expect(label(0.98314)).not.toBe(label(0.98316));
		expect(label(0.98314)).toBe('0.98314');
	});

	it('keeps tiny values readable', () => {
		const label = preciseAxis(3e-6, 3.0001e-6)!;
		expect(label(3.00002e-6)).toBe('3.000020e-6');
		expect(label(0)).toBe('0');
	});

	it('handles negative ranges', () => {
		const label = preciseAxis(-1000.2, -1000.1)!;
		expect(label(-1000.15)).toBe('-1,000.15');
	});
});
