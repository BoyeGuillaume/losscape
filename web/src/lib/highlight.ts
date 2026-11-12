// Syntax highlighting for code shown in the UI, with Shiki: only the languages and themes
// used here, loaded on first use, and its JavaScript regex engine (no WebAssembly, so the
// Content-Security-Policy stays as is). Tokens are rendered as text by CodeBlock, never
// as HTML.

import type { HighlighterCore } from 'shiki/core';

export type Lang = 'python' | 'shellscript' | 'yaml';
/** One piece of a line, with its color in the light and dark theme. */
export type Token = { content: string; light?: string; dark?: string };

let highlighter: Promise<HighlighterCore> | null = null;

function load(): Promise<HighlighterCore> {
	highlighter ??= (async () => {
		const [{ createHighlighterCore }, { createJavaScriptRegexEngine }] = await Promise.all([
			import('shiki/core'),
			import('shiki/engine/javascript')
		]);
		return createHighlighterCore({
			themes: [import('shiki/themes/github-light.mjs'), import('shiki/themes/github-dark.mjs')],
			langs: [
				import('shiki/langs/python.mjs'),
				import('shiki/langs/shellscript.mjs'),
				import('shiki/langs/yaml.mjs')
			],
			engine: createJavaScriptRegexEngine()
		});
	})();
	return highlighter;
}

/** Lines of tokens; null if highlighting isn't available (the code shows plain). */
export async function tokenize(code: string, lang: Lang): Promise<Token[][] | null> {
	try {
		const shiki = await load();
		const { tokens } = shiki.codeToTokens(code, {
			lang,
			themes: { light: 'github-light', dark: 'github-dark' },
			defaultColor: false
		});
		return tokens.map((line) =>
			line.map((t) => ({
				content: t.content,
				light: t.htmlStyle?.['--shiki-light'],
				dark: t.htmlStyle?.['--shiki-dark']
			}))
		);
	} catch (err) {
		console.warn('losscape: highlighting failed', err);
		return null;
	}
}

/** A token, split where search matches start or end: `match` for the parts inside. */
export type MarkedToken = Token & { match?: boolean };

/**
 * Lines with every occurrence of `terms` marked, ignoring case. A match can span tokens
 * (`lr: 0.1` is several): each token is split where a match starts or ends in it.
 */
export function markMatches(lines: Token[][], terms: string[]): MarkedToken[][] {
	const wanted = terms.map((t) => t.toLowerCase()).filter(Boolean);
	if (!wanted.length) return lines;
	return lines.map((line) => {
		const text = line.map((t) => t.content).join('');
		const lower = text.toLowerCase();
		const marked = new Array<boolean>(text.length).fill(false);
		for (const term of wanted) {
			for (let i = lower.indexOf(term); i >= 0; i = lower.indexOf(term, i + 1)) {
				marked.fill(true, i, i + term.length);
			}
		}
		if (!marked.includes(true)) return line;
		const out: MarkedToken[] = [];
		let at = 0;
		for (const token of line) {
			let start = 0;
			for (let i = 1; i <= token.content.length; i++) {
				if (i === token.content.length || marked[at + i] !== marked[at + start]) {
					out.push({ ...token, content: token.content.slice(start, i), match: marked[at + start] });
					start = i;
				}
			}
			at += token.content.length;
		}
		return out;
	});
}
