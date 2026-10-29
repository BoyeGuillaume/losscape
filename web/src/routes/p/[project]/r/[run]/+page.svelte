<script lang="ts">
	import Check from '@lucide/svelte/icons/check';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import Copy from '@lucide/svelte/icons/copy';
	import GitCommitHorizontal from '@lucide/svelte/icons/git-commit-horizontal';
	import Search from '@lucide/svelte/icons/search';
	import Server from '@lucide/svelte/icons/server';
	import { ChartGroup } from 'layerchart';
	import { page } from '$app/state';

	import { apiJson } from '#lib/api/client.js';
	import { subscribeLive } from '#lib/api/live.js';
	import type { RunDetail } from '#lib/api/types.js';
	import { tileStore } from '#lib/charts/tiles.svelte.js';
	import KeyValueList from '#lib/components/app/KeyValueList.svelte';
	import MetricChart from '#lib/components/app/MetricChart.svelte';
	import StateBadge from '#lib/components/app/StateBadge.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as Tabs from '#lib/components/ui/tabs/index.js';
	import { dateTime, duration, flatten, timeAgo } from '#lib/format.js';
	import { nav } from '#lib/nav.svelte.js';
	import { poll, type Polled } from '#lib/poll.svelte.js';

	type Extent = { min: number; max: number };

	const runId = $derived(page.params.run ?? '');
	const run: Polled<RunDetail> = poll(
		() => apiJson<RunDetail>(`/runs/${runId}`),
		(): number | false => (run.data?.state === 'running' ? 5_000 : false)
	);
	const running = $derived(run.data?.state === 'running');

	$effect(() => {
		nav.runName = run.data?.name ?? null;
		return () => (nav.runName = null);
	});

	// Step range per metric: from the server, extended by live points.
	let live = $state<Record<string, Extent>>({});
	$effect(() => {
		void runId;
		live = {};
	});

	$effect(() => {
		if (!running) return;
		const id = runId;
		return subscribeLive(id, {
			onPoints: (points) => {
				const next = { ...live };
				for (let i = 0; i < points.step.length; i++) {
					const key = points.keys[points.key_idx[i]];
					const step = points.step[i];
					tileStore.addLivePoint(id, key, step, points.value[i] ?? NaN);
					const e = next[key];
					next[key] = {
						min: Math.min(e?.min ?? step, step),
						max: Math.max(e?.max ?? step, step)
					};
				}
				live = next;
			},
			onState: () => run.refresh(),
			onReset: () => {
				tileStore.invalidate(id);
				run.refresh();
			}
		});
	});

	const extents = $derived.by(() => {
		const merged: Record<string, Extent> = { ...live };
		for (const m of run.data?.metrics ?? []) {
			const l = merged[m.key];
			merged[m.key] = {
				min: Math.min(l?.min ?? m.min_step, m.min_step),
				max: Math.max(l?.max ?? m.max_step, m.max_step)
			};
		}
		return merged;
	});

	let metricQuery = $state('');
	let collapsed = $state<Record<string, boolean>>({});

	/** Charts grouped by the metric name's prefix: `train/loss` goes to "train". */
	const sections = $derived.by(() => {
		const q = metricQuery.trim().toLowerCase();
		const groups: Record<string, string[]> = {};
		for (const key of Object.keys(extents).sort()) {
			if (q && !key.toLowerCase().includes(q)) continue;
			const section = key.includes('/') ? key.slice(0, key.indexOf('/')) : '';
			(groups[section] ??= []).push(key);
		}
		return Object.entries(groups).sort(([a], [b]) =>
			a === '' ? -1 : b === '' ? 1 : a.localeCompare(b)
		);
	});

	let configQuery = $state('');
	const configRows = $derived(
		flatten(run.data?.config ?? {}).filter(([k]) =>
			k.toLowerCase().includes(configQuery.trim().toLowerCase())
		)
	);

	// Ticks once a second while running, for the live duration.
	let now = $state(Date.now());
	$effect(() => {
		if (!running) return;
		const t = setInterval(() => (now = Date.now()), 1000);
		return () => clearInterval(t);
	});

	let copied = $state(false);
	async function copyId() {
		await navigator.clipboard?.writeText(run.data?.id ?? '');
		copied = true;
		setTimeout(() => (copied = false), 1500);
	}
</script>

{#if run.error}
	<p class="text-sm text-destructive">{String(run.error)}</p>
{:else if !run.data}
	<div class="grid gap-4">
		<Skeleton class="h-24 rounded-xl" />
		<div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
			{#each [0, 1, 2] as i (i)}<Skeleton class="h-60 rounded-xl" />{/each}
		</div>
	</div>
{:else}
	{@const r = run.data}
	<div class="mb-5 flex flex-wrap items-start gap-x-6 gap-y-3">
		<div class="min-w-0">
			<div class="flex flex-wrap items-center gap-3">
				<h1 class="truncate text-2xl font-semibold tracking-tight">{r.name}</h1>
				<StateBadge state={r.state} />
			</div>
			<p
				class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-muted-foreground"
			>
				<span title={dateTime(r.created_at_ms)}>Started {timeAgo(r.created_at_ms)}</span>
				<span class="tabular-nums">
					{duration(
						r.created_at_ms,
						running ? now : (r.finished_at_ms ?? r.heartbeat_at_ms)
					)}
				</span>
				{#if r.group_name}<span>Group {r.group_name}</span>{/if}
				{#if r.environment.hostname}
					<span class="inline-flex items-center gap-1"
						><Server class="size-3.5" />{r.environment.hostname}</span
					>
				{/if}
				{#if r.environment.git?.commit}
					<span class="inline-flex items-center gap-1 font-mono text-xs">
						<GitCommitHorizontal class="size-3.5" />{r.environment.git.commit.slice(
							0,
							8
						)}{r.environment.git.dirty ? '*' : ''}
					</span>
				{/if}
				{#each r.tags as tag (tag)}
					<span class="rounded-md border px-1.5 text-xs text-foreground">{tag}</span>
				{/each}
			</p>
		</div>
		<Button
			variant="outline"
			size="sm"
			class="ml-auto font-mono text-xs"
			onclick={copyId}
			title="Copy run id"
		>
			{#if copied}<Check />{:else}<Copy />{/if}
			{r.id.replaceAll('-', '').slice(0, 12)}…
		</Button>
	</div>

	<Tabs.Root value="charts">
		<div class="mb-4 flex flex-wrap items-center justify-between gap-2">
			<Tabs.List>
				<Tabs.Trigger value="charts">Charts</Tabs.Trigger>
				<Tabs.Trigger value="overview">Overview</Tabs.Trigger>
			</Tabs.List>
		</div>

		<Tabs.Content value="charts">
			<div class="relative mb-4 w-full sm:w-72">
				<Search
					class="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
				/>
				<Input class="pl-8" placeholder="Filter metrics" bind:value={metricQuery} />
			</div>
			{#if sections.length === 0}
				<p class="text-sm text-muted-foreground">
					{Object.keys(extents).length ? 'No metric matches.' : 'No metrics logged yet.'}
				</p>
			{/if}
			<!-- Hovering one chart shows the same step on all of them; zoom stays per chart. -->
			<ChartGroup brush={false} domain={false}>
				<div class="grid gap-6">
					{#each sections as [section, keys] (section)}
						{@const open = !collapsed[section]}
						<section>
							<button
								class="mb-3 flex items-center gap-1.5 text-sm font-medium text-muted-foreground hover:text-foreground"
								onclick={() => (collapsed[section] = open)}
								aria-expanded={open}
							>
								<ChevronRight
									class={['size-4 transition-transform', open && 'rotate-90']}
								/>
								{section || 'Charts'}
								<span class="text-xs tabular-nums">{keys.length}</span>
							</button>
							{#if open}
								<div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
									{#each keys as key (key)}
										<MetricChart
											run={runId}
											metric={key}
											minStep={extents[key].min}
											maxStep={extents[key].max}
										/>
									{/each}
								</div>
							{/if}
						</section>
					{/each}
				</div>
			</ChartGroup>
		</Tabs.Content>

		<Tabs.Content value="overview">
			<div class="grid gap-4 lg:grid-cols-2">
				<Card.Root>
					<Card.Header class="flex-row items-center justify-between gap-2">
						<Card.Title>Config</Card.Title>
						<Input class="h-8 max-w-48" placeholder="Filter" bind:value={configQuery} />
					</Card.Header>
					<Card.Content
						><KeyValueList rows={configRows} empty="No config." /></Card.Content
					>
				</Card.Root>
				<Card.Root>
					<Card.Header><Card.Title>Summary</Card.Title></Card.Header>
					<Card.Content
						><KeyValueList
							rows={flatten(r.summary)}
							empty="No summary yet."
						/></Card.Content
					>
				</Card.Root>
				<Card.Root>
					<Card.Header><Card.Title>Run</Card.Title></Card.Header>
					<Card.Content>
						<KeyValueList
							mono={false}
							rows={[
								['Run id', r.id],
								['Created', dateTime(r.created_at_ms)],
								['Finished', r.finished_at_ms ? dateTime(r.finished_at_ms) : null],
								['Exit code', r.exit_code],
								['Group', r.group_name],
								['Notes', r.notes]
							]}
						/>
					</Card.Content>
				</Card.Root>
				<Card.Root>
					<Card.Header><Card.Title>Environment</Card.Title></Card.Header>
					<Card.Content>
						<KeyValueList
							rows={[
								['Host', r.environment.hostname],
								['Command', r.environment.command],
								['Python', r.environment.python_version],
								['SDK', r.environment.sdk_version],
								['Git commit', r.environment.git?.commit],
								[
									'Git dirty',
									r.environment.git?.commit ? r.environment.git.dirty : null
								],
								['Git remote', r.environment.git?.remote]
							]}
							empty="Not captured."
						/>
					</Card.Content>
				</Card.Root>
			</div>
		</Tabs.Content>
	</Tabs.Root>
{/if}
