import { describe, expect, it } from 'vitest';

import { phasePath } from './phase.js';

const series = (points: [number, number | null][]) => ({
	x: points.map((p) => p[0]),
	mean: points.map((p) => p[1])
});

describe('phasePath', () => {
	it('pairs metrics logged at the same steps', () => {
		const path = phasePath(
			series([
				[0, 1],
				[1, 2]
			]),
			series([
				[0, 10],
				[1, 20]
			])
		);
		expect(path).toEqual([
			{ step: 0, x: 1, y: 10 },
			{ step: 1, x: 2, y: 20 }
		]);
	});

	it('follows the sparser metric, with the latest value of the other', () => {
		const train = series([
			[0, 5],
			[1, 4],
			[2, 3],
			[3, 2],
			[4, 1]
		]);
		const val = series([
			[1, 50],
			[4, 80]
		]);
		expect(phasePath(train, val)).toEqual([
			{ step: 1, x: 4, y: 50 },
			{ step: 4, x: 1, y: 80 }
		]);
		// The same with the axes swapped.
		expect(phasePath(val, train)).toEqual([
			{ step: 1, x: 50, y: 4 },
			{ step: 4, x: 80, y: 1 }
		]);
	});

	it('can follow the metric that moves, for one logged only when it changes', () => {
		const params = series([
			[0, 100],
			[3, 500]
		]);
		const loss = series([
			[0, 4],
			[1, 3],
			[2, 2],
			[3, 1],
			[4, 0.5]
		]);
		expect(phasePath(params, loss)).toHaveLength(2);
		expect(phasePath(params, loss, 'y').map((p) => [p.x, p.y])).toEqual([
			[100, 4],
			[100, 3],
			[100, 2],
			[500, 1],
			[500, 0.5]
		]);
		expect(phasePath(loss, params, 'x').map((p) => [p.x, p.y])).toEqual([
			[4, 100],
			[3, 100],
			[2, 100],
			[1, 500],
			[0.5, 500]
		]);
	});

	it('starts once both have a value, and skips gaps', () => {
		const late = series([
			[2, 7],
			[3, 8]
		]);
		const early = series([
			[0, 1],
			[1, null],
			[2, NaN],
			[3, 4],
			[4, 5],
			[5, 6]
		]);
		expect(phasePath(late, early)).toEqual([
			{ step: 2, x: 7, y: 1 },
			{ step: 3, x: 8, y: 4 }
		]);
		expect(phasePath(series([]), early)).toEqual([]);
	});
});
