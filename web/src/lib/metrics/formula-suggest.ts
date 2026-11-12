// Suggestions while typing a formula, for what's at the caret and not the whole text: a
// metric inside brackets, a metric or function for a bare word, or both wherever a value
// can start (after an operator, say).

import type { Suggestion, Suggestions } from '#lib/filter-suggest.js';
import { AGGREGATE_NAMES, CONSTANT_NAMES, FUNCTION_NAMES, helpFor } from './formula.js';

const MAX_METRICS = 50;

/** Metrics having `query`: first at the start of the name, then of a part, then anywhere. */
function matchingMetrics(keys: string[], query: string): string[] {
	const q = query.toLowerCase();
	const rank = (key: string) => {
		const k = key.toLowerCase();
		if (k.startsWith(q)) return 0;
		const at = k.indexOf(q);
		if (at < 0) return null;
		return /[/._-]/.test(k[at - 1]) ? 1 : 2;
	};
	return keys
		.map((key) => ({ key, rank: rank(key) }))
		.filter((m): m is { key: string; rank: number } => m.rank !== null)
		.sort((a, b) => a.rank - b.rank || a.key.localeCompare(b.key))
		.slice(0, MAX_METRICS)
		.map((m) => m.key);
}

const metric = (key: string, insert: string): Suggestion => ({ insert, label: key });

/** Functions, aggregates and constants starting with `query`. */
function functions(query: string): Suggestion[] {
	const q = query.toLowerCase();
	return [
		...FUNCTION_NAMES.filter((name) => name.startsWith(q)).map((name) => ({
			insert: `${name}(`,
			label: `${name}(…)`,
			detail: helpFor(name)
		})),
		// Opened on the bracket: what comes next is the metric's name.
		...AGGREGATE_NAMES.filter((name) => name.startsWith(q)).map((name) => ({
			insert: `${name}([`,
			label: `${name}([metric])`,
			detail: helpFor(name)
		})),
		...CONSTANT_NAMES.filter((name) => name.startsWith(q)).map((name) => ({
			insert: name,
			label: name,
			detail: helpFor(name)
		}))
	];
}

export function formulaSuggestions(text: string, caret: number, keys: string[]): Suggestions {
	const before = text.slice(0, caret);
	const none = { from: caret, to: caret, query: '', items: [] };

	// Inside brackets: the name, up to the closing bracket if there is one.
	const open = before.lastIndexOf('[');
	if (open > before.lastIndexOf(']')) {
		const rest = text.slice(caret);
		const close = rest.search(/[[\]]/);
		const closed = close >= 0 && rest[close] === ']';
		const to = closed ? caret + close + 1 : caret + (/^[^\s[\]]*/.exec(rest)?.[0].length ?? 0);
		const query = before.slice(open + 1).trimStart();
		return {
			from: open + 1,
			to,
			query,
			items: matchingMetrics(keys, query).map((key) => metric(key, `${key}]`))
		};
	}

	// A bare word. Only functions are bare, so `val/acc` can be taken as one name.
	const word = /[A-Za-z_][\w./-]*$/.exec(before);
	if (word) {
		if (/[\d.]/.test(before[word.index - 1] ?? '')) return none; // the e of 1e3
		const query = word[0];
		return {
			from: word.index,
			to: caret + (/^[\w./-]*/.exec(text.slice(caret))?.[0].length ?? 0),
			query,
			items: [
				...functions(query),
				...matchingMetrics(keys, query).map((key) => metric(key, `[${key}]`))
			]
		};
	}

	// Nothing typed yet: only where a value can start, not after one.
	if (!/(^|[-+*/^(,])\s*$/.test(before)) return none;
	return {
		...none,
		items: [...matchingMetrics(keys, '').map((key) => metric(key, `[${key}]`)), ...functions('')]
	};
}
