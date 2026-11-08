<script lang="ts">
	import ChartLine from '@lucide/svelte/icons/chart-line';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
	import Search from '@lucide/svelte/icons/search';
	import Star from '@lucide/svelte/icons/star';
	import { ChartGroup } from 'layerchart';
	import { untrack, type Component } from 'svelte';

	import { apiJson } from '#lib/api/client.js';
	import { subscribeLive } from '#lib/api/live.js';
	import type { RunMetricInfo } from '#lib/api/types.js';
	import {
		xLabel,
		type ChartOverride,
		type ChartSeries,
		type ChartSettings,
		type XAxis
	} from '#lib/charts/settings.js';
	import { tileStore } from '#lib/charts/tiles.svelte.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Toggle } from '#lib/components/ui/toggle/index.js';
	import { MetricSource } from '#lib/charts/source.js';
	import { match } from '#lib/metrics/pattern.js';
	import { isModelMetric, type Renames } from '#lib/metrics/registry.js';
	import { currentRegistry } from '#lib/metrics/store.svelte.js';
	import { onRestored } from '#lib/connection.svelte.js';
	import { groupBy } from '#lib/group.js';
	import { metricHref } from '#lib/metrics/href.js';
	import CodeBlock from './CodeBlock.svelte';
	import EmptyCard from './EmptyCard.svelte';
	import FamilyChart from './FamilyChart.svelte';
	import MetricChart from './MetricChart.svelte';

	type Extent = { min: number; max: number };

	const SYSTEM_PREFIX = 'system/';

	let {
		project,
		series,
		running,
		settings = $bindable(),
		metricFilter = $bindable(''),
		view = $bindable(null),
		overrides = $bindable({}),
		onRunState,
		prefix,
		empty,
		renames,
		summaryOf,
		onRename,
		only,
		importantOnly = false,
		highlight = null
	}: {
		project: string;
		/** Lines to draw (runs, or groups of runs), in legend order. */
		series: ChartSeries[];
		/** Ids of drawn runs that are still running (followed live). */
		running: string[];
		settings: ChartSettings;
		/** Regular expression (or plain text) on metric names. */
		metricFilter?: string;
		view?: [number, number] | null;
		/** Settings changed for single charts, by metric. */
		overrides?: Record<string, ChartOverride>;
		/** A followed run changed state. */
		onRunState?: (run: string) => void;
		/** Only metrics under this prefix (e.g. `system/`), sectioned by what follows it.
		 * Without it, system metrics are left out. */
		prefix?: string;
		/** Shown when the runs have no metrics (default: how to log some); code in backticks. */
		empty?: { icon: Component; title: string; text: string };
		/** A run's renamed metrics (logged name -> shown name). */
		renames?: (run: string) => Renames | undefined;
		/** A run's summary as shown, for family charts (none without it). */
		summaryOf?: (run: string) => Record<string, unknown> | undefined;
		/** Offers renaming a metric (the run page). */
		onRename?: (key: string) => void;
		/** Just this metric's chart, without the toolbar (a metric's page). */
		only?: string;
		/** Only metrics marked important. */
		importantOnly?: boolean;
		/** The series to pick out in the charts, fading the others. */
		highlight?: string | null;
	} = $props();

	const sep = '\u0000';
	const ids = $derived([...new Set(series.flatMap((s) => s.runs.map((r) => r.id)))]);
	const grouped = $derived(series.some((s) => s.runs.length > 1));
	const idsKey = $derived(ids.join(','));
	const runningKey = $derived(running.join(','));

	// Metric step ranges per run, from the server; kept while a new list loads so charts
	// don't flash when a run is added.
	let stored = $state<Map<string, Extent>>(new Map());
	let loaded = $state(false);

	const logExample = $derived(
		[
			'import lsc',
			'',
			`run = lsc.init(project=${JSON.stringify(project)})`,
			'for step in range(100):',
			'    loss = 1 / (step + 1)',
			'    run.log({"train/loss": loss}, step=step)',
			'run.finish()'
		].join('\n')
	);
	let generation = 0;
	async function loadMetrics() {
		const gen = ++generation;
		const list = untrack(() => ids);
		const rows = list.length
			? await apiJson<RunMetricInfo[]>(`/projects/${encodeURIComponent(project)}/metrics`, {
					method: 'POST',
					headers: { 'Content-Type': 'application/json' },
					body: JSON.stringify({ runs: list })
				})
			: [];
		if (gen !== generation) return;
		stored = new Map(
			rows.map((m) => [m.run + sep + m.key, { min: m.min_step, max: m.max_step }])
		);
		loaded = true;
	}
	$effect(() => {
		void idsKey;
		void project;
		loadMetrics().catch((err) => console.warn('losscape: loading metrics failed', err));
	});
	// New metrics of running runs show up without a reload.
	$effect(() => {
		if (!runningKey) return;
		const timer = setInterval(() => void loadMetrics().catch(() => {}), 15_000);
		return () => clearInterval(timer);
	});
	$effect(() => onRestored(() => void loadMetrics().catch(() => {})));

	// Live points: folded into cached tiles, and extend the step ranges.
	let live = $state<Map<string, Extent>>(new Map());
	$effect(() => {
		if (!runningKey) return;
		const followed = runningKey.split(',');
		return subscribeLive(project, followed, {
			onPoints: (p) => {
				// eslint-disable-next-line svelte/prefer-svelte-reactivity -- copied, filled, then assigned once
				const next = new Map(live);
				for (let i = 0; i < p.step.length; i++) {
					const key = p.keys[p.key_idx[i]];
					const step = p.step[i];
					tileStore.addLivePoint(p.run, key, step, p.value[i] ?? NaN, p.time_ms[i]);
					const id = p.run + sep + key;
					const e = next.get(id);
					next.set(id, {
						min: Math.min(e?.min ?? step, step),
						max: Math.max(e?.max ?? step, step)
					});
				}
				live = next;
			},
			onState: (run) => onRunState?.(run),
			onReset: () => {
				for (const run of followed) tileStore.invalidate(run);
				void loadMetrics().catch(() => {});
			}
		});
	});

	function extentOf(run: string, key: string): Extent | undefined {
		const id = run + sep + key;
		const [a, b] = [stored.get(id), live.get(id)];
		if (!a || !b) return a ?? b;
		return { min: Math.min(a.min, b.min), max: Math.max(a.max, b.max) };
	}

	/** Metrics as shown: renamed, described, computed (the project's Metrics tab). */
	const registry = $derived(currentRegistry());
	const source = $derived.by(() => {
		// eslint-disable-next-line svelte/prefer-svelte-reactivity -- built once per change
		const rawByRun = new Map<string, Set<string>>();
		for (const id of [...stored.keys(), ...live.keys()]) {
			const at = id.indexOf(sep);
			const [run, key] = [id.slice(0, at), id.slice(at + 1)];
			if (!rawByRun.has(run)) rawByRun.set(run, new Set());
			rawByRun.get(run)!.add(key);
		}
		const runs = new Map(
			[...rawByRun].map(([run, raw]) => [run, registry.forRun(raw, renames?.(run))])
		);
		return new MetricSource(registry, runs, extentOf);
	});

	const keys = $derived(
		only
			? source.keys(true).filter((key) => key === only)
			: source
					.keys()
					.filter((key) =>
						prefix ? key.startsWith(prefix) : !key.startsWith(SYSTEM_PREFIX)
					)
					// Flat lines, on every run: charted only if asked for.
					.filter((key) => !isModelMetric(key) || registry.describe(key).important)
					.filter((key) => !importantOnly || registry.describe(key).important)
	);

	/** Families (`val/pck-{k}`) with at least two members here: charted against `k`. */
	const families = $derived(
		prefix || !summaryOf
			? []
			: registry.families
					.map((def) => ({
						def,
						// Hidden metrics are still members of their family.
						members: source.keys(true).filter((k) => match(def.name, k))
					}))
					.filter((f) => f.members.length >= 2)
	);
	const ranked = $derived(keys.some((k) => registry.describe(k).direction));

	/** The filter as a case-insensitive regular expression, or plain text if it isn't one. */
	const metricMatch = $derived.by(() => {
		const q = metricFilter.trim();
		if (!q) return () => true;
		try {
			const re = new RegExp(q, 'i');
			return (key: string) => re.test(key);
		} catch {
			return (key: string) => key.toLowerCase().includes(q.toLowerCase());
		}
	});

	let collapsed = $state<Record<string, boolean>>({});

	const shownKeys = $derived(
		keys.filter((key) => metricMatch(prefix ? key.slice(prefix.length) : key))
	);
	/** Metrics marked important: charted first, in a section of their own. */
	const important = $derived(shownKeys.filter((key) => registry.describe(key).important));
	/** The other charts, grouped by the metric name's prefix: `train/loss` goes to "train". */
	const sections = $derived.by(() => {
		const groups = groupBy(shownKeys, (key) => {
			if (registry.describe(key).important) return null;
			const name = prefix ? key.slice(prefix.length) : key;
			return name.includes('/') ? name.slice(0, name.indexOf('/')) : '';
		});
		return groups.sort(([a], [b]) => (a === '' ? -1 : b === '' ? 1 : a.localeCompare(b)));
	});

	const xOptions = $derived<XAxis[]>([
		'step',
		'time',
		'relative',
		...keys.map((k): XAxis => `metric:${k}`)
	]);

	function setOverride(key: string, override: ChartOverride | null) {
		const next = { ...overrides };
		if (override) next[key] = override;
		else delete next[key];
		overrides = next;
	}

	function setX(x: string) {
		settings = { ...settings, x: x as XAxis };
		view = null;
	}
</script>

{#if only}
	{#if keys.length}
		<ChartGroup brush={false} domain={false}>
			<MetricChart
				{highlight}
				{series}
				metric={only}
				title={only}
				{source}
				{settings}
				override={overrides[only]}
				onOverride={(o) => setOverride(only, o)}
				{xOptions}
				bind:view
			/>
		</ChartGroup>
	{:else if loaded}
		<p class="text-sm text-muted-foreground">These runs have no points of it yet.</p>
	{/if}
{:else if series.length === 0}
	<EmptyCard icon={ChartLine} title="No runs charted">
		{#snippet description()}
			Pick the runs to chart with the dots next to their names in the table above.
		{/snippet}
	</EmptyCard>
{:else if loaded && keys.length === 0 && empty}
	<EmptyCard icon={empty.icon} title={empty.title}>
		{#snippet description()}
			{#each empty.text.split('`') as part, i (i)}{#if i % 2}<code class="font-mono text-xs"
						>{part}</code
					>{:else}{part}{/if}{/each}
		{/snippet}
	</EmptyCard>
{:else if loaded && keys.length === 0}
	<EmptyCard icon={ChartLine} title="No metrics yet">
		{#snippet description()}
			Metrics logged with <code class="font-mono text-xs">run.log()</code> show here as
			charts, grouped by their prefix (<code class="font-mono text-xs">train/</code>,
			<code class="font-mono text-xs">val/</code>).
		{/snippet}
		<CodeBlock lang="python" title="train.py" code={logExample} />
	</EmptyCard>
{:else}
	<div class="mb-4 flex flex-wrap items-center gap-2">
		<div class="relative w-full sm:w-64">
			<Search
				class="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
			/>
			<Input
				class="pl-8"
				placeholder="Metrics (regex)"
				aria-label="Filter metrics"
				bind:value={metricFilter}
			/>
		</div>
		<Select.Root type="single" value={settings.x} onValueChange={setX}>
			<Select.Trigger size="sm" class="w-44" aria-label="X axis">
				<span class="truncate"
					><span class="text-muted-foreground">X</span> {xLabel(settings.x)}</span
				>
			</Select.Trigger>
			<Select.Content>
				{#each xOptions as x (x)}
					<Select.Item value={x} label={xLabel(x)} />
				{/each}
			</Select.Content>
		</Select.Root>
		<label class="flex h-8 items-center gap-2 rounded-md border px-2.5 text-sm">
			<span class="text-muted-foreground">Smoothing</span>
			<input
				type="range"
				min="0"
				max="0.99"
				step="0.01"
				class="w-24 accent-primary"
				value={settings.smoothing}
				oninput={(e) =>
					(settings = { ...settings, smoothing: Number(e.currentTarget.value) })}
			/>
			<span class="w-8 text-right font-mono text-xs tabular-nums"
				>{settings.smoothing.toFixed(2)}</span
			>
		</label>
		<Toggle
			size="sm"
			variant="outline"
			pressed={settings.logY}
			onPressedChange={(logY) => (settings = { ...settings, logY })}
		>
			Log y
		</Toggle>
		<Toggle
			size="sm"
			variant="outline"
			pressed={settings.band}
			onPressedChange={(band) => (settings = { ...settings, band })}
			title="Min/max of downsampled points"
		>
			Range band
		</Toggle>
		{#if ranked}
			<Toggle
				size="sm"
				variant="outline"
				pressed={settings.best}
				onPressedChange={(best) => (settings = { ...settings, best })}
				title="Best value so far, for metrics where higher or lower is better"
			>
				Best so far
			</Toggle>
		{/if}
		{#if grouped}
			<Select.Root
				type="single"
				value={settings.spread}
				onValueChange={(v) =>
					(settings = { ...settings, spread: v as ChartSettings['spread'] })}
			>
				<Select.Trigger size="sm" class="w-40" aria-label="Group band">
					<span class="text-muted-foreground">Groups</span>
					{settings.spread === 'std' ? 'mean ± std' : 'min – max'}
				</Select.Trigger>
				<Select.Content>
					<Select.Item value="std" label="mean ± std" />
					<Select.Item value="range" label="min – max" />
				</Select.Content>
			</Select.Root>
		{/if}
		{#if view}
			<Button variant="ghost" size="sm" onclick={() => (view = null)}>
				<RotateCcw /> Reset zoom
			</Button>
		{/if}
	</div>

	{#if loaded && shownKeys.length === 0}
		<p class="text-sm text-muted-foreground">No metric matches.</p>
	{/if}

	{#snippet chartSection(id: string, label: string, sectionKeys: string[])}
		{@const open = !collapsed[`section:${id}`]}
		<section>
			<button
				class="mb-3 flex items-center gap-1.5 text-sm font-medium text-muted-foreground hover:text-foreground"
				onclick={() => (collapsed[`section:${id}`] = open)}
				aria-expanded={open}
			>
				<ChevronRight class={['size-4 transition-transform', open && 'rotate-90']} />
				{#if id === '\u0000important'}<Star class="size-3.5 fill-current" />{/if}
				{label}
				<span class="text-xs tabular-nums">{sectionKeys.length}</span>
			</button>
			{#if open}
				<div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
					{#each sectionKeys as key (key)}
						<MetricChart
							{highlight}
							{series}
							metric={key}
							title={prefix ? key.slice(prefix.length) : key}
							href={metricHref(project, key)}
							{source}
							{onRename}
							{settings}
							override={overrides[key]}
							onOverride={(o) => setOverride(key, o)}
							{xOptions}
							bind:view
						/>
					{/each}
				</div>
			{/if}
		</section>
	{/snippet}

	<!-- Hovering one chart marks the same x on all of them; zoom is shared through `view`. -->
	<ChartGroup brush={false} domain={false}>
		<div class="grid gap-6">
			{#if important.length}
				{@render chartSection('\u0000important', 'Important', important)}
			{/if}
			{#if families.length && summaryOf}
				{@const open = !collapsed['\u0000families']}
				<section>
					<button
						class="mb-3 flex items-center gap-1.5 text-sm font-medium text-muted-foreground hover:text-foreground"
						onclick={() => (collapsed['\u0000families'] = open)}
						aria-expanded={open}
					>
						<ChevronRight
							class={['size-4 transition-transform', open && 'rotate-90']}
						/>
						Families
						<span class="text-xs tabular-nums">{families.length}</span>
					</button>
					{#if open}
						<div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
							{#each families as f (f.def.name)}
								<FamilyChart def={f.def} members={f.members} {series} {summaryOf} />
							{/each}
						</div>
					{/if}
				</section>
			{/if}
			{#each sections as [section, sectionKeys] (section)}
				{@render chartSection(section, section || 'Charts', sectionKeys)}
			{/each}
		</div>
	</ChartGroup>
{/if}
