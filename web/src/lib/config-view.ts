// A run's config, pruned to what a search matches, for the run page's config tab.

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

const isObject = (v: unknown): v is Record<string, Json> =>
	v !== null && typeof v === 'object' && !Array.isArray(v);

/** Containers (objects, and lists holding objects) open as nodes; the rest are leaves. */
export function isNode(v: unknown): v is Record<string, Json> | Json[] {
	return (
		isObject(v) || (Array.isArray(v) && v.some((item) => item !== null && typeof item === 'object'))
	);
}

const text = (v: unknown) => (typeof v === 'string' ? v : JSON.stringify(v));

/**
 * The parts of `value` that match `query` (case-insensitive): a key that matches keeps
 * everything under it; otherwise only leaves whose value matches remain, with the keys
 * leading to them. Undefined when nothing matches.
 */
export function filterConfig(value: Json, query: string): Json | undefined {
	const q = query.trim().toLowerCase();
	if (!q) return value;
	const visit = (v: Json): Json | undefined => {
		if (isObject(v)) {
			const out: Record<string, Json> = {};
			for (const [key, child] of Object.entries(v)) {
				const kept = key.toLowerCase().includes(q) ? child : visit(child);
				if (kept !== undefined) out[key] = kept;
			}
			return Object.keys(out).length ? out : undefined;
		}
		if (isNode(v)) {
			const items = v.map(visit).filter((item): item is Json => item !== undefined);
			return items.length ? items : undefined;
		}
		return text(v).toLowerCase().includes(q) ? v : undefined;
	};
	return visit(value);
}

/** Leaves (values that aren't containers) in a config. */
export function countLeaves(value: Json): number {
	if (isObject(value)) return Object.values(value).reduce<number>((n, v) => n + countLeaves(v), 0);
	if (isNode(value)) return (value as Json[]).reduce<number>((n, v) => n + countLeaves(v), 0);
	return 1;
}

/** Splits `s` around case-insensitive matches of `query`, to highlight them. */
export function splitMatches(s: string, query: string): { text: string; match: boolean }[] {
	const q = query.trim().toLowerCase();
	if (!q) return [{ text: s, match: false }];
	const out: { text: string; match: boolean }[] = [];
	const lower = s.toLowerCase();
	let at = 0;
	for (let i = lower.indexOf(q); i >= 0; i = lower.indexOf(q, at)) {
		if (i > at) out.push({ text: s.slice(at, i), match: false });
		out.push({ text: s.slice(i, i + q.length), match: true });
		at = i + q.length;
	}
	if (at < s.length) out.push({ text: s.slice(at), match: false });
	return out;
}
