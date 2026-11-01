<script lang="ts">
	import Archive from '@lucide/svelte/icons/archive';
	import ArchiveRestore from '@lucide/svelte/icons/archive-arrow-up';
	import ChartLine from '@lucide/svelte/icons/chart-line';
	import CircleQuestionMark from '@lucide/svelte/icons/circle-question-mark';
	import Columns3 from '@lucide/svelte/icons/columns-3';
	import Funnel from '@lucide/svelte/icons/funnel';
	import Group from '@lucide/svelte/icons/group';
	import Settings from '@lucide/svelte/icons/settings';
	import X from '@lucide/svelte/icons/x';
	import { SvelteSet } from 'svelte/reactivity';
	import { toast } from 'svelte-sonner';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	import { apiJson, apiSend } from '#lib/api/client.js';
	import type { ProjectInfo, RunSummary } from '#lib/api/types.js';
	import type { ChartSeries, ChartSettings } from '#lib/charts/settings.js';
	import SdkQuickstart from '#lib/components/app/SdkQuickstart.svelte';
	import ChartsPanel from '#lib/components/app/ChartsPanel.svelte';
	import RunsTable from '#lib/components/app/RunsTable.svelte';
	import { Button, buttonVariants } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as ToggleGroup from '#lib/components/ui/toggle-group/index.js';
	import { FilterError, matches, parseFilter, type Node } from '#lib/filter.js';
	import { errorMessage } from '#lib/notify.js';
	import { poll, type Polled } from '#lib/poll.svelte.js';
	import { projectsChanged } from '#lib/projects.svelte.js';
	import {
		availableKeys,
		BASE_COLUMNS,
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
	/** Charted run ids (32 hex), or null: the first runs (or groups) of the table. */
	let picked = $state<string[] | null>(param('runs')?.split(',').filter(Boolean) ?? null);
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
		runs: picked?.join(',') ?? null,
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
	const runs: Polled<RunSummary[]> = poll(
		() => apiJson<RunSummary[]>(`${projectPath}/runs${archived ? '?archived=true' : ''}`),
		() => ((runs.data?.length ?? 0) > 5_000 ? 60_000 : 10_000)
	);
	const all = $derived(runs.data ?? []);

	// Selection for bulk actions; cleared when the list changes meaning.
	const selected = new SvelteSet<string>();
	const collapsed = new SvelteSet<string>();
	$effect(() => {
		void archived;
		void project;
		selected.clear();
	});

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

	// ---- Columns ----
	const columns = $derived(chosenColumns ?? defaultColumns(all));
	let columnQuery = $state('');
	const keys = $derived(availableKeys(all));
	const columnChoices = $derived.by(() => {
		const q = columnQuery.trim().toLowerCase();
		const choices: [string, string[]][] = [
			['Run', BASE_COLUMNS],
			['Config', keys.config.map((k) => `config.${k}`)],
			['Summary', keys.summary.map((k) => `summary.${k}`)]
		];
		return choices
			.map(([label, ids]): [string, string[]] => [
				label,
				ids.filter((id) => !q || id.toLowerCase().includes(q)).slice(0, 100)
			])
			.filter(([, ids]) => ids.length);
	});
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
	const charted = $derived.by((): RunSummary[] => {
		if (picked) return picked.map((id) => byId.get(id)).filter((r): r is RunSummary => !!r);
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

	function chart(list: RunSummary[], on: boolean) {
		const ids = list.map((r) => shortId(r.id));
		const current = picked ?? charted.map((r) => shortId(r.id));
		const next = on
			? [...current, ...ids.filter((id) => !current.includes(id))]
			: current.filter((id) => !ids.includes(id));
		if (next.length > MAX_CHARTED) {
			toast.warning(`Too many runs to chart`, {
				description: `At most ${MAX_CHARTED} runs can be charted at once; hide some first.`
			});
			return;
		}
		picked = next;
	}

	function select(list: RunSummary[], on: boolean) {
		for (const r of list) {
			if (on) selected.add(r.id);
			else selected.delete(r.id);
		}
	}

	function toggleGroup(key: string) {
		if (collapsed.has(key)) collapsed.delete(key);
		else collapsed.add(key);
	}

	// ---- Archive and restore ----
	const selectedRuns = $derived(all.filter((r) => selected.has(r.id)));

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
			selected.clear();
			runs.refresh();
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

<div class="mx-auto max-w-[1600px]">
	<div class="mb-4 flex flex-wrap items-start justify-between gap-2">
		<div>
			<h1 class="text-2xl font-semibold tracking-tight">{project}</h1>
			<p class="text-sm text-muted-foreground tabular-nums">
				{all.length}
				{archived ? 'archived' : ''} runs{#if running}{` · ${running} running`}{/if}{#if parsed.node}{` · ${filtered.length} match`}{/if}
			</p>
		</div>
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
			<a
				class={buttonVariants({ variant: 'outline', size: 'sm' })}
				href={resolve('/p/[project]/settings', { project })}
			>
				<Settings /> Settings
			</a>
		</div>
	</div>

	{#if runs.error}
		<p class="text-sm text-destructive">{String(runs.error)}</p>
	{:else if !runs.data}
		<Skeleton class="h-64 rounded-xl" />
	{:else if all.length === 0 && archived}
		<p class="text-sm text-muted-foreground">
			No archived runs. Archived runs are kept {info.data?.archive_days ?? 30} days, then deleted
			for good (change it in
			<a class="underline" href={resolve('/p/[project]/settings', { project })}>Settings</a>).
		</p>
	{:else if all.length === 0}
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
				<SdkQuickstart {project} />
			</Card.Content>
		</Card.Root>
	{:else}
		<div class="mb-2 flex flex-wrap items-center gap-2">
			<div class="relative min-w-64 flex-1">
				<Funnel
					class="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
				/>
				<Input
					class="pl-8 font-mono text-xs"
					placeholder="config.lr < 1e-3 and tag:baseline and not state:crashed"
					aria-label="Filter runs"
					aria-invalid={!!parsed.error}
					aria-describedby="filter-help"
					spellcheck={false}
					bind:value={filterText}
				/>
			</div>
			<DropdownMenu.Root>
				<DropdownMenu.Trigger
					class={buttonVariants({ variant: 'ghost', size: 'icon-sm' })}
					aria-label="Filter syntax"
				>
					<CircleQuestionMark />
				</DropdownMenu.Trigger>
				<DropdownMenu.Content align="end" class="w-96 p-3 text-xs leading-relaxed">
					<p class="mb-2 font-medium text-foreground">Filter syntax</p>
					<dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-muted-foreground">
						<dt class="font-mono text-foreground">config.lr &lt; 1e-3</dt>
						<dd>compare numbers: = != &lt; &lt;= &gt; &gt;=</dd>
						<dt class="font-mono text-foreground">summary.val/acc &gt; 0.9</dt>
						<dd>any config or summary key</dd>
						<dt class="font-mono text-foreground">tag:baseline</dt>
						<dd>also state:, alias:, group:, name:</dd>
						<dt class="font-mono text-foreground">name ~ "^bert"</dt>
						<dd>regular expression</dd>
						<dt class="font-mono text-foreground">a or (b and not c)</dt>
						<dd>combine; spaces mean and</dd>
						<dt class="font-mono text-foreground">falcon</dt>
						<dd>search names, tags, groups, aliases</dd>
					</dl>
				</DropdownMenu.Content>
			</DropdownMenu.Root>
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
			<DropdownMenu.Root onOpenChange={(open) => !open && (columnQuery = '')}>
				<DropdownMenu.Trigger class={buttonVariants({ variant: 'outline', size: 'sm' })}>
					<Columns3 /> Columns
				</DropdownMenu.Trigger>
				<DropdownMenu.Content align="end" class="max-h-96 w-72">
					<div class="p-1">
						<Input
							class="h-8"
							placeholder="Find a column"
							aria-label="Find a column"
							bind:value={columnQuery}
							onkeydown={(e) => e.stopPropagation()}
						/>
					</div>
					{#each columnChoices as [label, ids] (label)}
						<DropdownMenu.Group>
							<DropdownMenu.GroupHeading>{label}</DropdownMenu.GroupHeading>
							{#each ids as id (id)}
								<DropdownMenu.CheckboxItem
									checked={columns.includes(id)}
									closeOnSelect={false}
									onCheckedChange={(on) => setColumn(id, on)}
								>
									<span class="truncate">{column(id).label}</span>
								</DropdownMenu.CheckboxItem>
							{/each}
						</DropdownMenu.Group>
					{:else}
						<p class="px-2 py-1.5 text-sm text-muted-foreground">No column matches.</p>
					{/each}
					{#if chosenColumns}
						<DropdownMenu.Separator />
						<DropdownMenu.Item onSelect={() => ((chosenColumns = null), (pinned = []))}>
							Reset to defaults
						</DropdownMenu.Item>
					{/if}
				</DropdownMenu.Content>
			</DropdownMenu.Root>
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
			{:else if selected.size}
				<span class="font-medium tabular-nums">{selected.size} selected</span>
				<Button
					variant="outline"
					size="sm"
					class="h-7"
					onclick={() => {
						if (selectedRuns.length > MAX_CHARTED) {
							toast.warning(`Too many runs to chart`, {
								description: `At most ${MAX_CHARTED} runs can be charted at once; select fewer.`
							});
						} else {
							picked = selectedRuns.map((r) => shortId(r.id));
						}
					}}
				>
					<ChartLine /> Chart them
				</Button>
				{#if canEdit}
					{#if archived}
						<Button
							variant="outline"
							size="sm"
							class="h-7"
							onclick={() => setArchived([...selected], false)}
						>
							<ArchiveRestore /> Restore
						</Button>
					{:else}
						<Button
							variant="outline"
							size="sm"
							class="h-7"
							onclick={() => setArchived([...selected], true)}
						>
							<Archive /> Archive
						</Button>
					{/if}
				{/if}
				<Button variant="ghost" size="sm" class="h-7" onclick={() => selected.clear()}>
					<X /> Clear
				</Button>
			{:else}
				<span class="text-muted-foreground">
					{#if picked}
						{charted.length} runs charted ·
						<button
							class="underline hover:text-foreground"
							onclick={() => (picked = null)}
							>chart the first {AUTO_CHARTED}
							{groupBy ? 'groups' : 'runs'} instead</button
						>
					{:else if groupBy}
						Charting the first {Math.min(AUTO_CHARTED, groups?.length ?? 0)} groups (mean
						of their runs); click the dots to choose.
					{:else}
						Charting the first {Math.min(AUTO_CHARTED, sorted.length)} runs of the table;
						click the dots to choose, tick runs to archive them.
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
			{selected}
			{groupBy}
			{collapsed}
			archiveDays={archived ? (info.data?.archive_days ?? null) : null}
			onChart={chart}
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
			onRunState={() => runs.refresh()}
		/>
	{/if}
</div>
