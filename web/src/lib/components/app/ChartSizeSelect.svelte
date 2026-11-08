<script lang="ts">
	import * as Select from '#lib/components/ui/select/index.js';
	import { CHART_SIZES, chartSize, type ChartSizeName } from '#lib/charts/size.svelte.js';

	const sizes = Object.entries(CHART_SIZES) as [
		ChartSizeName,
		(typeof CHART_SIZES)[ChartSizeName]
	][];
</script>

<Select.Root
	type="single"
	value={chartSize.name}
	onValueChange={(name) => (chartSize.name = name as ChartSizeName)}
>
	<Select.Trigger size="sm" class="w-32" aria-label="Chart size">
		<span class="truncate"
			><span class="text-muted-foreground">Size</span>
			{CHART_SIZES[chartSize.name].label}</span
		>
	</Select.Trigger>
	<Select.Content>
		{#each sizes as [name, size] (name)}
			<Select.Item value={name} label={size.label} />
		{/each}
	</Select.Content>
</Select.Root>
