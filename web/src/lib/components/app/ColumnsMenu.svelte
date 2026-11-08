<script lang="ts">
	import Columns3 from '@lucide/svelte/icons/columns-3';

	import type { RunSummary } from '#lib/api/types.js';
	import { buttonVariants } from '#lib/components/ui/button/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { availableKeys, BASE_COLUMNS, column } from '#lib/runs/columns.js';

	let {
		runs,
		columns,
		customized,
		onSet,
		onReset
	}: {
		/** Where the config and summary keys to choose from come from. */
		runs: RunSummary[];
		columns: string[];
		/** Chosen by hand, so there's something to reset. */
		customized: boolean;
		onSet: (id: string, on: boolean) => void;
		onReset: () => void;
	} = $props();

	let query = $state('');
	const keys = $derived(availableKeys(runs));
	const choices = $derived.by(() => {
		const q = query.trim().toLowerCase();
		const groups: [string, string[]][] = [
			['Run', BASE_COLUMNS],
			['Config', keys.config.map((k) => `config.${k}`)],
			['Summary', keys.summary.map((k) => `summary.${k}`)]
		];
		return groups
			.map(([label, ids]): [string, string[]] => [
				label,
				ids.filter((id) => !q || id.toLowerCase().includes(q)).slice(0, 100)
			])
			.filter(([, ids]) => ids.length);
	});
</script>

<DropdownMenu.Root onOpenChange={(open) => !open && (query = '')}>
	<DropdownMenu.Trigger class={buttonVariants({ variant: 'outline', size: 'sm' })}>
		<Columns3 /> Columns
	</DropdownMenu.Trigger>
	<DropdownMenu.Content align="end" class="max-h-96 w-72">
		<div class="p-1">
			<Input
				class="h-8"
				placeholder="Find a column"
				aria-label="Find a column"
				bind:value={query}
				onkeydown={(e) => e.stopPropagation()}
			/>
		</div>
		{#each choices as [label, ids] (label)}
			<DropdownMenu.Group>
				<DropdownMenu.GroupHeading>{label}</DropdownMenu.GroupHeading>
				{#each ids as id (id)}
					<DropdownMenu.CheckboxItem
						checked={columns.includes(id)}
						closeOnSelect={false}
						onCheckedChange={(on) => onSet(id, on)}
					>
						<span class="truncate">{column(id).label}</span>
					</DropdownMenu.CheckboxItem>
				{/each}
			</DropdownMenu.Group>
		{:else}
			<p class="px-2 py-1.5 text-sm text-muted-foreground">No column matches.</p>
		{/each}
		{#if customized}
			<DropdownMenu.Separator />
			<DropdownMenu.Item onSelect={onReset}>Reset to defaults</DropdownMenu.Item>
		{/if}
	</DropdownMenu.Content>
</DropdownMenu.Root>
