// Words for metric settings, shared by the Metrics tab and metric pages. Units are picked
// as a kind (bytes) and how values were logged (in MiB): stored as a preset and a factor.

import type { Direction, Meta, UnitPreset, UnitSpec } from './registry.js';

export type UnitKind = 'number' | 'percent' | 'bytes' | 'bytes_per_s' | 'bps' | 'time' | 'custom';
export type LoggedAs = { label: string; preset: UnitPreset; scale: number; compact?: boolean };

export const UNIT_KINDS: { value: UnitKind; label: string }[] = [
	{ value: 'number', label: 'Number' },
	{ value: 'percent', label: 'Percent' },
	{ value: 'bytes', label: 'Bytes' },
	{ value: 'bytes_per_s', label: 'Bytes/s' },
	{ value: 'bps', label: 'Bits/s' },
	{ value: 'time', label: 'Time' },
	{ value: 'custom', label: 'Custom' }
];

/** How values of each kind may have been logged (the first is the default). */
export const LOGGED_AS: Record<UnitKind, LoggedAs[]> = {
	number: [
		{ label: 'plain (0.123)', preset: 'none', scale: 1 },
		{ label: 'compact (12.3k)', preset: 'count', scale: 1 }
	],
	percent: [
		{ label: '0–1', preset: 'fraction', scale: 1 },
		{ label: '0–100', preset: 'percent', scale: 1 }
	],
	bytes: [
		{ label: 'B', preset: 'bytes', scale: 1 },
		{ label: 'KiB', preset: 'bytes', scale: 2 ** 10 },
		{ label: 'MiB', preset: 'bytes', scale: 2 ** 20 },
		{ label: 'GiB', preset: 'bytes', scale: 2 ** 30 }
	],
	bytes_per_s: [
		{ label: 'B/s', preset: 'bytes_per_s', scale: 1 },
		{ label: 'KiB/s', preset: 'bytes_per_s', scale: 2 ** 10 },
		{ label: 'MiB/s', preset: 'bytes_per_s', scale: 2 ** 20 },
		{ label: 'GiB/s', preset: 'bytes_per_s', scale: 2 ** 30 }
	],
	bps: [
		{ label: 'bit/s', preset: 'bps', scale: 1 },
		{ label: 'kbit/s', preset: 'bps', scale: 1e3 },
		{ label: 'Mbit/s', preset: 'bps', scale: 1e6 }
	],
	time: [
		{ label: 'ms', preset: 'seconds', scale: 1e-3 },
		{ label: 's', preset: 'seconds', scale: 1 },
		{ label: 'min', preset: 'seconds', scale: 60 },
		{ label: 'h', preset: 'seconds', scale: 3600 }
	],
	custom: [
		{ label: 'plain (950 px)', preset: 'custom', scale: 1 },
		{ label: 'compact (12.3k px)', preset: 'custom', scale: 1, compact: true }
	]
};

export function unitKind(spec: UnitSpec | null | undefined): UnitKind {
	switch (spec?.preset) {
		case 'percent':
		case 'fraction':
			return 'percent';
		case 'bytes':
			return 'bytes';
		case 'bytes_per_s':
			return 'bytes_per_s';
		case 'bps':
			return 'bps';
		case 'seconds':
			return 'time';
		case 'custom':
			return 'custom';
		default:
			return 'number';
	}
}

/** The "logged as" choice of a unit, or null when its factor is another one. */
export function loggedAs(spec: UnitSpec | null | undefined): LoggedAs | null {
	const preset = spec?.preset ?? 'none';
	const scale = spec?.scale ?? 1;
	const compact = spec?.compact ?? false;
	return (
		LOGGED_AS[unitKind(spec)].find(
			(o) => o.preset === preset && o.scale === scale && (o.compact ?? false) === compact
		) ?? null
	);
}

/** A unit in a few words: "% (0–1)", "bytes (GiB)", "number". */
export function unitLabel(spec: UnitSpec | null | undefined): string {
	const kind = unitKind(spec);
	const as = loggedAs(spec);
	const factor = as ? '' : ` × ${spec?.scale ?? 1}`;
	switch (kind) {
		case 'number':
			return (spec?.preset === 'count' ? 'compact number' : 'number') + factor;
		case 'percent':
			return (spec?.preset === 'fraction' ? '% (0–1)' : '%') + factor;
		case 'custom':
			return `“${spec?.suffix ?? ''}”${spec?.compact ? ' (compact)' : ''}${factor}`;
		default:
			return `${UNIT_KINDS.find((k) => k.value === kind)!.label.toLowerCase()} (${as?.label ?? `×${spec?.scale}`})`;
	}
}

export const directionLabel = (d: Direction | null | undefined) =>
	d === 'max' ? '↑ higher' : d === 'min' ? '↓ lower' : 'no direction';

export const summaryLabel = (s: 'last' | 'best' | undefined) => (s === 'best' ? 'best' : 'last');

/** A metric's settings in one line: "% (0–1) · ↑ higher · best". */
export function settingsLine(meta: Meta): string {
	const parts = [unitLabel(meta.unitSpec)];
	if (meta.direction) parts.push(directionLabel(meta.direction), summaryLabel(meta.summary));
	return parts.join(' · ');
}
