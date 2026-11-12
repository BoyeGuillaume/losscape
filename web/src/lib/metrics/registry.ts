// Custom metrics of a project (its Metrics tab). Definitions describe logged metrics
// (unit, range, which way is better, summary, important, hidden), one by one or by pattern
// (`val/*`), group families (`val/pck-{k}`) and compute metrics from others. Runs can also
// rename a metric (`pck-8` shown as `pck-8px`, rescaled if needed). All of it applies when
// reading: logged data never changes.

import {
	BITS_PER_SECOND,
	BYTES,
	BYTES_PER_SECOND,
	FLOPS,
	COUNT,
	PERCENT,
	SECONDS,
	scaled,
	siUnit,
	type Prefixes,
	type Unit
} from '#lib/units.js';
import { allReferences, FormulaError, parseFormula, type Formula } from './formula.js';
import { compile, isPattern, match } from './pattern.js';
import { own } from '#lib/group.js';

export type Direction = 'max' | 'min';
export type UnitPreset =
	| 'none'
	| 'count'
	| 'flops'
	| 'percent'
	| 'fraction'
	| 'bytes'
	| 'bytes_per_s'
	| 'bps'
	| 'seconds'
	| 'watts'
	| 'custom';
export type UnitSpec = {
	preset: UnitPreset;
	/** Stored values times this are in the preset's base unit (GiB stored: 2^30 bytes). */
	scale?: number;
	/** Custom unit's suffix. */
	suffix?: string;
	/** Custom unit and watts: SI prefixes, none (950 px), large (12.3 kpx) or all (4.5 mW). */
	prefixes?: Prefixes;
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
	/** Shown first: in charts, the run table's default columns and summaries. */
	important?: boolean;
	/** Left out of charts and default columns (still filterable and in the Metrics tab). */
	hidden?: boolean;
	/** Computed metric: e.g. `2 * [val/p] * [val/r] / ([val/p] + [val/r])`. */
	expression?: string;
};
/**
 * `describe`: a metric, or a pattern of them (`val/*`: everything under val/);
 * `family`: metrics named by a number (`val/pck-{k}`), also charted against it;
 * `formula`: a metric computed from others.
 */
export type Kind = 'describe' | 'family' | 'formula';
export type MetricDef = { name: string; kind: Kind; spec: Spec };

/** What the UI needs to know about a metric. */
export type Meta = {
	unit: Unit | null;
	/** How the unit was set (for editors). */
	unitSpec: UnitSpec | null;
	bounds: [number | null, number | null] | null;
	direction: Direction | null;
	summary: 'last' | 'best';
	label: string | null;
	description: string | null;
	important: boolean;
	hidden: boolean;
	/** The metric's own definition, if it has one. */
	def: MetricDef | null;
};

/** How to read a metric of one run. */
export type Resolution =
	| { kind: 'key'; raw: string; scale: number }
	| { kind: 'formula'; formula: Formula; refs: Map<string, { raw: string; scale: number }> };

/** A run's renamed metrics: logged name -> shown name, and factors other than 1. */
export type Renames = {
	renames?: Record<string, string>;
	rename_scales?: Record<string, number>;
};

/** Built-in descriptions (lowest precedence): system metrics. */
/** Recorded by `run.describe_model()`: one value per run, only logged again if it changes. */
export const isModelMetric = (key: string) =>
	/^model\/(.+\/)?(params|trainable_params|size_bytes|flops)$/.test(key);

export const BUILTIN: MetricDef[] = [
	{ name: 'model/*params', kind: 'describe', spec: { unit: { preset: 'count' } } },
	{ name: 'model/*flops', kind: 'describe', spec: { unit: { preset: 'flops' } } },
	{ name: 'model/*size_bytes', kind: 'describe', spec: { unit: { preset: 'bytes' } } },
	// Names say what is measured; the unit is here (see crates/losscape-client/src/system.rs).
	{
		name: 'system/*util',
		kind: 'describe',
		spec: { unit: { preset: 'percent' }, bounds: [0, 100] }
	},
	{
		name: 'system/*usage',
		kind: 'describe',
		spec: { unit: { preset: 'percent' }, bounds: [0, 100] }
	},
	{
		name: 'system/cpu/process',
		kind: 'describe',
		spec: {
			unit: { preset: 'percent' },
			bounds: [0, null],
			description: 'Across all its threads: 100% is one full core, so it can exceed 100%.'
		}
	},
	{ name: 'system/memory/process', kind: 'describe', spec: { unit: { preset: 'bytes' } } },
	{ name: 'system/gpu*/memory', kind: 'describe', spec: { unit: { preset: 'bytes' } } },
	{ name: 'system/gpu*/power', kind: 'describe', spec: { unit: { preset: 'watts' } } },
	{
		name: 'system/gpu*/temp',
		kind: 'describe',
		spec: { unit: { preset: 'custom', suffix: '°C' } }
	},
	{ name: 'system/disk/read', kind: 'describe', spec: { unit: { preset: 'bytes_per_s' } } },
	{ name: 'system/disk/write', kind: 'describe', spec: { unit: { preset: 'bytes_per_s' } } },
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
		bytes_per_s: BYTES_PER_SECOND,
		flops: FLOPS,
		bps: BITS_PER_SECOND,
		seconds: SECONDS,
		watts: siUnit('W', spec.prefixes ?? 'all'),
		custom: siUnit(spec.suffix ?? '', spec.prefixes ?? 'none')
	}[spec.preset];
	let unit = spec.scale ? scaled(base, (base.scale ?? 1) * spec.scale) : base;
	if (spec.digits !== undefined || spec.decimals !== undefined) {
		unit = { ...unit, digits: spec.digits, decimals: spec.decimals };
	}
	return unit;
}

/** One spec with the fields of `specs` (later ones win field by field). */
function mergeSpecs(specs: Spec[]): Spec {
	const out: Spec = {};
	for (const spec of specs) {
		for (const [k, v] of Object.entries(spec)) {
			if (v !== undefined) (out as Record<string, unknown>)[k] = v;
		}
	}
	return out;
}

function metaOf(s: Spec, def: MetricDef | null): Meta {
	return {
		unit: unitOf(s.unit),
		unitSpec: s.unit ?? null,
		bounds: s.bounds ?? null,
		direction: s.direction ?? null,
		summary: s.direction && s.summary === 'best' ? 'best' : 'last',
		label: s.label?.trim() || null,
		description: s.description?.trim() || null,
		important: s.important ?? false,
		hidden: s.hidden ?? false,
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
	if (def.kind === 'formula' && isPattern(def.name)) {
		out.push('A computed metric has a plain name, without * or {…}.');
	}
	// No formula yet is fine: the metric exists, without values.
	if (def.kind === 'formula' && def.spec.expression?.trim()) {
		try {
			parseFormula(def.spec.expression);
		} catch (err) {
			out.push(err instanceof FormulaError ? `Formula: ${err.message}.` : 'Invalid formula.');
		}
	}
	return out;
}

export class Registry {
	/** Valid definitions of the project, then built-in ones. */
	readonly defs: MetricDef[];
	/** Built-in patterns, then the project's: least specific first. */
	private builtinPatterns: MetricDef[];
	private patterns: MetricDef[];
	private exact = new Map<string, MetricDef>();
	private builtinExact = new Map<string, MetricDef>();
	private formulas = new Map<string, Formula>();
	private formulaDefs = new Map<string, MetricDef>();
	private metaCache = new Map<string, Meta>();

	constructor(defs: MetricDef[], builtin: MetricDef[] = BUILTIN) {
		const valid = defs.filter((d) => problems(d).length === 0);
		this.defs = [...valid, ...builtin];
		for (const d of valid) {
			if (d.kind === 'formula') {
				if (d.spec.expression?.trim()) {
					this.formulas.set(d.name, parseFormula(d.spec.expression));
				}
				this.formulaDefs.set(d.name, d);
			} else if (!isPattern(d.name) && !this.exact.has(d.name)) this.exact.set(d.name, d);
		}
		for (const d of builtin) if (!isPattern(d.name)) this.builtinExact.set(d.name, d);
		// Least specific first, so more specific ones override them; among equally specific
		// ones, earlier definitions win.
		const rank = (d: MetricDef) => compile(d.name).literal;
		const byRank = (list: MetricDef[]) =>
			list
				.map((d, i) => ({ d, i }))
				.sort((a, b) => rank(a.d) - rank(b.d) || b.i - a.i)
				.map(({ d }) => d);
		this.builtinPatterns = byRank(builtin.filter((d) => isPattern(d.name)));
		this.patterns = byRank(valid.filter((d) => d.kind !== 'formula' && isPattern(d.name)));
	}

	/** Names of computed metrics. */
	get computedNames(): string[] {
		return [...this.formulas.keys()];
	}

	get families(): MetricDef[] {
		return this.defs.filter((d) => d.kind === 'family');
	}

	isComputed(key: string): boolean {
		return this.formulas.has(key);
	}

	/**
	 * Description of a metric, field by field: its own definition, then a computed
	 * metric's, then patterns (the most specific first), then built-in defaults.
	 */
	describe(key: string): Meta {
		let meta = this.metaCache.get(key);
		if (meta) return meta;
		const own = this.exact.get(key) ?? null;
		const specs = [
			...this.inheritedSpecs(key),
			this.formulaDefs.get(key)?.spec ?? {},
			own?.spec ?? {}
		];
		meta = metaOf(mergeSpecs(specs), own ?? this.formulaDefs.get(key) ?? null);
		this.metaCache.set(key, meta);
		return meta;
	}

	/** Built-in descriptions, then the project's patterns, least specific first. */
	private inheritedSpecs(key: string): Spec[] {
		return [
			...this.builtinPatterns.filter((d) => match(d.name, key)).map((d) => d.spec),
			this.builtinExact.get(key)?.spec ?? {},
			...this.patterns.filter((d) => d.name !== key && match(d.name, key)).map((d) => d.spec)
		];
	}

	/** The description `key` gets without its own definition (for "inherited" hints). */
	inherited(key: string): Meta {
		return metaOf(mergeSpecs(this.inheritedSpecs(key)), null);
	}

	/** The metrics of one run as shown: renamed (and rescaled), plus the computed ones. */
	forRun(rawKeys: Iterable<string>, renames: Renames = {}): RunMetrics {
		return new RunMetrics(this, rawKeys, renames);
	}

	formula(name: string): Formula | undefined {
		return this.formulas.get(name);
	}
}

export class RunMetrics {
	/** Shown name -> logged name and factor. */
	readonly shown = new Map<string, { raw: string; scale: number }>();
	private cache = new Map<string, Resolution | null>();

	constructor(
		private registry: Registry,
		rawKeys: Iterable<string>,
		{ renames = {}, rename_scales = {} }: Renames
	) {
		for (const raw of rawKeys) {
			this.shown.set(own(renames, raw) ?? raw, { raw, scale: own(rename_scales, raw) ?? 1 });
		}
	}

	/** Shown names, logged and computed (hidden ones left out, unless asked). */
	keys(withHidden = false): string[] {
		const out = [...this.shown.keys()];
		for (const name of this.registry.computedNames) if (this.resolve(name)) out.push(name);
		return withHidden ? out : out.filter((k) => !this.registry.describe(k).hidden);
	}

	/** How to read `key` for this run, or null if the run doesn't have it. */
	resolve(key: string): Resolution | null {
		if (this.cache.has(key)) return this.cache.get(key)!;
		let r: Resolution | null = null;
		const shown = this.shown.get(key);
		if (shown && !this.registry.isComputed(key)) {
			r = { kind: 'key', ...shown };
		} else if (this.registry.formula(key)) {
			const formula = this.registry.formula(key)!;
			const refs = new Map<string, { raw: string; scale: number }>();
			for (const ref of allReferences(formula)) {
				// A formula reads logged (or renamed) metrics, not other formulas.
				const inner = this.registry.isComputed(ref) ? null : this.resolve(ref);
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

	/** Which logged metric a shown one reads, when it isn't itself (for hints). */
	source(key: string): string | null {
		const r = this.resolve(key);
		return r?.kind === 'key' && r.raw !== key ? r.raw : null;
	}
}
