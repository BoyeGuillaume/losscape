<script lang="ts">
	import Search from '@lucide/svelte/icons/search';

	import type { RunSummary } from '#lib/api/types.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as ToggleGroup from '#lib/components/ui/toggle-group/index.js';
	import { formatValue } from '#lib/format.js';
	import { configDiff } from '#lib/runs/compare.js';

	let {
		runs,
		reference,
		colors
	}: {
		/** Compared runs, the reference first. */
		runs: RunSummary[];
		reference: string | null;
		colors: Map<string, string>;
	} = $props();

	let show = $state<'differ' | 'all'>('differ');
	let query = $state('');
	const rows = $derived(
		configDiff(
			runs.map((r) => r.config),
			Math.max(
				0,
				runs.findIndex((r) => r.id === reference)
			)
		)
	);
	const differing = $derived(rows.filter((r) => r.differs).length);
	const shown = $derived(
		rows.filter(
			(r) =>
				(show === 'all' || r.differs) &&
				r.key.toLowerCase().includes(query.trim().toLowerCase())
		)
	);
	const text = (v: unknown) =>
		v === undefined
			? '—'
			: typeof v === 'string'
				? v
				: Array.isArray(v)
					? JSON.stringify(v)
					: formatValue(v);
</script>

<div class="grid gap-3">
	<div class="flex flex-wrap items-center gap-2">
		<div class="relative w-full sm:w-64">
			<Search
				class="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
			/>
			<Input
				class="h-8 pl-8"
				placeholder="Find a key"
				aria-label="Find a config key"
				bind:value={query}
			/>
		</div>
		<ToggleGroup.Root
			type="single"
			variant="outline"
			size="sm"
			value={show}
			onValueChange={(v) => v && (show = v as typeof show)}
			aria-label="Config keys"
		>
			<ToggleGroup.Item value="differ">Different ({differing})</ToggleGroup.Item>
			<ToggleGroup.Item value="all">All ({rows.length})</ToggleGroup.Item>
		</ToggleGroup.Root>
		<span class="text-xs text-muted-foreground">
			<span
				class="inline-block size-2.5 rounded-sm bg-amber-200 align-middle dark:bg-amber-500/30"
			></span>
			differs from the reference
		</span>
	</div>

	{#if rows.length === 0}
		<p class="text-sm text-muted-foreground">These runs have no config.</p>
	{:else if shown.length === 0}
		<p class="text-sm text-muted-foreground">
			{show === 'differ' && !query.trim()
				? 'Every key has the same value in these runs.'
				: 'No key matches.'}
		</p>
	{:else}
		<div class="max-h-[32rem] overflow-auto rounded-lg border">
			<table class="w-full font-mono text-xs">
				<thead class="sticky top-0 z-10 bg-muted text-muted-foreground">
					<tr>
						<th class="sticky left-0 bg-muted px-3 py-2 text-left font-sans font-medium"
							>Key</th
						>
						{#each runs as run (run.id)}
							<th class="px-3 py-2 text-left font-sans font-medium whitespace-nowrap">
								<span class="inline-flex items-center gap-1.5">
									<span
										class="size-2.5 rounded-full"
										style:background-color={colors.get(run.id)}
									></span>
									{run.name}
									{#if run.id === reference}<span class="font-normal"
											>(reference)</span
										>{/if}
								</span>
							</th>
						{/each}
					</tr>
				</thead>
				<tbody>
					{#each shown as row (row.key)}
						<tr class="border-t">
							<td class="sticky left-0 bg-card px-3 py-1.5 whitespace-nowrap"
								>{row.key}</td
							>
							{#each row.values as v, i (i)}
								<td
									class={[
										'max-w-72 truncate px-3 py-1.5',
										row.changed[i] && 'bg-amber-100 dark:bg-amber-500/20',
										v === undefined && 'text-muted-foreground'
									]}
									title={v === undefined ? 'Not set' : text(v)}>{text(v)}</td
								>
							{/each}
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</div>
