<script lang="ts">
	import { Input } from '#lib/components/ui/input/index.js';
	import * as ToggleGroup from '#lib/components/ui/toggle-group/index.js';
	import {
		DEFAULT_PREFIXES,
		LOGGED_AS,
		UNIT_KINDS,
		loggedAs,
		prefixChoices,
		unitKind,
		unitLabel,
		type UnitKind
	} from '#lib/metrics/labels.js';
	import type { Prefixes } from '#lib/units.js';
	import type { UnitSpec } from '#lib/metrics/registry.js';

	let {
		value,
		inherited,
		disabled = false,
		onChange
	}: {
		/** The metric's own unit; undefined: inherited. */
		value: UnitSpec | undefined;
		/** What it gets without one. */
		inherited: UnitSpec | null;
		disabled?: boolean;
		onChange: (unit: UnitSpec | undefined) => void;
	} = $props();

	const kind = $derived(value ? unitKind(value) : null);
	const as = $derived(value ? loggedAs(value) : null);
	/** A factor that isn't one of the choices (or being typed). */
	let otherFactor = $state(false);
	const showFactor = $derived(!!value && (otherFactor || !as));

	/** The unit with these fields changed; a factor of 1 isn't stored. */
	function build(fields: Partial<UnitSpec>): UnitSpec {
		const out: UnitSpec = { ...value!, ...fields } as UnitSpec;
		if (out.scale === 1) delete out.scale;
		if (out.preset !== 'custom') delete out.suffix;
		// Prefixes only for units that take them, and only when not their default.
		if (out.prefixes === DEFAULT_PREFIXES[out.preset]) delete out.prefixes;
		if (!(out.preset in DEFAULT_PREFIXES)) delete out.prefixes;
		if (out.decimals === undefined) delete out.decimals;
		return out;
	}

	function setKind(next: string) {
		otherFactor = false;
		if (next === 'auto') return onChange(undefined);
		const first = LOGGED_AS[next as UnitKind][0];
		onChange(
			build({
				preset: first.preset,
				scale: first.scale,
				suffix: next === 'custom' ? (value?.suffix ?? '') : undefined
			})
		);
	}

	function setLoggedAs(label: string) {
		if (!label || !kind) return;
		if (label === 'other') {
			otherFactor = true;
			return;
		}
		otherFactor = false;
		const option = LOGGED_AS[kind].find((o) => o.label === label)!;
		onChange(build({ preset: option.preset, scale: option.scale }));
	}

	function setFactor(text: string) {
		const factor = Number(text);
		if (text.trim() && Number.isFinite(factor) && factor > 0)
			onChange(build({ scale: factor }));
	}
</script>

<div class="grid gap-2">
	<ToggleGroup.Root
		type="single"
		variant="outline"
		size="sm"
		class="flex-wrap"
		value={kind ?? 'auto'}
		onValueChange={(v) => v && setKind(v)}
		{disabled}
		aria-label="Unit"
	>
		<ToggleGroup.Item value="auto" title="Inherited from its group or the defaults">
			Auto <span class="text-muted-foreground">({unitLabel(inherited)})</span>
		</ToggleGroup.Item>
		{#each UNIT_KINDS as k (k.value)}
			<ToggleGroup.Item value={k.value}>{k.label}</ToggleGroup.Item>
		{/each}
	</ToggleGroup.Root>

	{#if value && kind}
		<div class="flex flex-wrap items-center gap-2 text-sm">
			{#if kind === 'custom'}
				<span class="text-muted-foreground">Suffix</span>
				<Input
					class="h-8 w-24"
					aria-label="Suffix"
					placeholder="px"
					{disabled}
					value={value.suffix ?? ''}
					onchange={(e) => onChange(build({ suffix: e.currentTarget.value }))}
				/>
			{/if}
			<span class="text-muted-foreground">Logged as</span>
			<ToggleGroup.Root
				type="single"
				variant="outline"
				size="sm"
				value={showFactor ? 'other' : (as?.label ?? '')}
				onValueChange={(v) => v && setLoggedAs(v)}
				{disabled}
				aria-label="Logged as"
			>
				{#each LOGGED_AS[kind] as option (option.label)}
					<ToggleGroup.Item value={option.label}>{option.label}</ToggleGroup.Item>
				{/each}
				<ToggleGroup.Item value="other" title="Values were logged times another factor"
					>other ×</ToggleGroup.Item
				>
			</ToggleGroup.Root>
			{#if showFactor}
				<Input
					class="h-8 w-28"
					aria-label="Factor"
					placeholder="e.g. 1000"
					title="Logged values times this are in the unit"
					{disabled}
					value={value.scale ?? ''}
					onchange={(e) => setFactor(e.currentTarget.value)}
				/>
			{/if}
		</div>
		{#if value.preset in DEFAULT_PREFIXES}
			<div class="flex flex-wrap items-center gap-2 text-sm">
				<span class="text-muted-foreground">Prefixes</span>
				<ToggleGroup.Root
					type="single"
					variant="outline"
					size="sm"
					class="flex-wrap"
					value={value.prefixes ?? DEFAULT_PREFIXES[value.preset]}
					onValueChange={(v) => v && onChange(build({ prefixes: v as Prefixes }))}
					{disabled}
					aria-label="Prefixes"
				>
					{#each prefixChoices(value.preset === 'watts' ? 'W' : (value.suffix ?? '')) as choice (choice.value)}
						<ToggleGroup.Item value={choice.value}>{choice.label}</ToggleGroup.Item>
					{/each}
				</ToggleGroup.Root>
			</div>
		{/if}
		<div class="flex flex-wrap items-center gap-2 text-sm">
			<span class="text-muted-foreground">Precision</span>
			<ToggleGroup.Root
				type="single"
				variant="outline"
				size="sm"
				value={value.decimals === undefined ? 'auto' : String(value.decimals)}
				onValueChange={(v) =>
					v && onChange(build({ decimals: v === 'auto' ? undefined : Number(v) }))}
				{disabled}
				aria-label="Precision"
			>
				<ToggleGroup.Item value="auto">Auto</ToggleGroup.Item>
				{#each [0, 1, 2, 3] as d (d)}
					<ToggleGroup.Item value={String(d)}
						>{d} {d === 1 ? 'decimal' : 'decimals'}</ToggleGroup.Item
					>
				{/each}
			</ToggleGroup.Root>
		</div>
	{/if}
</div>
