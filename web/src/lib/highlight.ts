// Syntax highlighting for code shown in the UI, with Shiki: only the languages and themes
// used here, loaded on first use, and its JavaScript regex engine (no WebAssembly, so the
// Content-Security-Policy stays as is). Tokens are rendered as text by CodeBlock, never
// as HTML.

import type { HighlighterCore } from 'shiki/core';

export type Lang = 'python' | 'shellscript';
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
			langs: [import('shiki/langs/python.mjs'), import('shiki/langs/shellscript.mjs')],
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
