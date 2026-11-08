// How long a password would take to guess, as bits of entropy. It's cut into the cheapest
// pieces a guesser could use: a common password or word, a keyboard run, a repeat, a year,
// the owner's name, or single characters. A letter costs its lowest surprise among letter
// statistics: on its own, after one letter, after two. So words cost far less than random
// letters. The plain formula, length times the log of the alphabet, calls "Password123!"
// strong. Only runs in the browser: passwords aren't sent anywhere to be rated.

import { BIGRAMS, TRIGRAMS, UNIGRAMS, WORDS } from './password-data.js';

export type StrengthLevel = 0 | 1 | 2 | 3 | 4;
export type Strength = { bits: number; level: StrengthLevel; label: string; tips: string[] };

const LABELS = ['Very weak', 'Weak', 'Fair', 'Strong', 'Very strong'];
/** Lower bounds, in bits. Slow hashing helps, but a leak can be attacked for a long time. */
const LEVELS = [0, 25, 40, 60, 80];
/** Beyond this, characters only add their plain cost: no pattern is worth the time. */
const MAX_ANALYZED = 128;
const MAX_TIPS = 3;

// The most likely first: its rank is how soon a guesser tries it.
const WORD_RANK = new Map(WORDS.split(',').map((word, i) => [word, i + 1]));
const MIN_WORD = 3;
const MAX_WORD = 16;

const ROWS = ['abcdefghijklmnopqrstuvwxyz', '01234567890', 'qwertyuiop', 'asdfghjkl', 'zxcvbnm'];
const SEQUENCES = ROWS.flatMap((row) => [row, [...row].reverse().join('')]);

const LEET: Record<string, string> = {
	'0': 'o',
	'1': 'i',
	'3': 'e',
	'4': 'a',
	'5': 's',
	'7': 't',
	'@': 'a',
	$: 's',
	'!': 'i'
};

type Kind = 'common' | 'personal' | 'repeat' | 'sequence' | 'year' | 'chars';
type Piece = { end: number; bits: number; kind: Kind };

const LETTER_CHOICES = Math.log2(26);
const ALPHABET_START = 'a'.charCodeAt(0);

const letterIndex = (c: string | undefined) => {
	const i = c ? c.toLowerCase().charCodeAt(0) - ALPHABET_START : -1;
	return i >= 0 && i < 26 && c!.length === 1 ? i : -1;
};
const tableBits = (table: string, at: number) => {
	const digit = table.charCodeAt(at) - 48;
	return digit === 9 ? LETTER_CHOICES : digit / 2;
};

/** The lowest surprise of this letter, given what we know: alone, after one, after two. */
function letterBits(chars: string[], i: number, letter: number): number {
	let bits = Math.min(LETTER_CHOICES, tableBits(UNIGRAMS, letter));
	const one = letterIndex(chars[i - 1]);
	if (one < 0) return bits;
	bits = Math.min(bits, tableBits(BIGRAMS, one * 26 + letter));
	const two = letterIndex(chars[i - 2]);
	if (two < 0) return bits;
	return Math.min(bits, tableBits(TRIGRAMS, (two * 676 + one * 26 + letter) | 0));
}

function charBits(chars: string[], i: number): number {
	const c = chars[i];
	const letter = letterIndex(c);
	if (letter >= 0) return letterBits(chars, i, letter) + (c === c.toUpperCase() ? 1 : 0);
	if (/[0-9]/.test(c)) return Math.log2(10);
	if (c.charCodeAt(0) >= 128) return 7; // accents, other alphabets, emoji
	// A symbol. The same one as the last: a separator, and a guesser only pays for it once.
	for (let j = i - 1; j >= 0; j--) {
		if (/[a-zA-Z0-9]/.test(chars[j]) || chars[j].charCodeAt(0) >= 128) continue;
		if (chars[j] === c) return 1;
		break;
	}
	return 5;
}

/** A word's capitals and swapped letters only cost a few guesses more. */
function variationBits(original: string[], plain: string): number {
	const text = original.join('');
	let bits = 0;
	if (text !== plain) {
		const lower = text.toLowerCase();
		if (lower !== text) bits += text === text.toUpperCase() || /^[A-Z]/.test(text) ? 1 : 2;
		if (lower !== plain) bits += 1;
	}
	return bits;
}

function piecesFrom(
	i: number,
	chars: string[],
	lower: string[],
	plain: string[],
	personal: string[]
) {
	const pieces: Piece[] = [];

	// The same character, or the same few, over and over.
	let same = i + 1;
	while (same < chars.length && lower[same] === lower[i]) same++;
	if (same - i >= 3) {
		pieces.push({
			end: same,
			bits: charBits(chars, i) + Math.log2(same - i),
			kind: 'repeat'
		});
	}
	for (let unit = 2; unit <= 6 && i + unit * 2 <= chars.length; unit++) {
		let end = i + unit;
		while (end < chars.length && lower[end] === lower[end - unit]) end++;
		const times = Math.floor((end - i) / unit);
		if (times >= 2) {
			let bits = Math.log2(times);
			for (let j = i; j < i + unit; j++) bits += charBits(chars, j);
			pieces.push({ end: i + unit * times, bits, kind: 'repeat' });
		}
	}

	// Along the alphabet, the digits or a keyboard row, either way.
	for (const row of SEQUENCES) {
		const start = row.indexOf(lower[i]);
		if (start < 0) continue;
		let length = 1;
		while (i + length < chars.length && row[start + length] === lower[i + length]) length++;
		if (length >= 3) {
			pieces.push({ end: i + length, bits: 4 + Math.log2(length), kind: 'sequence' });
		}
	}

	// A common password or word, also with its letters swapped for digits and symbols.
	const longest = Math.min(MAX_WORD, chars.length - i);
	for (let length = MIN_WORD; length <= longest; length++) {
		const end = i + length;
		const asTyped = lower.slice(i, end).join('');
		const swapped = plain.slice(i, end).join('');
		const direct = WORD_RANK.get(asTyped);
		const rank = direct ?? WORD_RANK.get(swapped);
		if (rank) {
			const word = direct ? asTyped : swapped;
			const bits = 1 + Math.log2(rank) + variationBits(chars.slice(i, end), word);
			pieces.push({ end, bits, kind: 'common' });
		}
	}

	// The user's own name, known to anyone guessing their password.
	for (const name of personal) {
		if (lower.slice(i, i + name.length).join('') === name) {
			pieces.push({ end: i + name.length, bits: 2, kind: 'personal' });
		}
	}

	// A year.
	if (i + 4 <= chars.length && /^(19|20)\d\d$/.test(chars.slice(i, i + 4).join(''))) {
		pieces.push({ end: i + 4, bits: Math.log2(100), kind: 'year' });
	}
	return pieces;
}

const tipFor: Record<Exclude<Kind, 'chars'>, string> = {
	common: 'It contains a very common password or word. Guessers try those first.',
	personal: "Leave out your name and username: they're the first thing anyone would try.",
	repeat: 'Avoid repeated characters or patterns: they add almost nothing.',
	sequence: 'Avoid runs like abc, 123 or qwerty: they add almost nothing.',
	year: 'Avoid years and dates: there are only so many.'
};

/** Rates a password. `personal`: what's known about its owner (username, display name). */
export function passwordStrength(password: string, personal: string[] = []): Strength {
	const all = [...password];
	const chars = all.slice(0, MAX_ANALYZED);
	const lower = chars.map((c) => c.toLowerCase());
	const plain = lower.map((c) => LEET[c] ?? c);
	const names = [
		...new Set(
			personal.flatMap((p) => p.toLowerCase().split(/\s+/)).filter((p) => [...p].length >= 3)
		)
	];

	// The cheapest way to spell it, piece by piece.
	const best = new Array<number>(chars.length + 1).fill(Infinity);
	const via = new Array<(Piece & { from: number }) | undefined>(chars.length + 1);
	best[0] = 0;
	for (let i = 0; i < chars.length; i++) {
		const pieces: Piece[] = [
			{ end: i + 1, bits: charBits(chars, i), kind: 'chars' },
			...piecesFrom(i, chars, lower, plain, names)
		];
		for (const piece of pieces) {
			if (best[i] + piece.bits < best[piece.end]) {
				best[piece.end] = best[i] + piece.bits;
				via[piece.end] = { ...piece, from: i };
			}
		}
	}
	const kinds = new Set<Kind>();
	for (let at = chars.length; at > 0; at = via[at]!.from) kinds.add(via[at]!.kind);

	let bits = best[chars.length];
	for (let i = MAX_ANALYZED; i < all.length; i++) bits += charBits(all, i);

	let level = 0;
	for (let l = LEVELS.length - 1; l > 0 && !level; l--) if (bits >= LEVELS[l]) level = l;

	const tips: string[] = [];
	if (level < 3) {
		// Several words are the right idea; it's just not there yet.
		const phrase = password.split(/[\s\-_.,+]+/).filter(Boolean).length >= 3;
		for (const kind of ['personal', 'common', 'sequence', 'repeat', 'year'] as const) {
			if (kinds.has(kind) && !(kind === 'common' && phrase)) tips.push(tipFor[kind]);
		}
		if (phrase) tips.push('A good start: add another word or two, the less common the better.');
		else {
			if (all.length < 12) {
				tips.push('Make it longer: length helps more than anything else.');
			}
			tips.push('Try four or five unrelated words, or let a password manager pick one.');
		}
	}
	return {
		bits,
		level: level as StrengthLevel,
		label: LABELS[level],
		tips: tips.slice(0, MAX_TIPS)
	};
}
