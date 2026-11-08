import { describe, expect, it } from 'vitest';

import { passwordStrength as rate } from './password.js';

const levelOf = (password: string, personal: string[] = []) => rate(password, personal).level;

describe('password strength', () => {
	it('sees through the usual tricks', () => {
		for (const weak of [
			'password',
			'Password',
			'PASSWORD',
			'p@ssw0rd',
			'123456',
			'1234567890',
			'qwertyuiop',
			'poiuytrewq',
			'abcdefgh',
			'aaaaaaaaaaaaaaaaaaaa',
			'abababababab',
			'letmein'
		]) {
			expect(levelOf(weak), weak).toBe(0);
		}
		expect(levelOf('Password123!')).toBeLessThanOrEqual(1);
		expect(levelOf('qwerty2024')).toBeLessThanOrEqual(1);
	});

	it('rates real length and randomness highly', () => {
		expect(levelOf('correct horse battery staple')).toBeGreaterThanOrEqual(3);
		expect(levelOf('xK9#mQ2$vL7!pR4@nT6&')).toBe(4);
		expect(levelOf('h7Xq2mZp')).toBeGreaterThanOrEqual(1);
		expect(levelOf('zjxqkvwpfgbnm')).toBeGreaterThanOrEqual(2);
	});

	it('costs words less than random letters', () => {
		expect(rate('thereand').bits).toBeLessThan(rate('zkxqjvpw').bits);
		// Not in the dictionary, but it reads like English.
		expect(rate('abstraction').bits).toBeLessThan(rate('xjqwvzkbfpg').bits);
		// The lowest cost among letter statistics, never more than a random letter.
		expect(rate('zzqzxj').bits).toBeLessThanOrEqual(6 * Math.log2(26));
	});

	it('finds words wherever they are, also with capitals and swapped letters', () => {
		const random = rate('zkxqjvpwhbmf').bits;
		for (const p of ['sunshine', 'SUNSHINE', 'Sunshine', 's4nsh1ne'.replace('4', 'u')]) {
			expect(rate(p).bits, p).toBeLessThan(25);
		}
		expect(rate('sunshineyellowbridge').bits).toBeLessThan(random + 10);
	});

	it('pays for a separator once', () => {
		const same = rate('tiger-lamp-orbit-moss-kettle').bits;
		const mixed = rate('tiger-lamp_orbit.moss+kettle').bits;
		expect(same).toBeLessThan(mixed - 8);
	});

	it('encourages a phrase on its way', () => {
		const tips = rate('red blue car').tips.join(' ');
		expect(tips).toMatch(/add another word/);
		expect(tips).not.toMatch(/very common password/);
	});

	it('counts what is known about the owner as next to nothing', () => {
		const alone = rate('adalovelace99x').bits;
		const known = rate('adalovelace99x', ['Ada Lovelace', 'ada']).bits;
		expect(known).toBeLessThan(alone - 10);
		expect(levelOf('ada', ['ada'])).toBe(0);
		// Too short to mean anything.
		expect(rate('xy9', ['xy']).bits).toBe(rate('xy9').bits);
	});

	it('grows with length', () => {
		let last = -1;
		for (const n of [1, 4, 8, 16, 32]) {
			const { bits } = rate('kT7#qZ2!wX9$mP4&vR6@bN3^hL8*jD5%'.slice(0, n));
			expect(bits).toBeGreaterThan(last);
			last = bits;
		}
	});

	it('gives ideas for what is actually wrong, and none when it is fine', () => {
		const tips = (p: string, personal?: string[]) => rate(p, personal).tips.join(' ');
		expect(tips('password')).toMatch(/common password/);
		expect(tips('rstuvw')).toMatch(/runs like/);
		expect(tips('aaaaaaa')).toMatch(/repeated/);
		expect(tips('ada', ['ada'])).toMatch(/your name/);
		expect(tips('x1990y')).toMatch(/years/);
		expect(tips('abc')).toMatch(/longer/);
		expect(rate('correct horse battery staple').tips).toEqual([]);
		expect(rate('password').tips.length).toBeLessThanOrEqual(3);
		expect(rate('password').tips.join(' ')).not.toContain('password123');
	});

	it('never repeats the password in its advice', () => {
		const secret = 'zebra-sunshine-1984';
		expect(rate(secret).tips.join(' ')).not.toMatch(/zebra|sunshine|1984/);
	});

	it('takes any text', () => {
		expect(rate('').bits).toBe(0);
		expect(rate('').level).toBe(0);
		expect(rate('日本語のパスワードです').bits).toBeGreaterThan(40);
		expect(rate('🦀🦀🦀🦀🦀🦀🦀🦀').level).toBe(0);
		expect(rate('🦀🐍🚀🔥🌈🎉').bits).toBeGreaterThan(30);
	});

	it('stays quick on very long passwords', () => {
		const long = 'kT7#qZ2!wX9$mP4&'.repeat(64);
		const start = performance.now();
		const { bits } = rate(long);
		expect(performance.now() - start).toBeLessThan(50);
		expect(bits).toBeGreaterThan(80);
	});
});
