import { describe, expect, it } from 'vitest';

import { splitRanges } from './tiles.svelte.js';

const ranges = (count: number, tiles: number) =>
	Array.from({ length: count }, () => ({ from_tile: 0, to_tile: tiles }));

describe('tile requests', () => {
	it('stay within what the server accepts', () => {
		const within = (requests: ReturnType<typeof splitRanges>) => {
			for (const request of requests) {
				expect(request.length).toBeLessThanOrEqual(512);
				expect(request.reduce((n, r) => n + r.to_tile - r.from_tile, 0)).toBeLessThanOrEqual(2048);
			}
			return requests.flat().length;
		};
		expect(splitRanges([])).toEqual([]);
		expect(within(splitRanges(ranges(3, 1)))).toBe(3);
		expect(splitRanges(ranges(3, 1))).toHaveLength(1);
		expect(within(splitRanges(ranges(1000, 1)))).toBe(1000);
		expect(within(splitRanges(ranges(100, 64)))).toBe(100);
		expect(splitRanges(ranges(100, 64))).toHaveLength(4);
	});
});
