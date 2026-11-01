import { describe, expect, it } from 'vitest';

import { parseAnsi, stripAnsi } from './ansi.js';

const plain = (line: string) => parseAnsi(line).map((s) => [s.text, s.style]);

describe('parseAnsi', () => {
	it('returns plain text as one segment', () => {
		expect(plain('epoch 1: loss 0.5')).toEqual([['epoch 1: loss 0.5', {}]]);
		expect(parseAnsi('')).toEqual([]);
	});

	it('applies and resets colors and styles', () => {
		expect(plain('\x1b[1;31mERROR\x1b[0m: disk')).toEqual([
			['ERROR', { bold: true, fg: 'var(--ansi-1)' }],
			[': disk', {}]
		]);
		expect(plain('\x1b[92mok\x1b[39m done\x1b[m')).toEqual([
			['ok', { fg: 'var(--ansi-10)' }],
			[' done', {}]
		]);
	});

	it('reads 256 colors and true colors', () => {
		expect(plain('\x1b[38;5;196mred\x1b[48;2;0;128;255mon blue')).toEqual([
			['red', { fg: 'rgb(255, 0, 0)' }],
			['on blue', { fg: 'rgb(255, 0, 0)', bg: 'rgb(0, 128, 255)' }]
		]);
		expect(plain('\x1b[38;5;244mgray')).toEqual([['gray', { fg: 'rgb(128, 128, 128)' }]]);
	});

	it('drops other escape sequences and hostile parameters', () => {
		expect(plain('\x1b]8;;https://x\x07link\x1b]8;;\x07 \x1b[2Kok')).toEqual([['link ok', {}]]);
		expect(plain('\x1b[38;2;999;0;0mx')).toEqual([['x', {}]]);
		expect(plain('<img src=x onerror=alert(1)>')).toEqual([['<img src=x onerror=alert(1)>', {}]]);
	});
});

describe('stripAnsi', () => {
	it('keeps only the text', () => {
		expect(stripAnsi('\x1b[31mred\x1b[0m and \x1b[1mbold')).toBe('red and bold');
	});
});
