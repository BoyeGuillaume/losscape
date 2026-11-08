<script lang="ts">
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import ChevronsDownUp from '@lucide/svelte/icons/chevrons-down-up';
	import ChevronsUpDown from '@lucide/svelte/icons/chevrons-up-down';
	import Search from '@lucide/svelte/icons/search';
	import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal';
	import { SvelteSet } from 'svelte/reactivity';

	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as ToggleGroup from '#lib/components/ui/toggle-group/index.js';
	import {
		compact,
		countLeaves,
		filterConfig,
		isNode,
		splitKeyMatches,
		splitMatches,
		type Json
	} from '#lib/config-view.js';
	import CodeBlock from './CodeBlock.svelte';
	import EmptyCard from './EmptyCard.svelte';

	/** Configs larger than this open with their top-level sections collapsed. */
	const LARGE = 300;

	let {
		project,
		config,
		view = $bindable('tree')
	}: {
		project: string;
		config: Record<string, unknown>;
		/** A collapsible tree, or YAML. */
		view?: 'tree' | 'yaml';
	} = $props();

	let query = $state('');
	const searching = $derived(query.trim() !== '');
	const total = $derived(countLeaves(config as Json));
	const shown = $derived(filterConfig(config as Json, query));
	const matched = $derived(shown === undefined ? 0 : countLeaves(shown));

	/** Paths of collapsed sections (everything is open while searching). */
	const closed = new SvelteSet<string>();
	/** Collapsible sections, as JSON arrays of their keys. */
	const nodePaths = (value: Json, keys: string[] = []): string[] => {
		if (!isNode(value)) return [];
		const entries = Array.isArray(value)
			? value.map((v, i): [string, Json] => [String(i), v])
			: Object.entries(value);
		return entries.flatMap(([k, v]) =>
			isNode(v) ? [JSON.stringify([...keys, k]), ...nodePaths(v, [...keys, k])] : []
		);
	};
	let started = false;
	$effect(() => {
		if (started) return;
		started = true;
		if (total > LARGE) {
			for (const p of nodePaths(config as Json))
				if (JSON.parse(p).length === 1) closed.add(p);
		}
	});

	// YAML is written on demand (the library loads with the first YAML view).
	let yaml = $state<string | null>(null);
	$effect(() => {
		if (view !== 'yaml') return;
		const value = shown;
		let current = true;
		import('yaml').then(({ stringify }) => {
			if (current)
				yaml = value === undefined ? '' : stringify(compact(value), { lineWidth: 0 });
		});
		return () => (current = false);
	});

	/** Entries with their keys or indices (filtered lists have holes: they are skipped). */
	const entriesOf = (value: Record<string, Json> | Json[]): [string, Json][] =>
		Object.entries(value);

	/** Values colored by type, like the YAML view. */
	function leaf(value: Json): { text: string; color?: string } {
		if (typeof value === 'string') return { text: `"${value}"`, color: 'var(--ansi-2)' };
		if (typeof value === 'number') return { text: String(value), color: 'var(--ansi-4)' };
		if (value === null || typeof value === 'boolean')
			return { text: String(value), color: 'var(--ansi-5)' };
		if (Array.isArray(value))
			return { text: `[${value.map((v) => JSON.stringify(v)).join(', ')}]` };
		return { text: JSON.stringify(value) };
	}

	const example = $derived(
		[
			'import losscape',
			'',
			'config = {"lr": 3e-4, "batch_size": 128, "model": {"depth": 18}}',
			`run = losscape.init(project=${JSON.stringify(project)}, config=config)`,
			'run.config.update({"epochs": 10})  # more keys while the run goes',
			'run.finish()'
		].join('\n')
	);
</script>

{#snippet marked(
	parts: { text: string; match: boolean }[]
)}{#each parts as part, i (i)}{#if part.match}<mark
				class="rounded-sm bg-yellow-300/60 text-inherit dark:bg-yellow-500/35"
				>{part.text}</mark
			>{:else}{part.text}{/if}{/each}{/snippet}

{#snippet node(keys: string[], value: Json, depth: number)}
	{@const path = JSON.stringify(keys)}
	{#if isNode(value)}
		{@const open = searching || !closed.has(path)}
		{@const entries = entriesOf(value)}
		<div role="treeitem" aria-expanded={open} aria-selected="false">
			<button
				class="flex w-full items-center gap-1 rounded-md py-0.5 pr-2 text-left hover:bg-muted/60"
				style:padding-left={`${depth * 16}px`}
				onclick={() => (closed.has(path) ? closed.delete(path) : closed.add(path))}
				disabled={searching}
			>
				<ChevronRight
					class={[
						'size-3.5 shrink-0 text-muted-foreground transition-transform',
						open && 'rotate-90'
					]}
				/>
				<span class="font-medium">{@render marked(splitKeyMatches(keys, query))}</span>
				<span class="text-muted-foreground">
					{Array.isArray(value)
						? `[${entries.length} ${entries.length === 1 ? 'item' : 'items'}]`
						: `{${entries.length} ${entries.length === 1 ? 'key' : 'keys'}}`}
				</span>
			</button>
			{#if open}
				<div role="group">
					{#each entries as [k, v] (k)}
						{@render node([...keys, k], v, depth + 1)}
					{/each}
				</div>
			{/if}
		</div>
	{:else}
		{@const l = leaf(value)}
		<div
			role="treeitem"
			aria-selected="false"
			class="flex gap-2 rounded-md py-0.5 pr-2 hover:bg-muted/60"
			style:padding-left={`${depth * 16 + 18}px`}
		>
			<span class="shrink-0">{@render marked(splitKeyMatches(keys, query))}:</span>
			<span class="min-w-0 break-all" style:color={l.color}
				>{@render marked(splitMatches(l.text, query))}</span
			>
		</div>
	{/if}
{/snippet}

{#if total > 0}<div class="mb-3 flex flex-wrap items-center gap-2">
		<div class="relative w-full sm:w-80">
			<Search
				class="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
			/>
			<Input
				class="pl-8"
				placeholder="Search keys, values or paths (optimizer.lr)"
				aria-label="Search config"
				bind:value={query}
			/>
		</div>
		<span class="text-sm text-muted-foreground tabular-nums">
			{searching ? `${matched} of ${total} values` : `${total} values`}
		</span>
		<div class="ml-auto flex items-center gap-2">
			{#if view === 'tree' && !searching}
				<Button variant="ghost" size="sm" onclick={() => closed.clear()} title="Expand all">
					<ChevronsUpDown /> Expand
				</Button>
				<Button
					variant="ghost"
					size="sm"
					onclick={() => nodePaths(config as Json).forEach((p) => closed.add(p))}
					title="Collapse all"
				>
					<ChevronsDownUp /> Collapse
				</Button>
			{/if}
			<ToggleGroup.Root
				type="single"
				variant="outline"
				size="sm"
				value={view}
				onValueChange={(v) => v && (view = v as typeof view)}
				aria-label="Config view"
			>
				<ToggleGroup.Item value="tree">Tree</ToggleGroup.Item>
				<ToggleGroup.Item value="yaml">YAML</ToggleGroup.Item>
			</ToggleGroup.Root>
		</div>
	</div>{/if}

{#if total === 0}
	<EmptyCard icon={SlidersHorizontal} title="No config">
		{#snippet description()}
			This run was started without a config. Hyperparameters and settings passed as
			<code class="font-mono text-xs">config</code> show here, and become table columns, filters
			and groups on the project page.
		{/snippet}
		<CodeBlock lang="python" title="train.py" code={example} />
	</EmptyCard>
{:else if shown === undefined}
	<p class="text-sm text-muted-foreground">No key or value matches “{query.trim()}”.</p>
{:else if view === 'yaml'}
	{#if yaml !== null}
		<CodeBlock lang="yaml" title="config.yaml" code={yaml.trimEnd()} />
	{/if}
{:else}
	<div
		role="tree"
		aria-label="Config"
		class="rounded-lg border bg-muted/40 p-2 font-mono text-[13px] leading-6"
	>
		{#each entriesOf(shown as Record<string, Json>) as [k, v] (k)}
			{@render node([k], v, 0)}
		{/each}
	</div>
{/if}
