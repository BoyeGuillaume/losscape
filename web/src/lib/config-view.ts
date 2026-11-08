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

/** A key's path as searched: keys joined with dots (`data.key`, `blocks.0.kind`). */
export const dotted = (path: string[]) => path.join('.');

/**
 * The parts of `value` that match `query` (case-insensitive): a key whose dotted path
 * matches (`lr`, or `data.key` for keys under `data` starting with `key`) keeps everything
 * under it; otherwise only leaves whose value matches remain, with the keys leading to
 * them. Lists keep their indices (items left out are holes; see `compact`). Undefined
 * when nothing matches.
 */
export function filterConfig(value: Json, query: string): Json | undefined {
	const q = query.trim().toLowerCase();
	if (!q) return value;
	const visit = (v: Json, path: string[]): Json | undefined => {
		const child = (key: string, c: Json) => {
			const at = [...path, key];
			return dotted(at).toLowerCase().includes(q) ? c : visit(c, at);
		};
		if (isObject(v)) {
			const out: Record<string, Json> = {};
			for (const [key, c] of Object.entries(v)) {
				const kept = child(key, c);
				if (kept === undefined) continue;
				// Not plain assignment: for `__proto__` that would set the prototype.
				Object.defineProperty(out, key, {
					value: kept,
					enumerable: true,
					writable: true,
					configurable: true
				});
			}
			return Object.keys(out).length ? out : undefined;
		}
		if (isNode(v)) {
			const items: Json[] = [];
			v.forEach((c, i) => {
				const kept = child(String(i), c);
				if (kept !== undefined) items[i] = kept;
			});
			return items.length ? items : undefined;
		}
		return text(v).toLowerCase().includes(q) ? v : undefined;
	};
	return visit(value, []);
}

/** `value` with the holes of filtered lists removed (for YAML). */
export function compact(value: Json): Json {
	if (Array.isArray(value)) return value.filter(() => true).map(compact);
	if (isObject(value))
		return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, compact(v)]));
	return value;
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

/**
 * Splits the last key of `path` around the parts of it that `query` matches in the dotted
 * path: matches inside the path, and a match that starts here and goes on into a child
 * (`data` for `data.key`).
 */
export function splitKeyMatches(path: string[], query: string): { text: string; match: boolean }[] {
	const key = path[path.length - 1];
	const q = query.trim().toLowerCase();
	if (!q || !q.includes('.')) return splitMatches(key, query);
	const full = dotted(path).toLowerCase();
	const start = full.length - key.length;
	const marked = new Array<boolean>(key.length).fill(false);
	const mark = (from: number, to: number) => {
		for (let i = Math.max(from, start); i < Math.min(to, full.length); i++)
			marked[i - start] = true;
	};
	for (let i = full.indexOf(q); i >= 0; i = full.indexOf(q, i + 1)) mark(i, i + q.length);
	for (let i = 0; i < full.length; i++) {
		if (q.startsWith(`${full.slice(i)}.`)) mark(i, full.length);
	}
	const out: { text: string; match: boolean }[] = [];
	for (let i = 0; i < key.length; i++) {
		const last = out[out.length - 1];
		if (last && last.match === marked[i]) last.text += key[i];
		else out.push({ text: key[i], match: marked[i] });
	}
	return out;
}
