import { describe, expect, it } from 'vitest';

import { tokenize } from './highlight.js';

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
