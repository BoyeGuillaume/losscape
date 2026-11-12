import { describe, expect, it } from 'vitest';

import { formulaSuggestions } from './formula-suggest.js';
import { parseFormula } from './formula.js';

const KEYS = ['train/loss', 'val/acc', 'val/loss', 'val/p', 'val/r'];

/** Suggestions at `|`, and the text after accepting the first one. */
function at(marked: string, keys = KEYS) {
	const caret = marked.indexOf('|');
	const text = marked.replace('|', '');
	const s = formulaSuggestions(text, caret, keys);
	const accept = (i = 0) => text.slice(0, s.from) + s.items[i].insert + text.slice(s.to);
	return { ...s, labels: s.items.map((i) => i.label), accept };
}

describe('formula suggestions', () => {
	it('offers metrics and functions in an empty formula', () => {
		const { labels } = at('|');
		expect(labels.slice(0, 5)).toEqual(KEYS);
		expect(labels).toContain('sqrt(…)');
	});

	it('offers the math functions and constants by name', () => {
		expect(at('si|').labels).toEqual(['sign(…)', 'sin(…)', 'sinh(…)']);
		expect(at('p|').labels).toEqual(expect.arrayContaining(['pow(…)', 'pi']));
		expect(at('2 * p|').accept(at('2 * p|').labels.indexOf('pi'))).toBe('2 * pi');
		expect(at('cl|').items[0].detail).toMatch(/between lo and hi/);
	});

	it('opens aggregates on the metric to aggregate', () => {
		const typed = at('[loss] / me|');
		expect(typed.labels[0]).toBe('mean([metric])');
		expect(typed.items[0].detail).toMatch(/whole run/);
		const opened = typed.accept();
		expect(opened).toBe('[loss] / mean([');
		expect(at(`${opened}|`).labels, 'then metrics').toEqual(KEYS);
	});

	it('keeps going after the first metric', () => {
		expect(at('[val/p] * |').labels).toContain('val/r');
		expect(at('[val/p] * [|').accept(3)).toBe('[val/p] * [val/p]');
		expect(at('[val/p] * [val/r|').labels).toEqual(['val/r']);
		expect(at('[val/p] * va|').accept(0)).toBe('[val/p] * [val/acc]');
	});

	it('works in the middle of a formula', () => {
		expect(at('2 * [val/r|] / 3').accept()).toBe('2 * [val/r] / 3');
		expect(at('[val/|acc] + 1').accept(1)).toBe('[val/loss] + 1');
		expect(at('[a] + lo| * 2').accept()).toBe('[a] + log( * 2');
		expect(at('min([val/p], |)').labels).toContain('val/r');
		expect(at('max(|').labels).toContain('val/acc');
	});

	it('takes a bare path as one metric name', () => {
		expect(at('val/lo|').accept()).toBe('[val/loss]');
		expect(at('[val/p]/lo|').accept()).toBe('[val/p]/log(');
		expect(at('[val/p]/lo|').labels).toContain('val/loss');
	});

	it('ranks the start of a name, then of a part, then anywhere', () => {
		const keys = ['aloss', 'loss/x', 'val/loss'];
		expect(at('[loss|', keys).labels).toEqual(['loss/x', 'val/loss', 'aloss']);
	});

	it('ignores case', () => {
		expect(at('[VAL/R|').labels).toEqual(['val/r']);
		expect(at('SQ|').labels).toEqual(['sqrt(…)']);
	});

	it('stays out of the way where a value is complete', () => {
		expect(at('[val/p]|').labels).toEqual([]);
		expect(at('[val/p] |').labels).toEqual([]);
		expect(at('2|').labels).toEqual([]);
		expect(at('(1 + 2)|').labels).toEqual([]);
		expect(at('1e|').labels).toEqual([]);
	});

	it('says what was typed (nothing: accepting is not implied)', () => {
		expect(at('[val/p] * |').query).toBe('');
		expect(at('[val/p] * [v|').query).toBe('v');
		expect(at('va|').query).toBe('va');
	});

	it('builds a formula that parses', () => {
		let text = at('|').accept(3); // [val/p]
		text = at(`${text} * [|`).accept(4); // [val/r]
		expect(text).toBe('[val/p] * [val/r]');
		expect(() => parseFormula(text)).not.toThrow();
	});
});
