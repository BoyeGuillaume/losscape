import { describe, expect, it } from 'vitest';

import { markMatches, tokenize, type Token } from './highlight.js';

describe('tokenize', () => {
	it('colors Python for both themes and keeps the text', async () => {
		const code = 'import lsc\n\nwith lsc.init(project="mnist") as run:\n    run.log({"loss": 0.5})';
		const lines = (await tokenize(code, 'python'))!;
		expect(lines).toHaveLength(4);
		expect(lines.map((l) => l.map((t) => t.content).join('')).join('\n')).toBe(code);
		const keyword = lines[0].find((t) => t.content === 'import')!;
		expect(keyword.light).toMatch(/^#/);
		expect(keyword.dark).toMatch(/^#/);
		expect(keyword.light).not.toBe(keyword.dark);
		expect(lines[1], 'empty lines stay').toEqual([]);
	});

	it('colors shell commands', async () => {
		const lines = (await tokenize(
			'pip install losscape-sdk\nexport LOSSCAPE_API_KEY=df_x',
			'shellscript'
		))!;
		expect(lines[1].map((t) => t.content).join('')).toBe('export LOSSCAPE_API_KEY=df_x');
	});
});

describe('markMatches', () => {
	const line = (...parts: string[]): Token[] => parts.map((content) => ({ content, light: 'c' }));
	const show = (lines: ReturnType<typeof markMatches>) =>
		lines.map((l) => l.map((t) => (t.match ? `[${t.content}]` : t.content)).join('|'));

	it('marks matches inside tokens, keeping their colors', () => {
		const out = markMatches([line('optimizer', ':', ' adam')], ['tim']);
		expect(show(out)).toEqual(['op|[tim]|izer|:| adam']);
		expect(out[0][1]).toEqual({ content: 'tim', light: 'c', match: true });
	});

	it('marks matches across tokens, ignoring case', () => {
		expect(show(markMatches([line('LR', ':', ' 0.1')], ['r: 0']))).toEqual(['L|[R]|[:]|[ 0]|.1']);
	});

	it('marks every term, on every line', () => {
		const out = markMatches([line('lr: 1'), line('other'), line('lr2: lr')], ['lr']);
		expect(show(out)).toEqual(['[lr]|: 1', 'other', '[lr]|2: |[lr]']);
	});

	it('leaves lines alone without terms', () => {
		const lines = [line('a')];
		expect(markMatches(lines, [])).toBe(lines);
		expect(markMatches(lines, [''])).toBe(lines);
	});
});
