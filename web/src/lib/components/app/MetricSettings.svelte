<script lang="ts">
	import { Input } from '#lib/components/ui/input/index.js';
	import * as ToggleGroup from '#lib/components/ui/toggle-group/index.js';
	import { directionLabel, summaryLabel } from '#lib/metrics/labels.js';
	import { Registry, type Direction, type Meta } from '#lib/metrics/registry.js';
	import { projectMetrics } from '#lib/metrics/store.svelte.js';
	import UnitPicker from './UnitPicker.svelte';

	let {
		name,
		canEdit
	}: {
		/** A metric, or a group's pattern (`val/*`). */
		name: string;
		canEdit: boolean;
	} = $props();

	/** What a group starts from: nothing. */
	const BLANK: Meta = new Registry([], []).describe('');
	const group = $derived(name.endsWith('/*'));
	const own = $derived(projectMetrics.own(name));
	const spec = $derived(own?.spec ?? {});
	const inherited = $derived(group ? BLANK : projectMetrics.registry.inherited(name));
	const disabled = $derived(!canEdit);
	const patch = (change: Parameters<typeof projectMetrics.patch>[1]) =>
		projectMetrics.patch(name, change);

	// Range ends are saved when they are numbers (or empty: open).
	const parseEnd = (text: string): number | null | undefined => {
		if (!text.trim()) return null;
		const n = Number(text);
		return Number.isFinite(n) ? n : undefined;
	};
	function setBound(end: 0 | 1, text: string) {
		const value = parseEnd(text);
		if (value === undefined) return;
		const bounds: [number | null, number | null] = [...(spec.bounds ?? [null, null])];
		bounds[end] = value;
		patch({ bounds: bounds[0] === null && bounds[1] === null ? undefined : bounds });
	}
</script>

{#snippet field(label: string, hint?: string)}
	<div class="pt-1.5 text-sm font-medium">
		{label}
		{#if hint}<p class="text-xs font-normal text-muted-foreground">{hint}</p>{/if}
	</div>
{/snippet}

<div class="grid gap-x-6 gap-y-5 sm:grid-cols-[10rem_1fr]">
	{#if !group}
		{@render field('Display name', 'Shown instead of the name')}
		<Input
			aria-label="Display name"
			placeholder={inherited.label ?? name}
			{disabled}
			value={spec.label ?? ''}
			onchange={(e) => patch({ label: e.currentTarget.value.trim() || undefined })}
		/>
		{@render field('Description', 'Shown when hovering its chart')}
		<Input
			aria-label="Description"
			placeholder={inherited.description ?? ''}
			{disabled}
			value={spec.description ?? ''}
			onchange={(e) => patch({ description: e.currentTarget.value.trim() || undefined })}
		/>
	{/if}

	{@render field('Better', 'Which way is better')}
	<ToggleGroup.Root
		type="single"
		variant="outline"
		size="sm"
		class="justify-self-start"
		value={spec.direction ?? 'auto'}
		onValueChange={(v) =>
			v && patch({ direction: v === 'auto' ? undefined : (v as Direction) })}
		{disabled}
		aria-label="Better"
	>
		<ToggleGroup.Item value="auto">
			Auto <span class="text-muted-foreground">({directionLabel(inherited.direction)})</span>
		</ToggleGroup.Item>
		<ToggleGroup.Item value="max">↑ Higher</ToggleGroup.Item>
		<ToggleGroup.Item value="min">↓ Lower</ToggleGroup.Item>
	</ToggleGroup.Root>

	{@render field('Summary', 'The value of a run in tables')}
	<ToggleGroup.Root
		type="single"
		variant="outline"
		size="sm"
		class="justify-self-start"
		value={spec.summary ?? 'auto'}
		onValueChange={(v) =>
			v && patch({ summary: v === 'auto' ? undefined : (v as 'last' | 'best') })}
		{disabled}
		aria-label="Summary"
	>
		<ToggleGroup.Item value="auto">
			Auto <span class="text-muted-foreground">({summaryLabel(inherited.summary)})</span>
		</ToggleGroup.Item>
		<ToggleGroup.Item value="last">Last</ToggleGroup.Item>
		<ToggleGroup.Item value="best" title="Needs a direction">Best</ToggleGroup.Item>
	</ToggleGroup.Root>

	{@render field('Unit')}
	<UnitPicker
		value={spec.unit}
		inherited={inherited.unitSpec}
		{disabled}
		onChange={(unit) => patch({ unit })}
	/>

	{@render field('Range', 'Fixes the y axis; an empty end stays open')}
	<div class="flex items-center gap-2">
		<Input
			class="w-28"
			aria-label="Lower bound"
			placeholder={inherited.bounds?.[0]?.toString() ?? 'min'}
			{disabled}
			value={spec.bounds?.[0] ?? ''}
			onchange={(e) => setBound(0, e.currentTarget.value)}
		/>
		<span class="text-sm text-muted-foreground">to</span>
		<Input
			class="w-28"
			aria-label="Upper bound"
			placeholder={inherited.bounds?.[1]?.toString() ?? 'max'}
			{disabled}
			value={spec.bounds?.[1] ?? ''}
			onchange={(e) => setBound(1, e.currentTarget.value)}
		/>
	</div>
</div>
