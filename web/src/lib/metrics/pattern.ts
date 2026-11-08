// Metric name patterns: `*` matches anything, `{name}` matches one path segment and is
// captured (`val/pck-{k}` matches `val/pck-16` with k = "16").

export type Pattern = { source: string; regex: RegExp; params: string[]; literal: number };

const cache = new Map<string, Pattern>();

export const isPattern = (name: string) => /[*{]/.test(name);

/**
 * Matching takes time that grows with the power of the number of `*` and `{name}`, on names
 * anyone logging to the project can choose. Patterns over this limit, or with two of them
 * side by side, match nothing. The server refuses them too (metric_defs.rs).
 */
const MAX_WILDCARDS = 3;
const NOTHING = /(?!)/;

/** Whether a pattern is within those limits. */
export const isAccepted = (source: string) => compile(source).regex !== NOTHING;

export function compile(source: string): Pattern {
	let pattern = cache.get(source);
	if (pattern) return pattern;
	const params: string[] = [];
	let literal = 0;
	let wildcards = 0;
	let adjacent = false;
	let afterWildcard = false;
	const body = source.replace(/\{([A-Za-z_]\w*)\}|\*|[^{*]+|\{/g, (part, param) => {
		if (param || part === '*') {
			wildcards++;
			adjacent ||= afterWildcard;
			afterWildcard = true;
			if (part === '*') return '.*';
			params.push(param);
			return '([^/]+?)';
		}
		afterWildcard = false;
		literal += part.length;
		return part.replace(/[.+?^$()|[\]\\{}]/g, '\\$&');
	});
	const safe = wildcards <= MAX_WILDCARDS && !adjacent;
	pattern = { source, regex: safe ? new RegExp(`^${body}$`) : NOTHING, params, literal };
	cache.set(source, pattern);
	return pattern;
}

/** Captured parameters if `key` matches, else null. */
export function match(source: string, key: string): Record<string, string> | null {
	const p = compile(source);
	const m = p.regex.exec(key);
	if (!m) return null;
	return Object.fromEntries(p.params.map((name, i) => [name, m[i + 1]]));
}
