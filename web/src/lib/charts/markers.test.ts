import { describe, expect, it } from 'vitest';

import { nearestMarker } from './markers.js';

describe('nearestMarker', () => {
	const markers = [{ step: 100 }, { step: 110 }, { step: 500 }];
	const toPx = (step: number) => step; // a pixel per step

	it('takes the closest line within reach', () => {
		expect(nearestMarker(markers, 103, toPx)?.step).toBe(100);
		expect(nearestMarker(markers, 106, toPx)?.step).toBe(110);
		expect(nearestMarker(markers, 495, toPx)?.step).toBe(500);
	});

	it('takes none further away', () => {
		expect(nearestMarker(markers, 300, toPx)).toBeNull();
		expect(nearestMarker(markers, 120, toPx, 5)).toBeNull();
		expect(nearestMarker([], 0, toPx)).toBeNull();
	});
});
