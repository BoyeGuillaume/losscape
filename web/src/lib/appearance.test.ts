import { describe, expect, it } from 'vitest';

import { parseAppearance } from './appearance.svelte.js';

describe('parseAppearance', () => {
	it('keeps valid settings', () => {
		const all = {
			theme: 'dark',
			palette: 'ocean',
			curve: 'smooth',
			chartSize: 'large',
			columns: '3',
			dots: false
		};
		expect(parseAppearance(all)).toEqual(all);
	});

	it('leaves out anything else, whatever was stored', () => {
		expect(
			parseAppearance({
				theme: 'neon',
				palette: 'toString',
				curve: 'bezier',
				chartSize: 'constructor',
				columns: 3,
				dots: 'yes',
				other: 1
			})
		).toEqual({});
		for (const junk of [null, undefined, 'dark', 42, ['dark']]) {
			expect(parseAppearance(junk)).toEqual({});
		}
	});

	it('keeps what is valid next to what isn’t', () => {
		expect(parseAppearance({ theme: 'light', columns: '9' })).toEqual({ theme: 'light' });
	});
});
