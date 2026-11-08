<script lang="ts">
	import ArrowDownWideNarrow from '@lucide/svelte/icons/arrow-down-wide-narrow';
	import ArrowUpNarrowWide from '@lucide/svelte/icons/arrow-up-narrow-wide';
	import FlaskConical from '@lucide/svelte/icons/flask-conical';
	import GitCompareArrows from '@lucide/svelte/icons/git-compare-arrows';
	import Star from '@lucide/svelte/icons/star';
	import X from '@lucide/svelte/icons/x';
	import { untrack } from 'svelte';
	import { toast } from 'svelte-sonner';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	import { apiJson, apiSend } from '#lib/api/client.js';
	import type { ProjectInfo, RunSummary } from '#lib/api/types.js';
	import {
		DEFAULT_SETTINGS,
		type ChartSeries,
		type ChartSettings
	} from '#lib/charts/settings.js';
	import ChartsPanel from '#lib/components/app/ChartsPanel.svelte';
	import EmptyCard from '#lib/components/app/EmptyCard.svelte';
	import FilterHelp from '#lib/components/app/FilterHelp.svelte';
	import FilterInput from '#lib/components/app/FilterInput.svelte';
	import RunsTable from '#lib/components/app/RunsTable.svelte';
	import SdkQuickstart from '#lib/components/app/SdkQuickstart.svelte';
	import { Button, buttonVariants } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { FilterError, parseFilter, type Node } from '#lib/filter.js';
	import { literal } from '#lib/filter-suggest.js';
	import { withMetrics } from '#lib/metrics/apply.js';
	import { currentRegistry } from '#lib/metrics/store.svelte.js';
	import { errorMessage } from '#lib/notify.js';
	import { poll, type Polled } from '#lib/poll.svelte.js';
	import {
		assignSlots,
		boardColumns,
		chartedRuns,
		clampLimit,
		DEFAULT_BOARD,
		importantKeys,
		matchingRuns,
		MAX_LIMIT,
		naturalSort,
		sortChoices,
		sortLabel,
		type Board
	} from '#lib/runs/board.js';
	import { buildCatalog } from '#lib/runs/catalog.js';
	import { RunList } from '#lib/runs/list.svelte.js';
	import { selectionOf } from '#lib/runs/selection.svelte.js';

	/** Rows before the leaderboard scrolls. */
	const MAX_ROWS = 15;
	const NO_COLLAPSED: ReadonlySet<string> = new Set();
	/** A dot next to a run stands for its line on a chart. */
	const NO_COLORS = new Map<string, string>();

	const project = $derived(page.params.project ?? '');
	const projectPath = $derived(`/projects/${encodeURIComponent(project)}`);
	const compareHref = $derived(resolve('/p/[project]/(tabs)/compare', { project }));
	const metricsHref = $derived(resolve('/p/[project]/(tabs)/metrics', { project }));

	const info: Polled<ProjectInfo> = poll(
		() => apiJson<ProjectInfo>(projectPath),
		() => 30_000
	);
	const canEdit = $derived(info.data?.role === 'owner' || info.data?.role === 'editor');

	let list = $state<RunList | null>(null);
	$effect(() => {
		const l = new RunList(projectPath, false, (n) => (n > 5_000 ? 30_000 : 10_000));
		list = l;
		l.start();
		return () => l.stop();
	});
	const registry = $derived(currentRegistry());
	const describe = (key: string) => registry.describe(key);
	const all = $derived((list?.runs ?? []).map((r) => withMetrics(r, registry)));
	const runById = $derived(new Map(all.map((r) => [r.id, r])));

	// ---- The board: what it shows is edited here, and saved with the project ----
	let query = $state(DEFAULT_BOARD.query);
	let sort = $state({ ...DEFAULT_BOARD.sort });
	let limit = $state(DEFAULT_BOARD.limit);
	let loaded = $state(false);
	let status = $state<'idle' | 'pending' | 'saving' | 'saved'>('idle');

	const asText = (b: Board) =>
		JSON.stringify({
			query: b.query.trim(),
			sort: { id: b.sort.id, desc: b.sort.desc },
			limit: b.limit
		});
	let savedText = '';

	$effect(() => {
		const path = projectPath;
		loaded = false;
		status = 'idle';
		apiJson<Board>(`${path}/board`).then(
			(board) => {
				if (path !== projectPath) return;
				query = board.query;
				sort = board.sort;
				limit = board.limit;
				savedText = asText(board);
				loaded = true;
			},
			(err) => {
				toast.error("Couldn't load the board", {
					description: `${errorMessage(err)} It shows the latest runs until it can.`
				});
				loaded = true;
			}
		);
	});

	let lastValid: Node | null = null;
	const parsed = $derived.by((): { node: Node | null; error: FilterError | null } => {
		try {
			lastValid = parseFilter(query);
			return { node: lastValid, error: null };
		} catch (err) {
			if (!(err instanceof FilterError)) throw err;
			// Keep showing the last valid result while the query is being typed.
			return { node: lastValid, error: err };
		}
	});

	async function save(path: string, text: string) {
		status = 'saving';
		try {
			await apiSend('PUT', `${path}/board`, JSON.parse(text));
			savedText = path === projectPath ? text : savedText;
			if (status === 'saving') status = 'saved';
		} catch (err) {
			status = 'idle';
			toast.error("Couldn't save the board", {
				description: `${errorMessage(err)} Others still see it as it was.`
			});
		}
	}

	// A save waiting when leaving still happens: it isn't tied to this page.
	let saveTimer: ReturnType<typeof setTimeout> | undefined;
	let savePath = '';
	$effect(() => {
		const text = asText({ query, sort, limit });
		const path = projectPath;
		if (!loaded || !canEdit) return;
		if (savePath === path) clearTimeout(saveTimer);
		if (text === savedText) {
			if (untrack(() => status) === 'pending') status = 'idle';
			return;
		}
		if (parsed.error) return;
		savePath = path;
		status = 'pending';
		saveTimer = setTimeout(() => void save(path, text), 500);
	});

	// ---- Its runs ----
	const matching = $derived(matchingRuns(all, parsed.node, sort));
	const shown = $derived(matching.slice(0, limit));
	const columns = $derived(boardColumns(matching, describe, sort.id));
	const choices = $derived(sortChoices(all, describe));
	const catalog = $derived(buildCatalog(all));
	const selection = $derived(selectionOf(project));

	const allSelected = $derived(shown.length > 0 && shown.every((r) => selection.has(r.id)));

	function chooseSort(id: string) {
		if (id && id !== sort.id) sort = naturalSort(id, describe);
	}

	function setLimit(e: Event & { currentTarget: HTMLInputElement }) {
		const n = Number(e.currentTarget.value);
		if (e.currentTarget.value.trim() && Number.isFinite(n)) limit = clampLimit(n);
		e.currentTarget.value = String(limit);
	}

	/** Narrows the board to this tag. */
	function addTag(tag: string) {
		if (!canEdit) return;
		const term = `tag:${literal(tag)}`;
		const current = query.trim();
		if (!current.split(/\s+/).includes(term)) query = current ? `${current} ${term}` : term;
	}

	function compareShown() {
		selection.replace(shown.map((r) => r.id));

		void goto(compareHref);
	}

	// ---- Charts: the important metrics of the first runs ----
	const charted = $derived(chartedRuns(shown, (id) => selection.has(id)));

	let slots: ReadonlyMap<string, number> = new Map();
	const colors = $derived.by(() => {
		slots = assignSlots(
			charted.map((r) => r.id),
			slots
		);
		return new Map(
			charted.map((r): [string, string] => [r.id, `var(--chart-${slots.get(r.id)! + 1})`])
		);
	});

	const series = $derived<ChartSeries[]>(
		charted.map((r: RunSummary) => ({
			id: r.id,
			name: r.name,
			color: colors.get(r.id)!,
			runs: [{ id: r.id, startMs: r.start_ms }]
		}))
	);
	const hasImportant = $derived(importantKeys(charted, describe).length > 0);
	let settings = $state<ChartSettings>({ ...DEFAULT_SETTINGS });
	let metricFilter = $state('');
	let view = $state<[number, number] | null>(null);
	let overrides = $state({});
</script>

{#if list?.error}
	<p class="text-sm text-destructive">{String(list.error)}</p>
{:else if !list?.runs || !loaded}
	<Skeleton class="h-64 rounded-xl" />
{:else if all.length === 0}
	<EmptyCard icon={FlaskConical} title="No runs yet">
		{#snippet description()}
			Runs are logged from your code with the Python SDK. Three steps to the first one:
		{/snippet}
		<SdkQuickstart {project} />
	</EmptyCard>
{:else}
	<div class="mb-2 flex flex-wrap items-center gap-2">
		<FilterInput
			bind:value={query}
			catalog={() => catalog}
			invalid={!!parsed.error}
			describedby="filter-help"
			disabled={!canEdit}
		/>
		<FilterHelp />
		<Select.Root type="single" value={sort.id} onValueChange={chooseSort} disabled={!canEdit}>
			<Select.Trigger size="sm" class="w-56" aria-label="Sort by">
				<span class="truncate">Sort: {sortLabel(sort.id, describe)}</span>
			</Select.Trigger>
			<Select.Content class="max-h-80">
				<Select.Group>
					<Select.GroupHeading>Run</Select.GroupHeading>
					{#each choices.run as id (id)}
						<Select.Item value={id} label={sortLabel(id, describe)} />
					{/each}
				</Select.Group>
				{#if choices.important.length}
					<Select.Group>
						<Select.GroupHeading>Important metrics</Select.GroupHeading>
						{#each choices.important as id (id)}
							<Select.Item value={id} label={sortLabel(id, describe)} />
						{/each}
					</Select.Group>
				{/if}
				{#if choices.metrics.length}
					<Select.Group>
						<Select.GroupHeading>Metrics</Select.GroupHeading>
						{#each choices.metrics as id (id)}
							<Select.Item value={id} label={sortLabel(id, describe)} />
						{/each}
					</Select.Group>
				{/if}
			</Select.Content>
		</Select.Root>
		<Button
			variant="outline"
			size="icon-sm"
			disabled={!canEdit}
			aria-label={sort.desc ? 'Highest first' : 'Lowest first'}
			title={sort.desc ? 'Highest first' : 'Lowest first'}
			onclick={() => (sort = { id: sort.id, desc: !sort.desc })}
		>
			{#if sort.desc}<ArrowDownWideNarrow />{:else}<ArrowUpNarrowWide />{/if}
		</Button>
		<label class="flex items-center gap-2 text-sm text-muted-foreground">
			Show
			<Input
				class="h-8 w-16 text-foreground tabular-nums"
				type="number"
				min="1"
				max={MAX_LIMIT}
				disabled={!canEdit}
				value={limit}
				onchange={setLimit}
			/>
		</label>
		<span class="w-14 text-xs text-muted-foreground" aria-live="polite">
			{status === 'pending' || status === 'saving'
				? 'Saving…'
				: status === 'saved'
					? 'Saved'
					: ''}
		</span>
	</div>

	<div
		id="filter-help"
		class="mb-2 flex min-h-8 flex-wrap items-center gap-2 text-xs"
		aria-live="polite"
	>
		{#if parsed.error}
			<span class="text-destructive">
				{parsed.error.message} (at character {parsed.error.at + 1})
			</span>
		{:else if selection.size}
			<span class="font-medium tabular-nums">
				{selection.size.toLocaleString('en')} selected
			</span>
			{#if selection.size > 1}
				<!-- eslint-disable svelte/no-navigation-without-resolve -- resolved above -->
				<a
					class={buttonVariants({ variant: 'outline', size: 'sm', class: 'h-7' })}
					href={compareHref}
				>
					<GitCompareArrows /> Compare
				</a>
				<!-- eslint-enable svelte/no-navigation-without-resolve -->
			{:else}
				<Button
					variant="outline"
					size="sm"
					class="h-7"
					disabled
					title="Comparing takes two runs or more"
				>
					<GitCompareArrows /> Compare
				</Button>
			{/if}
			{#if !allSelected}
				<Button
					variant="outline"
					size="sm"
					class="h-7"
					onclick={() =>
						selection.toggle(
							shown.map((r) => r.id),
							true
						)}
				>
					Add the {shown.length} shown
				</Button>
			{/if}
			<Button variant="ghost" size="sm" class="h-7" onclick={() => selection.clear()}>
				<X /> Clear selection
			</Button>
		{:else}
			<span class="text-muted-foreground tabular-nums">
				{shown.length.toLocaleString('en')} of {matching.length.toLocaleString('en')}
				{parsed.node ? 'matching' : ''} runs{#if list?.partial}&nbsp;(loading more…){/if}.
				Tick runs to chart or compare them.
				{#if !canEdit}Only editors can change what the board shows.{/if}
			</span>
			{#if shown.length > 1}
				<Button variant="outline" size="sm" class="h-7" onclick={compareShown}>
					<GitCompareArrows /> Compare these {shown.length}
				</Button>
			{/if}
		{/if}
	</div>

	<RunsTable
		{project}
		runs={shown}
		{columns}
		pinned={[]}
		bind:sort
		colors={hasImportant ? colors : NO_COLORS}
		{selection}
		collapsed={NO_COLLAPSED}
		maxRows={MAX_ROWS}
		onTag={addTag}
		onSelect={(runs, on) =>
			selection.toggle(
				runs.map((r) => r.id),
				on
			)}
	/>
	{#if shown.length === 0}
		<p class="mt-2 text-sm text-muted-foreground">
			No run matches.
			{#if canEdit}
				<Button variant="link" class="h-auto p-0" onclick={() => (query = '')}
					>Clear the query</Button
				>
			{/if}
		</p>
	{/if}

	{#if shown.length}
		{#if hasImportant}
			<div class="mt-6 mb-3 flex min-h-4 flex-wrap gap-x-4 gap-y-1 text-xs">
				{#if series.length > 1}
					{#each series as s (s.id)}
						<span class="inline-flex items-center gap-1.5">
							<span class="size-2.5 rounded-sm" style:background-color={s.color}
							></span>
							{s.name}
						</span>
					{/each}
				{/if}
				{#if charted.length < shown.length}
					<span class="text-muted-foreground">
						The first {charted.length} are charted; tick runs to chart others.
					</span>
				{/if}
			</div>
			<ChartsPanel
				{project}
				{series}
				running={charted.filter((r) => r.state === 'running').map((r) => r.id)}
				bind:settings
				bind:metricFilter
				bind:view
				bind:overrides
				renames={(id) => runById.get(id)}
				importantOnly
				onRunState={() => list?.refresh()}
				empty={{
					icon: Star,
					title: 'No points yet',
					text: 'These runs have important metrics, but nothing to chart yet.'
				}}
			/>
		{:else}
			<div class="mt-6">
				<EmptyCard icon={Star} title="No important metrics in these runs">
					{#snippet description()}
						The board charts the metrics that matter most: mark them with a star on the
						<!-- eslint-disable svelte/no-navigation-without-resolve -- resolved above -->
						<a class="underline" href={metricsHref}>Metrics tab</a>.
						<!-- eslint-enable svelte/no-navigation-without-resolve -->
					{/snippet}
				</EmptyCard>
			</div>
		{/if}
	{/if}
{/if}
