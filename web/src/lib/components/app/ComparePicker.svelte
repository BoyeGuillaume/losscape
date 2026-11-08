<script lang="ts">
	import Flag from '@lucide/svelte/icons/flag';
	import Minus from '@lucide/svelte/icons/minus';
	import Plus from '@lucide/svelte/icons/plus';
	import Replace from '@lucide/svelte/icons/replace';
	import X from '@lucide/svelte/icons/x';
	import { resolve } from '$app/paths';

	import type { RunSummary } from '#lib/api/types.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { FilterError, matches, parseFilter } from '#lib/filter.js';
	import { buildCatalog } from '#lib/runs/catalog.js';
	import type { Selection } from '#lib/runs/selection.svelte.js';
	import FilterInput from './FilterInput.svelte';

	let {
		project,
		runs,
		selection,
		picked,
		max,
		reference,
		colors,
		onReference
	}: {
		project: string;
		/** Every run of the project (summaries as shown). */
		runs: RunSummary[];
		selection: Selection;
		/** Picked runs that exist, in the order picked. */
		picked: RunSummary[];
		/** Runs compared at most (the first picked). */
		max: number;
		reference: string | null;
		colors: Map<string, string>;
		onReference: (id: string) => void;
	} = $props();

	let filterText = $state('');
	const parsed = $derived.by(
		(): { node: ReturnType<typeof parseFilter>; error: FilterError | null } => {
			try {
				return { node: parseFilter(filterText), error: null };
			} catch (err) {
				if (!(err instanceof FilterError)) throw err;
				return { node: null, error: err };
			}
		}
	);
	const matching = $derived(
		parsed.node && !parsed.error ? runs.filter((r) => matches(parsed.node, r)) : []
	);
	const newOnes = $derived(matching.filter((r) => !selection.has(r.id)).length);
	const pickedMatching = $derived(matching.length - newOnes);
	const catalog = $derived(buildCatalog(runs));
	const ids = (list: RunSummary[]) => list.map((r) => r.id);

	const runHref = (r: RunSummary) =>
		resolve('/p/[project]/r/[run]', { project, run: r.id.replaceAll('-', '') });
</script>

<div class="grid gap-3">
	<div class="grid gap-1">
		<!-- The buttons beside the input: its suggestions open below it. -->
		<div class="flex flex-wrap items-center gap-2">
			<FilterInput
				bind:value={filterText}
				catalog={() => catalog}
				invalid={!!parsed.error}
				describedby="compare-filter-help"
			/>
			<Button
				size="sm"
				variant="outline"
				disabled={!newOnes}
				onclick={() => selection.toggle(ids(matching), true)}
				title="Add the matching runs to the picked ones"
			>
				<Plus /> Add {newOnes || ''}
			</Button>
			<Button
				size="sm"
				variant="outline"
				disabled={!matching.length}
				onclick={() => selection.replace(ids(matching))}
				title="Pick exactly the matching runs"
			>
				<Replace /> Replace
			</Button>
			<Button
				size="sm"
				variant="outline"
				disabled={!pickedMatching}
				onclick={() => selection.toggle(ids(matching), false)}
				title="Remove the matching runs from the picked ones"
			>
				<Minus /> Remove {pickedMatching || ''}
			</Button>
		</div>
		<p id="compare-filter-help" class="text-xs text-muted-foreground">
			{#if parsed.error}
				<span class="text-destructive">{parsed.error.message}</span>
			{:else if filterText.trim()}
				<span class="tabular-nums">
					{matching.length.toLocaleString('en')}
					{matching.length === 1
						? 'run matches'
						: 'runs match'}{#if pickedMatching}{` (${pickedMatching} already picked)`}{/if}.
				</span>
			{:else}
				Type a filter, like <span class="font-mono">config.model.name = "emb1"</span> or
				<span class="font-mono">tag:candidate</span>, to add, replace or remove the runs it
				matches.
			{/if}
		</p>
	</div>

	{#if picked.length}
		<div class="flex flex-wrap items-center gap-1.5">
			{#each picked as run, i (run.id)}
				{@const isRef = run.id === reference}
				{@const shown = i < max}
				<span
					class={[
						'inline-flex h-7 items-center gap-1.5 rounded-md border pr-1 pl-2 text-sm',
						isRef && 'border-foreground/40 bg-muted',
						!shown && 'opacity-50'
					]}
					title={shown ? undefined : `Only the first ${max} picked runs are compared`}
				>
					<span
						class="size-2.5 shrink-0 rounded-full"
						style:background-color={colors.get(run.id) ?? 'var(--muted-foreground)'}
					></span>
					<a class="max-w-48 truncate hover:underline" href={runHref(run)}>{run.name}</a>
					{#if isRef}
						<span class="text-xs text-muted-foreground">reference</span>
					{:else if shown && picked.length > 1}
						<button
							class="rounded-sm p-0.5 text-muted-foreground hover:bg-background hover:text-foreground"
							title="Compare the others to this run"
							aria-label={`Use ${run.name} as the reference`}
							onclick={() => onReference(run.id)}
						>
							<Flag class="size-3.5" />
						</button>
					{/if}
					<button
						class="rounded-sm p-0.5 text-muted-foreground hover:bg-background hover:text-foreground"
						aria-label={`Remove ${run.name}`}
						onclick={() => selection.toggle([run.id], false)}
					>
						<X class="size-3.5" />
					</button>
				</span>
			{/each}
			<Button variant="ghost" size="sm" class="h-7" onclick={() => selection.clear()}>
				Clear
			</Button>
		</div>
	{/if}
</div>
