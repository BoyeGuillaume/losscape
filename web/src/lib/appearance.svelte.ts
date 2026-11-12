// How the app looks, at two levels: the account's settings follow the user from browser to
// browser, and this browser's override them where it has its own (a laptop's small screen,
// say). Anything else gets the defaults.

import { apiSend } from '#lib/api/client.js';
import { auth } from '#lib/auth.svelte.js';
import { notifyError } from '#lib/notify.js';

export const THEMES = { system: 'System', light: 'Light', dark: 'Dark' } as const;
/** Colours of the interface, in light and dark (src/routes/layout.css, `data-palette`). */
export const PALETTES = {
	neutral: 'Neutral',
	slate: 'Slate',
	stone: 'Stone',
	ocean: 'Ocean',
	forest: 'Forest',
	rose: 'Rose',
	violet: 'Violet'
} as const;
/** Lines between points: straight, or curves through every point (not smoothing them). */
export const CURVES = { straight: 'Straight', smooth: 'Curved' } as const;
export const CHART_SIZES = {
	small: { label: 'Small', height: 'h-44' },
	medium: { label: 'Medium', height: 'h-56' },
	large: { label: 'Large', height: 'h-80' }
} as const;
/** Charts per row on a wide screen; fewer on narrow ones, where they wouldn't fit. */
export const CHART_COLUMNS = {
	auto: { label: 'Auto', grid: 'md:grid-cols-2 xl:grid-cols-3' },
	'1': { label: '1', grid: '' },
	'2': { label: '2', grid: 'md:grid-cols-2' },
	'3': { label: '3', grid: 'md:grid-cols-2 lg:grid-cols-3' },
	'4': { label: '4', grid: 'md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4' }
} as const;

export type Appearance = {
	theme: keyof typeof THEMES;
	palette: keyof typeof PALETTES;
	curve: keyof typeof CURVES;
	chartSize: keyof typeof CHART_SIZES;
	columns: keyof typeof CHART_COLUMNS;
	/** Points drawn as dots when a line has few of them in view. */
	dots: boolean;
};

export const DEFAULTS: Appearance = {
	theme: 'system',
	palette: 'neutral',
	curve: 'straight',
	chartSize: 'medium',
	columns: 'auto',
	dots: false
};

/** Lines with fewer points than this in view show them as dots (when `dots` is on). */
export const DOTS_BELOW = 100;

const STORAGE_KEY = 'losscape:appearance';
/** The palette last in use, for src/app.html to apply before the page draws. */
export const PALETTE_KEY = 'losscape:palette';

/** The valid settings in `raw`; the rest is left out: storage and the server hold anything. */
export function parseAppearance(raw: unknown): Partial<Appearance> {
	if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return {};
	const r = raw as Record<string, unknown>;
	const out: Partial<Appearance> = {};
	const pick = <T extends object>(choices: T, v: unknown) =>
		typeof v === 'string' && Object.hasOwn(choices, v) ? (v as keyof T & string) : undefined;
	const theme = pick(THEMES, r.theme);
	if (theme) out.theme = theme;
	const palette = pick(PALETTES, r.palette);
	if (palette) out.palette = palette;
	const curve = pick(CURVES, r.curve);
	if (curve) out.curve = curve;
	const chartSize = pick(CHART_SIZES, r.chartSize);
	if (chartSize) out.chartSize = chartSize;
	const columns = pick(CHART_COLUMNS, r.columns);
	if (columns) out.columns = columns;
	if (typeof r.dots === 'boolean') out.dots = r.dots;
	return out;
}

function loadBrowser(): Partial<Appearance> {
	try {
		return parseAppearance(JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}'));
	} catch {
		// Storage unavailable or unreadable: nothing set here.
		return {};
	}
}

/** `settings` with `key` set to `value`, or without it for `undefined`. */
function withSetting<K extends keyof Appearance>(
	settings: Partial<Appearance>,
	key: K,
	value: Appearance[K] | undefined
): Partial<Appearance> {
	const out = { ...settings };
	if (value === undefined) delete out[key];
	else out[key] = value;
	return out;
}

class AppearanceStore {
	#browser = $state<Partial<Appearance>>(loadBrowser());
	/** Account saves, one after the other: each sends everything, so the last one wins. */
	#saving: Promise<unknown> = Promise.resolve();

	/** What the account says (signed out: nothing). */
	get account(): Partial<Appearance> {
		return parseAppearance(auth.user?.preferences?.appearance);
	}

	/** What this browser says, over the account. */
	get browser(): Partial<Appearance> {
		return this.#browser;
	}

	/** What's in use here. */
	get current(): Appearance {
		return { ...DEFAULTS, ...this.account, ...this.#browser };
	}

	/** Sets this browser's value; `undefined`: as the account says. */
	setBrowser<K extends keyof Appearance>(key: K, value: Appearance[K] | undefined) {
		this.#browser = withSetting(this.#browser, key, value);
		try {
			localStorage.setItem(STORAGE_KEY, JSON.stringify(this.#browser));
		} catch {
			// Not kept across visits.
		}
	}

	/** Sets the account's value (shown at once, then saved); `undefined`: the default. */
	setAccount<K extends keyof Appearance>(key: K, value: Appearance[K] | undefined) {
		const user = auth.user;
		if (!user) return;
		const before = user.preferences ?? {};
		const preferences = {
			...before,
			appearance: withSetting(this.account, key, value)
		};
		user.preferences = preferences;
		this.#saving = this.#saving.then(() =>
			apiSend('PUT', '/me/preferences', preferences).catch((err) => {
				if (auth.user === user && user.preferences === preferences) user.preferences = before;
				notifyError(err, "Couldn't save your appearance settings");
			})
		);
	}
}

export const appearance = new AppearanceStore();
