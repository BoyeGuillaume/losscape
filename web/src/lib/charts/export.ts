// Saving a chart: as an image, as the points it shows, or every point its runs logged.

import { scaleLinear, scaleLog } from 'd3-scale';
import { area, curveLinear, curveMonotoneX, line } from 'd3-shape';

import { isolatedPoints } from './isolated.js';
import type { XAxis } from './settings.js';
import { xMetric } from './settings.js';

/** Runs a full-data download can ask for at once (the server's limit). */
export const MAX_EXPORT_RUNS = 500;
/** Longer URLs are refused by some servers and proxies. */
const MAX_URL_LENGTH = 8000;

/** One CSV field: quoted when it has to be, empty for nothing. */
export function csvField(value: string | number | null | undefined): string {
	if (value === null || value === undefined) return '';
	const text = String(value);
	return /[",\r\n]|^\s|\s$/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

export type ShownLine = {
	name: string;
	rows: {
		x: number;
		y: number | null;
		raw: number | null;
		min: number | null;
		max: number | null;
	}[];
};

/** Column of the x values, named for what they are. */
function xColumn(x: XAxis): { name: string; value: (x: number) => string | number } {
	if (x === 'step') return { name: 'step', value: (v) => v };
	if (x === 'time') return { name: 'time_utc', value: (v) => new Date(v).toISOString() };
	if (x === 'relative') return { name: 'relative_seconds', value: (v) => v };
	return { name: xMetric(x)!, value: (v) => v };
}

/**
 * The points a chart shows, within `range` of x: one row per line and point, with the
 * (smoothed) value, the unsmoothed one if smoothed, and the band if drawn. Points are
 * downsampled: each is the mean of the steps it covers.
 */
export function shownCsv(
	lines: ShownLine[],
	x: XAxis,
	range: [number, number],
	{ smoothed, band }: { smoothed: boolean; band: boolean }
): string {
	const column = xColumn(x);
	const header = ['series', column.name, 'value'];
	if (smoothed) header.push('unsmoothed');
	if (band) header.push('band_low', 'band_high');
	const out = [header.map(csvField).join(',')];
	for (const line of lines) {
		for (const row of line.rows) {
			if (row.y === null || row.x < range[0] || row.x > range[1]) continue;
			const fields: (string | number | null)[] = [line.name, column.value(row.x), row.y];
			if (smoothed) fields.push(row.raw);
			if (band) fields.push(row.min, row.max);
			out.push(fields.map(csvField).join(','));
		}
	}
	return out.join('\n') + '\n';
}

/**
 * Where every logged point of `keys` for `runs` downloads from, as CSV (see
 * crates/losscape-server/src/history.rs). Null when that's more than one request can ask.
 */
export function historyUrl(project: string, runs: string[], keys: string[]): string | null {
	if (!runs.length || !keys.length || runs.length > MAX_EXPORT_RUNS) return null;
	const query = new URLSearchParams();
	for (const run of runs) query.append('run', run);
	for (const key of keys) query.append('key', key);
	const url = `/api/v0/projects/${encodeURIComponent(project)}/history?${query}`;
	return url.length <= MAX_URL_LENGTH ? url : null;
}

export type ImageParts = {
	/** The chart's canvas, as drawn. */
	plot: HTMLCanvasElement;
	title: string;
	/** What the chart shows, under the title (x axis, smoothing...). */
	subtitle: string;
	legend: { name: string; color: string }[];
	colors: { background: string; text: string; muted: string };
	font: string;
};

/** Legend entries drawn before "and N more". */
const MAX_LEGEND = 24;

/**
 * The chart as a PNG: title, what it shows, a legend of its lines (which hovering gives on
 * the page) and the plot, at the resolution of the screen.
 */
export function chartImage({ plot, title, subtitle, legend, colors, font }: ImageParts) {
	const width = plot.getBoundingClientRect().width || plot.width;
	const scale = plot.width / width;
	const pad = 16;
	const out = document.createElement('canvas');
	const ctx = out.getContext('2d')!;
	const fontAt = (size: number, weight = 400) => `${weight} ${size}px ${font}`;

	// Legend entries in rows, as wide as the plot.
	ctx.font = fontAt(12);
	const entries = legend.slice(0, MAX_LEGEND).map((e) => ({ ...e, label: e.name }));
	if (legend.length > MAX_LEGEND) {
		entries.push({ name: '', color: '', label: `and ${legend.length - MAX_LEGEND} more` });
	}
	const placed: { x: number; row: number; color: string; label: string }[] = [];
	let [x, row] = [0, 0];
	for (const e of entries) {
		const w = (e.color ? 14 : 0) + Math.min(ctx.measureText(e.label).width, width - 14) + 16;
		if (x > 0 && x + w > width) [x, row] = [0, row + 1];
		placed.push({ x, row, color: e.color, label: e.label });
		x += w;
	}
	const titleH = 22;
	const subtitleH = subtitle ? 18 : 0;
	const legendH = placed.length ? (row + 1) * 18 + 6 : 0;
	const top = pad + titleH + subtitleH + legendH;
	const plotH = plot.height / scale;
	out.width = Math.round((width + 2 * pad) * scale);
	out.height = Math.round((top + plotH + pad) * scale);

	ctx.scale(scale, scale);
	ctx.fillStyle = colors.background;
	ctx.fillRect(0, 0, width + 2 * pad, top + plotH + pad);
	ctx.textBaseline = 'top';
	ctx.fillStyle = colors.text;
	ctx.font = fontAt(15, 600);
	ctx.fillText(title, pad, pad, width);
	if (subtitle) {
		ctx.fillStyle = colors.muted;
		ctx.font = fontAt(12);
		ctx.fillText(subtitle, pad, pad + titleH, width);
	}
	ctx.font = fontAt(12);
	const legendTop = pad + titleH + subtitleH;
	for (const p of placed) {
		const y = legendTop + p.row * 18;
		let at = pad + p.x;
		if (p.color) {
			ctx.fillStyle = p.color;
			ctx.fillRect(at, y + 3, 10, 3);
			at += 14;
		}
		ctx.fillStyle = p.color ? colors.text : colors.muted;
		ctx.fillText(p.label, at, y, width - 14);
	}
	ctx.drawImage(plot, pad, top, width, plotH);

	return new Promise<Blob>((resolve, reject) =>
		out.toBlob((blob) => (blob ? resolve(blob) : reject(new Error('no image'))), 'image/png')
	);
}

/** Ticks as a chart gives them: values, how many, or a function of the scale. */
export type TickSpec = number | number[] | ((scale: never) => number[]);

export type SvgParts = {
	/** The plot's box, axes included, and the room the axes take in it. */
	width: number;
	height: number;
	padding: { top: number; right: number; bottom: number; left: number };
	xDomain: [number, number];
	yDomain: [number, number];
	log: boolean;
	/** Round the y range to ticks, as the chart does. */
	nice: boolean;
	xTicks: TickSpec;
	yTicks: TickSpec;
	formatX: (value: number) => string;
	formatY: (value: number) => string;
	lines: {
		name: string;
		color: string;
		/** A group of runs, drawn thicker. */
		group: boolean;
		/** Every point in view as a dot (few of them), not only those standing alone. */
		dots?: boolean;
		/** Curves through the points rather than straight lines between them. */
		curved?: boolean;
		rows: {
			x: number;
			y: number | null;
			raw: number | null;
			min: number | null;
			max: number | null;
			best: number | null;
		}[];
	}[];
	show: { band: boolean; raw: boolean; best: boolean };
	title: string;
	subtitle: string;
	colors: { background: string; text: string; muted: string; grid: string };
	font: string;
};

/** Text and attribute values as XML: names are whatever runs and metrics were called. */
export const xml = (text: string) =>
	text.replace(
		/[&<>"']/g,
		(c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!
	);

const num = (v: number) => String(Math.round(v * 100) / 100);

/**
 * The chart as SVG, drawn from its data rather than copied from the screen: scalable, and
 * editable in a vector editor. Same marks, scales and ticks as the chart; no tooltip or
 * fading of other lines.
 */
export function chartSvg(p: SvgParts): string {
	const { top, right, bottom, left } = p.padding;
	const x = scaleLinear()
		.domain(p.xDomain)
		.range([left, p.width - right]);
	const y = (p.log ? scaleLog() : scaleLinear()).domain(p.yDomain).range([p.height - bottom, top]);
	if (p.nice) y.nice();
	const ticksOf = (spec: TickSpec, scale: { ticks: (count?: number) => number[] }) =>
		Array.isArray(spec)
			? spec
			: typeof spec === 'function'
				? (spec as (scale: unknown) => number[])(scale)
				: scale.ticks(spec);
	const [x0, x1] = x.range();
	const [y0, y1] = y.range();
	const usable = (v: number | null): v is number =>
		v !== null && Number.isFinite(v) && (!p.log || v > 0);

	type Rows = SvgParts['lines'][number]['rows'];
	/** A path through the points where `value` is usable, broken at gaps. */
	const path = (rows: Rows, value: (r: Rows[number]) => number | null, curved = false) =>
		line<Rows[number]>()
			.defined((r) => usable(value(r)))
			.x((r) => x(r.x))
			.y((r) => y(value(r)!))
			.curve(curved ? curveMonotoneX : curveLinear)
			.digits(2)(rows) ?? '';
	/** Areas between `min` and `max`, one per stretch where both are usable. */
	const band = (rows: Rows, curved = false) =>
		area<Rows[number]>()
			.defined((r) => usable(r.min) && usable(r.max))
			.x((r) => x(r.x))
			.y0((r) => y(r.min!))
			.y1((r) => y(r.max!))
			.curve(curved ? curveMonotoneX : curveLinear)
			.digits(2)(rows) ?? '';

	// Header: title, what the chart shows, and a legend in rows (text widths estimated).
	const pad = 16;
	const charWidth = 6.6;
	const entries: { at: number; row: number; color: string; label: string }[] = [];
	let [at, row] = [0, 0];
	const shownLines = p.lines.slice(0, MAX_LEGEND);
	const labels = shownLines.map((l) => ({ color: l.color, label: l.name }));
	if (p.lines.length > MAX_LEGEND) {
		labels.push({ color: '', label: `and ${p.lines.length - MAX_LEGEND} more` });
	}
	for (const e of labels) {
		const w = (e.color ? 14 : 0) + e.label.length * charWidth + 16;
		if (at > 0 && at + w > p.width) [at, row] = [0, row + 1];
		entries.push({ at, row, ...e });
		at += w;
	}
	const titleH = 22;
	const subtitleH = p.subtitle ? 18 : 0;
	const legendTop = pad + titleH + subtitleH;
	const plotTop = legendTop + (entries.length ? (row + 1) * 18 + 6 : 0);
	const width = p.width + 2 * pad;
	const height = plotTop + p.height + pad;

	const out: string[] = [
		`<svg xmlns="http://www.w3.org/2000/svg" width="${num(width)}" height="${num(height)}" viewBox="0 0 ${num(width)} ${num(height)}" font-family="${xml(p.font)}">`,
		`<rect width="100%" height="100%" fill="${xml(p.colors.background)}"/>`,
		`<text x="${pad}" y="${pad + 12}" font-size="15" font-weight="600" fill="${xml(p.colors.text)}">${xml(p.title)}</text>`
	];
	if (p.subtitle) {
		out.push(
			`<text x="${pad}" y="${pad + titleH + 10}" font-size="12" fill="${xml(p.colors.muted)}">${xml(p.subtitle)}</text>`
		);
	}
	for (const e of entries) {
		const ey = legendTop + e.row * 18;
		const ex = pad + e.at;
		if (e.color) {
			out.push(
				`<rect x="${num(ex)}" y="${num(ey + 3)}" width="10" height="3" fill="${xml(e.color)}"/>`
			);
		}
		out.push(
			`<text x="${num(ex + (e.color ? 14 : 0))}" y="${num(ey + 10)}" font-size="12" fill="${xml(e.color ? p.colors.text : p.colors.muted)}">${xml(e.label)}</text>`
		);
	}

	out.push(`<g transform="translate(${pad},${num(plotTop)})">`);
	out.push(
		`<defs><clipPath id="plot"><rect x="${num(x0)}" y="${num(y1)}" width="${num(x1 - x0)}" height="${num(y0 - y1)}"/></clipPath></defs>`
	);
	const label = `font-size="10" fill="${xml(p.colors.muted)}"`;
	for (const t of ticksOf(p.yTicks, y as never)) {
		const ty = y(t);
		if (!(ty >= y1 - 0.5 && ty <= y0 + 0.5)) continue;
		out.push(
			`<line x1="${num(x0)}" x2="${num(x1)}" y1="${num(ty)}" y2="${num(ty)}" stroke="${xml(p.colors.grid)}"/>`,
			`<text x="${num(x0 - 4)}" y="${num(ty)}" text-anchor="end" dominant-baseline="middle" ${label}>${xml(p.formatY(t))}</text>`
		);
	}
	out.push(
		`<line x1="${num(x0)}" x2="${num(x1)}" y1="${num(y0)}" y2="${num(y0)}" stroke="${xml(p.colors.muted)}" stroke-opacity="0.6"/>`
	);
	for (const t of ticksOf(p.xTicks, x as never)) {
		const tx = x(t);
		if (!(tx >= x0 - 0.5 && tx <= x1 + 0.5)) continue;
		out.push(
			`<text x="${num(tx)}" y="${num(y0 + 14)}" text-anchor="middle" ${label}>${xml(p.formatX(t))}</text>`
		);
	}

	out.push('<g clip-path="url(#plot)" fill="none">');
	for (const l of p.lines) {
		const color = xml(l.color);
		if (p.show.band) {
			const d = band(l.rows, l.curved);
			const opacity = l.group ? 0.2 : p.lines.length > 1 ? 0.1 : 0.16;
			if (d) out.push(`<path d="${d}" fill="${color}" fill-opacity="${opacity}" stroke="none"/>`);
		}
		if (p.show.raw) {
			const d = path(l.rows, (r) => r.raw, l.curved);
			if (d) out.push(`<path d="${d}" stroke="${color}" stroke-width="1" stroke-opacity="0.25"/>`);
		}
	}
	for (const l of p.lines) {
		const color = xml(l.color);
		if (p.show.best) {
			const d = path(l.rows, (r) => r.best, l.curved);
			if (d) {
				out.push(
					`<path d="${d}" stroke="${color}" stroke-width="1.25" stroke-dasharray="4 3" stroke-opacity="0.8"/>`
				);
			}
		}
		const d = path(l.rows, (r) => r.y, l.curved);
		if (d) out.push(`<path d="${d}" stroke="${color}" stroke-width="${l.group ? 2 : 1.5}"/>`);
		const inView = (r: (typeof l.rows)[number]) =>
			usable(r.y) && r.x >= p.xDomain[0] && r.x <= p.xDomain[1];
		const shown = l.dots ? l.rows.filter(inView) : isolatedPoints(l.rows, (r) => usable(r.y));
		for (const pt of shown) {
			out.push(
				`<circle cx="${num(x(pt.x))}" cy="${num(y(pt.y!))}" r="${l.dots ? 2.5 : 3}" fill="${color}"/>`
			);
		}
	}
	out.push('</g></g></svg>');
	return out.join('\n') + '\n';
}
