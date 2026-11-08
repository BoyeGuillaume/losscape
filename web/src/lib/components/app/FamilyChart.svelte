<script lang="ts">
	import { Chart, Points, Spline } from 'layerchart/canvas';
	import { Tooltip } from 'layerchart';
	import { mode } from 'mode-watcher';

	import { distinctTicks, isFlat, type ChartSeries } from '#lib/charts/settings.js';
	import { chartSize } from '#lib/charts/size.svelte.js';
	import { formatMetric, metricAxis } from '#lib/metric-units.js';
	import { compile, match } from '#lib/metrics/pattern.js';
	import type { MetricDef } from '#lib/metrics/registry.js';
	import { currentRegistry } from '#lib/metrics/store.svelte.js';

	type Row = { x: number; y: number; series: number };

	let {
		def,
		members,
		series,
		summaryOf
	}: {
		/** A family: a pattern with one parameter, e.g. `val/pck-{k}`. */
		def: MetricDef;
		/** Its metrics present here. */
		members: string[];
		series: ChartSeries[];
		summaryOf: (run: string) => Record<string, unknown> | undefined;
	} = $props();

	const param = $derived(compile(def.name).params[0]);
	/** Members by parameter value: numbers in order, or names in order of appearance. */
	const points = $derived.by(() => {
		let raw = members.map((key) => ({ key, value: match(def.name, key)![param] }));
		// Numbers on a numeric axis (others, e.g. `8-fine`, left out); else names in order.
		const isNumber = (v: string) => v.trim() !== '' && Number.isFinite(Number(v));
		const numeric = raw.some((p) => isNumber(p.value));
		if (numeric) raw = raw.filter((p) => isNumber(p.value));
		const sorted = numeric ? [...raw].sort((a, b) => Number(a.value) - Number(b.value)) : raw;
		return sorted.map((p, i) => ({ ...p, x: numeric ? Number(p.value) : i }));
	});

	/** One line per series: the summary of each member (mean over a group's runs). */
	const lines = $derived(
		series.map((s, index) => {
			const rows: Row[] = [];
			for (const p of points) {
				const values = s.runs
					.map((r) => summaryOf(r.id)?.[p.key])
					.filter((v): v is number => typeof v === 'number' && Number.isFinite(v));
				if (values.length) {
					rows.push({
						x: p.x,
						y: values.reduce((a, b) => a + b, 0) / values.length,
						series: index
					});
				}
			}
			return { series: s, rows };
		})
	);
	const all = $derived(lines.flatMap((l) => l.rows).sort((a, b) => a.x - b.x));
	const meta = $derived(currentRegistry().describe(members[0]));
	const yDomain = $derived.by((): [number, number] | undefined => {
		if (!all.length) return undefined;
		let [lo, hi] = [Math.min(...all.map((r) => r.y)), Math.max(...all.map((r) => r.y))];
		if (meta.bounds) [lo, hi] = [meta.bounds[0] ?? lo, meta.bounds[1] ?? hi];
		return isFlat(lo, hi) ? [lo - 1, hi + 1] : [lo, hi];
	});
	const xDomain = $derived.by((): [number, number] => {
		const xs = points.map((p) => p.x);
		const [lo, hi] = [Math.min(...xs), Math.max(...xs)];
		return lo === hi ? [lo - 1, hi + 1] : [lo, hi];
	});
	const labelOf = (x: number) => points.find((p) => p.x === x)?.value ?? '';

	// Theme colors as literal values (see MetricChart).
	let themeTick = $state(0);
	$effect(() => {
		void mode.current;
		const frame = requestAnimationFrame(() => themeTick++);
		return () => cancelAnimationFrame(frame);
	});
	const resolveColor = $derived.by(() => {
		void themeTick;
		const style = getComputedStyle(document.documentElement);
		return (value: string) => {
			const name = /^var\((--[\w-]+)\)$/.exec(value.trim())?.[1];
			return name ? style.getPropertyValue(name).trim() : value;
		};
	});
	const colors = $derived(series.map((s) => resolveColor(s.color)));
	const grid = $derived(resolveColor('var(--chart-grid)'));
	const label = $derived(resolveColor('var(--muted-foreground)'));
	let hovered = $state(false);
</script>

<section
	class="flex min-w-0 flex-col rounded-xl border bg-card p-3 pb-1 text-card-foreground shadow-xs"
	aria-label={`${def.name} family chart`}
	onpointerenter={() => (hovered = true)}
	onpointerleave={() => (hovered = false)}
>
	<header class="flex min-h-7 items-baseline gap-2">
		<h3 class="truncate font-mono text-sm font-medium" title={def.spec.description ?? def.name}>
			{def.spec.label || def.name}
		</h3>
		<span class="ml-auto shrink-0 text-xs text-muted-foreground">
			{meta.summary === 'best' ? 'best' : 'last'} value by {param}
		</span>
	</header>
	<div class={['w-full', chartSize.height]}>
		{#if all.length}
			<Chart
				data={all}
				x="x"
				y="y"
				{xDomain}
				{yDomain}
				yNice
				padding={{ top: 6, right: 10, bottom: 20, left: 44 }}
				axis
				grid={{ x: false, y: true, stroke: grid, yTicks: distinctTicks() }}
				tooltipContext={{ mode: 'bisect-x' }}
				props={{
					xAxis: {
						ticks: points.map((p) => p.x),
						format: (x: number) => labelOf(x),
						tickLabelProps: { fill: label }
					},
					yAxis: {
						format: metricAxis(members[0]),
						ticks: 4,
						tickLabelProps: { fill: label }
					}
				}}
			>
				{#snippet marks()}
					{#each lines as line, i (line.series.id)}
						<Spline
							data={line.rows}
							x="x"
							y="y"
							stroke={colors[i]}
							strokeWidth={1.5}
							fill="none"
						/>
						<Points data={line.rows} x="x" y="y" r={2.5} fill={colors[i]} />
					{/each}
				{/snippet}
				{#snippet tooltip()}
					{#if hovered}
						<Tooltip.Root
							variant="none"
							class="max-w-80 rounded-lg border bg-popover px-2.5 py-1.5 text-xs text-popover-foreground shadow-md"
						>
							{#snippet children({ data })}
								{@const at = lines
									.map((l, i) => ({
										l,
										i,
										row: l.rows.find((r) => r.x === data.x)
									}))
									.filter((v) => v.row)
									.sort((a, b) => b.row!.y - a.row!.y)}
								<Tooltip.Header value={`${param} = ${labelOf(data.x)}`} />
								<Tooltip.List>
									{#each at as v (v.l.series.id)}
										<Tooltip.Item
											label={v.l.series.name}
											value={formatMetric(members[0], v.row!.y)}
											color={colors[v.i]}
											classes={{ label: 'truncate' }}
										/>
									{/each}
								</Tooltip.List>
							{/snippet}
						</Tooltip.Root>
					{/if}
				{/snippet}
			</Chart>
		{:else}
			<p class="pt-16 text-center text-sm text-muted-foreground">No summary values yet.</p>
		{/if}
	</div>
</section>
