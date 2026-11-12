// Saving a chart: as an image, as the points it shows, or every point its runs logged.

import type { XAxis } from './settings.js';
import { xMetric } from './settings.js';

/** Runs a full-data download can ask for at once (the server's limit). */
export const MAX_EXPORT_RUNS = 500;
/** Longer URLs are refused by some servers and proxies. */
const MAX_URL_LENGTH = 8000;

/** A name that works as a file name everywhere: train/loss -> train_loss. */
export function fileName(name: string, extension: string): string {
	const base = name.replace(/[^\w.-]+/g, '_').replace(/^[._]+|[._]+$/g, '');
	return `${base || 'chart'}.${extension}`;
}

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

/** Has the browser save `what` (a URL, or data) as `name`. */
export function save(what: Blob | string, name: string) {
	const url = typeof what === 'string' ? what : URL.createObjectURL(what);
	const a = document.createElement('a');
	a.href = url;
	a.download = name;
	a.click();
	if (typeof what !== 'string') setTimeout(() => URL.revokeObjectURL(url), 60_000);
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
