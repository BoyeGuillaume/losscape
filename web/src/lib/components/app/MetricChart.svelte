<script lang="ts">
	import { Area, Chart, Spline } from 'layerchart/canvas';
	import { Tooltip, type BrushContext } from 'layerchart';
	import type { ComponentProps } from 'svelte';
	import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
	import { mode } from 'mode-watcher';

	import { Button } from '#lib/components/ui/button/index.js';
	import { seriesInRange, tileStore } from '#lib/charts/tiles.svelte.js';
	import { formatCompact, formatValue } from '#lib/format.js';

	/** BrushContext's event payload (not exported by name). */
	type BrushEvent = Parameters<NonNullable<ComponentProps<typeof BrushContext>['onBrushEnd']>>[0];
	type Row = { x: number; mean: number | null; min: number | null; max: number | null };

	let {
		run,
		metric,
		minStep,
		maxStep,
		color = 'var(--color-chart-1)'
	}: { run: string; metric: string; minStep: number; maxStep: number; color?: string } = $props();

	let card: HTMLElement;
	let visible = $state(false);
	/** The tooltip shows on the hovered chart only; the others just mark the synced step. */
	let hovered = $state(false);
	let width = $state(0);
	/** Zoomed step range; null follows the whole run (and live data). */
	let view = $state<[number, number] | null>(null);

	// Only charts on (or near) the screen fetch tiles and draw.
	$effect(() => {
		const io = new IntersectionObserver(([e]) => (visible = e.isIntersecting), {
			rootMargin: '300px'
		});
		io.observe(card);
		return () => io.disconnect();
	});

	const from = $derived(view ? view[0] : minStep);
	const to = $derived(view ? view[1] : Math.max(maxStep, minStep + 1));

	const series = $derived.by(() => {
		void tileStore.version; // recompute when tiles load or live points arrive
		if (!visible || width === 0) return null;
		return seriesInRange(run, metric, Math.max(0, Math.floor(from)), Math.ceil(to), width);
	});

	const rows = $derived.by((): Row[] => {
		if (!series) return [];
		return series.x.map((x, i) => ({
			x,
			mean: series.mean[i],
			min: series.min[i],
			max: series.max[i]
		}));
	});

	/** y range of what is visible, band included. */
	const yDomain = $derived.by((): [number, number] | undefined => {
		let lo = Infinity;
		let hi = -Infinity;
		for (const r of rows) {
			if (r.x < from || r.x > to || r.min === null || r.max === null) continue;
			lo = Math.min(lo, r.min);
			hi = Math.max(hi, r.max);
		}
		if (!Number.isFinite(lo)) return undefined;
		if (lo === hi) return [lo - Math.abs(lo || 1) * 0.05, hi + Math.abs(hi || 1) * 0.05];
		return [lo, hi];
	});

	const latest = $derived.by(() => {
		for (let i = rows.length - 1; i >= 0; i--) if (rows[i].mean !== null) return rows[i];
		return null;
	});

	const defined = (d: Row) => d.mean !== null;

	/**
	 * Theme colors as literal values. LayerChart memoizes the colors it resolves from CSS
	 * variables for canvas, so after a theme toggle they would go stale; literal colors
	 * need no resolution. Re-read on every mode change, a frame after the class flips.
	 */
	let palette = $state(readPalette());
	$effect(() => {
		void mode.current;
		const frame = requestAnimationFrame(() => (palette = readPalette()));
		return () => cancelAnimationFrame(frame);
	});

	function readPalette() {
		const style = getComputedStyle(document.documentElement);
		const token = (value: string) => {
			const name = /^var\((--[\w-]+)\)$/.exec(value.trim())?.[1];
			return name ? style.getPropertyValue(name).trim() : value;
		};
		return {
			line: token(color),
			grid: token('var(--chart-grid)'),
			label: token('var(--muted-foreground)')
		};
	}

	function onBrushEnd(e: BrushEvent) {
		// Read the selection before reset() clears it.
		const active = e.brush.active;
		const [lo, hi] = e.brush.x;
		e.brush.reset();
		if (!active || typeof lo !== 'number' || typeof hi !== 'number') return;
		// Double-click selects the whole visible range: treat that as "zoom out".
		const full = view === null && lo <= from && hi >= to;
		view = full || hi - lo < 1 ? null : [lo, hi];
	}
</script>

<section
	bind:this={card}
	class="flex min-w-0 flex-col rounded-xl border bg-card p-3 pb-1 text-card-foreground shadow-xs"
	onpointerenter={() => (hovered = true)}
	onpointerleave={() => (hovered = false)}
	aria-label={`${metric} chart`}
>
	<header class="flex min-h-7 items-center gap-2">
		<h3 class="truncate text-sm font-medium" title={metric}>{metric}</h3>
		{#if series && !series.complete}
			<span class="size-1.5 animate-pulse rounded-full bg-muted-foreground" title="Loading"
			></span>
		{/if}
		<span class="ml-auto font-mono text-xs text-muted-foreground tabular-nums">
			{#if latest}{formatValue(latest.mean)}{/if}
		</span>
		{#if view}
			<Button
				variant="ghost"
				size="icon-xs"
				onclick={() => (view = null)}
				title="Reset zoom"
				aria-label="Reset zoom"
			>
				<RotateCcw />
			</Button>
		{/if}
	</header>

	<div
		class="h-48 w-full"
		bind:clientWidth={width}
		ondblclick={() => (view = null)}
		role="presentation"
	>
		{#if series}
			<Chart
				data={rows}
				x="x"
				y="mean"
				xDomain={[from, to]}
				{yDomain}
				yNice
				clip
				padding={{ top: 6, right: 6, bottom: 20, left: 40 }}
				axis
				grid={{ x: false, y: true, stroke: palette.grid }}
				highlight={{ points: true, lines: true }}
				tooltipContext={{ mode: 'bisect-x' }}
				brush={{ axis: 'x', onBrushEnd }}
				props={{
					xAxis: {
						format: formatCompact,
						ticks: 5,
						tickLabelProps: { fill: palette.label }
					},
					yAxis: {
						format: formatCompact,
						ticks: 4,
						tickLabelProps: { fill: palette.label }
					}
				}}
			>
				{#snippet belowMarks()}
					<Area
						y0="min"
						y1="max"
						{defined}
						fill={palette.line}
						opacity={0.16}
						line={false}
					/>
				{/snippet}
				{#snippet marks()}
					<Spline
						y="mean"
						{defined}
						stroke={palette.line}
						strokeWidth={1.5}
						fill="none"
					/>
				{/snippet}
				{#snippet tooltip()}
					{#if hovered}
						<Tooltip.Root
							variant="none"
							class="rounded-lg border bg-popover px-2.5 py-1.5 text-xs text-popover-foreground shadow-md"
						>
							{#snippet children({ data })}
								<Tooltip.Header
									value={`step ${Math.round(data.x).toLocaleString('en')}`}
								/>
								<Tooltip.List>
									<Tooltip.Item
										label={metric}
										value={formatValue(data.mean)}
										color={palette.line}
									/>
									{#if data.min !== data.max}
										<Tooltip.Item
											label="range"
											value={`${formatValue(data.min)} – ${formatValue(data.max)}`}
										/>
									{/if}
								</Tooltip.List>
							{/snippet}
						</Tooltip.Root>
					{/if}
				{/snippet}
			</Chart>
		{/if}
	</div>
</section>
