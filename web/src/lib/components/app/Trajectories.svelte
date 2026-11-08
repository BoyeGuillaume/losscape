<script lang="ts">
	import { untrack } from 'svelte';

	import { apiJson } from '#lib/api/client.js';
	import type { RunMetricInfo, RunSummary } from '#lib/api/types.js';
	import { phasePath } from '#lib/charts/phase.js';
	import { MetricSource, type Extent } from '#lib/charts/source.js';
	import { tileStore } from '#lib/charts/tiles.svelte.js';
	import { currentRegistry } from '#lib/metrics/store.svelte.js';
	import type { Dot } from '#lib/runs/board.js';
	import DotChart from './DotChart.svelte';

	/** Points asked for per metric of a run: enough for a path, without noise at every step. */
	const WIDTH = 240;
	const SEP = '\u0000';

	let {
		project,
		runs,
		colors,
		xKey,
		yKeys,
		highlight = null,
		onHover,
		onOpen,
		metricHref
	}: {
		project: string;
		/** The runs drawn, each with a color. */
		runs: RunSummary[];
		colors: ReadonlyMap<string, string>;
		/** The metric on the x-axis. */
		xKey: string;
		/** One chart per metric on the y-axis. */
		yKeys: string[];
		highlight?: string | null;
		onHover?: (run: string | null) => void;
		onOpen?: (run: string) => void;
		metricHref: (name: string) => string;
	} = $props();

	const registry = $derived(currentRegistry());

	// Which steps each run has each metric for; again as runs go on.
	let extents = $state.raw<Map<string, Extent>>(new Map());
	let loaded = $state(false);
	const progressKey = $derived(runs.map((r) => `${r.id}:${r.last_step}`).join(','));
	let generation = 0;
	$effect(() => {
		void progressKey;
		const gen = ++generation;
		const ids = untrack(() => runs.map((r) => r.id));
		apiJson<RunMetricInfo[]>(`/projects/${encodeURIComponent(project)}/metrics`, {
			method: 'POST',
			headers: { 'Content-Type': 'application/json' },
			body: JSON.stringify({ runs: ids })
		}).then(
			(rows) => {
				if (gen !== generation) return;
				extents = new Map(
					rows.map((m) => [m.run + SEP + m.key, { min: m.min_step, max: m.max_step }])
				);
				loaded = true;
			},
			(err) => console.warn('losscape: loading metrics failed', err)
		);
	});

	const source = $derived.by(() => {
		// eslint-disable-next-line svelte/prefer-svelte-reactivity -- built once per change
		const rawByRun = new Map<string, Set<string>>();
		for (const id of extents.keys()) {
			const at = id.indexOf(SEP);
			const [run, key] = [id.slice(0, at), id.slice(at + 1)];
			if (!rawByRun.has(run)) rawByRun.set(run, new Set());
			rawByRun.get(run)!.add(key);
		}
		const byId = new Map(runs.map((r) => [r.id, r]));
		return new MetricSource(
			registry,
			new Map([...rawByRun].map(([run, raw]) => [run, registry.forRun(raw, byId.get(run))])),
			(run, key) => extents.get(run + SEP + key)
		);
	});

	const plots = $derived.by(() => {
		void tileStore.version; // recompute when tiles load or live points arrive
		return yKeys.map((key) => {
			let complete = true;
			const paths: { id: string; points: Dot[] }[] = [];
			for (const run of runs) {
				const [ex, ey] = [source.extentOf(run.id, xKey), source.extentOf(run.id, key)];
				if (!ex || !ey) continue;
				const from = Math.min(ex.min, ey.min);
				const to = Math.max(ex.max, ey.max, from + 1);
				// The same steps and width for both, so their buckets line up.
				const xs = source.points(run.id, xKey, from, to, WIDTH);
				const ys = source.points(run.id, key, from, to, WIDTH);
				if (!xs || !ys) continue;
				complete &&= xs.complete && ys.complete;
				const points = phasePath(xs, ys).map((p): Dot => ({
					id: run.id,
					name: run.name,
					x: p.x,
					y: p.y,
					step: p.step,
					running: run.state === 'running'
				}));
				if (points.length) paths.push({ id: run.id, points });
			}
			return { key, paths, complete };
		});
	});
	const xName = $derived(registry.describe(xKey).label || xKey);
</script>

<div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
	{#each plots as p (p.key)}
		<DotChart
			metric={p.key}
			href={metricHref(p.key)}
			dots={p.paths.flatMap((path) => path.points)}
			paths={p.paths}
			{colors}
			x={{ kind: 'metric', key: xKey }}
			{xName}
			{highlight}
			{onHover}
			{onOpen}
			note="step by step"
			emptyText={!loaded || !p.complete
				? 'Loading…'
				: 'No charted run has both of these yet.'}
		/>
	{/each}
</div>
