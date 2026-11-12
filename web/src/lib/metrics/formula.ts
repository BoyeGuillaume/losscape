// Formulas for virtual metrics: numbers, metrics in brackets ([val/p]), + - * / ^,
// parentheses, the functions and constants below. Parsed into a tree and
// evaluated directly: no `eval`, nothing else is reachable.

export type Formula =
	| { kind: 'num'; value: number }
	| { kind: 'ref'; key: string }
	| { kind: 'neg'; arg: Formula }
	| { kind: 'op'; op: '+' | '-' | '*' | '/' | '^'; left: Formula; right: Formula }
	| { kind: 'call'; fn: string; args: Formula[] };

type Fn = {
	fn: (...args: number[]) => number;
	/** How many values it takes: exactly `args`, or at least 1 (`'any'`). */
	args: number | 'any';
	/** What it gives, for suggestions; with its arguments when there are several. */
	help: string;
};

const FUNCTIONS: Record<string, Fn> = {
	min: { fn: Math.min, args: 'any', help: 'smallest of its values' },
	max: { fn: Math.max, args: 'any', help: 'largest of its values' },
	abs: { fn: Math.abs, args: 1, help: 'absolute value' },
	sign: { fn: Math.sign, args: 1, help: '-1, 0 or 1' },
	clamp: {
		fn: (x, lo, hi) => Math.min(Math.max(x, lo), hi),
		args: 3,
		help: 'clamp(x, lo, hi): x kept between lo and hi'
	},
	round: { fn: Math.round, args: 1, help: 'nearest integer' },
	floor: { fn: Math.floor, args: 1, help: 'integer below' },
	ceil: { fn: Math.ceil, args: 1, help: 'integer above' },
	trunc: { fn: Math.trunc, args: 1, help: 'integer part' },
	pow: { fn: Math.pow, args: 2, help: 'pow(x, y): x to the power y, as x ^ y' },
	sqrt: { fn: Math.sqrt, args: 1, help: 'square root' },
	cbrt: { fn: Math.cbrt, args: 1, help: 'cube root' },
	hypot: { fn: Math.hypot, args: 'any', help: 'square root of the sum of squares' },
	exp: { fn: Math.exp, args: 1, help: 'e to the power' },
	expm1: { fn: Math.expm1, args: 1, help: 'exp(x) - 1, precise near 0' },
	log: { fn: Math.log, args: 1, help: 'natural logarithm' },
	log2: { fn: Math.log2, args: 1, help: 'base 2 logarithm' },
	log10: { fn: Math.log10, args: 1, help: 'base 10 logarithm' },
	log1p: { fn: Math.log1p, args: 1, help: 'log(1 + x), precise near 0' },
	sin: { fn: Math.sin, args: 1, help: 'sine (radians)' },
	cos: { fn: Math.cos, args: 1, help: 'cosine (radians)' },
	tan: { fn: Math.tan, args: 1, help: 'tangent (radians)' },
	asin: { fn: Math.asin, args: 1, help: 'arcsine, in radians' },
	acos: { fn: Math.acos, args: 1, help: 'arccosine, in radians' },
	atan: { fn: Math.atan, args: 1, help: 'arctangent, in radians' },
	atan2: { fn: Math.atan2, args: 2, help: 'atan2(y, x): angle of the point (x, y)' },
	sinh: { fn: Math.sinh, args: 1, help: 'hyperbolic sine' },
	cosh: { fn: Math.cosh, args: 1, help: 'hyperbolic cosine' },
	tanh: { fn: Math.tanh, args: 1, help: 'hyperbolic tangent' }
};

/** Named numbers, written bare: `2 * pi`. */
const CONSTANTS: Record<string, { value: number; help: string }> = {
	pi: { value: Math.PI, help: '3.14159…' },
	e: { value: Math.E, help: '2.71828…' }
};

// Own entries only: `constructor` or `__proto__` are names of every object.
const isFunction = (name: string) => Object.hasOwn(FUNCTIONS, name);
const isConstant = (name: string) => Object.hasOwn(CONSTANTS, name);

export const FUNCTION_NAMES = Object.keys(FUNCTIONS);
export const CONSTANT_NAMES = Object.keys(CONSTANTS);

/** What a function or constant gives, for suggestions. */
export const helpFor = (name: string): string | undefined =>
	isFunction(name) ? FUNCTIONS[name].help : isConstant(name) ? CONSTANTS[name].help : undefined;

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
			const name = (tok as { v: string }).v;
			if (peek() !== '(' && isConstant(name)) return { kind: 'num', value: CONSTANTS[name].value };
			if (!isFunction(name))
				throw new FormulaError(`unknown function "${name}" (metrics go in brackets: [name])`);
			expect('(');
			const args = [sum()];
			while (peek() === ',') {
				pos++;
				args.push(sum());
			}
			expect(')');
			const wanted = FUNCTIONS[name].args;
			if (wanted !== 'any' && args.length !== wanted) {
				throw new FormulaError(`${name} takes ${wanted} value${wanted === 1 ? '' : 's'}`);
			}
			return { kind: 'call', fn: name, args };
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
				return isFunction(n.fn) ? FUNCTIONS[n.fn].fn(...n.args.map(go)) : NaN;
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
