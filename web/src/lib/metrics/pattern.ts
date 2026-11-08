// Metric name patterns: `*` matches anything, `{name}` matches one path segment and is
// captured (`val/pck-{k}` matches `val/pck-16` with k = "16").

export type Pattern = { source: string; regex: RegExp; params: string[]; literal: number };

const cache = new Map<string, Pattern>();

export const isPattern = (name: string) => /[*{]/.test(name);

export function compile(source: string): Pattern {
	let pattern = cache.get(source);
	if (pattern) return pattern;
	const params: string[] = [];
	let literal = 0;
	const body = source.replace(/\{([A-Za-z_]\w*)\}|\*|[^{*]+/g, (part, param) => {
		if (param) {
			params.push(param);
			return '([^/]+?)';
		}
		if (part === '*') return '.*';
		literal += part.length;
		return part.replace(/[.+?^$()|[\]\\]/g, '\\$&');
	});
	pattern = { source, regex: new RegExp(`^${body}$`), params, literal };
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
