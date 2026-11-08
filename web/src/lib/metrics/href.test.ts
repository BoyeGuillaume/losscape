import { describe, expect, it } from 'vitest';

import { fitsInAPath } from './href.js';

describe('metric links', () => {
	it('tells which names can be part of a path', () => {
		for (const name of ['loss', 'val/acc', 'a.b/c', '..a', 'a..', '__proto__/x']) {
			expect(fitsInAPath(name), name).toBe(true);
		}
		for (const name of ['', '/', 'x/', '/x', 'a//b', '.', '..', './x', 'a/../b', 'a/.']) {
			expect(fitsInAPath(name), name).toBe(false);
		}
	});
});
