<script lang="ts">
	import Archive from '@lucide/svelte/icons/archive';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import ArchiveRestore from '@lucide/svelte/icons/archive-arrow-up';
	import AtSign from '@lucide/svelte/icons/at-sign';
	import Tag from '@lucide/svelte/icons/tag';
	import Check from '@lucide/svelte/icons/check';
	import Copy from '@lucide/svelte/icons/copy';
	import Cpu from '@lucide/svelte/icons/cpu';
	import GitCommitHorizontal from '@lucide/svelte/icons/git-commit-horizontal';
	import GitCompareArrows from '@lucide/svelte/icons/git-compare-arrows';
	import Plus from '@lucide/svelte/icons/plus';
	import Server from '@lucide/svelte/icons/server';
	import X from '@lucide/svelte/icons/x';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	import { apiJson, apiSend } from '#lib/api/client.js';
	import type { ProjectInfo, RunDetail } from '#lib/api/types.js';
	import {
		DEFAULT_SETTINGS,
		type ChartSeries,
		type ChartSettings
	} from '#lib/charts/settings.js';
	import AbortRuns from '#lib/components/app/AbortRuns.svelte';
	import ChartsPanel from '#lib/components/app/ChartsPanel.svelte';
	import ConsoleLog from '#lib/components/app/ConsoleLog.svelte';
	import ModelInfo from '#lib/components/app/ModelInfo.svelte';
	import RunConfig from '#lib/components/app/RunConfig.svelte';
	import RunEnvironmentCards from '#lib/components/app/RunEnvironmentCards.svelte';
	import KeyValueList from '#lib/components/app/KeyValueList.svelte';
	import StateBadge from '#lib/components/app/StateBadge.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { buttonVariants } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { dateTime, duration, flatten, timeAgo } from '#lib/format.js';
	import { formatMetric, formatSteps } from '#lib/metric-units.js';
	import { effectiveSummary } from '#lib/metrics/apply.js';
	import { currentRegistry, useProjectMetrics } from '#lib/metrics/store.svelte.js';
	import TagChip from '#lib/components/app/TagChip.svelte';
	import TagsMenu from '#lib/components/app/TagsMenu.svelte';
	import UnderlineTabs from '#lib/components/app/UnderlineTabs.svelte';
	import { literal } from '#lib/filter-suggest.js';
	import { changeTags } from '#lib/tags/actions.js';
	import { useProjectTags } from '#lib/tags/store.svelte.js';
	import { nav } from '#lib/nav.svelte.js';
	import { selectionOf } from '#lib/runs/selection.svelte.js';
	import { notifyError, withToast } from '#lib/notify.js';
	import { projectsChanged } from '#lib/projects.svelte.js';
	import { toast } from 'svelte-sonner';
	import ErrorCard from '#lib/components/app/ErrorCard.svelte';
	import { poll, type Polled } from '#lib/poll.svelte.js';
	import { metricHref } from '#lib/metrics/href.js';
	import {
		chartSettingsParams,
		param,
		readChartSettings,
		readZoom,
		overridesParam,
		readOverrides,
		syncUrl,
		zoomParam
	} from '#lib/url-state.svelte.js';

	const project = $derived(page.params.project ?? '');
	/** A run id, or `@alias`. */
	const runParam = $derived(page.params.run ?? '');
	const aliasPath = (alias: string) =>
		`/projects/${encodeURIComponent(project)}/aliases/${encodeURIComponent(alias)}`;

	const run: Polled<RunDetail> = poll(
		async () => {
			let id = runParam;
			if (id.startsWith('@'))
				id = (await apiJson<{ run: string }>(aliasPath(id.slice(1)))).run;
			return apiJson<RunDetail>(`/runs/${encodeURIComponent(id)}`);
		},
		(): number | false => (run.data?.state === 'running' ? 5_000 : false)
	);
	const running = $derived(run.data?.state === 'running');

	useProjectMetrics(() => project);
	useProjectTags(() => project);

	/** The project's runs with this tag. */
	const tagFilterHref = (tag: string) =>
		`${resolve('/p/[project]/(tabs)/runs', { project })}?q=${encodeURIComponent(`tag:${literal(tag)}`)}`;

	/** The project's selection: what the Runs and Compare tabs show. */
	const selection = $derived(selectionOf(project));

	function toggleCompare(id: string, name: string) {
		const on = !selection.has(id);
		selection.toggle([id], on);
		const n = selection.size;
		toast.success(on ? `${name} added to compare` : `${name} removed from compare`, {
			description:
				n === 1
					? 'Comparing takes two runs or more: pick at least one more.'
					: `${n} runs are picked in ${project}.`,
			action:
				n > 1
					? {
							label: 'Open Compare',
							onClick: () =>
								void goto(resolve('/p/[project]/(tabs)/compare', { project }))
						}
					: undefined
		});
	}

	function openTag(tag: string) {
		// eslint-disable-next-line svelte/no-navigation-without-resolve -- resolved, plus a query
		void goto(tagFilterHref(tag));
	}

	async function removeTag(tag: string) {
		if (run.data && (await changeTags(project, [run.data.id], { remove: [tag] })))
			run.refresh();
	}
	const summary = $derived(run.data ? effectiveSummary(run.data, currentRegistry()) : {});
	const renames = (): Record<string, string> => run.data?.renames ?? {};
	const renameScale = (from: string) => run.data?.rename_scales[from] ?? 1;
	/** Where the back link goes: the overview if opened from there, or the run list. */
	const backTo = $derived(
		nav.runBack?.project === project
			? nav.runBack
			: {
					href: `${resolve('/p/[project]/(tabs)/runs', { project })}${nav.runsQuery[project] ?? ''}`,
					label: `Runs of ${project}`
				}
	);

	const isHeadline = ([key, value]: readonly [unknown, unknown]) => {
		const meta = currentRegistry().describe(String(key));
		return meta.important && !meta.hidden && typeof value === 'number';
	};
	/** Important metrics: at the top of the charts and the overview, as figures. */
	const headlines = $derived(
		flatten(summary)
			.filter(isHeadline)
			.map(([key, value]) => ({ key: String(key), value }))
	);
	/** The rest of the summary, hidden metrics last. */
	const summaryRows = $derived.by(() => {
		const registry = currentRegistry();
		return flatten(summary)
			.filter((row) => !isHeadline(row))
			.map((row, i) => ({ row, i, hidden: registry.describe(String(row[0])).hidden ? 1 : 0 }))
			.sort((a, b) => a.hidden - b.hidden || a.i - b.i)
			.map(({ row }) => row);
	});

	function rememberRun() {
		nav.back = { href: page.url.pathname + page.url.search, label: nav.runName ?? 'Run' };
	}

	const projectInfo = poll(() =>
		apiJson<ProjectInfo>(`/projects/${encodeURIComponent(project)}`)
	);
	const canEdit = $derived(
		projectInfo.data?.role === 'owner' || projectInfo.data?.role === 'editor'
	);
	const deleteAt = $derived(
		run.data?.archived_at_ms && projectInfo.data
			? run.data.archived_at_ms + projectInfo.data.archive_days * 86_400_000
			: null
	);

	async function setArchived(archive: boolean, undoable = true) {
		const id = run.data?.id;
		if (!id) return;
		const action = archive ? 'archive' : 'restore';
		try {
			await apiSend('POST', `/projects/${encodeURIComponent(project)}/runs/${action}`, {
				runs: [id]
			});
			const name = run.data?.name ?? 'Run';
			toast.success(archive ? `${name} archived` : `${name} restored`, {
				description: archive
					? `It is hidden from ${project} and deleted for good after ${projectInfo.data?.archive_days ?? 30} days unless restored.`
					: `It is back in the ${project} run list.`,
				action: undoable
					? { label: 'Undo', onClick: () => void setArchived(!archive, false) }
					: undefined
			});
		} catch (err) {
			notifyError(err, archive ? "Couldn't archive this run" : "Couldn't restore this run");
		}
		run.refresh();
		projectsChanged();
	}

	$effect(() => {
		nav.runName = run.data?.name ?? null;
		return () => (nav.runName = null);
	});

	const chartSeries = $derived<ChartSeries[]>(
		run.data
			? [
					{
						id: run.data.id,
						name: run.data.name,
						color: 'var(--chart-1)',
						runs: [{ id: run.data.id, startMs: run.data.start_ms }]
					}
				]
			: []
	);

	let settings = $state<ChartSettings>(readChartSettings());
	let metricFilter = $state(param('metrics') ?? '');
	let view = $state(readZoom());
	let overrides = $state(readOverrides());
	let systemOverrides = $state({});
	const TAB_ITEMS = [
		{ id: 'charts', label: 'Charts' },
		{ id: 'system', label: 'System' },
		{ id: 'logs', label: 'Logs' },
		{ id: 'config', label: 'Config' },
		{ id: 'model', label: 'Model' },
		{ id: 'overview', label: 'Overview' }
	];
	const TABS = TAB_ITEMS.map((t) => t.id);
	let tab = $state(TABS.includes(param('tab') ?? '') ? param('tab')! : 'charts');
	// System metrics: against time by default, settings of their own.
	let systemSettings = $state<ChartSettings>({ ...DEFAULT_SETTINGS, x: 'relative' });
	let systemFilter = $state('');
	let systemView = $state<[number, number] | null>(null);
	let configView = $state<'tree' | 'yaml'>(param('cfg') === 'yaml' ? 'yaml' : 'tree');
	syncUrl(() => ({
		...chartSettingsParams(settings),
		metrics: metricFilter.trim() || null,
		zoom: zoomParam(view),
		charts: overridesParam(overrides),
		tab: tab === 'charts' ? null : tab,
		cfg: configView === 'yaml' ? 'yaml' : null
	}));

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

	let addingAlias = $state(false);
	let newAlias = $state('');

	async function addAlias(event: SubmitEvent) {
		event.preventDefault();
		const alias = newAlias.trim().replace(/^@/, '');
		const id = run.data?.id;
		if (!alias || !id) return;
		const ok = await withToast(
			() => apiSend('PUT', aliasPath(alias), { run: id }).then(() => true),
			{
				done: `Alias @${alias} set`,
				detail: `It points to this run now (moved here if another run had it). Open it at /p/${project}/r/@${alias}.`,
				failed: `Couldn't set @${alias}`
			}
		);
		run.refresh();
		if (ok) {
			newAlias = '';
			addingAlias = false;
		}
	}

	async function removeAlias(alias: string) {
		await withToast(() => apiSend('DELETE', aliasPath(alias)), {
			done: `Alias @${alias} removed`,
			detail: 'Links and filters using it no longer find this run.',
			failed: `Couldn't remove @${alias}`
		});
		run.refresh();
	}

	// ---- Metric renames (this run only) ----
	/** The logged name being renamed, the new name and its factor (text, may be empty). */
	let renaming = $state<{ from: string; to: string; scale: string } | null>(null);
	const renamePath = (from: string) =>
		`/runs/${encodeURIComponent(run.data?.id ?? '')}/metric-renames/${from
			.split('/')
			.map(encodeURIComponent)
			.join('/')}`;

	function startRename(shown: string) {
		const from = Object.entries(renames()).find(([, to]) => to === shown)?.[0] ?? shown;
		const scale = renameScale(from);
		renaming = { from, to: shown, scale: scale === 1 ? '' : String(scale) };
	}

	async function saveRename(event: SubmitEvent) {
		event.preventDefault();
		if (!renaming) return;
		const { from } = renaming;
		const to = renaming.to.trim();
		const scale = renaming.scale.trim() === '' ? 1 : Number(renaming.scale);
		if (to === from && scale === 1) {
			if (renames()[from]) await removeRename(from);
			renaming = null;
			return;
		}
		const ok = await withToast(
			() => apiSend('PUT', renamePath(from), { to, scale }).then(() => true),
			{
				done: to === from ? `${from} rescaled` : `${from} renamed to ${to}`,
				detail: `Only in this run${scale === 1 ? '' : `, times ${scale}`}: charts, tables and filters show it as ${to}. Logged data is unchanged. To rename it in many runs, use the Metrics tab.`,
				failed: `Couldn't rename ${from}`
			}
		);
		run.refresh();
		if (ok) renaming = null;
	}

	async function removeRename(from: string) {
		await withToast(() => apiSend('DELETE', renamePath(from)), {
			done: `${from} has its logged name again`,
			detail: `It was shown as ${renames()[from] ?? 'another name'} in this run.`,
			failed: `Couldn't undo the rename of ${from}`
		});
		run.refresh();
	}
</script>

{#if run.error}
	<ErrorCard error={run.error} what="run" retry={run.refresh}>
		<a
			class={buttonVariants({ variant: 'outline', size: 'sm' })}
			href={resolve('/p/[project]/(tabs)/runs', { project })}>Back to the runs of {project}</a
		>
	</ErrorCard>
{:else if !run.data}
	<div class="grid gap-4">
		<Skeleton class="h-24 rounded-xl" />
		<div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
			{#each [0, 1, 2] as i (i)}<Skeleton class="h-60 rounded-xl" />{/each}
		</div>
	</div>
{:else}
	{@const r = run.data}
	<div class="mx-auto max-w-[1600px]">
		<header class="mb-5">
			<!-- eslint-disable svelte/no-navigation-without-resolve -- resolved, plus the list's query -->
			<a
				href={backTo.href}
				class="mb-2 inline-flex w-fit items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
			>
				<ArrowLeft class="size-4" />
				{backTo.label}
			</a>
			<!-- eslint-enable svelte/no-navigation-without-resolve -->
			<div class="flex flex-wrap items-start gap-x-6 gap-y-3">
				<div class="min-w-0">
					<div class="flex flex-wrap items-center gap-3">
						<h1 class="truncate text-2xl font-semibold tracking-tight">{r.name}</h1>
						<StateBadge state={r.state} />
					</div>
					<p
						class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-muted-foreground"
					>
						<span title={dateTime(r.created_at_ms)}
							>Started {timeAgo(r.created_at_ms)}</span
						>
						<span class="tabular-nums">
							{duration(
								r.created_at_ms,
								running ? now : (r.finished_at_ms ?? r.heartbeat_at_ms)
							)}
						</span>
						{#if r.last_step !== null}
							<span
								class="tabular-nums"
								title={`Last step ${r.last_step.toLocaleString('en')}`}
								>{formatSteps(r.last_step + 1)} steps</span
							>
						{/if}
						{#if r.group_name}<span>Group {r.group_name}</span>{/if}
						{#if r.environment.hostname}
							<span class="inline-flex items-center gap-1"
								><Server class="size-3.5" />{r.environment.hostname}</span
							>
						{/if}
						{#if r.environment.git?.commit}
							<span class="inline-flex items-center gap-1 font-mono text-xs">
								<GitCommitHorizontal
									class="size-3.5"
								/>{r.environment.git.commit.slice(0, 8)}{r.environment.git.dirty
									? '*'
									: ''}
							</span>
						{/if}
					</p>
					<div class="mt-2 flex flex-wrap items-center gap-1.5 text-sm">
						{#each r.tags as tag (tag)}
							<TagChip
								name={tag}
								onclick={() => openTag(tag)}
								onremove={canEdit ? () => removeTag(tag) : undefined}
							/>
						{/each}
						{#if canEdit}
							<TagsMenu {project} runs={[r]} onChanged={() => run.refresh()}>
								{#snippet trigger(props)}
									<Button
										{...props}
										variant="ghost"
										size="sm"
										class="h-7 text-muted-foreground"
										title="Add or remove tags"
										aria-label="Edit tags"
									>
										<Tag />{#if !r.tags.length}Add tag{/if}
									</Button>
								{/snippet}
							</TagsMenu>
							<span class="mx-1 h-4 w-px bg-border" aria-hidden="true"></span>
						{/if}
						{#each r.aliases as alias (alias)}
							<span
								class="inline-flex items-center gap-0.5 rounded-md bg-secondary py-0.5 pr-0.5 pl-1.5 font-mono text-xs text-secondary-foreground"
							>
								@{alias}
								<button
									class="rounded-sm p-0.5 text-muted-foreground hover:bg-background hover:text-foreground"
									onclick={() => removeAlias(alias)}
									title={`Remove @${alias}`}
									aria-label={`Remove alias ${alias}`}
								>
									<X class="size-3" />
								</button>
							</span>
						{/each}
						{#if addingAlias}
							<form class="flex items-center gap-1" onsubmit={addAlias}>
								<Input
									class="h-7 w-40 font-mono text-xs"
									placeholder="best"
									aria-label="New alias"
									autocomplete="off"
									bind:value={newAlias}
									onkeydown={(e) => e.key === 'Escape' && (addingAlias = false)}
								/>
								<Button
									type="submit"
									size="sm"
									class="h-7"
									disabled={!newAlias.trim()}>Set</Button
								>
								<Button
									variant="ghost"
									size="sm"
									class="h-7"
									onclick={() => (addingAlias = false)}
								>
									Cancel
								</Button>
							</form>
						{:else}
							<Button
								variant="ghost"
								size="sm"
								class="h-7 text-muted-foreground"
								onclick={() => (addingAlias = true)}
								title="An alias names one run of the project, e.g. @best. Setting it here moves it from another run."
								aria-label="Add alias"
							>
								{#if r.aliases.length}<Plus />{:else}<AtSign /> Add alias{/if}
							</Button>
						{/if}
					</div>
				</div>
				<div class="ml-auto flex items-center gap-2">
					<Button
						variant="outline"
						size="sm"
						aria-pressed={selection.has(r.id)}
						title={selection.has(r.id)
							? 'Picked for comparing: click to remove it'
							: 'Pick it for the Compare tab'}
						onclick={() => toggleCompare(r.id, r.name)}
					>
						{#if selection.has(r.id)}<Check /> In compare{:else}<GitCompareArrows /> Add to
							compare{/if}
					</Button>
					{#if canEdit}
						<AbortRuns {project} runs={[r]} onDone={() => run.refresh()} />
						<Button
							variant="outline"
							size="sm"
							onclick={() => setArchived(!r.archived_at_ms)}
						>
							{#if r.archived_at_ms}<ArchiveRestore /> Restore{:else}<Archive /> Archive{/if}
						</Button>
					{/if}
					<Button
						variant="outline"
						size="sm"
						class="font-mono text-xs"
						onclick={copyId}
						title="Copy run id"
					>
						{#if copied}<Check />{:else}<Copy />{/if}
						{r.id.replaceAll('-', '').slice(0, 12)}…
					</Button>
				</div>
			</div>
			<UnderlineTabs
				class="mt-3"
				label="Run"
				items={TAB_ITEMS}
				current={tab}
				onSelect={(id) => (tab = id)}
			/>
		</header>
		{#if r.archived_at_ms}
			<div
				class="mb-5 flex items-center gap-2 rounded-lg border border-dashed px-3 py-2 text-sm text-muted-foreground"
			>
				<Archive class="size-4" />
				Archived {timeAgo(r.archived_at_ms)}{#if deleteAt}; deleted for good on {dateTime(
						deleteAt
					)}{/if}.
			</div>
		{/if}

		<!-- Not on the other tabs: the log's height is fitted to the window. -->
		{#if headlines.length && (tab === 'charts' || tab === 'overview')}
			<dl
				class="mb-4 grid gap-4 sm:grid-cols-2 lg:grid-cols-4"
				aria-label="Important metrics"
			>
				{#each headlines as { key, value } (key)}
					<div class="min-w-0 rounded-xl border bg-card p-4">
						<dt class="truncate text-sm text-muted-foreground" title={key}>
							<!-- eslint-disable svelte/no-navigation-without-resolve -- metricHref resolves it -->
							<a
								class="hover:underline"
								href={metricHref(project, key)}
								onclick={rememberRun}
							>
								<!-- eslint-enable svelte/no-navigation-without-resolve -->
								{currentRegistry().describe(key).label || key}
							</a>
						</dt>
						<dd class="mt-1 truncate text-2xl font-semibold">
							{formatMetric(key, value)}
						</dd>
					</div>
				{/each}
			</dl>
		{/if}
		<div
			role="tabpanel"
			id="panel-charts"
			aria-labelledby="tab-charts"
			hidden={tab !== 'charts'}
		>
			<ChartsPanel
				{project}
				series={chartSeries}
				running={running ? [r.id] : []}
				bind:settings
				bind:metricFilter
				bind:view
				bind:overrides
				renames={(id) => (id === run.data?.id ? run.data : undefined)}
				summaryOf={() => summary}
				onRename={canEdit ? startRename : undefined}
				onRunState={() => run.refresh()}
			/>
		</div>

		<!-- The charts tab stays mounted when hidden (its live updates keep cached tiles
		     current); the others mount only while shown. -->
		{#if tab === 'system'}<div role="tabpanel" id="panel-system" aria-labelledby="tab-system">
				<ChartsPanel
					{project}
					prefix="system/"
					series={chartSeries}
					running={running ? [r.id] : []}
					bind:settings={systemSettings}
					bind:metricFilter={systemFilter}
					bind:view={systemView}
					bind:overrides={systemOverrides}
					renames={(id) => (id === run.data?.id ? run.data : undefined)}
					summaryOf={() => summary}
					onRename={canEdit ? startRename : undefined}
					empty={{
						icon: Cpu,
						title: running ? 'No system metrics yet' : 'No system metrics',
						text: running
							? 'CPU, memory, disk, network and GPU usage are sampled every 5 seconds while the run is going; the first ones show up shortly.'
							: 'CPU, memory, disk, network and GPU usage are sampled every 5 seconds while a run is going. This one has none: it ran with `system_metrics=False` (or `LOSSCAPE_SYSTEM_METRICS=0`), or ended before the first sample.'
					}}
				/>
			</div>{/if}

		{#if tab === 'logs'}<div role="tabpanel" id="panel-logs" aria-labelledby="tab-logs">
				<ConsoleLog {project} run={r.id} {running} capture={r.environment.console} />
			</div>{/if}

		{#if tab === 'model'}<div role="tabpanel" id="panel-model" aria-labelledby="tab-model">
				<ModelInfo {project} run={r.id} {running} />
			</div>{/if}
		{#if tab === 'config'}<div role="tabpanel" id="panel-config" aria-labelledby="tab-config">
				<RunConfig {project} config={r.config} bind:view={configView} />
			</div>{/if}

		{#if tab === 'overview'}<div
				role="tabpanel"
				id="panel-overview"
				aria-labelledby="tab-overview"
			>
				<div class="grid gap-4 lg:grid-cols-2">
					{#if summaryRows.length || !headlines.length}
						<Card.Root>
							<Card.Header><Card.Title>Summary</Card.Title></Card.Header>
							<Card.Content
								><KeyValueList
									rows={summaryRows}
									format={formatMetric}
									empty="No summary yet."
								/></Card.Content
							>
						</Card.Root>
					{/if}
					{#if Object.keys(r.renames).length}
						<Card.Root>
							<Card.Header>
								<Card.Title>Renamed metrics</Card.Title>
								<Card.Description>
									Shown under a new name in this run only; the logged data keeps
									its name.
								</Card.Description>
							</Card.Header>
							<Card.Content>
								<ul class="grid gap-1 font-mono text-sm">
									{#each Object.entries(r.renames) as [from, to] (from)}
										<li class="flex items-center gap-2">
											<span class="truncate text-muted-foreground"
												>{from}</span
											>
											<span aria-hidden="true" class="text-muted-foreground"
												>→</span
											>
											<span class="truncate">{to}</span>
											{#if renameScale(from) !== 1}<span
													class="shrink-0 text-xs text-muted-foreground"
													>× {renameScale(from)}</span
												>{/if}
											{#if canEdit}
												<span class="ml-auto flex shrink-0 gap-1">
													<Button
														variant="ghost"
														size="sm"
														class="h-7"
														onclick={() => startRename(to)}>Edit</Button
													>
													<Button
														variant="ghost"
														size="icon"
														class="size-7 text-muted-foreground"
														onclick={() => removeRename(from)}
														title={`Show ${from} under its logged name again`}
														aria-label={`Undo rename of ${from}`}
													>
														<X />
													</Button>
												</span>
											{/if}
										</li>
									{/each}
								</ul>
							</Card.Content>
						</Card.Root>
					{/if}
					<Card.Root>
						<Card.Header><Card.Title>Run</Card.Title></Card.Header>
						<Card.Content>
							<KeyValueList
								mono={false}
								rows={[
									['Run id', r.id],
									['Created', dateTime(r.created_at_ms)],
									[
										'Finished',
										r.finished_at_ms ? dateTime(r.finished_at_ms) : null
									],
									['Exit code', r.exit_code],
									['Group', r.group_name],
									['Aliases', r.aliases.map((a) => `@${a}`).join(', ') || null],
									['Notes', r.notes]
								]}
							/>
						</Card.Content>
					</Card.Root>
					<RunEnvironmentCards run={r.id} env={r.environment} />
				</div>
			</div>{/if}
	</div>
{/if}

{#if renaming}
	<Dialog.Root open onOpenChange={(open) => !open && (renaming = null)}>
		<Dialog.Content class="sm:max-w-md">
			<form class="grid gap-4" onsubmit={saveRename}>
				<Dialog.Header>
					<Dialog.Title>Rename a metric in this run</Dialog.Title>
					<Dialog.Description>
						<span class="font-mono">{renaming.from}</span> is shown under the new name in
						this run: on charts, beside other runs' metric of that name, and in tables and
						filters. The logged data keeps its name.
					</Dialog.Description>
				</Dialog.Header>
				<div class="grid grid-cols-[1fr_6rem] gap-2">
					<Input
						class="font-mono"
						aria-label="New name"
						autocomplete="off"
						bind:value={renaming.to}
					/>
					<Input
						aria-label="Times"
						placeholder="× 1"
						title="Shown values are the logged ones times this (0.01 for a percent shown as 0-1)"
						aria-invalid={renaming.scale.trim() !== '' &&
							!(
								Number(renaming.scale) > 0 &&
								Number.isFinite(Number(renaming.scale))
							)}
						bind:value={renaming.scale}
					/>
				</div>
				<Dialog.Footer>
					<Button variant="outline" type="button" onclick={() => (renaming = null)}
						>Cancel</Button
					>
					<Button
						type="submit"
						disabled={!renaming.to.trim() ||
							(renaming.scale.trim() !== '' &&
								!(
									Number(renaming.scale) > 0 &&
									Number.isFinite(Number(renaming.scale))
								))}>Rename</Button
					>
				</Dialog.Footer>
			</form>
		</Dialog.Content>
	</Dialog.Root>
{/if}
