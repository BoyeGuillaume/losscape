<script lang="ts">
	import Eye from '@lucide/svelte/icons/eye';
	import EyeOff from '@lucide/svelte/icons/eye-off';
	import Star from '@lucide/svelte/icons/star';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import Undo2 from '@lucide/svelte/icons/undo-2';
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
	import ComputedDefinition from '#lib/components/app/ComputedDefinition.svelte';
	import MetricSettings from '#lib/components/app/MetricSettings.svelte';
	import RenameMetricDialog from '#lib/components/app/RenameMetricDialog.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { withMetrics } from '#lib/metrics/apply.js';
	import { match } from '#lib/metrics/pattern.js';
	import { projectMetrics } from '#lib/metrics/store.svelte.js';
	import { withToast } from '#lib/notify.js';
	import { poll } from '#lib/poll.svelte.js';
	import { RunList } from '#lib/runs/list.svelte.js';

	/** Runs charted in the preview. */
	const PREVIEW_RUNS = 5;

	const project = $derived(page.params.project ?? '');
	/** A metric, or a group's pattern (`val/*`). */
	const name = $derived(page.params.name ?? '');
	const group = $derived(name.endsWith('/*'));
	const prefix = $derived(group ? name.slice(0, -1) : '');
	const projectPath = $derived(`/projects/${encodeURIComponent(project)}`);
	const listHref = $derived(resolve('/p/[project]/(tabs)/metrics', { project }));

	const info = poll(() => apiJson<ProjectInfo>(projectPath));
	const canEdit = $derived(info.data?.role === 'owner' || info.data?.role === 'editor');

	const registry = $derived(projectMetrics.registry);
	const own = $derived(projectMetrics.own(name));
	const computed = $derived(own?.kind === 'formula');
	const meta = $derived(registry.describe(name));
	const inherited = $derived(registry.inherited(name));

	// ---- Runs: who logged it, renamed it, or shows another metric as it ----
	let list = $state<RunList | null>(null);
	$effect(() => {
		const l = new RunList(projectPath, false, () => 30_000);
		list = l;
		l.start();
		return () => l.stop();
	});
	const runs = $derived(list?.runs ?? []);
	const runById = $derived(new Map(runs.map((r) => [r.id, r])));
	/** System metrics aren't part of summaries: any run may have them. */
	const isSystem = $derived(name.startsWith('system/'));
	/** Runs that have it, as shown (renames and formulas applied), newest first. */
	const shownIn = $derived(
		runs
			.map((r) => withMetrics(r, registry))
			.filter((r) => isSystem || r.summary[name] !== undefined)
			.sort((a, b) => b.created_at_ms - a.created_at_ms)
	);
	/** Runs that logged it but show it under another name. */
	const renamedAway = $derived(runs.filter((r) => r.renames[name] !== undefined));
	/** Runs that show another logged metric as this one. */
	const renamedHere = $derived(
		runs.flatMap((r) =>
			Object.entries(r.renames)
				.filter(([, to]) => to === name)
				.map(([from]) => ({ run: r, from }))
		)
	);
	const families = $derived(registry.families.filter((d) => match(d.name, name)));

	// ---- Preview: the latest runs that have it ----
	const series = $derived<ChartSeries[]>(
		shownIn.slice(0, PREVIEW_RUNS).map((r, i) => ({
			id: r.id,
			name: r.name,
			color: `var(--chart-${(i % 8) + 1})`,
			runs: [{ id: r.id, startMs: r.start_ms }]
		}))
	);
	let settings = $state<ChartSettings>({ ...DEFAULT_SETTINGS });
	let metricFilter = $state('');
	let view = $state<[number, number] | null>(null);
	let overrides = $state({});

	// ---- A group: its metrics ----
	let keys = $state<string[]>([]);
	$effect(() => {
		apiJson<string[]>(`${projectPath}/metric-keys`).then(
			(k) => (keys = k),
			() => (keys = [])
		);
	});
	const members = $derived(group ? keys.filter((k) => k.startsWith(prefix)) : []);

	// ---- Actions ----
	let renaming = $state(false);
	const runHref = (r: RunSummary) =>
		resolve('/p/[project]/r/[run]', { project, run: r.id.replaceAll('-', '') });
	const metricHref = (metric: string) =>
		resolve('/p/[project]/(tabs)/metrics/[...name]', { project, name: metric });

	function toggle(field: 'important' | 'hidden') {
		const on = meta[field];
		projectMetrics.patch(name, { [field]: on ? (inherited[field] ? false : undefined) : true });
	}

	async function undoRename(run: RunSummary, from: string) {
		await withToast(
			() =>
				apiSend('POST', `${projectPath}/runs/metric-renames`, {
					runs: [run.id],
					from,
					to: null
				}),
			{
				done: `${from} has its logged name again`,
				detail: `In ${run.name}.`,
				failed: `Couldn't undo the rename in ${run.name}`
			}
		);
		list?.refresh();
	}

	function deleteComputed() {
		projectMetrics.remove(name);

		void goto(listHref);
	}

	function clearOwn() {
		if (!own) return;
		projectMetrics.replace(own, null);
		toast.success(`${name} follows ${group ? 'the defaults' : 'its group'} again`, {
			description: 'Its own settings were cleared.'
		});
	}
</script>

<div class="grid max-w-5xl gap-6">
	<div class="grid gap-2">
		<div class="flex flex-wrap items-center gap-3">
			{#if !group}
				<button
					class={[
						'flex size-8 items-center justify-center rounded-md hover:bg-muted disabled:pointer-events-none',
						meta.important ? 'text-amber-500' : 'text-muted-foreground'
					]}
					aria-pressed={meta.important}
					aria-label={meta.important ? 'Unmark as important' : 'Mark as important'}
					title={meta.important
						? 'Important: charted first, a default column'
						: 'Mark as important'}
					disabled={!canEdit}
					onclick={() => toggle('important')}
				>
					<Star class={['size-5', meta.important && 'fill-current']} />
				</button>
			{/if}
			<h2 class="flex min-w-0 items-baseline gap-2 text-xl font-semibold">
				{#if computed}<span
						class="font-serif text-base text-primary italic"
						title="Computed">ƒ</span
					>{/if}
				<span class="truncate font-mono">{group ? prefix : name}</span>
				{#if group}<span class="text-base font-normal text-muted-foreground"
						>settings for all its metrics</span
					>{:else if meta.label}<span
						class="truncate text-base font-normal text-muted-foreground"
						>{meta.label}</span
					>{/if}
			</h2>
			<div class="ml-auto flex items-center gap-3">
				<span class="text-xs text-muted-foreground" aria-live="polite">
					{projectMetrics.status === 'pending' || projectMetrics.status === 'saving'
						? 'Saving…'
						: projectMetrics.status === 'saved'
							? 'Saved'
							: ''}
				</span>
				<Button
					variant="outline"
					size="sm"
					disabled={!canEdit}
					aria-pressed={meta.hidden}
					onclick={() => toggle('hidden')}
					title="Hidden metrics are left out of charts and default columns"
				>
					{#if meta.hidden}<EyeOff /> Hidden{:else}<Eye /> Shown{/if}
				</Button>
			</div>
		</div>
	</div>

	{#if computed}
		<!-- Above the preview, which appears as it's typed. Not clipped: suggestions reach past it. -->
		<Card.Root class="overflow-visible">
			<Card.Header>
				<Card.Title>Definition</Card.Title>
				{#if !canEdit}<Card.Description>Only editors can change it.</Card.Description>{/if}
				{#if canEdit}
					<Card.Action>
						<Button variant="outline" size="sm" onclick={deleteComputed}>
							<Trash2 /> Delete
						</Button>
					</Card.Action>
				{/if}
			</Card.Header>
			<Card.Content>
				<ComputedDefinition
					{name}
					{canEdit}
					{keys}
					onRename={(to) => goto(metricHref(to), { replaceState: true })}
				/>
			</Card.Content>
		</Card.Root>
	{/if}

	{#if !group}
		<Card.Root>
			<Card.Header>
				<Card.Title>Preview</Card.Title>
				<Card.Description>
					{#if computed && !own?.spec.expression?.trim()}Shows up here once it has a
						formula.{:else if !list?.runs}Loading runs…{:else if series.length}The {series.length}
						latest
						{series.length === 1 ? 'run' : 'runs'}{isSystem ? '' : ' with it'}; settings
						below apply here at once.{:else}No run has it yet.{/if}
				</Card.Description>
			</Card.Header>
			{#if series.length}
				<Card.Content>
					<div class="mb-2 flex flex-wrap gap-x-4 gap-y-1 text-xs">
						{#each series as s (s.id)}
							<span class="inline-flex items-center gap-1.5">
								<span class="size-2.5 rounded-sm" style:background-color={s.color}
								></span>
								{s.name}
							</span>
						{/each}
					</div>
					<ChartsPanel
						{project}
						{series}
						running={series
							.filter((s) => runById.get(s.id)?.state === 'running')
							.map((s) => s.id)}
						bind:settings
						bind:metricFilter
						bind:view
						bind:overrides
						renames={(id) => runById.get(id)}
						only={name}
					/>
				</Card.Content>
			{/if}
		</Card.Root>
	{/if}

	<Card.Root>
		<Card.Header>
			<Card.Title>Settings</Card.Title>
			<Card.Description>
				{#if group}
					They apply to every metric under <span class="font-mono">{prefix}</span>, unless
					the metric sets its own. Auto keeps the defaults.
				{:else}
					Auto keeps what it gets from its group or the defaults (shown in brackets).
				{/if}
				{#if !canEdit}Only editors can change them.{/if}
			</Card.Description>
		</Card.Header>
		<Card.Content>
			<MetricSettings {name} {canEdit} />
		</Card.Content>
	</Card.Root>

	{#if group}
		<Card.Root>
			<Card.Header>
				<Card.Title>Metrics in {prefix}</Card.Title>
			</Card.Header>
			<Card.Content>
				<ul class="grid gap-1 sm:grid-cols-2 lg:grid-cols-3">
					{#each members as member (member)}
						<li>
							<a class="font-mono text-sm hover:underline" href={metricHref(member)}
								>{member}</a
							>
						</li>
					{:else}
						<li class="text-sm text-muted-foreground">None logged yet.</li>
					{/each}
				</ul>
			</Card.Content>
		</Card.Root>
	{:else}
		<Card.Root>
			<Card.Header>
				<Card.Title>Runs</Card.Title>
				<Card.Description>
					{#if !list?.runs}Loading…{:else if isSystem}
						Sampled every 5 seconds while a run is going, unless it turned system
						metrics off.
					{:else}
						{shownIn.length.toLocaleString('en')}
						{shownIn.length === 1 ? 'run has' : 'runs have'} it{#if families.length};
							part of the family
							{#each families as f, i (f.name)}{i ? ', ' : ''}<span class="font-mono"
									>{f.name}</span
								>{/each}{/if}.
					{/if}
				</Card.Description>
				{#if canEdit && !computed}
					<Card.Action>
						<Button size="sm" variant="outline" onclick={() => (renaming = true)}>
							Rename in runs…
						</Button>
					</Card.Action>
				{/if}
			</Card.Header>
			{#if renamedAway.length || renamedHere.length}
				<Card.Content class="grid gap-4 text-sm">
					{#if renamedHere.length}
						<div>
							<p class="mb-1 font-medium">Shown as {name} in</p>
							<ul class="grid gap-1">
								{#each renamedHere as { run, from } (`${run.id}:${from}`)}
									<li class="flex items-center gap-2">
										<a class="hover:underline" href={runHref(run)}>{run.name}</a
										>
										<span
											class="truncate font-mono text-xs text-muted-foreground"
											>logged as {from}{#if (run.rename_scales[from] ?? 1) !== 1},
												× {run.rename_scales[from]}{/if}</span
										>
										{#if canEdit}
											<Button
												variant="ghost"
												size="sm"
												class="ml-auto h-7"
												onclick={() => undoRename(run, from)}
											>
												<Undo2 /> Undo
											</Button>
										{/if}
									</li>
								{/each}
							</ul>
						</div>
					{/if}
					{#if renamedAway.length}
						<div>
							<p class="mb-1 font-medium">Logged, but shown under another name, in</p>
							<ul class="grid gap-1">
								{#each renamedAway as run (run.id)}
									<li class="flex items-center gap-2">
										<a class="hover:underline" href={runHref(run)}>{run.name}</a
										>
										<span
											class="truncate font-mono text-xs text-muted-foreground"
											>as {run.renames[name]}</span
										>
										{#if canEdit}
											<Button
												variant="ghost"
												size="sm"
												class="ml-auto h-7"
												onclick={() => undoRename(run, name)}
											>
												<Undo2 /> Undo
											</Button>
										{/if}
									</li>
								{/each}
							</ul>
						</div>
					{/if}
				</Card.Content>
			{/if}
		</Card.Root>
	{/if}

	{#if canEdit && own && !computed}
		<Card.Root>
			<Card.Header>
				<Card.Title>Reset</Card.Title>
				<Card.Description>
					{#if group}
						Clears the settings of the group: its metrics keep only their own and the
						defaults.
					{:else}
						Clears its own settings: it follows its group and the defaults again.
					{/if}
				</Card.Description>
				<Card.Action>
					<Button variant="outline" size="sm" onclick={clearOwn}
						>Clear its settings</Button
					>
				</Card.Action>
			</Card.Header>
		</Card.Root>
	{/if}
</div>

{#if renaming}
	<RenameMetricDialog
		{project}
		from={name}
		{keys}
		onClose={() => ((renaming = false), list?.refresh())}
	/>
{/if}
