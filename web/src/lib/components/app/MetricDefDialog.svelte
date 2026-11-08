<script lang="ts" module>
	import type { Kind } from '#lib/metrics/registry.js';

	/** What each kind of definition is, for the editor and the list. */
	export const KINDS: Record<Kind, { title: string; help: string; placeholder: string }> = {
		describe: {
			title: 'Describe metrics',
			help: 'Unit, range and which way is better for a metric, or for every metric a pattern matches (* matches anything, {k} one path segment).',
			placeholder: 'val/acc or val/*'
		},
		merge: {
			title: 'Merge metrics',
			help: 'One metric read from the first of these metrics a run has: e.g. runs that logged val/pck-8 and runs that logged val/pck-8-fine, compared on one chart.',
			placeholder: 'val/pck-8px'
		},
		family: {
			title: 'Metric family',
			help: 'Metrics named by a parameter, e.g. val/pck-{k} for val/pck-8, val/pck-16, …: charted together as value against the parameter, and described alike.',
			placeholder: 'val/pck-{k}'
		},
		formula: {
			title: 'Formula',
			help: 'A metric computed from others, step by step: [metric] names a metric; + - * / ^, min, max, abs, sqrt, log, log10, exp.',
			placeholder: 'val/f1'
		}
	};
</script>

<script lang="ts">
	import Plus from '@lucide/svelte/icons/plus';
	import X from '@lucide/svelte/icons/x';
	import { untrack } from 'svelte';

	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { FormulaError, parseFormula, references } from '#lib/metrics/formula.js';
	import { isPattern, match } from '#lib/metrics/pattern.js';
	import {
		problems,
		unitOf,
		type Direction,
		type MetricDef,
		type Spec,
		type UnitPreset
	} from '#lib/metrics/registry.js';
	import { formatUnit } from '#lib/units.js';

	let {
		def,
		keys,
		taken,
		onSave,
		onCancel
	}: {
		/** The definition to edit (a new one has an empty name). */
		def: MetricDef;
		/** Metric names logged in the project, for suggestions and the preview. */
		keys: string[];
		/** Names of the project's other definitions. */
		taken: string[];
		onSave: (def: MetricDef) => void;
		onCancel: () => void;
	} = $props();

	const PRESETS: { value: UnitPreset; label: string }[] = [
		{ value: 'none', label: 'Plain number' },
		{ value: 'count', label: 'Count (1.2k, 3.4M)' },
		{ value: 'percent', label: 'Percent (logged 0–100)' },
		{ value: 'fraction', label: 'Percent (logged 0–1)' },
		{ value: 'bytes', label: 'Bytes (KiB, MiB, …)' },
		{ value: 'bps', label: 'Bits per second' },
		{ value: 'seconds', label: 'Duration (s, min, h)' },
		{ value: 'custom', label: 'Custom suffix' }
	];
	const DIRECTIONS = [
		{ value: '', label: 'Not set' },
		{ value: 'max', label: 'Higher is better' },
		{ value: 'min', label: 'Lower is better' }
	];

	// The form, as text where numbers may be left empty.
	// The dialog opens on one definition: its fields are copied once.
	const spec: Spec = untrack(() => def.spec);
	const initialName = untrack(() => def.name);
	let name = $state(initialName);
	let label = $state(spec.label ?? '');
	let description = $state(spec.description ?? '');
	let preset = $state<UnitPreset>(spec.unit?.preset ?? 'none');
	let unitScale = $state(spec.unit?.scale?.toString() ?? '');
	let suffix = $state(spec.unit?.suffix ?? '');
	let decimals = $state(spec.unit?.decimals?.toString() ?? '');
	let low = $state(spec.bounds?.[0]?.toString() ?? '');
	let high = $state(spec.bounds?.[1]?.toString() ?? '');
	let direction = $state<string>(spec.direction ?? '');
	let summary = $state<'last' | 'best'>(spec.summary ?? 'last');
	let sources = $state(
		(spec.sources?.length ? spec.sources : [{ key: '' }, { key: '' }]).map((s) => ({
			key: s.key,
			scale: s.scale?.toString() ?? ''
		}))
	);
	let expression = $state(spec.expression ?? '');
	let hideSources = $state(spec.hideSources ?? false);

	const info = $derived(KINDS[def.kind]);
	const number = (s: string): number | undefined => {
		const t = s.trim();
		return t === '' || !Number.isFinite(Number(t)) ? undefined : Number(t);
	};
	const badNumber = (s: string) => s.trim() !== '' && number(s) === undefined;

	/** The definition as the form stands. */
	const draft = $derived.by((): MetricDef => {
		const out: Spec = {};
		if (label.trim()) out.label = label.trim();
		if (description.trim()) out.description = description.trim();
		if (preset !== 'none') {
			out.unit = { preset };
			if (number(unitScale) !== undefined && number(unitScale) !== 1)
				out.unit.scale = number(unitScale);
			if (preset === 'custom') out.unit.suffix = suffix;
			if (number(decimals) !== undefined) out.unit.decimals = number(decimals);
		}
		const [lo, hi] = [number(low) ?? null, number(high) ?? null];
		if (lo !== null || hi !== null) out.bounds = [lo, hi];
		if (direction) out.direction = direction as Direction;
		if (direction && summary === 'best') out.summary = 'best';
		if (def.kind === 'merge') {
			out.sources = sources
				.filter((s) => s.key.trim())
				.map((s) =>
					number(s.scale) !== undefined && number(s.scale) !== 1
						? { key: s.key.trim(), scale: number(s.scale) }
						: { key: s.key.trim() }
				);
		}
		if (def.kind === 'formula') out.expression = expression.trim();
		if ((def.kind === 'merge' || def.kind === 'formula') && hideSources) out.hideSources = true;
		return { name: name.trim(), kind: def.kind, spec: out };
	});

	const issues = $derived.by(() => {
		const out = problems(draft);
		if (draft.name && draft.name !== initialName && taken.includes(draft.name))
			out.push(`${draft.name} is already defined.`);
		if ([unitScale, decimals, low, high, ...sources.map((s) => s.scale)].some(badNumber))
			out.push('Numbers only (or leave empty).');
		const [lo, hi] = [number(low), number(high)];
		if (lo !== undefined && hi !== undefined && lo >= hi)
			out.push('The lower bound is below the upper one.');
		return out;
	});
	// Show problems once the user has typed something (not on an empty new form).
	const touched = $derived(name.trim() !== '' || expression.trim() !== '');

	/** What the definition reads, for the preview. */
	const preview = $derived.by((): { text: string; keys: string[] } | null => {
		const n = draft.name;
		if (!n) return null;
		if (def.kind === 'describe' || def.kind === 'family') {
			const hits = isPattern(n)
				? keys.filter((k) => match(n, k))
				: keys.filter((k) => k === n);
			return {
				text: hits.length
					? `Matches ${hits.length} logged ${hits.length === 1 ? 'metric' : 'metrics'}`
					: 'Matches no logged metric yet',
				keys: hits
			};
		}
		const wanted =
			def.kind === 'merge'
				? (draft.spec.sources ?? []).map((s) => s.key)
				: (() => {
						try {
							return references(parseFormula(expression));
						} catch (err) {
							if (err instanceof FormulaError) return [];
							throw err;
						}
					})();
		const missing = [...wanted].filter((k) => !keys.includes(k));
		return missing.length
			? { text: `Not logged in this project yet: ${missing.join(', ')}`, keys: [] }
			: null;
	});

	/** Typical values in the unit, as they will show. */
	const SAMPLES: Record<UnitPreset, number[]> = {
		none: [],
		count: [0.5, 42, 12_345],
		percent: [0.5, 42, 100],
		fraction: [0.5, 42, 100],
		bytes: [512, 1.5 * 2 ** 20, 3.2 * 2 ** 30],
		bps: [800, 2.5e6, 1e9],
		seconds: [0.25, 95, 7300],
		custom: [0.5, 42, 12_345]
	};
	const unitExample = $derived.by(() => {
		const unit = unitOf(draft.spec.unit);
		if (!unit) return null;
		return SAMPLES[preset].map((v) => formatUnit(v / (unit.scale ?? 1), unit)).join(' · ');
	});

	function save(event: SubmitEvent) {
		event.preventDefault();
		if (!issues.length) onSave(draft);
	}
</script>

<datalist id="metric-keys">
	{#each keys as key (key)}<option value={key}></option>{/each}
</datalist>

<Dialog.Root open onOpenChange={(open) => !open && onCancel()}>
	<Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl">
		<form class="grid gap-4" onsubmit={save}>
			<Dialog.Header>
				<Dialog.Title>{info.title}</Dialog.Title>
				<Dialog.Description>{info.help}</Dialog.Description>
			</Dialog.Header>

			<div class="grid gap-1.5">
				<Label for="def-name">
					{def.kind === 'describe'
						? 'Metric or pattern'
						: def.kind === 'family'
							? 'Pattern'
							: 'Name of the new metric'}
				</Label>
				<Input
					id="def-name"
					class="font-mono"
					placeholder={info.placeholder}
					autocomplete="off"
					list={def.kind === 'describe' ? 'metric-keys' : undefined}
					bind:value={name}
				/>
				{#if preview}
					<p class="text-xs text-muted-foreground">
						{preview.text}{#if preview.keys.length}:
							<span class="font-mono"
								>{preview.keys.slice(0, 6).join(', ')}{preview.keys.length > 6
									? ', …'
									: ''}</span
							>{/if}
					</p>
				{/if}
			</div>

			{#if def.kind === 'merge'}
				<fieldset class="grid gap-1.5">
					<legend class="mb-1.5 text-sm font-medium">
						Sources, first found wins
						<span class="font-normal text-muted-foreground"
							>(scale: e.g. 100 for a source logged as a fraction)</span
						>
					</legend>
					{#each sources as source, i (i)}
						<div class="flex items-center gap-2">
							<Input
								class="font-mono"
								placeholder="val/pck-8"
								aria-label={`Source ${i + 1}`}
								list="metric-keys"
								autocomplete="off"
								bind:value={source.key}
							/>
							<Input
								class="w-24"
								placeholder="× 1"
								aria-label={`Scale of source ${i + 1}`}
								aria-invalid={badNumber(source.scale)}
								bind:value={source.scale}
							/>
							<Button
								variant="ghost"
								size="icon"
								class="shrink-0 text-muted-foreground"
								disabled={sources.length === 1}
								onclick={() => sources.splice(i, 1)}
								aria-label={`Remove source ${i + 1}`}
							>
								<X />
							</Button>
						</div>
					{/each}
					<Button
						variant="ghost"
						size="sm"
						class="justify-self-start"
						onclick={() => sources.push({ key: '', scale: '' })}
					>
						<Plus /> Add a source
					</Button>
				</fieldset>
			{:else if def.kind === 'formula'}
				<div class="grid gap-1.5">
					<Label for="def-expression">Formula</Label>
					<Input
						id="def-expression"
						class="font-mono"
						placeholder="2 * [val/p] * [val/r] / ([val/p] + [val/r])"
						autocomplete="off"
						bind:value={expression}
					/>
				</div>
			{/if}
			{#if def.kind === 'merge' || def.kind === 'formula'}
				<label class="flex items-center gap-2 text-sm">
					<input
						type="checkbox"
						class="size-4 accent-primary"
						bind:checked={hideSources}
					/>
					Hide the metrics it reads from charts and tables
				</label>
			{/if}

			<div class="grid gap-3 sm:grid-cols-2">
				<div class="grid gap-1.5">
					<Label for="def-label"
						>Display name <span class="font-normal text-muted-foreground"
							>(optional)</span
						></Label
					>
					<Input id="def-label" autocomplete="off" bind:value={label} />
				</div>
				<div class="grid gap-1.5">
					<Label for="def-description"
						>Description <span class="font-normal text-muted-foreground"
							>(optional)</span
						></Label
					>
					<Input id="def-description" autocomplete="off" bind:value={description} />
				</div>
			</div>

			<div class="grid gap-3 sm:grid-cols-2">
				<div class="grid gap-1.5">
					<Label>Unit</Label>
					<Select.Root type="single" bind:value={preset}>
						<Select.Trigger class="w-full"
							>{PRESETS.find((p) => p.value === preset)?.label}</Select.Trigger
						>
						<Select.Content>
							{#each PRESETS as p (p.value)}
								<Select.Item value={p.value} label={p.label} />
							{/each}
						</Select.Content>
					</Select.Root>
					{#if unitExample}
						<p class="text-xs text-muted-foreground">Shown as {unitExample}</p>
					{/if}
				</div>
				{#if preset !== 'none'}
					<div class="grid grid-cols-2 gap-2">
						{#if preset === 'custom'}
							<div class="col-span-2 grid gap-1.5">
								<Label for="def-suffix">Suffix</Label>
								<Input id="def-suffix" placeholder="px" bind:value={suffix} />
							</div>
						{/if}
						<div class="grid gap-1.5">
							<Label
								for="def-scale"
								title="Logged values times this are in the unit (1048576 for bytes logged in MiB)"
								>Logged ×</Label
							>
							<Input
								id="def-scale"
								placeholder="1"
								aria-invalid={badNumber(unitScale)}
								bind:value={unitScale}
							/>
						</div>
						<div class="grid gap-1.5">
							<Label for="def-decimals">Decimals</Label>
							<Input
								id="def-decimals"
								placeholder="auto"
								aria-invalid={badNumber(decimals)}
								bind:value={decimals}
							/>
						</div>
					</div>
				{/if}
			</div>

			<div class="grid gap-3 sm:grid-cols-2">
				<div class="grid gap-1.5">
					<Label>Better</Label>
					<Select.Root type="single" bind:value={direction}>
						<Select.Trigger class="w-full"
							>{DIRECTIONS.find((d) => d.value === direction)?.label}</Select.Trigger
						>
						<Select.Content>
							{#each DIRECTIONS as d (d.value)}
								<Select.Item value={d.value} label={d.label} />
							{/each}
						</Select.Content>
					</Select.Root>
				</div>
				<div class="grid gap-1.5">
					<Label>Summary value</Label>
					<Select.Root type="single" bind:value={summary} disabled={!direction}>
						<Select.Trigger class="w-full"
							>{summary === 'best' && direction
								? 'Best value'
								: 'Last value'}</Select.Trigger
						>
						<Select.Content>
							<Select.Item value="last" label="Last value" />
							<Select.Item value="best" label="Best value" />
						</Select.Content>
					</Select.Root>
				</div>
				<div class="grid gap-1.5 sm:col-span-2">
					<Label for="def-low"
						>Range <span class="font-normal text-muted-foreground"
							>(fixes the y axis; leave an end empty to keep it open)</span
						></Label
					>
					<div class="flex items-center gap-2">
						<Input
							id="def-low"
							class="w-28"
							placeholder="min"
							aria-label="Lower bound"
							aria-invalid={badNumber(low)}
							bind:value={low}
						/>
						<span class="text-muted-foreground">to</span>
						<Input
							class="w-28"
							placeholder="max"
							aria-label="Upper bound"
							aria-invalid={badNumber(high)}
							bind:value={high}
						/>
					</div>
				</div>
			</div>

			{#if touched && issues.length}
				<ul class="grid gap-0.5 text-sm text-destructive">
					{#each issues as issue (issue)}<li>{issue}</li>{/each}
				</ul>
			{/if}

			<Dialog.Footer>
				<Button variant="outline" type="button" onclick={onCancel}>Cancel</Button>
				<Button type="submit" disabled={issues.length > 0}>
					{initialName ? 'Save' : 'Add'}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
