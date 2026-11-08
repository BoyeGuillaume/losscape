// Formulas for virtual metrics: numbers, metrics in brackets ([val/p]), + - * / ^,
// parentheses, and min, max, abs, sqrt, log, log10, exp. Parsed into a tree and
// evaluated directly: no `eval`, nothing else is reachable.

export type Formula =
	| { kind: 'num'; value: number }
	| { kind: 'ref'; key: string }
	| { kind: 'neg'; arg: Formula }
	| { kind: 'op'; op: '+' | '-' | '*' | '/' | '^'; left: Formula; right: Formula }
	| { kind: 'call'; fn: string; args: Formula[] };

const FUNCTIONS: Record<string, (...args: number[]) => number> = {
	min: Math.min,
	max: Math.max,
	abs: Math.abs,
	sqrt: Math.sqrt,
	log: Math.log,
	log10: Math.log10,
	exp: Math.exp
};

export const FUNCTION_NAMES = Object.keys(FUNCTIONS);

export class FormulaError extends Error {}

type Token =
	{ t: 'num'; v: number } | { t: 'ref'; v: string } | { t: 'id'; v: string } | { t: string };

function tokenize(src: string): Token[] {
	const out: Token[] = [];
	let i = 0;
	while (i < src.length) {
		const c = src[i];
		if (/\s/.test(c)) i++;
		else if (c === '[') {
			const end = src.indexOf(']', i);
			if (end < 0) throw new FormulaError('missing "]" after a metric name');
			const key = src.slice(i + 1, end).trim();
			if (!key) throw new FormulaError('empty metric name "[]"');
			out.push({ t: 'ref', v: key });
			i = end + 1;
		} else if (/[\d.]/.test(c)) {
			const m = /^(\d+\.?\d*|\.\d+)(e[+-]?\d+)?/i.exec(src.slice(i));
			if (!m) throw new FormulaError(`unexpected "${c}"`);
			out.push({ t: 'num', v: Number(m[0]) });
			i += m[0].length;
		} else if (/[A-Za-z_]/.test(c)) {
			const m = /^[A-Za-z_]\w*/.exec(src.slice(i))!;
			out.push({ t: 'id', v: m[0] });
			i += m[0].length;
		} else if ('+-*/^(),'.includes(c)) {
			out.push({ t: c });
			i++;
		} else throw new FormulaError(`unexpected "${c}"`);
	}
	return out;
}

export function parseFormula(src: string): Formula {
	const tokens = tokenize(src);
	let pos = 0;
	const peek = () => tokens[pos]?.t;
	const expect = (t: string) => {
		if (peek() !== t) throw new FormulaError(`expected "${t}"`);
		pos++;
	};
	// sum := term (('+'|'-') term)*; term := unary (('*'|'/') unary)*;
	// unary := '-' unary | power; power := atom ('^' unary)?  (so -2^2 is -4, 2^-1 is 0.5)
	function sum(): Formula {
		let left = term();
		while (peek() === '+' || peek() === '-') {
			const op = tokens[pos++].t as '+' | '-';
			left = { kind: 'op', op, left, right: term() };
		}
		return left;
	}
	function term(): Formula {
		let left = unary();
		while (peek() === '*' || peek() === '/') {
			const op = tokens[pos++].t as '*' | '/';
			left = { kind: 'op', op, left, right: unary() };
		}
		return left;
	}
	function unary(): Formula {
		if (peek() === '-') {
			pos++;
			return { kind: 'neg', arg: unary() };
		}
		return power();
	}
	function power(): Formula {
		const base = atom();
		if (peek() === '^') {
			pos++;
			return { kind: 'op', op: '^', left: base, right: unary() };
		}
		return base;
	}
	function atom(): Formula {
		const tok = tokens[pos++];
		if (!tok) throw new FormulaError('the formula ends too early');
		if (tok.t === 'num') return { kind: 'num', value: (tok as { v: number }).v };
		if (tok.t === 'ref') return { kind: 'ref', key: (tok as { v: string }).v };
		if (tok.t === '(') {
			const inner = sum();
			expect(')');
			return inner;
		}
		if (tok.t === 'id') {
			const fn = (tok as { v: string }).v;
			if (!FUNCTIONS[fn])
				throw new FormulaError(`unknown function "${fn}" (metrics go in brackets: [name])`);
			expect('(');
			const args = [sum()];
			while (peek() === ',') {
				pos++;
				args.push(sum());
			}
			expect(')');
			return { kind: 'call', fn, args };
		}
		throw new FormulaError(`unexpected "${tok.t}"`);
	}
	if (tokens.length === 0) throw new FormulaError('empty formula');
	const tree = sum();
	if (pos < tokens.length) throw new FormulaError(`unexpected "${tokens[pos].t}"`);
	return tree;
}

/** Metric names a formula reads. */
export function references(f: Formula, out = new Set<string>()): Set<string> {
	if (f.kind === 'ref') out.add(f.key);
	else if (f.kind === 'neg') references(f.arg, out);
	else if (f.kind === 'op') {
		references(f.left, out);
		references(f.right, out);
	} else if (f.kind === 'call') f.args.forEach((a) => references(a, out));
	return out;
}

/** The value, or null if a metric is missing or the result isn't a finite number. */
export function evaluate(
	f: Formula,
	value: (key: string) => number | null | undefined
): number | null {
	const go = (n: Formula): number => {
		switch (n.kind) {
			case 'num':
				return n.value;
			case 'ref':
				return value(n.key) ?? NaN;
			case 'neg':
				return -go(n.arg);
			case 'call':
				return FUNCTIONS[n.fn](...n.args.map(go));
			case 'op': {
				const [a, b] = [go(n.left), go(n.right)];
				return n.op === '+'
					? a + b
					: n.op === '-'
						? a - b
						: n.op === '*'
							? a * b
							: n.op === '/'
								? a / b
								: a ** b;
			}
		}
	};
	const result = go(f);
	return Number.isFinite(result) ? result : null;
}
