// Custom metrics of a project (docs: project settings > Metrics). Definitions describe
// logged metrics (unit, bounds, which way is better, summary) or add virtual ones:
// merges of differently named metrics, families (`val/pck-{k}`) and formulas. Runs can
// also rename a metric (`pck-8` shown as `pck-8px`). All of it applies when reading.

import { BITS_PER_SECOND, BYTES, COUNT, PERCENT, SECONDS, scaled, type Unit } from '#lib/units.js';
import { FormulaError, parseFormula, references, type Formula } from './formula.js';
import { compile, isPattern, match } from './pattern.js';

export type Direction = 'max' | 'min';
export type UnitPreset =
	'none' | 'count' | 'percent' | 'fraction' | 'bytes' | 'bps' | 'seconds' | 'custom';
export type UnitSpec = {
	preset: UnitPreset;
	/** Stored values times this are in the preset's base unit (GiB stored: 2^30 bytes). */
	scale?: number;
	/** Custom unit's suffix. */
	suffix?: string;
	digits?: number;
	decimals?: number;
};
export type Spec = {
	label?: string;
	description?: string;
	unit?: UnitSpec;
	/** Fixed range, e.g. [0, 1]; null for an open end. */
	bounds?: [number | null, number | null];
	direction?: Direction;
	/** Summary value: the last one logged, or the best (needs `direction`). */
	summary?: 'last' | 'best';
	/** Merge: the first of these a run has (with a factor, for different scales). */
	sources?: { key: string; scale?: number }[];
	/** Formula: e.g. `2 * [val/p] * [val/r] / ([val/p] + [val/r])`. */
	expression?: string;
	/** Merges and formulas: hide the metrics they read from the charts. */
	hideSources?: boolean;
};
export type Kind = 'describe' | 'merge' | 'family' | 'formula';
export type MetricDef = { name: string; kind: Kind; spec: Spec };

/** What the UI needs to know about a metric. */
export type Meta = {
	unit: Unit | null;
	bounds: [number | null, number | null] | null;
	direction: Direction | null;
	summary: 'last' | 'best';
	label: string | null;
	description: string | null;
	/** The definition it comes from. */
	def: MetricDef | null;
};

/** How to read a metric of one run. */
export type Resolution =
	| { kind: 'key'; raw: string; scale: number }
	| { kind: 'formula'; formula: Formula; refs: Map<string, { raw: string; scale: number }> };

/** Built-in descriptions (lowest precedence): system metrics. */
export const BUILTIN: MetricDef[] = [
	{
		name: 'system/*_pct',
		kind: 'describe',
		spec: { unit: { preset: 'percent' }, bounds: [0, 100] }
	},
	{
		name: 'system/memory/process_gb',
		kind: 'describe',
		spec: { unit: { preset: 'bytes', scale: 2 ** 30 } }
	},
	{
		name: 'system/gpu*/memory_gb',
		kind: 'describe',
		spec: { unit: { preset: 'bytes', scale: 2 ** 30 } }
	},
	{ name: 'system/network/recv', kind: 'describe', spec: { unit: { preset: 'bps' } } },
	{ name: 'system/network/sent', kind: 'describe', spec: { unit: { preset: 'bps' } } }
];

export function unitOf(spec: UnitSpec | undefined): Unit | null {
	if (!spec || spec.preset === 'none') return null;
	const base: Unit = {
		count: COUNT,
		percent: PERCENT,
		fraction: { ...PERCENT, scale: 100 },
		bytes: BYTES,
		bps: BITS_PER_SECOND,
		seconds: SECONDS,
		custom: { steps: [{ factor: 1, suffix: spec.suffix ?? '' }] }
	}[spec.preset];
	let unit = spec.scale ? scaled(base, (base.scale ?? 1) * spec.scale) : base;
	if (spec.digits !== undefined || spec.decimals !== undefined) {
		unit = { ...unit, digits: spec.digits, decimals: spec.decimals };
	}
	return unit;
}

const NO_META: Meta = {
	unit: null,
	bounds: null,
	direction: null,
	summary: 'last',
	label: null,
	description: null,
	def: null
};

function metaOf(def: MetricDef): Meta {
	const s = def.spec;
	return {
		unit: unitOf(s.unit),
		bounds: s.bounds ?? null,
		direction: s.direction ?? null,
		summary: s.direction && s.summary === 'best' ? 'best' : 'last',
		label: s.label?.trim() || null,
		description: s.description?.trim() || null,
		def
	};
}

/** Problems with a definition, for the editor (empty when fine). */
export function problems(def: MetricDef): string[] {
	const out: string[] = [];
	if (!def.name.trim()) out.push('A name is needed.');
	if (def.kind === 'family' && compile(def.name).params.length !== 1) {
		out.push('A family pattern has exactly one {parameter}, e.g. val/pck-{k}.');
	}
	if ((def.kind === 'merge' || def.kind === 'formula') && isPattern(def.name)) {
		out.push('A virtual metric has a plain name, without * or {…}.');
	}
	if (def.kind === 'merge' && !(def.spec.sources ?? []).some((s) => s.key.trim())) {
		out.push('A merge needs at least one source metric.');
	}
	if (def.kind === 'formula') {
		try {
			parseFormula(def.spec.expression ?? '');
		} catch (err) {
			out.push(err instanceof FormulaError ? `Formula: ${err.message}.` : 'Invalid formula.');
		}
	}
	if (def.spec.summary === 'best' && !def.spec.direction) {
		out.push('"Best" needs to know whether higher or lower is better.');
	}
	return out;
}

export class Registry {
	readonly defs: MetricDef[];
	private virtual = new Map<string, MetricDef>();
	private formulas = new Map<string, Formula>();
	private metaCache = new Map<string, Meta>();
	private hiddenCache: Set<string> | null = null;

	constructor(defs: MetricDef[], builtin: MetricDef[] = BUILTIN) {
		this.defs = [...defs.filter((d) => problems(d).length === 0), ...builtin];
		for (const d of defs) {
			if (problems(d).length) continue;
			if (d.kind === 'merge' || d.kind === 'formula') this.virtual.set(d.name, d);
			if (d.kind === 'formula') this.formulas.set(d.name, parseFormula(d.spec.expression!));
		}
	}

	/** Virtual metric names (merges and formulas). */
	get virtualNames(): string[] {
		return [...this.virtual.keys()];
	}

	get families(): MetricDef[] {
		return this.defs.filter((d) => d.kind === 'family');
	}

	isVirtual(key: string): boolean {
		return this.virtual.has(key);
	}

	/**
	 * Description of a metric: an exact definition, then the virtual metric's own, then
	 * the most specific family or pattern, then built-in defaults.
	 */
	describe(key: string): Meta {
		let meta = this.metaCache.get(key);
		if (meta) return meta;
		const exact = this.defs.find((d) => d.kind === 'describe' && d.name === key);
		const virtual = this.virtual.get(key);
		const patterns = this.defs
			.filter((d) => (d.kind === 'family' || d.kind === 'describe') && isPattern(d.name))
			.filter((d) => match(d.name, key))
			.sort((a, b) => compile(b.name).literal - compile(a.name).literal);
		const def = exact ?? virtual ?? patterns[0];
		meta = def ? metaOf(def) : NO_META;
		this.metaCache.set(key, meta);
		return meta;
	}

	/** Metrics merged into others or read by formulas that hide their sources. */
	hidden(): Set<string> {
		if (this.hiddenCache) return this.hiddenCache;
		const out = (this.hiddenCache = new Set<string>());
		for (const d of this.virtual.values()) {
			if (!d.spec.hideSources) continue;
			if (d.kind === 'merge') d.spec.sources?.forEach((s) => out.add(s.key));
			else references(this.formulas.get(d.name)!).forEach((k) => out.add(k));
		}
		return out;
	}

	/** The metrics of one run as shown: renamed, plus the virtual ones it can compute. */
	forRun(rawKeys: Iterable<string>, renames: Record<string, string> = {}): RunMetrics {
		return new RunMetrics(this, rawKeys, renames);
	}

	formula(name: string): Formula | undefined {
		return this.formulas.get(name);
	}

	merge(name: string): MetricDef | undefined {
		const d = this.virtual.get(name);
		return d?.kind === 'merge' ? d : undefined;
	}
}

export class RunMetrics {
	/** Shown name -> logged name. */
	readonly shown = new Map<string, string>();
	private cache = new Map<string, Resolution | null>();

	constructor(
		private registry: Registry,
		rawKeys: Iterable<string>,
		renames: Record<string, string>
	) {
		for (const raw of rawKeys) this.shown.set(renames[raw] ?? raw, raw);
	}

	/** Shown names, real and virtual (sources hidden by their definition left out, unless asked). */
	keys(withHidden = false): string[] {
		const hidden = withHidden ? new Set<string>() : this.registry.hidden();
		const out = [...this.shown.keys()].filter((k) => !hidden.has(k));
		for (const name of this.registry.virtualNames) if (this.resolve(name)) out.push(name);
		return out;
	}

	/** How to read `key` for this run, or null if the run doesn't have it. */
	resolve(key: string): Resolution | null {
		if (this.cache.has(key)) return this.cache.get(key)!;
		let r: Resolution | null = null;
		const raw = this.shown.get(key);
		if (raw !== undefined && !this.registry.isVirtual(key)) {
			r = { kind: 'key', raw, scale: 1 };
		} else if (this.registry.merge(key)) {
			for (const source of this.registry.merge(key)!.spec.sources ?? []) {
				const sourceRaw = this.shown.get(source.key);
				if (sourceRaw !== undefined) {
					r = { kind: 'key', raw: sourceRaw, scale: source.scale ?? 1 };
					break;
				}
			}
		} else if (this.registry.formula(key)) {
			const formula = this.registry.formula(key)!;
			const refs = new Map<string, { raw: string; scale: number }>();
			for (const ref of references(formula)) {
				// A formula reads logged (or renamed) metrics and merges, not other formulas.
				const inner = this.registry.formula(ref) ? null : this.resolve(ref);
				if (!inner || inner.kind !== 'key') {
					refs.clear();
					break;
				}
				refs.set(ref, { raw: inner.raw, scale: inner.scale });
			}
			if (refs.size) r = { kind: 'formula', formula, refs };
		}
		this.cache.set(key, r);
		return r;
	}

	/** Which logged metric a shown one reads (for "from val/pck-8-fine" hints). */
	source(key: string): string | null {
		const r = this.resolve(key);
		return r?.kind === 'key' && r.raw !== key ? r.raw : null;
	}
}
