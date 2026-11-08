import { describe, expect, it } from 'vitest';

import { parseChartSize } from './size.svelte.js';

describe('parseChartSize', () => {
	it('keeps a known size', () => {
		expect(parseChartSize('large')).toBe('large');
	});

	it('falls back on anything else', () => {
		expect(parseChartSize(null)).toBe('medium');
		expect(parseChartSize('huge')).toBe('medium');
		expect(parseChartSize('constructor')).toBe('medium');
	});
});
