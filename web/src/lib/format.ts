/** The browser's locale, or English if it's one Intl rejects (e.g. "en-US@posix"). */
function withLocale<T>(create: (locale: string | undefined) => T): T {
	try {
		return create(undefined);
	} catch {
		return create('en');
	}
}

const compact = withLocale(
	(l) => new Intl.NumberFormat(l, { notation: 'compact', maximumFractionDigits: 2 })
);

/** Axis labels: 1.5K, 2M, 0.001, 1.2e-7. */
export function formatCompact(value: number | null | undefined): string {
	if (value == null || !Number.isFinite(value)) return '';
	const abs = Math.abs(value);
	if (abs !== 0 && (abs < 1e-3 || abs >= 1e15)) return value.toExponential(1);
	if (abs < 1000) return String(Number(value.toPrecision(4)));
	return compact.format(value);
}

/** Metric values: 4 significant digits. */
export function formatValue(value: unknown): string {
	if (value === null || value === undefined) return '–';
	if (typeof value === 'number') {
		if (!Number.isFinite(value)) return String(value);
		const abs = Math.abs(value);
		if (abs !== 0 && (abs < 1e-4 || abs >= 1e9)) return value.toExponential(3);
		return String(Number(value.toPrecision(4)));
	}
	if (typeof value === 'string') return value;
	return JSON.stringify(value);
}

const relative = withLocale((l) => new Intl.RelativeTimeFormat(l, { numeric: 'auto' }));

export function timeAgo(ms: number | null | undefined): string {
	if (!ms) return '–';
	const seconds = (ms - Date.now()) / 1000;
	const units: [Intl.RelativeTimeFormatUnit, number][] = [
		['day', 86400],
		['hour', 3600],
		['minute', 60],
		['second', 1]
	];
	for (const [unit, size] of units) {
		if (Math.abs(seconds) >= size || unit === 'second')
			return relative.format(Math.round(seconds / size), unit);
	}
	return '';
}

export function duration(startMs: number, endMs: number | null | undefined): string {
	const total = Math.max(0, Math.round(((endMs ?? Date.now()) - startMs) / 1000));
	const h = Math.floor(total / 3600);
	const m = Math.floor((total % 3600) / 60);
	const s = total % 60;
	if (h) return `${h}h ${m}m`;
	if (m) return `${m}m ${s}s`;
	return `${s}s`;
}

const dateTimeFormat = withLocale(
	(l) => new Intl.DateTimeFormat(l, { dateStyle: 'medium', timeStyle: 'short' })
);

export function dateTime(ms: number): string {
	return dateTimeFormat.format(ms);
}

/** Nested objects as dotted keys, for config and summary tables. */
export function flatten(value: Record<string, unknown>, prefix = ''): [string, unknown][] {
	return Object.entries(value).flatMap(([key, v]) =>
		v && typeof v === 'object' && !Array.isArray(v)
			? flatten(v as Record<string, unknown>, `${prefix}${key}.`)
			: [[`${prefix}${key}`, v] as [string, unknown]]
	);
}
