import { describe, expect, it } from 'vitest';

import { groupBy, own } from './group.js';

describe('groupBy', () => {
	it('groups in order of first appearance, and can skip items', () => {
		const groups = groupBy(['a1', 'b1', 'a2', 'x'], (s) => (s === 'x' ? null : s[0]));
		expect(groups).toEqual([
			['a', ['a1', 'a2']],
			['b', ['b1']]
		]);
	});

	it('takes any name as a key', () => {
		const names = ['__proto__', 'constructor', 'toString', 'hasOwnProperty'];
		const groups = groupBy(names, (name) => name);
		expect(groups.map(([key]) => key)).toEqual(names);
		expect(({} as { polluted?: unknown }).polluted).toBeUndefined();
	});

	it('finds only what an object really has', () => {
		const values: Record<string, number> = JSON.parse('{"a": 1, "__proto__": 2}');
		expect(own(values, 'a')).toBe(1);
		expect(own(values, '__proto__')).toBe(2);
		for (const name of ['constructor', 'toString', 'hasOwnProperty', 'missing']) {
			expect(own(values, name), name).toBeUndefined();
		}
		expect(own(null, 'a')).toBeUndefined();
		expect(own(undefined, 'a')).toBeUndefined();
	});
});
