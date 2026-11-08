<script lang="ts">
	import Archive from '@lucide/svelte/icons/archive';
	import ArchiveRestore from '@lucide/svelte/icons/archive-arrow-up';
	import FlaskConical from '@lucide/svelte/icons/flask-conical';
	import GitCompareArrows from '@lucide/svelte/icons/git-compare-arrows';
	import Group from '@lucide/svelte/icons/group';
	import X from '@lucide/svelte/icons/x';
	import { SvelteSet } from 'svelte/reactivity';
	import { toast } from 'svelte-sonner';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	import { apiJson, apiSend } from '#lib/api/client.js';
	import type { ProjectInfo, RunSummary } from '#lib/api/types.js';
	import type { ChartSeries, ChartSettings } from '#lib/charts/settings.js';
	import AbortRuns from '#lib/components/app/AbortRuns.svelte';
	import EmptyCard from '#lib/components/app/EmptyCard.svelte';
	import ErrorCard from '#lib/components/app/ErrorCard.svelte';
	import SdkQuickstart from '#lib/components/app/SdkQuickstart.svelte';
	import ChartsPanel from '#lib/components/app/ChartsPanel.svelte';
	import ColumnsMenu from '#lib/components/app/ColumnsMenu.svelte';
	import FilterHelp from '#lib/components/app/FilterHelp.svelte';
	import FilterInput from '#lib/components/app/FilterInput.svelte';
	import RunsTable from '#lib/components/app/RunsTable.svelte';
	import TagsMenu from '#lib/components/app/TagsMenu.svelte';
	import { Button, buttonVariants } from '#lib/components/ui/button/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as ToggleGroup from '#lib/components/ui/toggle-group/index.js';
	import { FilterError, matches, parseFilter, type Node } from '#lib/filter.js';
	import { literal } from '#lib/filter-suggest.js';
	import { buildCatalog } from '#lib/runs/catalog.js';
	import { withMetrics } from '#lib/metrics/apply.js';
	import { currentRegistry } from '#lib/metrics/store.svelte.js';
	import { errorMessage } from '#lib/notify.js';
	import { poll, type Polled } from '#lib/poll.svelte.js';
	import { RunList } from '#lib/runs/list.svelte.js';
	import { selectionOf } from '#lib/runs/selection.svelte.js';
	import { nav } from '#lib/nav.svelte.js';
	import { projectsChanged } from '#lib/projects.svelte.js';
	import {
		availableKeys,
		column,
		compareRuns,
		defaultColumns,
		groupRuns
	} from '#lib/runs/columns.js';
	import {
		chartSettingsParams,
		overridesParam,
		param,
		readChartSettings,
		readOverrides,
		readZoom,
		syncUrl,
		zoomParam
	} from '#lib/url-state.svelte.js';

	/** Runs (or groups) charted when none are picked: the first ones of the table. */
	const AUTO_CHARTED = 5;
	const MAX_CHARTED = 40;
	const PALETTE = 8;
	const shortId = (id: string) => id.replaceAll('-', '');

	const project = $derived(page.params.project ?? '');
	const projectPath = $derived(`/projects/${encodeURIComponent(project)}`);
	// Run pages link back to the list as it is now.
	$effect(() => {
		nav.runsQuery[project] = page.url.search;
	});

	// ---- View state, mirrored in the URL ----
	let archived = $state(param('archived') === '1');
	let filterText = $state(param('q') ?? '');
	const sortParam = param('sort');
	let sort = $state(
		sortParam
			? { id: sortParam.replace(/^-/, ''), desc: sortParam.startsWith('-') }
			: { id: 'created', desc: true }
	);
	let chosenColumns = $state<string[] | null>(param('cols')?.split(',').filter(Boolean) ?? null);
	let pinned = $state<string[]>(param('pin')?.split(',').filter(Boolean) ?? []);
	let groupBy = $state<string | null>(param('group'));
	/** Picked runs: charted below the table, acted on in bulk (shared with other pages). */
	const selection = $derived(selectionOf(project));
	// A link with runs picks exactly those.
	const linked = param('runs')?.split(',').filter(Boolean);
	if (linked?.length) selectionOf(page.params.project ?? '').replace(linked);
	let settings = $state<ChartSettings>(readChartSettings());
	let metricFilter = $state(param('metrics') ?? '');
	let view = $state(readZoom());
	let overrides = $state(readOverrides());

	syncUrl(() => ({
		archived: archived ? '1' : null,
		q: filterText.trim() || null,
		sort: sort.id === 'created' && sort.desc ? null : `${sort.desc ? '-' : ''}${sort.id}`,
		cols: chosenColumns?.join(',') ?? null,
		pin: pinned.join(',') || null,
		group: groupBy,
		runs: selection.size && selection.size <= MAX_CHARTED ? selection.ids.join(',') : null,
		...chartSettingsParams(settings),
		metrics: metricFilter.trim() || null,
		zoom: zoomParam(view),
		charts: overridesParam(overrides)
	}));

	const info: Polled<ProjectInfo> = poll(
		() => apiJson<ProjectInfo>(projectPath),
		() => 30_000
	);
	const canEdit = $derived(info.data?.role === 'owner' || info.data?.role === 'editor');
	/** The runs of the project (or its archived ones): newest first, then kept current. */
	let list = $state<RunList | null>(null);
	$effect(() => {
		const l = new RunList(projectPath, archived, (n) => (n > 5_000 ? 30_000 : 10_000));
		list = l;
		l.start();
		return () => l.stop();
	});
	/** Runs with the project's custom metrics applied to their summaries. */
	const all = $derived((list?.runs ?? []).map((r) => withMetrics(r, currentRegistry())));
	const runById = $derived(new Map(all.map((r) => [r.id, r])));
	const collapsed = new SvelteSet<string>();

	// ---- Filter, sort, group ----
	let lastValid: Node | null = null;
	const parsed = $derived.by((): { node: Node | null; error: FilterError | null } => {
		try {
			lastValid = parseFilter(filterText);
			return { node: lastValid, error: null };
		} catch (err) {
			if (!(err instanceof FilterError)) throw err;
			// Keep showing the last valid result while the filter is being typed.
			return { node: lastValid, error: err };
		}
	});
	const filtered = $derived(all.filter((r) => matches(parsed.node, r)));
	const sorted = $derived([...filtered].sort(compareRuns(sort.id, sort.desc)));
	const running = $derived(all.filter((r) => r.state === 'running').length);
	const groups = $derived(groupBy ? groupRuns(sorted, groupBy) : null);
	/** Keys and values of all runs, for filter suggestions (built when first needed). */
	const catalog = $derived(buildCatalog(all));

	// ---- Columns ----
	const columns = $derived(chosenColumns ?? defaultColumns(all));
	const keys = $derived(availableKeys(all));
	/** Columns runs can be grouped by: run fields and config values. */
	const groupChoices = $derived([
		'group',
		'state',
		'tags',
		...keys.config.slice(0, 100).map((k) => `config.${k}`)
	]);

	function setColumn(id: string, on: boolean) {
		const current = columns.filter((c) => c !== id);
		chosenColumns = on ? [...current, id] : current;
		if (!on) pinned = pinned.filter((p) => p !== id);
	}

	function setPinned(id: string, pin: boolean) {
		const others = pinned.filter((p) => p !== id);
		pinned = pin ? [...others, id] : others;
	}

	// ---- Charted runs ----
	const byId = $derived(new Map(all.map((r) => [shortId(r.id), r])));
	/** Picked runs in this list, in the order picked. */
	const selectedRuns = $derived(
		selection.ids.map((id) => byId.get(id)).filter((r): r is RunSummary => !!r)
	);
	const charted = $derived.by((): RunSummary[] => {
		if (selection.size) return selectedRuns.slice(0, MAX_CHARTED);
		if (!groups) return sorted.slice(0, AUTO_CHARTED);
		const out: RunSummary[] = [];
		for (const g of groups.slice(0, AUTO_CHARTED)) out.push(...g.runs);
		return out.slice(0, MAX_CHARTED);
	});
	/** What the charts draw: one line per run, or per group of runs. */
	const series = $derived.by((): ChartSeries[] => {
		const color = (i: number) => `var(--chart-${(i % PALETTE) + 1})`;
		const chartRun = (r: RunSummary) => ({ id: r.id, startMs: r.start_ms });
		if (!groupBy) {
			return charted.map((r, i) => ({
				id: r.id,
				name: r.name,
				color: color(i),
				runs: [chartRun(r)]
			}));
		}
		return groupRuns(charted, groupBy).map((g, i) => ({
			id: `group:${g.key}`,
			name: g.label,
			color: color(i),
			runs: g.runs.map(chartRun)
		}));
	});
	const colors = $derived(
		new Map(series.flatMap((s) => s.runs.map((r): [string, string] => [r.id, s.color])))
	);
	const chartedRunning = $derived(charted.filter((r) => r.state === 'running').map((r) => r.id));

	function select(runs: RunSummary[], on: boolean) {
		selection.toggle(
			runs.map((r) => r.id),
			on
		);
	}

	/** Shows the runs with this tag (added to the filter). */
	function filterTag(tag: string) {
		const term = `tag:${literal(tag)}`;
		const current = filterText.trim();
		if (!current.split(/\s+/).includes(term))
			filterText = current ? `${current} ${term}` : term;
	}

	function toggleGroup(key: string) {
		if (collapsed.has(key)) collapsed.delete(key);
		else collapsed.add(key);
	}

	// ---- Archive and restore ----

	async function setArchived(ids: string[], archive: boolean, undoable = true) {
		const action = archive ? 'archive' : 'restore';
		const pending = toast.loading(`${archive ? 'Archiving' : 'Restoring'} ${ids.length} runs…`);
		try {
			const { changed } = await apiSend<{ changed: number }>(
				'POST',
				`${projectPath}/runs/${action}`,
				{ runs: ids }
			);
			const days = info.data?.archive_days;
			const what = `${changed} ${changed === 1 ? 'run' : 'runs'}`;
			toast.success(archive ? `${what} archived` : `${what} restored`, {
				id: pending,
				description: archive
					? `Hidden from the run list; deleted for good in ${days ?? 30} days unless restored (see Archived).`
					: `Back in the run list of ${project}.`,
				action: undoable
					? { label: 'Undo', onClick: () => void setArchived(ids, !archive, false) }
					: undefined
			});
			selection.toggle(ids, false);
			list?.refresh();
			info.refresh();
			projectsChanged();
		} catch (err) {
			toast.error(archive ? "Couldn't archive the runs" : "Couldn't restore the runs", {
				id: pending,
				description: errorMessage(err)
			});
		}
	}
</script>

<div>
	<div class="mb-3 flex flex-wrap items-center justify-between gap-2">
		<p class="text-sm text-muted-foreground tabular-nums">
			{all.length.toLocaleString('en')}
			{archived ? 'archived' : ''} runs{#if list?.partial}&nbsp;(loading more…){/if}{#if running}{` · ${running} running`}{/if}{#if parsed.node}{` · ${filtered.length.toLocaleString('en')} match`}{/if}
		</p>
		<div class="flex flex-wrap items-center gap-2">
			<ToggleGroup.Root
				type="single"
				variant="outline"
				size="sm"
				value={archived ? 'archived' : 'runs'}
				onValueChange={(v) => v && (archived = v === 'archived')}
			>
				<ToggleGroup.Item value="runs">
					Runs <span class="text-muted-foreground tabular-nums"
						>{info.data?.runs ?? ''}</span
					>
				</ToggleGroup.Item>
				<ToggleGroup.Item value="archived">
					<Archive /> Archived
					<span class="text-muted-foreground tabular-nums"
						>{info.data?.archived_runs ?? ''}</span
					>
				</ToggleGroup.Item>
			</ToggleGroup.Root>
		</div>
	</div>

	{#if list?.error}
		<ErrorCard error={list.error} what="run list" />
	{:else if !list?.runs}
		<Skeleton class="h-64 rounded-xl" />
	{:else if all.length === 0 && archived}
		<EmptyCard icon={Archive} title="No archived runs">
			{#snippet description()}
				Archived runs are hidden from the project and kept {info.data?.archive_days ?? 30} days,
				then deleted for good (change it in
				<a class="underline" href={resolve('/p/[project]/(tabs)/settings', { project })}
					>Settings</a
				>). Archive runs by ticking them in the table.
			{/snippet}
		</EmptyCard>
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
				bind:value={filterText}
				catalog={() => catalog}
				invalid={!!parsed.error}
				describedby="filter-help"
			/>
			<FilterHelp />
			<Select.Root
				type="single"
				value={groupBy ?? ''}
				onValueChange={(v) => {
					groupBy = v || null;
					collapsed.clear();
				}}
			>
				<Select.Trigger size="sm" class="w-48" aria-label="Group by">
					<span class="flex min-w-0 items-center gap-2">
						<Group class="size-4 shrink-0" />
						<span class="truncate">
							{groupBy ? `Group: ${column(groupBy).label}` : 'No grouping'}
						</span>
					</span>
				</Select.Trigger>
				<Select.Content class="max-h-80">
					<Select.Item value="" label="No grouping" />
					{#each groupChoices as id (id)}
						<Select.Item
							value={id}
							label={column(id).prefix
								? `${column(id).prefix}.${column(id).label}`
								: column(id).label}
						/>
					{/each}
				</Select.Content>
			</Select.Root>
			<ColumnsMenu
				runs={all}
				{columns}
				customized={!!chosenColumns}
				onSet={setColumn}
				onReset={() => ((chosenColumns = null), (pinned = []))}
			/>
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
					{selection.size.toLocaleString('en')} selected{#if selectedRuns.length < selection.size}<span
							class="font-normal text-muted-foreground"
						>
							({selectedRuns.length} in this list)</span
						>{/if}
				</span>
				<span class="text-muted-foreground">
					· {selectedRuns.length > MAX_CHARTED
						? `the first ${MAX_CHARTED} are charted below`
						: 'charted below'}
				</span>
				{#if selection.size > 1}
					<a
						class={buttonVariants({ variant: 'outline', size: 'sm', class: 'h-7' })}
						href={resolve('/p/[project]/(tabs)/compare', { project })}
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
				{#if canEdit && selectedRuns.length}
					<TagsMenu {project} runs={selectedRuns} onChanged={() => list?.refresh()} />
					<AbortRuns
						{project}
						runs={selectedRuns}
						class="h-7"
						onDone={() => list?.refresh()}
					/>
					{#if archived}
						<Button
							variant="outline"
							size="sm"
							class="h-7"
							onclick={() =>
								setArchived(
									selectedRuns.map((r) => r.id),
									false
								)}
						>
							<ArchiveRestore /> Restore
						</Button>
					{:else}
						<Button
							variant="outline"
							size="sm"
							class="h-7"
							onclick={() =>
								setArchived(
									selectedRuns.map((r) => r.id),
									true
								)}
						>
							<Archive /> Archive
						</Button>
					{/if}
				{/if}
				<Button variant="ghost" size="sm" class="h-7" onclick={() => selection.clear()}>
					<X /> Clear selection
				</Button>
			{:else}
				<span class="text-muted-foreground">
					{#if groupBy}
						Charting the first {Math.min(AUTO_CHARTED, groups?.length ?? 0)} groups (mean
						of their runs); tick runs to chart them, tag or archive them.
					{:else}
						Charting the first {Math.min(AUTO_CHARTED, sorted.length)} runs of the table;
						tick runs to chart them, tag or archive them.
					{/if}
				</span>
			{/if}
		</div>

		<RunsTable
			{project}
			runs={sorted}
			{columns}
			{pinned}
			bind:sort
			{colors}
			{selection}
			{groupBy}
			{collapsed}
			archiveDays={archived ? (info.data?.archive_days ?? null) : null}
			onTag={filterTag}
			onSelect={select}
			onToggleGroup={toggleGroup}
			onPin={setPinned}
			onHide={(id) => setColumn(id, false)}
		/>
		{#if sorted.length === 0}
			<p class="mt-2 text-sm text-muted-foreground">
				No run matches. <Button
					variant="link"
					class="h-auto p-0"
					onclick={() => (filterText = '')}>Clear the filter</Button
				>
			</p>
		{/if}

		<div class="mt-6 mb-3 flex min-h-4 flex-wrap gap-x-4 gap-y-1 text-xs">
			{#if series.length > 1 || groupBy}
				{#each series as s (s.id)}
					<span class="inline-flex items-center gap-1.5">
						<span class="size-2.5 rounded-sm" style:background-color={s.color}></span>
						{s.name}
						{#if s.runs.length > 1}<span class="text-muted-foreground"
								>({s.runs.length} runs)</span
							>{/if}
					</span>
				{/each}
			{/if}
		</div>

		<ChartsPanel
			{project}
			{series}
			running={chartedRunning}
			bind:settings
			bind:metricFilter
			bind:view
			bind:overrides
			renames={(id) => runById.get(id)}
			summaryOf={(id) => runById.get(id)?.summary}
			onRunState={() => list?.refresh()}
		/>
	{/if}
</div>
