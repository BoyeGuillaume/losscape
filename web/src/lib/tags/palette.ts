// Tag colors: a palette that reads well in light and dark mode. The server keeps a chosen
// color per project tag; tags without one get a color derived from their name.

export const TAG_COLORS = [
	'gray',
	'red',
	'orange',
	'amber',
	'green',
	'teal',
	'blue',
	'indigo',
	'purple',
	'pink'
] as const;
export type TagColor = (typeof TAG_COLORS)[number];

/** Chip classes per color (whole strings, so Tailwind finds them). */
export const TAG_CLASSES: Record<TagColor, string> = {
	gray: 'border-zinc-300 bg-zinc-100 text-zinc-700 dark:border-zinc-600 dark:bg-zinc-800 dark:text-zinc-300',
	red: 'border-red-200 bg-red-50 text-red-700 dark:border-red-500/30 dark:bg-red-500/15 dark:text-red-300',
	orange:
		'border-orange-200 bg-orange-50 text-orange-700 dark:border-orange-500/30 dark:bg-orange-500/15 dark:text-orange-300',
	amber:
		'border-amber-200 bg-amber-50 text-amber-800 dark:border-amber-500/30 dark:bg-amber-500/15 dark:text-amber-300',
	green:
		'border-green-200 bg-green-50 text-green-700 dark:border-green-500/30 dark:bg-green-500/15 dark:text-green-300',
	teal: 'border-teal-200 bg-teal-50 text-teal-700 dark:border-teal-500/30 dark:bg-teal-500/15 dark:text-teal-300',
	blue: 'border-blue-200 bg-blue-50 text-blue-700 dark:border-blue-500/30 dark:bg-blue-500/15 dark:text-blue-300',
	indigo:
		'border-indigo-200 bg-indigo-50 text-indigo-700 dark:border-indigo-500/30 dark:bg-indigo-500/15 dark:text-indigo-300',
	purple:
		'border-purple-200 bg-purple-50 text-purple-700 dark:border-purple-500/30 dark:bg-purple-500/15 dark:text-purple-300',
	pink: 'border-pink-200 bg-pink-50 text-pink-700 dark:border-pink-500/30 dark:bg-pink-500/15 dark:text-pink-300'
};

/** Swatch classes for the color picker. */
export const SWATCH_CLASSES: Record<TagColor, string> = {
	gray: 'bg-zinc-400',
	red: 'bg-red-500',
	orange: 'bg-orange-500',
	amber: 'bg-amber-500',
	green: 'bg-green-500',
	teal: 'bg-teal-500',
	blue: 'bg-blue-500',
	indigo: 'bg-indigo-500',
	purple: 'bg-purple-500',
	pink: 'bg-pink-500'
};

export const isTagColor = (c: unknown): c is TagColor =>
	typeof c === 'string' && (TAG_COLORS as readonly string[]).includes(c);

/** The color a tag gets from its name (not gray, kept for tags marked as such). */
export function colorFromName(name: string): TagColor {
	let h = 2166136261;
	for (let i = 0; i < name.length; i++) h = Math.imul(h ^ name.charCodeAt(i), 16777619);
	return TAG_COLORS[1 + ((h >>> 0) % (TAG_COLORS.length - 1))];
}
