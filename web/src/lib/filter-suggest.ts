// Suggestions while typing a filter: fields (name, state:, config.<key>, ...) for the word
// at the caret, and values when that word is a comparison (`config.lr = `, `tag:`).

import type { Catalog, ValueCount } from '#lib/runs/catalog.js';
import { valueLabel } from '#lib/runs/catalog.js';

export type Suggestion = {
	/** Text inserted in place of the word at the caret. */
	insert: string;
	label: string;
	/** Second line: runs having it, or what the field is. */
	detail?: string;
};

export type Suggestions = { from: number; to: number; items: Suggestion[] };

const MAX = 8;
const OPS = ['<=', '>=', '!=', ':', '=', '<', '>', '~'];
const KEYWORDS = new Set(['and', 'or', 'not', 'true', 'false', 'null']);

/** A value as filter text: bare when it reads back the same, quoted otherwise. */
export function literal(value: unknown): string {
	if (typeof value === 'number' || typeof value === 'boolean' || value === null) {
		return String(value);
	}
	const text = String(value);
	const bare =
		/^[A-Za-z0-9._/-]+$/.test(text) &&
		!KEYWORDS.has(text.toLowerCase()) &&
		!Number.isFinite(Number(text));
	return bare ? text : `"${text.replaceAll('\\', '\\\\').replaceAll('"', '\\"')}"`;
}

/** Start of the word ending at `caret` (words end at spaces and parentheses). */
function wordStart(text: string, caret: number): number {
	let i = caret;
	while (i > 0 && !/[\s()]/.test(text[i - 1])) i--;
	return i;
}

function splitOp(word: string): [string, string, string] | null {
	for (let i = 1; i < word.length; i++) {
		const op = OPS.find((o) => word.startsWith(o, i));
		if (op) return [word.slice(0, i), op, word.slice(i + op.length)];
	}
	return null;
}

function rank<T>(items: T[], text: (t: T) => string, typed: string): T[] {
	const q = typed.toLowerCase();
	const starts = items.filter((t) => text(t).toLowerCase().startsWith(q));
	const contains = items.filter(
		(t) => !text(t).toLowerCase().startsWith(q) && text(t).toLowerCase().includes(q)
	);
	return [...starts, ...contains];
}

function valuesOf(field: string, catalog: Catalog): ValueCount[] | null {
	const f = field.toLowerCase();
	if (f === 'state') return catalog.states;
	if (f === 'tag' || f === 'tags') return catalog.tags;
	if (f === 'alias' || f === 'aliases') return catalog.aliases;
	if (f === 'group') return catalog.groups;
	const key = f.startsWith('config.') ? field.slice('config.'.length) : field;
	return catalog.config.find((s) => s.key === key)?.values ?? null;
}

function fieldSuggestions(typed: string, catalog: Catalog): Suggestion[] {
	const fields: Suggestion[] = [
		{ insert: 'name', label: 'name', detail: 'run name' },
		{ insert: 'state:', label: 'state:', detail: 'running, finished, crashed…' },
		{ insert: 'tag:', label: 'tag:', detail: 'one of the run’s tags' },
		{ insert: 'alias:', label: 'alias:', detail: 'e.g. @best' },
		{ insert: 'group:', label: 'group:', detail: 'run group' },
		{ insert: 'steps', label: 'steps', detail: 'steps logged' },
		...catalog.config.map((s) => ({
			insert: `config.${s.key}`,
			label: `config.${s.key}`,
			detail: `${s.runs} runs · ${s.distinct} ${s.distinct === 1 ? 'value' : 'values'}`
		})),
		...catalog.summaryKeys.map((k) => ({ insert: `summary.${k}`, label: `summary.${k}` }))
	];
	return rank(fields, (f) => f.label, typed).filter((f) => f.insert !== typed);
}

export function suggest(text: string, caret: number, catalog: Catalog): Suggestions {
	const from = wordStart(text, caret);
	const word = text.slice(from, caret);
	const none = { from, to: caret, items: [] };

	// `field op value` in one word, or `field op ` with spaces around the operator.
	let field: string | null = null;
	let typed = word;
	let valueFrom = from;
	const inline = splitOp(word);
	if (inline) {
		[field, , typed] = inline;
		valueFrom = from + inline[0].length + inline[1].length;
	} else {
		const before = text
			.slice(0, from)
			.trimEnd()
			.split(/[\s()]+/);
		const [prevField, prevOp] = before.slice(-2);
		if (prevOp && OPS.includes(prevOp) && prevField) field = prevField;
	}

	if (field) {
		const values = valuesOf(field, catalog);
		if (!values) return none;
		const items = rank(values, (v) => valueLabel(v.value), typed.replace(/^"/, ''))
			.slice(0, MAX)
			.map((v) => ({
				insert: `${literal(v.value)} `,
				label: valueLabel(v.value),
				detail: `${v.count} ${v.count === 1 ? 'run' : 'runs'}`
			}));
		return { from: valueFrom, to: caret, items };
	}
	if (!word || KEYWORDS.has(word.toLowerCase())) return none;
	return { from, to: caret, items: fieldSuggestions(word, catalog).slice(0, MAX) };
}
