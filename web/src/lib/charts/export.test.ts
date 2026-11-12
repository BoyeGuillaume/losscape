import { describe, expect, it } from 'vitest';

import {
	MAX_EXPORT_RUNS,
	chartSvg,
	csvField,
	historyUrl,
	shownCsv,
	type SvgParts
} from './export.js';

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

describe('chartSvg', () => {
	const row = (x: number, y: number | null, band = 0.5) => ({
		x,
		y,
		raw: y,
		min: y === null ? null : y - band,
		max: y === null ? null : y + band,
		best: null
	});
	const parts = (over: Partial<SvgParts> = {}): SvgParts => ({
		width: 400,
		height: 200,
		padding: { top: 6, right: 6, bottom: 20, left: 40 },
		xDomain: [0, 10],
		yDomain: [0, 4],
		log: false,
		nice: true,
		xTicks: 5,
		yTicks: 4,
		formatX: String,
		formatY: String,
		lines: [
			{ name: 'run-1', color: '#2563eb', group: false, rows: [row(0, 3), row(5, 2), row(10, 1)] }
		],
		show: { band: false, raw: false, best: false },
		title: 'train/loss',
		subtitle: 'Step',
		colors: { background: '#fff', text: '#111', muted: '#666', grid: '#eee' },
		font: 'Inter, sans-serif',
		...over
	});
	const paths = (svg: string) => [...svg.matchAll(/<path d="([^"]*)" stroke=/g)].map((m) => m[1]);

	it('is an SVG document with the title, legend, axes and line', () => {
		const svg = chartSvg(parts());
		expect(svg.startsWith('<svg xmlns="http://www.w3.org/2000/svg"')).toBe(true);
		expect(svg).toContain('>train/loss</text>');
		expect(svg).toContain('>run-1</text>');
		expect(svg).toContain('>Step</text>');
		expect(paths(svg)).toHaveLength(1);
		expect(paths(svg)[0].match(/[ML]/g)).toEqual(['M', 'L', 'L']);
		expect([...svg.matchAll(/text-anchor="middle"/g)].length).toBeGreaterThan(2);
	});

	it('escapes names, which can be anything', () => {
		const svg = chartSvg(
			parts({
				title: '<script>alert(1)</script>',
				lines: [{ ...parts().lines[0], name: 'a & "b"', color: '"/><script>' }]
			})
		);
		expect(svg).not.toContain('<script');
		expect(svg).toContain('&lt;script&gt;alert(1)&lt;/script&gt;');
		expect(svg).toContain('a &amp; &quot;b&quot;');
	});

	it('breaks lines at gaps, and draws points standing alone as dots', () => {
		const rows = [
			row(0, 3),
			row(1, 2),
			row(2, null),
			row(3, 1),
			row(4, null),
			row(5, 2),
			row(6, 1)
		];
		const svg = chartSvg(parts({ lines: [{ ...parts().lines[0], rows }] }));
		expect(paths(svg)[0].match(/M/g)).toHaveLength(3);
		expect([...svg.matchAll(/<circle /g)]).toHaveLength(1);
	});

	it('leaves out what a log scale can’t show', () => {
		const rows = [row(0, 10), row(1, 0), row(2, 1)];
		const svg = chartSvg(
			parts({
				log: true,
				yDomain: [0.5, 20],
				yTicks: [1, 10],
				lines: [{ ...parts().lines[0], rows }]
			})
		);
		expect(paths(svg)[0]).not.toContain('NaN');
		expect(paths(svg)[0].match(/M/g)).toHaveLength(2);
	});

	it('draws the band, unsmoothed and best lines when shown', () => {
		const svg = chartSvg(parts({ show: { band: true, raw: true, best: false } }));
		expect(svg).toMatch(/<path d="M[^"]*Z" fill="#2563eb" fill-opacity="0.16"/);
		expect(svg).toContain('stroke-opacity="0.25"');
	});
});
