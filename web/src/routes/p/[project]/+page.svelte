<script lang="ts">
	import ArrowDown from '@lucide/svelte/icons/arrow-down';
	import ArrowUp from '@lucide/svelte/icons/arrow-up';
	import Search from '@lucide/svelte/icons/search';
	import Users from '@lucide/svelte/icons/users';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	import { apiJson } from '#lib/api/client.js';
	import type { RunState, RunSummary } from '#lib/api/types.js';
	import StateBadge from '#lib/components/app/StateBadge.svelte';
	import { buttonVariants } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import * as ToggleGroup from '#lib/components/ui/toggle-group/index.js';
	import { duration, formatValue, timeAgo } from '#lib/format.js';
	import { poll } from '#lib/poll.svelte.js';

	const SUMMARY_COLUMNS = 6;
	const project = $derived(page.params.project ?? '');
	const runs = poll(
		() => apiJson<RunSummary[]>(`/projects/${encodeURIComponent(project)}/runs`),
		() => 5_000
	);

	let query = $state('');
	let stateFilter = $state<'all' | RunState>('all');
	type SortKey = 'name' | 'state' | 'created' | 'duration' | `summary:${string}`;
	let sort = $state<{ key: SortKey; desc: boolean }>({ key: 'created', desc: true });

	/** The numeric summary keys most runs have. */
	const summaryColumns = $derived.by(() => {
		const counts: Record<string, number> = {};
		for (const run of runs.data ?? []) {
			for (const [key, value] of Object.entries(run.summary)) {
				if (typeof value === 'number') counts[key] = (counts[key] ?? 0) + 1;
			}
		}
		return Object.entries(counts)
			.sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
			.slice(0, SUMMARY_COLUMNS)
			.map(([key]) => key);
	});

	const endOf = (r: RunSummary) =>
		r.state === 'running' ? Date.now() : (r.finished_at_ms ?? r.heartbeat_at_ms);

	const counts = $derived.by(() => {
		const c: Record<string, number> = { all: 0 };
		for (const r of runs.data ?? []) {
			c.all += 1;
			c[r.state] = (c[r.state] ?? 0) + 1;
		}
		return c;
	});

	const shown = $derived.by(() => {
		const q = query.trim().toLowerCase();
		const value = (r: RunSummary): number | string => {
			const { key } = sort;
			if (key === 'name') return r.name.toLowerCase();
			if (key === 'state') return r.state;
			if (key === 'created') return r.created_at_ms;
			if (key === 'duration') return endOf(r) - r.created_at_ms;
			const v = r.summary[key.slice('summary:'.length)];
			return typeof v === 'number' ? v : sort.desc ? -Infinity : Infinity;
		};
		return (runs.data ?? [])
			.filter((r) => stateFilter === 'all' || r.state === stateFilter)
			.filter(
				(r) =>
					!q ||
					r.name.toLowerCase().includes(q) ||
					r.group_name?.toLowerCase().includes(q) ||
					r.tags.some((t) => t.toLowerCase().includes(q))
			)
			.sort((a, b) => {
				const [x, y] = [value(a), value(b)];
				const order = x < y ? -1 : x > y ? 1 : 0;
				return sort.desc ? -order : order;
			});
	});

	function sortBy(key: SortKey) {
		sort = sort.key === key ? { key, desc: !sort.desc } : { key, desc: key !== 'name' };
	}

	const runHref = (r: RunSummary) =>
		resolve('/p/[project]/r/[run]', { project, run: r.id.replaceAll('-', '') });
</script>

{#snippet sortHeader(key: SortKey, label: string, align: 'left' | 'right' = 'left')}
	<Table.Head class={align === 'right' ? 'text-right' : ''}>
		<button
			class={[
				'inline-flex max-w-40 items-center gap-1 hover:text-foreground',
				align === 'right' && 'flex-row-reverse'
			]}
			onclick={() => sortBy(key)}
			title={label}
		>
			<span class="truncate">{label}</span>
			{#if sort.key === key}
				{#if sort.desc}<ArrowDown class="size-3 shrink-0" />{:else}<ArrowUp
						class="size-3 shrink-0"
					/>{/if}
			{/if}
		</button>
	</Table.Head>
{/snippet}

<div class="mx-auto max-w-7xl">
	<div class="mb-5 flex flex-wrap items-start justify-between gap-2">
		<div>
			<h1 class="text-2xl font-semibold tracking-tight">{project}</h1>
			<p class="text-sm text-muted-foreground">
				{counts.all ?? 0} runs{#if counts.running}{` · ${counts.running} running`}{/if}
			</p>
		</div>
		<a
			class={buttonVariants({ variant: 'outline', size: 'sm' })}
			href={resolve('/p/[project]/members', { project })}
		>
			<Users /> Members
		</a>
	</div>

	<div class="mb-3 flex flex-wrap items-center gap-2">
		<div class="relative w-full sm:w-72">
			<Search
				class="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
			/>
			<Input class="pl-8" placeholder="Filter by name, tag or group" bind:value={query} />
		</div>
		<ToggleGroup.Root
			type="single"
			variant="outline"
			size="sm"
			value={stateFilter}
			onValueChange={(v) => (stateFilter = (v || 'all') as typeof stateFilter)}
		>
			{#each ['all', 'running', 'finished', 'failed', 'crashed', 'killed'] as s (s)}
				{#if s === 'all' || counts[s]}
					<ToggleGroup.Item value={s} class="capitalize">
						{s}<span class="text-muted-foreground tabular-nums">{counts[s] ?? 0}</span>
					</ToggleGroup.Item>
				{/if}
			{/each}
		</ToggleGroup.Root>
	</div>

	{#if runs.error}
		<p class="text-sm text-destructive">{String(runs.error)}</p>
	{:else if !runs.data}
		<Skeleton class="h-64 rounded-xl" />
	{:else if runs.data.length === 0}
		<Card.Root class="max-w-2xl">
			<Card.Header>
				<Card.Title>No runs yet</Card.Title>
				<Card.Description>
					Log one with the Python SDK and an API key from <a
						class="underline"
						href={resolve('/settings')}>Settings</a
					>:
				</Card.Description>
			</Card.Header>
			<Card.Content>
				<pre
					class="overflow-x-auto rounded-lg bg-muted p-4 font-mono text-xs leading-relaxed">pip install losscape-sdk
export LOSSCAPE_API_KEY=...

import losscape

with losscape.init(project="{project}") as run:
    for step in range(1000):
        run.log(&#123;"train/loss": 1 / (step + 1)&#125;)</pre>
			</Card.Content>
		</Card.Root>
	{:else}
		<div class="overflow-hidden rounded-xl border bg-card">
			<Table.Root>
				<Table.Header>
					<Table.Row>
						{@render sortHeader('name', 'Run')}
						{@render sortHeader('state', 'State')}
						{@render sortHeader('created', 'Created')}
						{@render sortHeader('duration', 'Duration')}
						<Table.Head>Tags</Table.Head>
						{#each summaryColumns as key (key)}
							{@render sortHeader(`summary:${key}`, key, 'right')}
						{/each}
					</Table.Row>
				</Table.Header>
				<Table.Body>
					{#each shown as run (run.id)}
						<Table.Row class="cursor-pointer" onclick={() => goto(runHref(run))}>
							<Table.Cell class="font-medium">
								<a
									href={runHref(run)}
									class="hover:underline"
									onclick={(e) => e.stopPropagation()}
								>
									{run.name}
								</a>
								{#if run.group_name}<span class="ml-1 text-muted-foreground"
										>· {run.group_name}</span
									>{/if}
							</Table.Cell>
							<Table.Cell><StateBadge state={run.state} /></Table.Cell>
							<Table.Cell class="text-muted-foreground"
								>{timeAgo(run.created_at_ms)}</Table.Cell
							>
							<Table.Cell class="text-muted-foreground tabular-nums">
								{duration(run.created_at_ms, endOf(run))}
							</Table.Cell>
							<Table.Cell>
								<div class="flex gap-1">
									{#each run.tags as tag (tag)}
										<span class="rounded-md border px-1.5 text-xs">{tag}</span>
									{/each}
								</div>
							</Table.Cell>
							{#each summaryColumns as key (key)}
								<Table.Cell class="text-right font-mono text-xs tabular-nums">
									{formatValue(run.summary[key])}
								</Table.Cell>
							{/each}
						</Table.Row>
					{:else}
						<Table.Row>
							<Table.Cell
								colspan={5 + summaryColumns.length}
								class="py-8 text-center text-muted-foreground"
							>
								No runs match.
							</Table.Cell>
						</Table.Row>
					{/each}
				</Table.Body>
			</Table.Root>
		</div>
	{/if}
</div>
