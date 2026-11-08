<script lang="ts">
	import Boxes from '@lucide/svelte/icons/boxes';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import Search from '@lucide/svelte/icons/search';
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';

	import { apiJson } from '#lib/api/client.js';
	import type { ModelDescription } from '#lib/api/types.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as ToggleGroup from '#lib/components/ui/toggle-group/index.js';
	import { Toggle } from '#lib/components/ui/toggle/index.js';
	import {
		allKeys,
		expandRepeats,
		hasRepeats,
		initiallyExpanded,
		modelRows
	} from '#lib/runs/model-tree.js';
	import { onRestored } from '#lib/connection.svelte.js';
	import { BYTES, COUNT, FLOPS, formatUnit } from '#lib/units.js';
	import CodeBlock from './CodeBlock.svelte';
	import EmptyCard from './EmptyCard.svelte';
	import ErrorCard from './ErrorCard.svelte';

	/** Seconds between looks while a run is going: a model can be described at any time. */
	const REFRESH_MS = 10_000;

	let {
		project,
		run,
		running
	}: {
		project: string;
		/** Hyphenated run id. */
		run: string;
		running: boolean;
	} = $props();

	// Effects see the id only: the page reloads its run every few seconds.
	const runId = $derived(run);

	let models = $state.raw<Record<string, ModelDescription> | null>(null);
	let error = $state<unknown>(null);
	$effect(() => {
		const id = runId;
		const load = () =>
			apiJson<Record<string, ModelDescription>>(
				`/runs/${encodeURIComponent(id)}/models`
			).then(
				(found) => {
					if (id !== runId) return;
					models = found;
					error = null;
				},
				(err) => {
					// What's here stays through a failed refresh.
					if (!models) error = err;
				}
			);
		void load();
		const stopListening = onRestored(load);
		if (!running) return stopListening;
		const timer = setInterval(load, REFRESH_MS);
		return () => {
			stopListening();
			clearInterval(timer);
		};
	});

	const names = $derived(Object.keys(models ?? {}).sort());
	let chosen = $state<string | null>(null);
	const name = $derived(chosen && names.includes(chosen) ? chosen : (names[0] ?? null));
	const model = $derived(name ? models![name] : null);

	const count = (n: number) => formatUnit(n, COUNT);
	const exact = (n: number) => n.toLocaleString('en');
	const trainableShare = $derived(
		model && model.params > 0 ? (model.trainable_params / model.params) * 100 : 0
	);

	// The tree: what's open stays open as a running model is described again.
	let query = $state('');
	let expanded = $state.raw<Set<string> | null>(null);
	let openedFor = '';
	$effect(() => {
		if (!model || openedFor === `${runId}/${name}`) return;
		openedFor = `${runId}/${name}`;
		expanded = initiallyExpanded(model.tree);
		query = '';
	});
	/** Identical numbered neighbours show as one (`Block × 12`) unless asked otherwise. */
	let everyLayer = $state(false);
	const collapsible = $derived(model ? hasRepeats(model.tree) : false);
	const tree = $derived(model && everyLayer ? expandRepeats(model.tree) : model?.tree);
	const rows = $derived(tree && expanded ? modelRows(tree, expanded, query) : []);
	function toggle(key: string) {
		// eslint-disable-next-line svelte/prefer-svelte-reactivity -- replaced as a whole
		const next = new Set(expanded);
		if (next.has(key)) next.delete(key);
		else next.add(key);
		expanded = next;
	}

	const shape = (dims: number[] | null) =>
		dims ? dims.join(' × ') || 'scalar' : 'not created yet';

	const example = $derived(
		[
			'import torch',
			'import lsc',
			'',
			'model = torch.nn.Sequential(torch.nn.Linear(10, 32), torch.nn.ReLU(), torch.nn.Linear(32, 2))',
			'',
			`run = lsc.init(project=${JSON.stringify(project)})`,
			'# Layers and parameters; args and kwargs add shapes and FLOPs.',
			'run.describe_model(model, args=torch.zeros(1, 10))',
			'run.finish()'
		].join('\n')
	);
</script>

{#snippet figure(label: string, value: string, detail?: string, title?: string)}
	<div class="min-w-0 rounded-xl border bg-card p-4">
		<dt class="truncate text-sm text-muted-foreground">{label}</dt>
		<dd class="mt-1 truncate text-2xl font-semibold" {title}>{value}</dd>
		{#if detail}<dd class="mt-1 truncate text-xs text-muted-foreground">{detail}</dd>{/if}
	</div>
{/snippet}

{#if error}
	<ErrorCard {error} what="model information" />
{:else if !models}
	<Skeleton class="h-64 rounded-xl" />
{:else if !model}
	<EmptyCard icon={Boxes} title="No model described">
		{#snippet description()}
			Give the SDK any <code class="font-mono text-xs">torch.nn.Module</code> to record its layers,
			parameters and shapes. Its weights are never uploaded.
		{/snippet}
		<CodeBlock code={example} lang="python" />
	</EmptyCard>
{:else}
	{#if names.length > 1}
		<ToggleGroup.Root
			type="single"
			variant="outline"
			size="sm"
			class="mb-3 justify-start"
			value={name ?? ''}
			onValueChange={(v) => v && (chosen = v)}
			aria-label="Model"
		>
			{#each names as n (n)}
				<ToggleGroup.Item value={n} class="font-mono text-xs">{n}</ToggleGroup.Item>
			{/each}
		</ToggleGroup.Root>
	{/if}

	<dl class="mb-3 grid gap-4 sm:grid-cols-2 lg:grid-cols-4" aria-label="Model figures">
		{@render figure('Parameters', count(model.params), model.class, exact(model.params))}
		{@render figure(
			'Trainable',
			count(model.trainable_params),
			`${trainableShare.toFixed(trainableShare === 100 || trainableShare === 0 ? 0 : 1)}% of parameters`,
			exact(model.trainable_params)
		)}
		{@render figure(
			'Size in memory',
			formatUnit(model.size_bytes, BYTES),
			[...model.dtypes, ...model.devices].join(' · ')
		)}
		{#if model.flops !== undefined}
			{@render figure(
				'Flops',
				formatUnit(model.flops, FLOPS),
				'estimated, for this input',
				exact(model.flops)
			)}
		{/if}
	</dl>

	<div class="mb-3 grid gap-x-6 gap-y-1 text-sm sm:grid-cols-[auto_1fr]">
		{#if model.input !== undefined}
			<span class="text-muted-foreground">Input</span>
			<span class="min-w-0 font-mono text-xs leading-5 break-words">{model.input}</span>
		{/if}
		{#if model.output !== undefined}
			<span class="text-muted-foreground">Output</span>
			<span class="min-w-0 font-mono text-xs leading-5 break-words">{model.output}</span>
		{/if}
		{#if model.path}
			<span class="text-muted-foreground">Path</span>
			<span class="min-w-0 font-mono text-xs leading-5 break-all">{model.path}</span>
		{/if}
		{#if model.wrappers?.length}
			<span class="text-muted-foreground">Wrapped in</span>
			<span class="min-w-0 font-mono text-xs leading-5">{model.wrappers.join(', ')}</span>
		{/if}
		<span class="text-muted-foreground">Described at</span>
		<span class="text-xs leading-5 tabular-nums">step {exact(model.step)}</span>
	</div>

	{#if model.forward_error}
		<p class="mb-3 flex items-start gap-2 text-sm text-destructive">
			<TriangleAlert class="mt-0.5 size-4 shrink-0" />
			<span class="min-w-0 break-words">
				Running it for shapes failed: {model.forward_error}
			</span>
		</p>
	{/if}
	{#if model.uninitialized}
		<p class="mb-3 flex items-start gap-2 text-sm text-muted-foreground">
			<TriangleAlert class="mt-0.5 size-4 shrink-0" />
			{model.uninitialized} lazy parameters aren't created yet, so they count as 0. Describe it
			after a forward pass, or with an input.
		</p>
	{/if}

	<div class="mb-2 flex flex-wrap items-center gap-2">
		<div class="relative w-full sm:w-72">
			<Search
				class="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
			/>
			<Input
				class="pl-8"
				placeholder="Find a module"
				aria-label="Find a module"
				bind:value={query}
			/>
		</div>
		<Button
			variant="outline"
			size="sm"
			onclick={() => (expanded = allKeys(tree ?? model.tree))}
		>
			Expand all
		</Button>
		<Button
			variant="outline"
			size="sm"
			onclick={() => (expanded = initiallyExpanded(model.tree))}
		>
			Collapse
		</Button>
		{#if collapsible}
			<Toggle
				size="sm"
				variant="outline"
				bind:pressed={everyLayer}
				title="Identical blocks show as one (× 12). This lists each of them."
			>
				Every layer
			</Toggle>
		{/if}
	</div>

	<div class="overflow-x-auto rounded-xl border bg-card text-sm" role="tree" aria-label="Modules">
		<div
			class="grid min-w-[46rem] grid-cols-[minmax(0,1fr)_16rem_6rem_6rem] items-center gap-3 border-b bg-muted/50 px-3 py-2 text-xs font-medium text-muted-foreground"
		>
			<span>Module</span>
			<span>Input → output</span>
			<span class="text-right">Parameters</span>
			<span>Share</span>
		</div>
		{#each rows as row (row.key)}
			<div
				class="grid min-h-8 min-w-[46rem] grid-cols-[minmax(0,1fr)_16rem_6rem_6rem] items-center gap-3 border-b px-3 py-1 last:border-b-0 hover:bg-muted/50"
				role="treeitem"
				aria-selected="false"
				aria-expanded={row.expandable ? row.expanded : undefined}
				tabindex="-1"
			>
				<div
					class="flex min-w-0 items-center gap-1.5"
					style:padding-left={`${row.depth * 1.25}rem`}
				>
					{#if row.expandable}
						<button
							class="flex size-5 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-muted"
							aria-label={row.expanded ? 'Collapse' : 'Expand'}
							onclick={() => toggle(row.key)}
						>
							<ChevronRight
								class={[
									'size-3.5 transition-transform',
									row.expanded && 'rotate-90'
								]}
							/>
						</button>
					{:else}
						<span class="size-5 shrink-0"></span>
					{/if}
					{#if row.node}
						{@const node = row.node}
						{#if node.name}
							<span class="shrink-0 font-mono text-xs"
								>{node.name}{#if node.last}…{node.last}{/if}</span
							>
						{/if}
						<span class="shrink-0 font-medium">{node.class}</span>
						{#if node.repeat}
							<span
								class="shrink-0 rounded bg-secondary px-1 text-[11px] tabular-nums"
								title="Identical modules, shown as one">× {node.repeat}</span
							>
						{/if}
						{#if node.calls}
							<span
								class="shrink-0 text-xs text-muted-foreground"
								title="Called this many times in the forward pass"
								>called {node.calls} times</span
							>
						{/if}
						{#if node.shared_with}
							<span class="truncate text-xs text-muted-foreground"
								>same module as {node.shared_with}</span
							>
						{:else if node.params > 0 && node.trainable === 0}
							<span class="shrink-0 rounded bg-secondary px-1 text-[11px]"
								>frozen</span
							>
						{:else if node.trainable < node.params}
							<span class="shrink-0 rounded bg-secondary px-1 text-[11px]"
								>partly frozen</span
							>
						{/if}
						{#if node.extra}
							<span
								class="min-w-0 truncate font-mono text-xs text-muted-foreground"
								title={node.extra}>{node.extra}</span
							>
						{/if}
					{:else if row.tensor}
						<span class="shrink-0 font-mono text-xs text-muted-foreground"
							>{row.tensor.name}</span
						>
						{#if row.tensor.buffer}
							<span class="shrink-0 rounded bg-secondary px-1 text-[11px]"
								>buffer</span
							>
						{:else if row.tensor.trainable === false}
							<span class="shrink-0 rounded bg-secondary px-1 text-[11px]"
								>frozen</span
							>
						{/if}
						{#if row.tensor.shared_with}
							<span class="truncate text-xs text-muted-foreground"
								>shared with {row.tensor.shared_with}</span
							>
						{/if}
					{:else if row.hiddenChildren}
						<span class="text-xs text-muted-foreground"
							>{exact(row.hiddenChildren)} more modules not recorded: this model is very
							large</span
						>
					{:else if row.hiddenRows}
						<span class="text-xs text-muted-foreground"
							>{exact(row.hiddenRows)} more rows not shown: search for a module, or collapse</span
						>
					{/if}
				</div>
				<span class="min-w-0 truncate font-mono text-xs text-muted-foreground">
					{#if row.tensor}
						{shape(row.tensor.shape)}
					{:else if row.node?.input !== undefined}
						<span title={`${row.node.input} → ${row.node.output ?? ''}`}
							>{row.node.input} → {row.node.output ?? ''}</span
						>
					{/if}
				</span>
				<span class="text-right font-mono text-xs tabular-nums">
					{#if row.node && row.node.params > 0}
						<span title={exact(row.node.params)}
							>{count(row.node.params)}{#if row.node.repeat}&nbsp;each{/if}</span
						>
					{/if}
				</span>
				<span>
					{#if row.node && row.node.params > 0 && model.params > 0 && row.depth > 0}
						{@const share = Math.min(1, (row.node.params * row.copies) / model.params)}
						<span
							class="flex items-center gap-1.5"
							title={`${(share * 100).toFixed(1)}% of the model`}
						>
							<span class="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
								<span
									class="block h-full rounded-full bg-primary/70"
									style:width={`${share * 100}%`}
								></span>
							</span>
							<span
								class="w-8 text-right text-[11px] text-muted-foreground tabular-nums"
								>{(share * 100).toFixed(0)}%</span
							>
						</span>
					{/if}
				</span>
			</div>
		{:else}
			<p class="px-3 py-4 text-sm text-muted-foreground">No module matches.</p>
		{/each}
	</div>
	{#if model.truncated}
		<p class="mt-2 text-xs text-muted-foreground">
			This model is very large: the deepest modules weren't recorded. Totals are complete.
		</p>
	{/if}
{/if}
