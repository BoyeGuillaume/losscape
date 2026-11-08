// View state kept in the URL's query, so a link opens the same view. Pages read their
// state once from `page.url` and mirror changes back with `syncUrl` (no history entries).

import { untrack } from 'svelte';
import { replaceState } from '$app/navigation';
import { page } from '$app/state';

import {
	DEFAULT_SETTINGS,
	type ChartOverride,
	type ChartSettings,
	type XAxis
} from '#lib/charts/settings.js';

export const param = (name: string) => page.url.searchParams.get(name);

/**
 * Mirrors `params()` (null or empty: parameter left out) into the URL, at most every
 * 300 ms. Call from a component's script.
 */
export function syncUrl(params: () => Record<string, string | null>) {
	let timer: ReturnType<typeof setTimeout> | undefined;
	$effect(() => {
		const next = params();
		clearTimeout(timer);
		timer = setTimeout(() => {
			// eslint-disable-next-line svelte/prefer-svelte-reactivity -- a one-off copy to edit
			const url = new URL(untrack(() => page.url.href));
			for (const [name, value] of Object.entries(next)) {
				if (value) url.searchParams.set(name, value);
				else url.searchParams.delete(name);
			}
			if (url.search !== window.location.search) {
				const state = untrack(() => page.state);
				// eslint-disable-next-line svelte/no-navigation-without-resolve -- the current page's own URL
				void replaceState(url, state);
			}
		}, 300);
		return () => clearTimeout(timer);
	});
}

/** Chart settings from `x`, `smooth`, `log`, `band`, `spread`. */
export function readChartSettings(): ChartSettings {
	const smoothing = Number(param('smooth'));
	return {
		x: (param('x') as XAxis | null) ?? DEFAULT_SETTINGS.x,
		smoothing: Number.isFinite(smoothing) ? Math.min(Math.max(smoothing, 0), 0.99) : 0,
		logY: param('log') === '1',
		band: param('band') !== '0',
		spread: param('spread') === 'range' ? 'range' : 'std',
		best: param('best') === '1'
	};
}

export function chartSettingsParams(s: ChartSettings): Record<string, string | null> {
	return {
		x: s.x === DEFAULT_SETTINGS.x ? null : s.x,
		smooth: s.smoothing > 0 ? String(s.smoothing) : null,
		log: s.logY ? '1' : null,
		band: s.band ? null : '0',
		spread: s.spread === 'range' ? 'range' : null,
		best: s.best ? '1' : null
	};
}

/** Zoom as `lo,hi`. */
export function readZoom(): [number, number] | null {
	const [lo, hi] = (param('zoom') ?? '').split(',').map(Number);
	return Number.isFinite(lo) && Number.isFinite(hi) && hi > lo ? [lo, hi] : null;
}

export const zoomParam = (view: [number, number] | null) =>
	view ? `${Number(view[0].toPrecision(8))},${Number(view[1].toPrecision(8))}` : null;

/** Per-chart settings, as JSON in `charts` (`{"train/loss": {"logY": true}}`). */
export function readOverrides(): Record<string, ChartOverride> {
	try {
		const parsed = JSON.parse(param('charts') ?? '{}');
		return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : {};
	} catch {
		return {};
	}
}

export const overridesParam = (overrides: Record<string, ChartOverride>) =>
	Object.keys(overrides).length ? JSON.stringify(overrides) : null;
