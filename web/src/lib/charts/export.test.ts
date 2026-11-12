import { describe, expect, it } from 'vitest';

import { MAX_EXPORT_RUNS, csvField, fileName, historyUrl, shownCsv } from './export.js';

describe('fileName', () => {
	it('works as a file name', () => {
		expect(fileName('train/loss', 'png')).toBe('train_loss.png');
		expect(fileName('val acc (top-5)', 'csv')).toBe('val_acc_top-5.csv');
		expect(fileName('../..', 'csv')).toBe('chart.csv');
	});
});

describe('csvField', () => {
	it('quotes only when it has to', () => {
		expect(csvField('plain')).toBe('plain');
		expect(csvField('a,b')).toBe('"a,b"');
		expect(csvField('say "hi"')).toBe('"say ""hi"""');
		expect(csvField('two\nlines')).toBe('"two\nlines"');
		expect(csvField(1.5)).toBe('1.5');
		expect(csvField(null)).toBe('');
	});
});

describe('shownCsv', () => {
	const lines = [
		{
			name: 'run, one',
			rows: [
				{ x: 0, y: 2, raw: 2.5, min: 1, max: 3 },
				{ x: 10, y: 1, raw: 0.5, min: 0, max: 2 },
				{ x: 20, y: null, raw: null, min: null, max: null }
			]
		},
		{ name: 'two', rows: [{ x: 10, y: 4, raw: 4, min: 4, max: 4 }] }
	];

	it('has the points in view, and only the columns drawn', () => {
		expect(shownCsv(lines, 'step', [5, 30], { smoothed: false, band: false })).toBe(
			'series,step,value\n"run, one",10,1\ntwo,10,4\n'
		);
	});

	it('adds the unsmoothed values and the band when drawn', () => {
		const csv = shownCsv(lines, 'step', [0, 30], { smoothed: true, band: true });
		expect(csv.split('\n')[0]).toBe('series,step,value,unsmoothed,band_low,band_high');
		expect(csv.split('\n')[1]).toBe('"run, one",0,2,2.5,1,3');
	});

	it('names x for what it is', () => {
		const one = [{ name: 'a', rows: [{ x: 0, y: 1, raw: 1, min: 1, max: 1 }] }];
		const time = shownCsv(one, 'time', [0, 1], { smoothed: false, band: false });
		expect(time).toBe('series,time_utc,value\na,1970-01-01T00:00:00.000Z,1\n');
		expect(shownCsv([], 'relative', [0, 1], { smoothed: false, band: false })).toBe(
			'series,relative_seconds,value\n'
		);
		expect(shownCsv([], 'metric:epoch', [0, 1], { smoothed: false, band: false })).toBe(
			'series,epoch,value\n'
		);
	});
});

describe('historyUrl', () => {
	it('asks for every run and metric', () => {
		const url = historyUrl('mnist', ['r1', 'r2'], ['train/loss', 'a&b']);
		expect(url).toBe('/api/v0/projects/mnist/history?run=r1&run=r2&key=train%2Floss&key=a%26b');
	});

	it('gives up on what one request can’t ask', () => {
		expect(historyUrl('mnist', [], ['loss'])).toBe(null);
		const many = Array.from({ length: MAX_EXPORT_RUNS + 1 }, (_, i) => `r${i}`);
		expect(historyUrl('mnist', many, ['loss'])).toBe(null);
		expect(historyUrl('mnist', ['r'], ['x'.repeat(9000)])).toBe(null);
	});
});
