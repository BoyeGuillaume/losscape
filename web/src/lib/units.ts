// Numbers with units, scaled to stay readable: 50k steps, 1.5 GiB, 45.3%. A unit is a
// ladder of steps (any factors: 1000 for counts, 1024 for bytes, 60 for minutes), the
// precision to show, and the scale of stored values (memory logged in GiB).

export type UnitStep = { factor: number; suffix: string };

export type Unit = {
	/** Scale steps, smallest factor first; the largest one not above the value is used. */
	steps: UnitStep[];
	/** Significant digits (default 3)... */
	digits?: number;
	/** ...or a fixed number of decimals. Trailing zeros are dropped either way. */
	decimals?: number;
	/** Stored values times this are in the unit of `steps` (GiB values in bytes: 2^30). */
	scale?: number;
	/** A space between number and suffix (default: when the suffix starts with a letter). */
	space?: boolean;
};

const KILO = 1000;
const KIBI = 1024;

/** Counts: 950, 50k, 1.2M, 3B. */
export const COUNT: Unit = {
	steps: [
		{ factor: 1, suffix: '' },
		{ factor: KILO, suffix: 'k' },
		{ factor: KILO ** 2, suffix: 'M' },
		{ factor: KILO ** 3, suffix: 'B' },
		{ factor: KILO ** 4, suffix: 'T' }
	],
	space: false
};

/** Bytes, binary: 512 B, 1.5 KiB, 3.2 GiB. */
export const BYTES: Unit = {
	steps: ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'].map((suffix, i) => ({
		factor: KIBI ** i,
		suffix
	}))
};

/** Bits per second, decimal: 800 bps, 12.5 Mbps, 1 Gbps. */
export const BITS_PER_SECOND: Unit = {
	steps: ['bps', 'kbps', 'Mbps', 'Gbps', 'Tbps'].map((suffix, i) => ({
		factor: KILO ** i,
		suffix
	}))
};

/** Seconds: 42 s, 1.5 min, 2.3 h, 3 d. */
export const SECONDS: Unit = {
	steps: [
		{ factor: 1, suffix: 's' },
		{ factor: 60, suffix: 'min' },
		{ factor: 3600, suffix: 'h' },
		{ factor: 86400, suffix: 'd' }
	]
};

/** Values already in percent (0-100): 45.3%. */
export const PERCENT: Unit = { steps: [{ factor: 1, suffix: '%' }], decimals: 1, space: false };

/** A unit for values stored in another unit of the same ladder (`scale` times it). */
export const scaled = (unit: Unit, scale: number): Unit => ({ ...unit, scale });

function round(n: number, unit: Unit): number {
	return unit.decimals !== undefined
		? Number(n.toFixed(unit.decimals))
		: Number(n.toPrecision(unit.digits ?? 3));
}

/** The step a value (already scaled) is shown in. */
export function stepFor(value: number, unit: Unit): UnitStep {
	return unit.steps[stepIndex(Math.abs(value), unit)];
}

function stepIndex(abs: number, unit: Unit): number {
	let i = 0;
	while (i + 1 < unit.steps.length && abs >= unit.steps[i + 1].factor) i++;
	return i;
}

export function formatUnit(value: number, unit: Unit): string {
	if (!Number.isFinite(value)) return String(value);
	const v = value * (unit.scale ?? 1);
	let i = stepIndex(Math.abs(v), unit);
	let n = round(v / unit.steps[i].factor, unit);
	// Rounding can reach the next step: 999.96k is 1M, not 1000k.
	while (
		i + 1 < unit.steps.length &&
		Math.abs(n) * unit.steps[i].factor >= unit.steps[i + 1].factor
	) {
		i++;
		n = round(v / unit.steps[i].factor, unit);
	}
	const { suffix } = unit.steps[i];
	if (!suffix) return String(n);
	const space = unit.space ?? /^[A-Za-z]/.test(suffix);
	return `${n}${space ? ' ' : ''}${suffix}`;
}
