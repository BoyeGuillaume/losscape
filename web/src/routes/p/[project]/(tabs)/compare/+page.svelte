<script lang="ts">
	import GitCompareArrows from '@lucide/svelte/icons/git-compare-arrows';
	import { page } from '$app/state';

	import type { RunSummary } from '#lib/api/types.js';
	import {
		DEFAULT_SETTINGS,
		type ChartSeries,
		type ChartSettings
	} from '#lib/charts/settings.js';
	import ChartsPanel from '#lib/components/app/ChartsPanel.svelte';
	import CompareMetricsTable from '#lib/components/app/CompareMetricsTable.svelte';
	import ComparePicker from '#lib/components/app/ComparePicker.svelte';
	import ConfigDiff from '#lib/components/app/ConfigDiff.svelte';
	import EmptyCard from '#lib/components/app/EmptyCard.svelte';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { withMetrics } from '#lib/metrics/apply.js';
	import { currentRegistry } from '#lib/metrics/store.svelte.js';
	import { compareMetrics } from '#lib/runs/compare.js';
	import { RunList } from '#lib/runs/list.svelte.js';
	import { selectionOf } from '#lib/runs/selection.svelte.js';
	import { param, syncUrl } from '#lib/url-state.svelte.js';

	/** Runs compared at most: the first picked ones (as many as a chart draws). */
	const MAX_COMPARED = 40;
	const shortId = (id: string) => id.replaceAll('-', '');

	const project = $derived(page.params.project ?? '');
	const projectPath = $derived(`/projects/${encodeURIComponent(project)}`);

	// ---- Runs and the selection (shared with the Runs tab) ----
	let list = $state<RunList | null>(null);
	$effect(() => {
		const l = new RunList(projectPath, false, (n) => (n > 5_000 ? 30_000 : 10_000));
		list = l;
		l.start();
		return () => l.stop();
	});
	const registry = $derived(currentRegistry());
	const all = $derived((list?.runs ?? []).map((r) => withMetrics(r, registry)));
	const byId = $derived(new Map(all.map((r) => [shortId(r.id), r])));

	const selection = $derived(selectionOf(project));
	// A link with runs compares exactly those.
	const linked = param('runs')?.split(',').filter(Boolean);
	if (linked?.length) selectionOf(page.params.project ?? '').replace(linked);
	let referenceId = $state<string | null>(param('ref'));

	/** Picked runs that are in the list (archived or deleted ones aren't), in order. */
	const picked = $derived(
		selection.ids.map((id) => byId.get(id)).filter((r): r is RunSummary => !!r)
	);
	const compared = $derived(picked.slice(0, MAX_COMPARED));
	const reference = $derived(
		compared.find((r) => shortId(r.id) === referenceId) ?? compared[0] ?? null
	);
	/** The reference first, then the others in the order picked. */
	const ordered = $derived(
		reference ? [reference, ...compared.filter((r) => r !== reference)] : compared
	);
	const colors = $derived(
		new Map(compared.map((r, i): [string, string] => [r.id, `var(--chart-${(i % 8) + 1})`]))
	);
	const metrics = $derived(compareMetrics(compared, registry));

	syncUrl(() => ({
		runs: selection.size && selection.size <= MAX_COMPARED ? selection.ids.join(',') : null,
		ref: reference && reference !== compared[0] ? shortId(reference.id) : null
	}));

	// ---- Charts ----
	const series = $derived<ChartSeries[]>(
		compared.map((r) => ({
			id: r.id,
			name: r.name,
			color: colors.get(r.id)!,
			runs: [{ id: r.id, startMs: r.start_ms }]
		}))
	);
	let settings = $state<ChartSettings>({ ...DEFAULT_SETTINGS });
	let metricFilter = $state('');
	let view = $state<[number, number] | null>(null);
	let overrides = $state({});
	const runById = $derived(new Map(all.map((r) => [r.id, r])));
</script>

<div class="grid gap-6">
	<Card.Root>
		<Card.Header>
			<Card.Title>Runs to compare</Card.Title>
			<Card.Description>
				The runs picked on the Runs tab (the same selection). Add or remove runs with a
				filter, or one by one. {#if picked.length > MAX_COMPARED}The first {MAX_COMPARED} of the
					{picked.length}
					picked runs are compared.{/if}
				{#if selection.size > picked.length}{selection.size - picked.length} picked
					{selection.size - picked.length === 1 ? 'run is' : 'runs are'} archived or deleted
					and left out.{/if}
			</Card.Description>
		</Card.Header>
		<Card.Content>
			{#if list?.runs}
				<ComparePicker
					{project}
					runs={all}
					{selection}
					{picked}
					max={MAX_COMPARED}
					reference={compared.length > 1 ? (reference?.id ?? null) : null}
					{colors}
					onReference={(id) => (referenceId = shortId(id))}
				/>
			{:else}
				<Skeleton class="h-16 rounded-lg" />
			{/if}
		</Card.Content>
	</Card.Root>

	{#if list?.runs && compared.length === 0}
		<EmptyCard icon={GitCompareArrows} title="No runs picked">
			{#snippet description()}
				Tick runs on the Runs tab, use "Add to compare" on a run's page, or add the runs a
				filter matches above. Their metrics, charts and configs show here side by side.
			{/snippet}
		</EmptyCard>
	{:else if compared.length === 1}
		<EmptyCard icon={GitCompareArrows} title="Pick at least one more run">
			{#snippet description()}
				Comparing takes two runs or more: only {compared[0].name} is picked. Add others with a
				filter above, by ticking them on the Runs tab, or with "Add to compare" on a run's page.
			{/snippet}
		</EmptyCard>
	{:else if compared.length}
		<Card.Root>
			<Card.Header>
				<Card.Title>Metrics</Card.Title>
				<Card.Description>
					{metrics.some((k) => registry.describe(k).important)
						? 'The important metrics'
						: 'The metrics most of these runs logged (mark some as important in the Metrics tab to choose)'}:
					★ marks the best value, and the difference to the reference is below each value.
				</Card.Description>
			</Card.Header>
			<Card.Content>
				{#if metrics.length}
					<CompareMetricsTable
						{project}
						runs={ordered}
						{metrics}
						reference={reference?.id ?? null}
						{colors}
					/>
				{:else}
					<p class="text-sm text-muted-foreground">
						These runs have no summary values yet.
					</p>
				{/if}
			</Card.Content>
		</Card.Root>

		<Card.Root>
			<Card.Header>
				<Card.Title>Config</Card.Title>
				<Card.Description
					>Each run's config, key by key, next to the reference's.</Card.Description
				>
			</Card.Header>
			<Card.Content>
				<ConfigDiff runs={ordered} reference={reference?.id ?? null} {colors} />
			</Card.Content>
		</Card.Root>

		<section class="grid gap-3">
			<h2 class="text-lg font-semibold">Charts</h2>
			<ChartsPanel
				{project}
				{series}
				running={compared.filter((r) => r.state === 'running').map((r) => r.id)}
				bind:settings
				bind:metricFilter
				bind:view
				bind:overrides
				renames={(id) => runById.get(id)}
				summaryOf={(id) => runById.get(id)?.summary}
				onRunState={() => list?.refresh()}
			/>
		</section>
	{/if}
</div>
