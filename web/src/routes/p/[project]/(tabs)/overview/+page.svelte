<script lang="ts">
	import ArrowDownWideNarrow from '@lucide/svelte/icons/arrow-down-wide-narrow';
	import ArrowUpNarrowWide from '@lucide/svelte/icons/arrow-up-narrow-wide';
	import Flag from '@lucide/svelte/icons/flag';
	import FlaskConical from '@lucide/svelte/icons/flask-conical';
	import GitCompareArrows from '@lucide/svelte/icons/git-compare-arrows';
	import Star from '@lucide/svelte/icons/star';
	import X from '@lucide/svelte/icons/x';
	import { untrack } from 'svelte';
	import { toast } from 'svelte-sonner';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	import { ApiError, apiJson, apiSend } from '#lib/api/client.js';
	import type { ProjectInfo, RunSummary } from '#lib/api/types.js';
	import {
		DEFAULT_SETTINGS,
		type ChartSeries,
		type ChartSettings
	} from '#lib/charts/settings.js';
	import ChartsPanel from '#lib/components/app/ChartsPanel.svelte';
	import ColumnsMenu from '#lib/components/app/ColumnsMenu.svelte';
	import DotChart from '#lib/components/app/DotChart.svelte';
	import EmptyCard from '#lib/components/app/EmptyCard.svelte';
	import ErrorCard from '#lib/components/app/ErrorCard.svelte';
	import FilterHelp from '#lib/components/app/FilterHelp.svelte';
	import FilterInput from '#lib/components/app/FilterInput.svelte';
	import RunsTable from '#lib/components/app/RunsTable.svelte';
	import SdkQuickstart from '#lib/components/app/SdkQuickstart.svelte';
	import Trajectories from '#lib/components/app/Trajectories.svelte';
	import { Button, buttonVariants } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as ToggleGroup from '#lib/components/ui/toggle-group/index.js';
	import { FilterError, parseFilter, type Node } from '#lib/filter.js';
	import { literal } from '#lib/filter-suggest.js';
	import { formatMetric } from '#lib/metric-units.js';
	import { withMetrics, type ValueMode } from '#lib/metrics/apply.js';
	import { isModelMetric } from '#lib/metrics/registry.js';
	import { currentRegistry } from '#lib/metrics/store.svelte.js';
	import { errorMessage } from '#lib/notify.js';
	import { onRestored, whenConnected } from '#lib/connection.svelte.js';
	import { isConnectionFailure } from '#lib/errors.js';
	import { poll, type Polled } from '#lib/poll.svelte.js';
	import {
		assignSlots,
		bestSoFar,
		boardColumns,
		changeFrom,
		chartedRuns,
		choiceMetric,
		clampLimit,
		DEFAULT_BOARD,
		explorePlot,
		exploreKeys,
		headlineFor,
		importantKeys,
		matchingRuns,
		MAX_LIMIT,
		metricChoice,
		metricValue,
		naturalSort,
		progressDots,
		sortChoices,
		sortLabel,
		thinDots,
		valueNote,
		type Board,
		type Change,
		type ExploreKey
	} from '#lib/runs/board.js';
	import { buildCatalog } from '#lib/runs/catalog.js';
	import { RunList } from '#lib/runs/list.svelte.js';
	import { selectionOf } from '#lib/runs/selection.svelte.js';
	import { SaveQueue, typingInField, type SaveStatus } from '#lib/save-queue.js';
	import { metricHref as metricHrefIn } from '#lib/metrics/href.js';

	/** Rows before the leaderboard scrolls. */
	const MAX_ROWS = 15;
	const NO_COLLAPSED: ReadonlySet<string> = new Set();
	/** A dot next to a run stands for its line on a chart. */
	const NO_COLORS = new Map<string, string>();
	const shortId = (id: string) => id.replaceAll('-', '');

	const project = $derived(page.params.project ?? '');
	const projectPath = $derived(`/projects/${encodeURIComponent(project)}`);
	const compareHref = $derived(resolve('/p/[project]/(tabs)/compare', { project }));
	const metricsHref = $derived(resolve('/p/[project]/(tabs)/metrics', { project }));
	const metricHref = (name: string) => metricHrefIn(project, name);
	const runHref = (run: string) =>
		resolve('/p/[project]/r/[run]', { project, run: shortId(run) });

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
	let baselineId = $state<string | null>(null);
	let exploreKey = $state<string | null>(null);
	let chosenColumns = $state<string[] | null>(null);
	/** Pinned for now, not saved with the board. */
	let pinned = $state<string[]>([]);
	let loaded = $state(false);
	let status = $state<SaveStatus>('idle');

	const asText = (b: Board) =>
		JSON.stringify({
			query: b.query.trim(),
			sort: { id: b.sort.id, desc: b.sort.desc },
			limit: b.limit,
			baseline: b.baseline ?? null,
			explore: b.explore ?? null,
			columns: b.columns ?? null
		});
	const currentText = () =>
		asText({
			query,
			sort,
			limit,
			baseline: baselineId,
			explore: exploreKey,
			columns: chosenColumns
		});
	let savedText = '';
	/** Of the project being viewed: others finish on their own when leaving it. */
	let queue: SaveQueue<string> | null = null;

	function queueFor(path: string, version: number) {
		return new SaveQueue<string>(
			async (text, based, closing) => {
				const body = { ...JSON.parse(text), version: based };
				const saved = await apiSend<{ version: number }>('PUT', `${path}/board`, body, {
					closing
				});
				if (path === projectPath) savedText = text;
				return saved.version;
			},
			version,
			{
				status: (s) => {
					if (path === projectPath) status = s;
				},
				retryAfter: (err) => (isConnectionFailure(err) ? whenConnected() : null),
				failed: (err) => {
					const stale = err instanceof ApiError && err.status === 409;
					toast.error(
						stale ? 'Someone else changed this board' : "Couldn't save the board",
						{
							description: stale
								? "So your last change wasn't saved, and their version is shown now."
								: `${errorMessage(err)} Others still see it as it was.`
						}
					);
					if (path === projectPath) {
						queue = null;
						void loadBoard(path);
					}
				}
			}
		);
	}

	async function loadBoard(path: string) {
		try {
			const board = await apiJson<Board>(`${path}/board`);
			if (path !== projectPath) return;
			query = board.query;
			sort = board.sort;
			limit = board.limit;
			baselineId = board.baseline ?? null;
			exploreKey = board.explore ?? null;
			chosenColumns = board.columns ?? null;
			pinned = [];
			savedText = asText(board);
			queue = queueFor(path, board.version ?? 0);
		} catch (err) {
			// Without a connection there's a banner saying so, and this loads when it's back.
			if (!isConnectionFailure(err)) {
				toast.error("Couldn't load the board", {
					description: `${errorMessage(err)} It shows the latest runs until it can.`
				});
			}
		}
		loaded = true;
	}

	$effect(() => {
		const path = projectPath;
		loaded = false;
		status = 'idle';
		queue = null;
		void loadBoard(path);
	});

	// Changes still waiting go when leaving. Coming back, others may have changed the board:
	// unless there's something here that isn't saved yet.
	$effect(() => {
		const leaving = () => queue?.flush(true);
		const refresh = () => {
			if (queue && !queue.busy && currentText() === savedText && !typingInField())
				void loadBoard(projectPath);
		};
		const visibility = () => {
			if (document.visibilityState === 'hidden') leaving();
			else refresh();
		};
		const stopListening = onRestored(() => {
			if (queue) refresh();
			else void loadBoard(projectPath);
		});
		window.addEventListener('pagehide', leaving);
		document.addEventListener('visibilitychange', visibility);
		return () => {
			stopListening();
			window.removeEventListener('pagehide', leaving);
			document.removeEventListener('visibilitychange', visibility);
		};
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

	$effect(() => {
		const text = currentText();
		if (!loaded || !canEdit || !queue) return;
		if (text === savedText) {
			if (untrack(() => status) === 'pending') queue.cancel();
			return;
		}
		if (!parsed.error) queue.schedule(text);
	});

	// ---- Its runs ----
	const baseline = $derived(baselineId ? (runById.get(baselineId) ?? null) : null);
	const matching = $derived(matchingRuns(all, parsed.node, sort));
	const others = $derived(matching.filter((r) => r.id !== baselineId));
	const top = $derived(others.slice(0, limit));
	/** The leaderboard: the baseline stays first, wherever it ranks. */
	const shown = $derived(baseline ? [baseline, ...top] : top);
	const columns = $derived(chosenColumns ?? boardColumns(shown, describe, sort.id));
	const choices = $derived(sortChoices(all, describe));
	const catalog = $derived(buildCatalog(all));
	const selection = $derived(selectionOf(project));
	const allSelected = $derived(shown.length > 0 && shown.every((r) => selection.has(r.id)));
	const important = $derived(importantKeys(matching.length ? matching : shown, describe));

	function chooseSort(id: string) {
		if (id && id !== sort.id) sort = naturalSort(id, describe);
	}

	function setLimit(e: Event & { currentTarget: HTMLInputElement }) {
		const n = Number(e.currentTarget.value);
		if (e.currentTarget.value.trim() && Number.isFinite(n)) limit = clampLimit(n);
		e.currentTarget.value = String(limit);
	}

	function setColumn(id: string, on: boolean) {
		const current = columns.filter((c) => c !== id);
		chosenColumns = on ? [...current, id] : current;
		if (!on) pinned = pinned.filter((p) => p !== id);
	}

	function setPinned(id: string, pin: boolean) {
		const others = pinned.filter((p) => p !== id);
		pinned = pin ? [...others, id] : others;
	}

	function setBaseline(run: RunSummary) {
		baselineId = baselineId === run.id ? null : run.id;
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

	function openRun(run: string) {
		void goto(runHref(run));
	}

	// ---- Against the baseline, and the best of each metric ----
	const cellNote = (run: RunSummary, columnId: string): Change | null => {
		if (!baseline || run.id === baseline.id) return null;
		const key = columnId.slice('summary.'.length);
		const [value, base] = [metricValue(run, key), metricValue(baseline, key)];
		return value === null || base === null ? null : changeFrom(value, base, describe(key));
	};
	const headlines = $derived(
		important.flatMap((key) => headlineFor(matching, key, describe(key), baseline) ?? [])
	);

	/** The run picked out in every chart and in the leaderboard. */
	let highlight = $state<string | null>(null);

	// ---- Charts of training: the important metrics of the first runs ----
	const charted = $derived(chartedRuns(shown, (id) => selection.has(id), baselineId));

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
	let settings = $state<ChartSettings>({ ...DEFAULT_SETTINGS });
	let metricFilter = $state('');
	let view = $state<[number, number] | null>(null);
	let overrides = $state({});

	// ---- Progress and Explore: every matching run, not only the first ones ----
	const rawById = $derived(new Map((list?.runs ?? []).map((r) => [r.id, r])));
	/** The matching runs with this value of each metric. */
	const matchingIn = (mode: ValueMode) =>
		mode === 'default'
			? matching
			: matching.map((r) => withMetrics(rawById.get(r.id)!, registry, mode));
	const noteFor = (key: string, mode: ValueMode) =>
		valueNote(mode, describe(key), registry.isComputed(key));
	const VALUE_MODES: { id: ValueMode; label: string; title?: string }[] = [
		{ id: 'default', label: 'Default', title: "Each metric's summary, as set on its page" },
		{ id: 'last', label: 'Last' },
		{ id: 'best', label: 'Best' }
	];

	// How these two are looked at isn't part of the board: it goes back to the default.
	let progressMode = $state<ValueMode>('default');
	let exploreMode = $state<ValueMode | 'trajectory'>('default');

	const keepDots = $derived(new Set(colors.keys()));
	const progress = $derived.by(() => {
		const runs = matchingIn(progressMode);
		return important.map((key) => {
			const dots = progressDots(runs, key);
			const { line, records } = bestSoFar(dots, describe(key).direction);
			const keep = new Set([...keepDots, ...records]);
			return { key, line, dots: thinDots(dots, keep), total: dots.length };
		});
	});
	const baseValue = (key: string, mode: ValueMode) => {
		const raw = baselineId ? rawById.get(baselineId) : undefined;
		return raw ? metricValue(withMetrics(raw, registry, mode), key) : null;
	};

	const metricKeys = $derived(
		[...choices.important, ...choices.metrics].map((id) => id.slice('summary.'.length))
	);
	const configChoices = $derived(exploreKeys(matching));
	const metricChoices = $derived(metricKeys.map(metricChoice));
	const explore = $derived(
		[...configChoices, ...metricChoices].find((c) => c.key === exploreKey) ??
			configChoices[0] ??
			metricChoices[0] ??
			null
	);
	const exploreName = (c: ExploreKey) => {
		if (c.kind !== 'metric') return c.key;
		const key = choiceMetric(c);
		return describe(key).label || key;
	};
	const exploreCharts = $derived.by(() => {
		if (!explore || exploreMode === 'trajectory') return [];
		const choice = explore;
		const runs = matchingIn(exploreMode);
		return important
			.filter((key) => choice.kind !== 'metric' || key !== choiceMetric(choice))
			.map((key) => {
				const plot = explorePlot(runs, key, choice);
				return { key, plot, dots: thinDots(plot.dots, keepDots) };
			});
	});

	// Trajectories: a metric on x too, since a run's config doesn't move as it trains. By
	// default the most common metric that isn't charted on y, so there's a chart for each.
	let phaseKey = $state<string | null>(null);
	const phaseX = $derived(
		phaseKey && metricKeys.includes(phaseKey)
			? phaseKey
			: (metricKeys.find((k) => !important.includes(k) && !isModelMetric(k)) ??
					metricKeys.find((k) => !isModelMetric(k)) ??
					metricKeys[0] ??
					null)
	);
	const phaseY = $derived(important.filter((key) => key !== phaseX));
</script>

{#snippet modeToggle(
	label: string,
	modes: { id: string; label: string; title?: string }[],
	value: string,
	set: (mode: string) => void
)}
	<ToggleGroup.Root
		type="single"
		variant="outline"
		size="sm"
		{value}
		onValueChange={(v) => v && set(v)}
		aria-label={label}
	>
		{#each modes as m (m.id)}
			<ToggleGroup.Item value={m.id} title={m.title}>{m.label}</ToggleGroup.Item>
		{/each}
	</ToggleGroup.Root>
{/snippet}

{#snippet figure(h: NonNullable<ReturnType<typeof headlineFor>>)}
	<div class="min-w-0 rounded-xl border bg-card p-4">
		<dt class="truncate text-sm text-muted-foreground" title={h.key}>
			<!-- eslint-disable svelte/no-navigation-without-resolve -- metricHref resolves it -->
			<a class="hover:underline" href={metricHref(h.key)}>{describe(h.key).label || h.key}</a>
			<!-- eslint-enable svelte/no-navigation-without-resolve -->
		</dt>
		<dd class="mt-1 truncate text-2xl font-semibold">{formatMetric(h.key, h.value)}</dd>
		<dd class="mt-1 truncate text-xs text-muted-foreground">
			<a class="font-medium text-foreground hover:underline" href={runHref(h.run.id)}
				>{h.run.name}</a
			>
			{#if h.isBaseline}· the baseline leads{/if}
		</dd>
		{#if h.change}
			<dd class="mt-0.5 truncate text-xs text-muted-foreground">
				<span
					class={[
						h.change.better === true && 'text-emerald-700 dark:text-emerald-400',
						h.change.better === false && 'text-destructive'
					]}>{h.change.text}</span
				>
				{h.against === 'baseline' ? 'vs the baseline' : 'vs the runner-up'}
			</dd>
		{/if}
	</div>
{/snippet}

{#if list?.error}
	<ErrorCard error={list.error} what="run list" />
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
		<ColumnsMenu
			runs={all}
			{columns}
			customized={!!chosenColumns}
			onSet={setColumn}
			onReset={() => ((chosenColumns = null), (pinned = []))}
		/>
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
		<span class="min-w-14 text-xs whitespace-nowrap text-muted-foreground" aria-live="polite">
			{status === 'waiting'
				? 'Will save when back'
				: status === 'pending' || status === 'saving'
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
				<a
					class={buttonVariants({ variant: 'outline', size: 'sm', class: 'h-7' })}
					href={compareHref}
				>
					<GitCompareArrows /> Compare
				</a>
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
				{top.length.toLocaleString('en')} of {others.length.toLocaleString('en')}
				{parsed.node ? 'matching' : ''} runs{#if baseline}, and the baseline{/if}{#if list?.partial}&nbsp;(loading
					more…){/if}.
				{#if canEdit && !baseline}Flag a run to measure the others against it.{/if}
				{#if !canEdit}Only editors can change what the board shows.{/if}
			</span>
			{#if shown.length > 1}
				<Button variant="outline" size="sm" class="h-7" onclick={compareShown}>
					<GitCompareArrows /> Compare these {shown.length}
				</Button>
			{/if}
		{/if}
		{#if baselineId && !baseline && list && !list.partial}
			<span class="inline-flex items-center gap-1 text-muted-foreground">
				<Flag class="size-3" /> The baseline run is gone (archived or deleted).
			</span>
			{#if canEdit}
				<Button variant="ghost" size="sm" class="h-7" onclick={() => (baselineId = null)}>
					<X /> Forget it
				</Button>
			{/if}
		{/if}
	</div>

	{#if headlines.length}
		<dl class="mb-4 grid gap-4 sm:grid-cols-2 lg:grid-cols-4" aria-label="Best runs">
			{#each headlines as h (h.key)}{@render figure(h)}{/each}
		</dl>
	{/if}

	<RunsTable
		{project}
		runs={shown}
		{columns}
		{pinned}
		bind:sort
		colors={important.length ? colors : NO_COLORS}
		{selection}
		collapsed={NO_COLLAPSED}
		maxRows={MAX_ROWS}
		onTag={addTag}
		onPin={setPinned}
		onHide={(id) => setColumn(id, false)}
		onSelect={(runs, on) =>
			selection.toggle(
				runs.map((r) => r.id),
				on
			)}
		{highlight}
		onHover={(run) => (highlight = run)}
		{baselineId}
		onBaseline={canEdit ? setBaseline : undefined}
		cellNote={baseline ? cellNote : undefined}
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
		{#if important.length}
			<section class="mt-8" aria-labelledby="board-progress">
				<div class="mb-3 flex flex-wrap items-center gap-x-3 gap-y-2">
					<div class="min-w-0 flex-1">
						<h2 id="board-progress" class="text-sm font-medium">Progress over time</h2>
						<p class="text-xs text-muted-foreground">
							Every matching run, by when it started; the line is the best so far.
							{#if baseline}The dashed line is the baseline.{/if}
							Colored dots are charted below, hollow ones still running. Click a dot to
							open its run.
						</p>
					</div>
					{@render modeToggle(
						'Value shown in progress',
						VALUE_MODES,
						progressMode,
						(mode) => (progressMode = mode as ValueMode)
					)}
				</div>
				<div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
					{#each progress as p (p.key)}
						<DotChart
							metric={p.key}
							href={metricHref(p.key)}
							dots={p.dots}
							line={p.line}
							{colors}
							x={{ kind: 'time' }}
							xName="Started"
							baseline={baseValue(p.key, progressMode)}
							{highlight}
							onHover={(run) => (highlight = run)}
							onOpen={openRun}
							note={`${noteFor(p.key, progressMode)}${p.dots.length < p.total ? `, ${p.dots.length.toLocaleString('en')} of ${p.total.toLocaleString('en')} runs` : ''}`}
						/>
					{/each}
				</div>
			</section>

			<section class="mt-8" aria-labelledby="board-explore">
				<div class="mb-3 flex flex-wrap items-center gap-x-3 gap-y-2">
					<div class="min-w-0 flex-1">
						<h2 id="board-explore" class="text-sm font-medium">Explore</h2>
						<p class="text-xs text-muted-foreground">
							{#if exploreMode === 'trajectory'}
								The path of each charted run through two metrics as it trains, from
								○ its start to ● where it has got to.
							{:else}
								Every matching run: how a config value or another metric relates to
								each metric.
							{/if}
						</p>
					</div>
					{#if exploreMode === 'trajectory'}
						{#if metricKeys.length && phaseX}
							<Select.Root
								type="single"
								value={phaseX}
								onValueChange={(v) => v && (phaseKey = v)}
							>
								<Select.Trigger
									size="sm"
									class="w-64"
									aria-label="Metric on the x-axis"
								>
									<span class="truncate font-mono text-xs"
										>{describe(phaseX).label || phaseX}</span
									>
								</Select.Trigger>
								<Select.Content class="max-h-80">
									{#each metricKeys as key (key)}
										<Select.Item
											value={key}
											label={describe(key).label || key}
										/>
									{/each}
								</Select.Content>
							</Select.Root>
						{/if}
					{:else if explore}
						<Select.Root
							type="single"
							value={explore.key}
							onValueChange={(v) => v && (exploreKey = v)}
						>
							<Select.Trigger size="sm" class="w-64" aria-label="Plotted against">
								<span class="truncate font-mono text-xs"
									>{exploreName(explore)}</span
								>
							</Select.Trigger>
							<Select.Content class="max-h-80">
								{#if configChoices.length}
									<Select.Group>
										<Select.GroupHeading>Config</Select.GroupHeading>
										{#each configChoices as c (c.key)}
											<Select.Item value={c.key} label={exploreName(c)} />
										{/each}
									</Select.Group>
								{/if}
								{#if metricChoices.length}
									<Select.Group>
										<Select.GroupHeading>Metrics</Select.GroupHeading>
										{#each metricChoices as c (c.key)}
											<Select.Item value={c.key} label={exploreName(c)} />
										{/each}
									</Select.Group>
								{/if}
							</Select.Content>
						</Select.Root>
					{/if}
					{@render modeToggle(
						'Value shown in explore',
						[...VALUE_MODES, { id: 'trajectory', label: 'Trajectory' }],
						exploreMode,
						(mode) => (exploreMode = mode as ValueMode | 'trajectory')
					)}
				</div>
				{#if exploreMode === 'trajectory'}
					{#if phaseX && phaseY.length}
						<Trajectories
							{project}
							runs={charted}
							{colors}
							xKey={phaseX}
							yKeys={phaseY}
							{highlight}
							onHover={(run) => (highlight = run)}
							onOpen={openRun}
							{metricHref}
						/>
					{:else}
						<p class="text-sm text-muted-foreground">
							A trajectory takes two metrics: these runs need another one to plot
							against, or pick a different one above.
						</p>
					{/if}
				{:else if explore && exploreCharts.length}
					<div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
						{#each exploreCharts as c (c.key)}
							{@const mode = exploreMode as ValueMode}
							<DotChart
								metric={c.key}
								href={metricHref(c.key)}
								dots={c.dots}
								{colors}
								x={explore.kind === 'metric'
									? {
											kind: 'metric',
											key: choiceMetric(explore),
											log: c.plot.log
										}
									: c.plot.categories
										? { kind: 'category', names: c.plot.categories }
										: { kind: 'number', log: c.plot.log }}
								xName={exploreName(explore)}
								baseline={baseValue(c.key, mode)}
								{highlight}
								onHover={(run) => (highlight = run)}
								onOpen={openRun}
								note={`${noteFor(c.key, mode)}${c.plot.log ? ', log scale' : ''}`}
							/>
						{/each}
					</div>
				{:else}
					<p class="text-sm text-muted-foreground">
						These runs share the same config, and have no other metric to plot against.
					</p>
				{/if}
			</section>

			<section class="mt-8" aria-labelledby="board-curves">
				<h2 id="board-curves" class="text-sm font-medium">Training curves</h2>
				<div class="mt-1 mb-3 flex min-h-4 flex-wrap gap-x-4 gap-y-1 text-xs">
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
					{highlight}
					onRunState={() => list?.refresh()}
					empty={{
						icon: Star,
						title: 'No points yet',
						text: 'These runs have important metrics, but nothing to chart yet.'
					}}
				/>
			</section>
		{:else}
			<div class="mt-6">
				<EmptyCard icon={Star} title="No important metrics in these runs">
					{#snippet description()}
						The board charts the metrics that matter most: mark them with a star on the
						<a class="underline" href={metricsHref}>Metrics tab</a>.
					{/snippet}
				</EmptyCard>
			</div>
		{/if}
	{/if}
{/if}
