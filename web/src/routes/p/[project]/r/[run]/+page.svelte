<script lang="ts">
	import Archive from '@lucide/svelte/icons/archive';
	import ArchiveRestore from '@lucide/svelte/icons/archive-arrow-up';
	import AtSign from '@lucide/svelte/icons/at-sign';
	import Check from '@lucide/svelte/icons/check';
	import Copy from '@lucide/svelte/icons/copy';
	import GitCommitHorizontal from '@lucide/svelte/icons/git-commit-horizontal';
	import Plus from '@lucide/svelte/icons/plus';
	import Server from '@lucide/svelte/icons/server';
	import X from '@lucide/svelte/icons/x';
	import { page } from '$app/state';

	import { apiJson, apiSend } from '#lib/api/client.js';
	import type { ProjectInfo, RunDetail } from '#lib/api/types.js';
	import {
		DEFAULT_SETTINGS,
		type ChartSeries,
		type ChartSettings
	} from '#lib/charts/settings.js';
	import ChartsPanel from '#lib/components/app/ChartsPanel.svelte';
	import ConsoleLog from '#lib/components/app/ConsoleLog.svelte';
	import ConsoleTail from '#lib/components/app/ConsoleTail.svelte';
	import RunConfig from '#lib/components/app/RunConfig.svelte';
	import RunEnvironmentCards from '#lib/components/app/RunEnvironmentCards.svelte';
	import KeyValueList from '#lib/components/app/KeyValueList.svelte';
	import StateBadge from '#lib/components/app/StateBadge.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as Tabs from '#lib/components/ui/tabs/index.js';
	import { dateTime, duration, flatten, timeAgo } from '#lib/format.js';
	import { formatMetric, formatSteps } from '#lib/metric-units.js';
	import { effectiveSummary } from '#lib/metrics/apply.js';
	import { currentRegistry, useProjectMetrics } from '#lib/metrics/store.svelte.js';
	import { nav } from '#lib/nav.svelte.js';
	import { notifyError, withToast } from '#lib/notify.js';
	import { projectsChanged } from '#lib/projects.svelte.js';
	import { toast } from 'svelte-sonner';
	import { poll, type Polled } from '#lib/poll.svelte.js';
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
	const summary = $derived(run.data ? effectiveSummary(run.data, currentRegistry()) : {});
	const renames = (): Record<string, string> => run.data?.renames ?? {};

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
	const TABS = ['charts', 'system', 'logs', 'config', 'overview'];
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
	/** The logged name being renamed, and the new name. */
	let renaming = $state<{ from: string; to: string } | null>(null);
	const renamePath = (from: string) =>
		`/runs/${encodeURIComponent(run.data?.id ?? '')}/metric-renames/${from
			.split('/')
			.map(encodeURIComponent)
			.join('/')}`;

	function startRename(shown: string) {
		const from = Object.entries(renames()).find(([, to]) => to === shown)?.[0] ?? shown;
		renaming = { from, to: shown };
	}

	async function saveRename(event: SubmitEvent) {
		event.preventDefault();
		if (!renaming) return;
		const { from } = renaming;
		const to = renaming.to.trim();
		if (to === from) {
			if (renames()[from]) await removeRename(from);
			renaming = null;
			return;
		}
		const ok = await withToast(
			() => apiSend('PUT', renamePath(from), { to }).then(() => true),
			{
				done: `${from} renamed to ${to}`,
				detail: 'Only in this run: charts, tables and filters show it as the new name. Logged data is unchanged.',
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
			<div class="mt-2 flex flex-wrap items-center gap-1.5 text-sm">
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
						<Button type="submit" size="sm" class="h-7" disabled={!newAlias.trim()}
							>Set</Button
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
			{#if canEdit}
				<Button variant="outline" size="sm" onclick={() => setArchived(!r.archived_at_ms)}>
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

	<Tabs.Root bind:value={tab}>
		<div class="mb-4 flex flex-wrap items-center justify-between gap-2">
			<Tabs.List>
				<Tabs.Trigger value="charts">Charts</Tabs.Trigger>
				<Tabs.Trigger value="system">System</Tabs.Trigger>
				<Tabs.Trigger value="logs">Logs</Tabs.Trigger>
				<Tabs.Trigger value="config">Config</Tabs.Trigger>
				<Tabs.Trigger value="overview">Overview</Tabs.Trigger>
			</Tabs.List>
		</div>

		<Tabs.Content value="charts">
			<ChartsPanel
				{project}
				series={chartSeries}
				running={running ? [r.id] : []}
				bind:settings
				bind:metricFilter
				bind:view
				bind:overrides
				{renames}
				summaryOf={() => summary}
				onRename={canEdit ? startRename : undefined}
				onRunState={() => run.refresh()}
			/>
		</Tabs.Content>

		<!-- Tab contents stay mounted when hidden; these mount only while shown (the
		     charts tab stays mounted: its live updates keep cached tiles current). -->
		<Tabs.Content value="system">
			{#if tab === 'system'}<ChartsPanel
					{project}
					prefix="system/"
					series={chartSeries}
					running={running ? [r.id] : []}
					bind:settings={systemSettings}
					bind:metricFilter={systemFilter}
					bind:view={systemView}
					bind:overrides={systemOverrides}
					{renames}
					summaryOf={() => summary}
					onRename={canEdit ? startRename : undefined}
					empty="No system metrics yet: they are sampled every 5 seconds while the run is going."
				/>{/if}
		</Tabs.Content>

		<Tabs.Content value="logs">
			{#if tab === 'logs'}<ConsoleLog {project} run={r.id} {running} />{/if}
		</Tabs.Content>

		<Tabs.Content value="config">
			{#if tab === 'config'}<RunConfig config={r.config} bind:view={configView} />{/if}
		</Tabs.Content>

		<Tabs.Content value="overview">
			{#if tab === 'overview'}<div class="grid gap-4 lg:grid-cols-2">
					{#if r.state === 'crashed' || r.state === 'failed' || r.state === 'killed'}
						<Card.Root class="lg:col-span-2">
							<Card.Header class="flex-row items-center justify-between gap-2">
								<Card.Title>Last output</Card.Title>
								<Button variant="outline" size="sm" onclick={() => (tab = 'logs')}>
									All logs
								</Button>
							</Card.Header>
							<Card.Content><ConsoleTail run={r.id} /></Card.Content>
						</Card.Root>
					{/if}
					<Card.Root>
						<Card.Header><Card.Title>Summary</Card.Title></Card.Header>
						<Card.Content
							><KeyValueList
								rows={flatten(summary)}
								format={formatMetric}
								empty="No summary yet."
							/></Card.Content
						>
					</Card.Root>
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
				</div>{/if}
		</Tabs.Content>
	</Tabs.Root>
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
				<Input
					class="font-mono"
					aria-label="New name"
					autocomplete="off"
					bind:value={renaming.to}
				/>
				<Dialog.Footer>
					<Button variant="outline" type="button" onclick={() => (renaming = null)}
						>Cancel</Button
					>
					<Button type="submit" disabled={!renaming.to.trim()}>Rename</Button>
				</Dialog.Footer>
			</form>
		</Dialog.Content>
	</Dialog.Root>
{/if}
