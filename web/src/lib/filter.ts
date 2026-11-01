// Run filter syntax, e.g. `config.lr < 1e-3 and tag:baseline and not state:crashed`.
//
//   term      field op value | word | "quoted words"   (a bare word matches name, group,
//                                                       tags and aliases)
//   op        :  contains (strings), has (tags, aliases), equals (numbers)
//             =  !=  <  <=  >  >=  ~ (regular expression)
//   field     name, state, group, tag, alias, id, steps, config.<key>, summary.<key>;
//             any other name is looked up in config, then summary
//   combine   and (or juxtaposition), or, not, parentheses
//
// Values are numbers, true/false/null, bare words or "quoted strings". Comparing a missing
// value is false (`!=` is true).

import type { RunSummary } from '#lib/api/types.js';
import { flatten } from '#lib/format.js';

export type Op = ':' | '=' | '!=' | '<' | '<=' | '>' | '>=' | '~';
type Value = string | number | boolean | null;

export type Node =
	| { kind: 'and' | 'or'; left: Node; right: Node }
	| { kind: 'not'; node: Node }
	| { kind: 'search'; text: string }
	| { kind: 'compare'; field: string; op: Op; value: Value; regex?: RegExp };

export class FilterError extends Error {
	readonly at: number;
	constructor(message: string, at: number) {
		super(message);
		this.at = at;
	}
}

type Token =
	| { type: 'word' | 'string'; text: string; at: number }
	| { type: 'op'; text: Op; at: number }
	| { type: '('; at: number }
	| { type: ')'; at: number };

const OPS: Op[] = ['<=', '>=', '!=', ':', '=', '<', '>', '~'];
const SPECIAL = /[\s()"<>=!:~]/;

function tokenize(input: string): Token[] {
	const tokens: Token[] = [];
	let i = 0;
	while (i < input.length) {
		const c = input[i];
		if (/\s/.test(c)) {
			i++;
		} else if (c === '(' || c === ')') {
			tokens.push({ type: c, at: i++ });
		} else if (c === '"') {
			const start = i++;
			let text = '';
			while (i < input.length && input[i] !== '"') {
				// \" and \\ are escapes; other backslashes stay (regular expressions need them).
				if (input[i] === '\\' && (input[i + 1] === '"' || input[i + 1] === '\\')) i++;
				text += input[i++];
			}
			if (i >= input.length) throw new FilterError('unclosed quote', start);
			i++;
			tokens.push({ type: 'string', text, at: start });
		} else {
			const op = OPS.find((o) => input.startsWith(o, i));
			if (op) {
				tokens.push({ type: 'op', text: op, at: i });
				i += op.length;
				continue;
			}
			const start = i;
			while (i < input.length && !SPECIAL.test(input[i])) i++;
			if (i === start) throw new FilterError(`unexpected "${c}"`, i);
			tokens.push({ type: 'word', text: input.slice(start, i), at: start });
		}
	}
	return tokens;
}

const keyword = (t: Token | undefined, word: string) =>
	t?.type === 'word' && t.text.toLowerCase() === word;

function literal(token: Token): Value {
	if (token.type === 'string') return token.text;
	if (token.type !== 'word') throw new FilterError('expected a value', token.at);
	const lower = token.text.toLowerCase();
	if (lower === 'true') return true;
	if (lower === 'false') return false;
	if (lower === 'null') return null;
	const n = Number(token.text);
	return token.text.trim() !== '' && Number.isFinite(n) ? n : token.text;
}

/** Parses a filter; an empty one gives null (matches everything). */
export function parseFilter(input: string): Node | null {
	const tokens = tokenize(input);
	let pos = 0;
	const peek = () => tokens[pos];
	const end = () => input.length;

	function or(): Node {
		let left = and();
		while (keyword(peek(), 'or')) {
			pos++;
			left = { kind: 'or', left, right: and() };
		}
		return left;
	}

	function and(): Node {
		let left = unary();
		for (;;) {
			const t = peek();
			if (!t || t.type === ')' || keyword(t, 'or')) return left;
			if (keyword(t, 'and')) pos++;
			left = { kind: 'and', left, right: unary() };
		}
	}

	function unary(): Node {
		if (keyword(peek(), 'not')) {
			pos++;
			return { kind: 'not', node: unary() };
		}
		return primary();
	}

	function primary(): Node {
		const t = peek();
		if (!t) throw new FilterError('expected a condition', end());
		if (t.type === '(') {
			pos++;
			const node = or();
			if (peek()?.type !== ')') throw new FilterError('missing ")"', peek()?.at ?? end());
			pos++;
			return node;
		}
		if (t.type === ')' || t.type === 'op') throw new FilterError('expected a condition', t.at);
		pos++;
		const op = peek();
		if (t.type === 'word' && op?.type === 'op') {
			pos++;
			const v = peek();
			if (!v || (v.type !== 'word' && v.type !== 'string')) {
				throw new FilterError(`expected a value after "${op.text}"`, v?.at ?? end());
			}
			pos++;
			const node: Node = { kind: 'compare', field: t.text, op: op.text, value: literal(v) };
			if (op.text === '~') {
				try {
					node.regex = new RegExp(String(node.value), 'i');
				} catch {
					throw new FilterError('invalid regular expression', v.at);
				}
			}
			return node;
		}
		return { kind: 'search', text: t.text.toLowerCase() };
	}

	if (tokens.length === 0) return null;
	const node = or();
	if (pos < tokens.length) throw new FilterError('unexpected input', tokens[pos].at);
	return node;
}

/** Flattened config and summary of a run (cached per run object). */
const flatCache = new WeakMap<
	RunSummary,
	{ config: Map<string, unknown>; summary: Map<string, unknown> }
>();

export function flatRun(run: RunSummary) {
	let flat = flatCache.get(run);
	if (!flat) {
		flat = { config: new Map(flatten(run.config)), summary: new Map(flatten(run.summary)) };
		flatCache.set(run, flat);
	}
	return flat;
}

/** A field's value for a run: a scalar, a list (tags, aliases), or undefined if missing. */
export function fieldValue(run: RunSummary, field: string): unknown {
	const lower = field.toLowerCase();
	switch (lower) {
		case 'name':
			return run.name;
		case 'state':
			return run.state;
		case 'group':
			return run.group_name ?? undefined;
		case 'tag':
		case 'tags':
			return run.tags;
		case 'alias':
		case 'aliases':
			return run.aliases;
		case 'id':
			return run.id.replaceAll('-', '');
		case 'steps':
			return run.last_step === null ? undefined : run.last_step + 1;
	}
	const flat = flatRun(run);
	if (lower.startsWith('config.')) return flat.config.get(field.slice(7));
	if (lower.startsWith('summary.')) return flat.summary.get(field.slice(8));
	return flat.config.has(field) ? flat.config.get(field) : flat.summary.get(field);
}

function compareOne(actual: unknown, op: Op, expected: Value, regex?: RegExp): boolean {
	if (op === '~') return actual != null && regex!.test(String(actual));
	if (typeof actual === 'number' && typeof expected === 'number') {
		switch (op) {
			case ':':
			case '=':
				return actual === expected;
			case '!=':
				return actual !== expected;
			case '<':
				return actual < expected;
			case '<=':
				return actual <= expected;
			case '>':
				return actual > expected;
			case '>=':
				return actual >= expected;
		}
	}
	if (expected === null) {
		if (op === '=' || op === ':') return actual == null;
		if (op === '!=') return actual != null;
		return false;
	}
	if (actual == null) return op === '!=';
	const a = String(actual).toLowerCase();
	const e = String(expected).toLowerCase();
	switch (op) {
		case ':':
			return a.includes(e);
		case '=':
			return a === e;
		case '!=':
			return a !== e;
		default:
			// Ordering is numeric only.
			return false;
	}
}

export function matches(node: Node | null, run: RunSummary): boolean {
	if (!node) return true;
	switch (node.kind) {
		case 'and':
			return matches(node.left, run) && matches(node.right, run);
		case 'or':
			return matches(node.left, run) || matches(node.right, run);
		case 'not':
			return !matches(node.node, run);
		case 'search': {
			const q = node.text;
			return (
				run.name.toLowerCase().includes(q) ||
				(run.group_name?.toLowerCase().includes(q) ?? false) ||
				run.tags.some((t) => t.toLowerCase().includes(q)) ||
				run.aliases.some((a) => a.toLowerCase().includes(q))
			);
		}
		case 'compare': {
			const actual = fieldValue(run, node.field);
			if (Array.isArray(actual)) {
				// Lists: `tag:x` / `tag=x` means one of them is x; `!=` means none is.
				if (node.op === '!=') return !actual.some((v) => compareOne(v, '=', node.value));
				const op = node.op === ':' ? '=' : node.op;
				return actual.some((v) => compareOne(v, op, node.value, node.regex));
			}
			return compareOne(actual, node.op, node.value, node.regex);
		}
	}
}
