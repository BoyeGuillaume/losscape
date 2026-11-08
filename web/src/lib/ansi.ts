// ANSI SGR (colors and styles) in console lines, parsed into styled segments. Rendered as
// text nodes, never as HTML, so console output can't inject markup.

export type Style = {
	fg?: string;
	bg?: string;
	bold?: boolean;
	dim?: boolean;
	italic?: boolean;
	underline?: boolean;
	inverse?: boolean;
};

export type Segment = { text: string; style: Style };

/** The 16 basic colors come from theme variables (`--ansi-0` ... `--ansi-15`). */
const basic = (n: number) => `var(--ansi-${n})`;

function color256(n: number): string | undefined {
	if (!Number.isInteger(n) || n < 0 || n > 255) return undefined;
	if (n < 16) return basic(n);
	if (n >= 232) {
		const v = 8 + (n - 232) * 10;
		return `rgb(${v}, ${v}, ${v})`;
	}
	const i = n - 16;
	const level = (c: number) => (c === 0 ? 0 : 55 + c * 40);
	return `rgb(${level(Math.floor(i / 36))}, ${level(Math.floor(i / 6) % 6)}, ${level(i % 6)})`;
}

function rgb(r: number, g: number, b: number): string | undefined {
	const ok = [r, g, b].every((c) => Number.isInteger(c) && c >= 0 && c <= 255);
	return ok ? `rgb(${r}, ${g}, ${b})` : undefined;
}

/** Applies one SGR sequence (its numeric parameters) to `style`. */
function apply(style: Style, params: number[]): Style {
	const next = { ...style };
	for (let i = 0; i < params.length; i++) {
		const p = params[i];
		if (p === 0) for (const k of Object.keys(next)) delete next[k as keyof Style];
		else if (p === 1) next.bold = true;
		else if (p === 2) next.dim = true;
		else if (p === 3) next.italic = true;
		else if (p === 4) next.underline = true;
		else if (p === 7) next.inverse = true;
		else if (p === 22) next.bold = next.dim = false;
		else if (p === 23) next.italic = false;
		else if (p === 24) next.underline = false;
		else if (p === 27) next.inverse = false;
		else if (p >= 30 && p <= 37) next.fg = basic(p - 30);
		else if (p >= 90 && p <= 97) next.fg = basic(p - 90 + 8);
		else if (p >= 40 && p <= 47) next.bg = basic(p - 40);
		else if (p >= 100 && p <= 107) next.bg = basic(p - 100 + 8);
		else if (p === 39) delete next.fg;
		else if (p === 49) delete next.bg;
		else if (p === 38 || p === 48) {
			const key = p === 38 ? 'fg' : 'bg';
			let value: string | undefined;
			if (params[i + 1] === 5) {
				value = color256(params[i + 2]);
				i += 2;
			} else if (params[i + 1] === 2) {
				value = rgb(params[i + 2], params[i + 3], params[i + 4]);
				i += 4;
			}
			if (value) next[key] = value;
		}
	}
	return next;
}

// eslint-disable-next-line no-control-regex -- matching escape sequences is the point
const ESCAPE = /\x1b\[([0-9;:]*)([A-Za-z])|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?|\x1b./g;

export function parseAnsi(line: string): Segment[] {
	const out: Segment[] = [];
	let style: Style = {};
	let at = 0;
	const push = (text: string) => {
		if (!text) return;
		const last = out[out.length - 1];
		if (last && last.style === style) last.text += text;
		else out.push({ text, style });
	};
	for (const match of line.matchAll(ESCAPE)) {
		push(line.slice(at, match.index));
		at = match.index + match[0].length;
		if (match[2] === 'm') {
			const params = match[1] === '' ? [0] : match[1].split(/[;:]/).map((p) => Number(p || 0));
			style = apply(style, params);
		}
	}
	push(line.slice(at));
	return out;
}

export function stripAnsi(line: string): string {
	return line.replace(ESCAPE, '');
}

/** Inline styles for a segment (applied through the DOM, not as HTML). */
export function css(style: Style): { color?: string; background?: string; class: string } {
	const [fg, bg] = style.inverse
		? [
				style.bg ?? 'var(--term-bg, var(--background))',
				style.fg ?? 'var(--term-fg, var(--foreground))'
			]
		: [style.fg, style.bg];
	const classes = [
		style.bold && 'font-bold',
		style.dim && 'opacity-60',
		style.italic && 'italic',
		style.underline && 'underline'
	];
	return { color: fg, background: bg, class: classes.filter(Boolean).join(' ') };
}
