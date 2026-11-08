<script lang="ts">
	import { Area, Chart, Points, Spline } from 'layerchart/canvas';
	import { Tooltip, type BrushContext } from 'layerchart';
	import { scaleLog } from 'd3-scale';
	import type { ComponentProps } from 'svelte';
	import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
	import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal';
	import { mode } from 'mode-watcher';
	import { page } from '$app/state';

	import { Button } from '#lib/components/ui/button/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import {
		aggregate,
		durationTicks,
		logTicks,
		smooth,
		SMOOTHING_PRESETS,
		xFormatter,
		xLabel,
		xMetric,
		xTitle,
		type ChartOverride,
		type ChartRun,
		type ChartSeries,
		type ChartSettings,
		type XAxis
	} from '#lib/charts/settings.js';
	import type { MetricSource } from '#lib/charts/source.js';
	import { isolatedPoints } from '#lib/charts/isolated.js';
	import { levelFor, tileStore, type SeriesPoints } from '#lib/charts/tiles.svelte.js';
	import { formatMetric, metricAxis, unitFor, unitTicks } from '#lib/metric-units.js';
	import { nav } from '#lib/nav.svelte.js';

	/** BrushContext's event payload (not exported by name). */
	type BrushEvent = Parameters<NonNullable<ComponentProps<typeof BrushContext>['onBrushEnd']>>[0];
	/**
	 * One bucket of one series: `y` is smoothed (or the mean), `raw` the unsmoothed value;
	 * `min`/`max` bound the band (the bucket's range for a run, the spread for a group).
	 */
	type Row = {
		x: number;
		y: number | null;
		raw: number | null;
		min: number | null;
		max: number | null;
		/** Runs behind the point (groups). */
		n: number;
		/** Best value up to here (metrics where higher or lower is better). */
		best: number | null;
	};
	type Line = { series: ChartSeries; rows: Row[]; ascending: boolean };

	const MAX_TOOLTIP_ROWS = 12;

	let {
		highlight = null,
		series,
		metric,
		source,
		settings: pageSettings,
		override = {},
		onOverride,
		xOptions = [],
		view: sharedView = $bindable(null),
		title = metric,
		href,
		onRename
	}: {
		/** The series to pick out, fading the others. */
		highlight?: string | null;
		series: ChartSeries[];
		metric: string;
		/** Shown name (defaults to the metric). */
		title?: string;
		/** Where the title links to: the metric's page. */
		href?: string;
		/** Reads metrics as shown (renamed, merged, computed). */
		source: MetricSource;
		/** Offers "Rename in this run" (the run page). */
		onRename?: (key: string) => void;
		/** The page's settings; `override` changes some of them for this chart. */
		settings: ChartSettings;
		override?: ChartOverride;
		/** Without it, the chart has no settings menu. */
		onOverride?: (override: ChartOverride | null) => void;
		xOptions?: XAxis[];
		/** Zoomed x range, shared by the charts of a page; null shows everything. */
		view?: [number, number] | null;
	} = $props();

	const settings = $derived<ChartSettings>({ ...pageSettings, ...override });

	/** The metric's page links back here: this run, or the run list. */
	const tabLabel = () => {
		const id = page.route.id ?? '';
		if (id.endsWith('/overview')) return 'Overview';
		return id.endsWith('/compare') ? 'Compare' : `Runs of ${page.params.project ?? ''}`;
	};
	function rememberWayBack() {
		nav.back = {
			href: page.url.pathname + page.url.search,
			label: nav.runName ?? tabLabel()
		};
	}
	const meta = $derived(source.registry.describe(metric));
	const computed = $derived(source.registry.isComputed(metric));
	/** The logged metric shown here, for a single run that renamed it. */
	const fromKey = $derived(
		series.length === 1 && series[0].runs.length === 1
			? source.sourceOf(series[0].runs[0].id, metric)
			: null
	);
	const overridden = $derived(Object.keys(override).length > 0);
	/** A chart with its own x-axis zooms on its own: the shared range is in other units. */
	const ownAxis = $derived(override.x !== undefined && override.x !== pageSettings.x);
	let localView = $state<[number, number] | null>(null);
	$effect(() => {
		void settings.x;
		localView = null;
	});
	const view = $derived(ownAxis ? localView : sharedView);
	function setView(next: [number, number] | null) {
		if (ownAxis) localView = next;
		else sharedView = next;
	}

	let card: HTMLElement;
	let visible = $state(false);
	/** The tooltip shows on the hovered chart only; the others just mark the synced x. */
	let hovered = $state(false);
	let width = $state(0);

	// Only charts on (or near) the screen fetch tiles and draw.
	$effect(() => {
		const io = new IntersectionObserver(([e]) => (visible = e.isIntersecting), {
			rootMargin: '300px'
		});
		io.observe(card);
		return () => io.disconnect();
	});

	/** x of each bucket for the current x-axis; null where unknown. */
	function xValues(run: ChartRun, s: SeriesPoints, from: number, to: number): (number | null)[] {
		const key = xMetric(settings.x);
		if (key) {
			// Same steps and width, so the same buckets: pair them by position.
			const other = source.points(run.id, key, from, to, width);
			if (!other) return s.x.map(() => null);
			const byStep = new Map(other.x.map((x, i) => [x, other.mean[i]]));
			return s.x.map((x) => byStep.get(x) ?? null);
		}
		if (settings.x === 'time') return s.time;
		if (settings.x === 'relative') return s.time.map((t) => (t - run.startMs) / 1000);
		return s.x;
	}

	/** Steps to load: the zoomed range in steps, found from a run's coarse data. */
	function stepWindow(run: ChartRun, full: [number, number]): [number, number] | null {
		if (!view) return full;
		const [lo, hi] = view;
		if (settings.x === 'step') {
			const from = Math.max(full[0], Math.floor(lo));
			const to = Math.min(full[1], Math.ceil(hi));
			return to >= from ? [from, to] : null;
		}
		const base = source.points(run.id, metric, full[0], full[1], width);
		if (!base) return null;
		const xs = xValues(run, base, full[0], full[1]);
		let first = Infinity;
		let last = -Infinity;
		xs.forEach((x, i) => {
			if (x !== null && x >= lo && x <= hi) {
				first = Math.min(first, base.x[i]);
				last = Math.max(last, base.x[i]);
			}
		});
		if (first > last) return null;
		const pad = 2 ** levelFor(full[1] - full[0] + 1, width);
		return [
			Math.max(full[0], Math.floor(first - pad)),
			Math.min(full[1], Math.ceil(last + pad))
		];
	}

	const positive = (v: number | null) => (settings.logY && v !== null && v <= 0 ? null : v);

	const drawn = $derived.by(() => {
		void tileStore.version; // recompute when tiles load or live points arrive
		if (!visible || width === 0) return null;
		const lines: Line[] = [];
		let complete = true;
		for (const s of series) {
			const runs = s.runs.filter((r) => source.extentOf(r.id, metric));
			if (!runs.length) continue;
			// One step range for all runs of a group: same buckets, so they line up.
			let [lo, hi] = [Infinity, -Infinity];
			for (const r of runs) {
				const e = source.extentOf(r.id, metric)!;
				[lo, hi] = [Math.min(lo, e.min), Math.max(hi, e.max)];
			}
			const window = stepWindow(runs[0], [lo, Math.max(hi, lo + 1)]);
			if (!window) continue;

			// Per step bucket: each run's x, smoothed and raw value, and bucket range.
			// eslint-disable-next-line svelte/prefer-svelte-reactivity -- local to this computation
			const buckets = new Map<
				number,
				{ x: number[]; y: number[]; raw: number[]; lo: number; hi: number }
			>();
			for (const r of runs) {
				const p = source.points(r.id, metric, window[0], window[1], width);
				if (!p) continue;
				complete &&= p.complete;
				const xs = xValues(r, p, window[0], window[1]);
				const ys = smooth(p.mean, p.n, settings.smoothing);
				for (let i = 0; i < xs.length; i++) {
					const x = xs[i];
					const y = ys[i];
					if (x === null || !Number.isFinite(x) || y === null || p.mean[i] === null)
						continue;
					let b = buckets.get(p.x[i]);
					if (!b)
						buckets.set(
							p.x[i],
							(b = { x: [], y: [], raw: [], lo: Infinity, hi: -Infinity })
						);
					b.x.push(x);
					b.y.push(y);
					b.raw.push(p.mean[i]!);
					b.lo = Math.min(b.lo, p.min[i] ?? Infinity);
					b.hi = Math.max(b.hi, p.max[i] ?? -Infinity);
				}
			}
			const rows: Row[] = [];
			let ascending = true;
			for (const step of [...buckets.keys()].sort((a, b) => a - b)) {
				const b = buckets.get(step)!;
				const x = b.x.reduce((a, v) => a + v, 0) / b.x.length;
				if (rows.length && x < rows[rows.length - 1].x) ascending = false;
				if (runs.length === 1) {
					rows.push({
						x,
						y: positive(b.y[0]),
						raw: positive(b.raw[0]),
						min: positive(b.lo),
						max: positive(b.hi),
						n: 1,
						best: null
					});
				} else {
					const { mean, lo: low, hi: high } = aggregate(b.y, settings.spread);
					const raw = b.raw.reduce((a, v) => a + v, 0) / b.raw.length;
					rows.push({
						x,
						y: positive(mean),
						raw: positive(raw),
						min: positive(low),
						max: positive(high),
						n: b.y.length,
						best: null
					});
				}
			}
			// Best so far, from the unsmoothed values, in step order.
			if (meta.direction) {
				let best: number | null = null;
				for (const r of rows) {
					if (r.raw !== null) {
						best =
							best === null
								? r.raw
								: meta.direction === 'max'
									? Math.max(best, r.raw)
									: Math.min(best, r.raw);
					}
					r.best = best;
				}
			}
			if (rows.length) lines.push({ series: s, rows, ascending });
		}
		return { lines, complete };
	});

	const lines = $derived(drawn?.lines ?? []);

	const xDomain = $derived.by((): [number, number] => {
		if (view) return view;
		let lo = Infinity;
		let hi = -Infinity;
		if (settings.x === 'step') {
			for (const s of series) {
				for (const r of s.runs) {
					const e = source.extentOf(r.id, metric);
					if (e) [lo, hi] = [Math.min(lo, e.min), Math.max(hi, e.max)];
				}
			}
		} else {
			for (const line of lines) {
				for (const r of line.rows) [lo, hi] = [Math.min(lo, r.x), Math.max(hi, r.x)];
			}
		}
		if (!Number.isFinite(lo)) return [0, 1];
		return hi > lo ? [lo, hi] : [lo - 0.5, lo + 0.5];
	});

	/** y range of what is visible, band and unsmoothed lines included when drawn. */
	const yDomain = $derived.by((): [number, number] | undefined => {
		let lo = Infinity;
		let hi = -Infinity;
		const take = (v: number | null) => {
			if (v === null || !Number.isFinite(v)) return;
			lo = Math.min(lo, v);
			hi = Math.max(hi, v);
		};
		for (const line of lines) {
			for (const r of line.rows) {
				if (r.x < xDomain[0] || r.x > xDomain[1]) continue;
				take(r.y);
				if (settings.smoothing > 0) take(r.raw);
				if (settings.band) {
					take(r.min);
					take(r.max);
				}
			}
		}
		if (!Number.isFinite(lo)) return undefined;
		// Bounded metrics (0-1, 0-100%) keep their full range, unless on a log scale.
		if (meta.bounds && !settings.logY) {
			[lo, hi] = [meta.bounds[0] ?? lo, meta.bounds[1] ?? hi];
		}
		if (lo === hi) {
			return settings.logY
				? [lo / 2, hi * 2]
				: [lo - Math.abs(lo || 1) * 0.05, hi + Math.abs(hi || 1) * 0.05];
		}
		return [lo, hi];
	});

	/** All rows by x, for the tooltip's nearest-x lookup. */
	const merged = $derived(lines.flatMap((l) => l.rows).sort((a, b) => a.x - b.x));

	/** Each series' value at x (nearest bucket, only within its range), highest first. */
	function valuesAt(x: number) {
		const out: { series: ChartSeries; color: string; value: number; n: number }[] = [];
		for (const line of lines) {
			const rows = line.rows;
			let best: Row | undefined;
			if (line.ascending) {
				if (x < rows[0].x || x > rows[rows.length - 1].x) continue;
				let lo = 0;
				let hi = rows.length - 1;
				while (lo < hi) {
					const mid = (lo + hi) >> 1;
					if (rows[mid].x < x) lo = mid + 1;
					else hi = mid;
				}
				best = lo > 0 && x - rows[lo - 1].x < rows[lo].x - x ? rows[lo - 1] : rows[lo];
			} else {
				for (const r of rows)
					if (!best || Math.abs(r.x - x) < Math.abs(best.x - x)) best = r;
			}
			if (best?.y != null) {
				const color = colors[line.series.id];
				out.push({ series: line.series, color, value: best.y, n: best.n });
			}
		}
		return out.sort((a, b) => b.value - a.value);
	}

	const single = $derived(
		series.length === 1 && series[0].runs.length === 1 ? lines[0] : undefined
	);
	const latest = $derived.by(() => {
		const rows = single?.rows ?? [];
		for (let i = rows.length - 1; i >= 0; i--) if (rows[i].y !== null) return rows[i];
		return null;
	});

	const definedY = (d: Row) => d.y !== null;
	const definedRaw = (d: Row) => d.raw !== null;
	const definedBand = (d: Row) => d.min !== null && d.max !== null;

	/**
	 * Theme colors as literal values. LayerChart memoizes the colors it resolves from CSS
	 * variables for canvas, so after a theme toggle they would go stale; literal colors
	 * need no resolution. Re-read on every mode change, a frame after the class flips.
	 */
	let themeTick = $state(0);
	$effect(() => {
		void mode.current;
		const frame = requestAnimationFrame(() => themeTick++);
		return () => cancelAnimationFrame(frame);
	});
	const resolve = $derived.by(() => {
		void themeTick;
		const style = getComputedStyle(document.documentElement);
		return (value: string) => {
			const name = /^var\((--[\w-]+)\)$/.exec(value.trim())?.[1];
			return name ? style.getPropertyValue(name).trim() : value;
		};
	});
	const grid = $derived(resolve('var(--chart-grid)'));
	const label = $derived(resolve('var(--muted-foreground)'));
	const colors = $derived(Object.fromEntries(series.map((s) => [s.id, resolve(s.color)])));

	const formatX = $derived(xFormatter(settings.x, xDomain[0], xDomain[1]));
	const xTicks = $derived(
		settings.x === 'relative' || settings.x === 'time'
			? durationTicks(xDomain[0], xDomain[1], settings.x === 'time' ? 1000 : 1)
			: 5
	);
	const unit = $derived(unitFor(metric));
	const formatY = $derived(metricAxis(metric));
	const yTicks = $derived(
		settings.logY
			? (scale: { domain: () => number[] }) =>
					logTicks(...(scale.domain() as [number, number]))
			: unit
				? (scale: { domain: () => number[] }) =>
						unitTicks(unit, ...(scale.domain() as [number, number]))
				: 4
	);
	/** Room for the longest y label (units make them long: "28.5 MiB"). */
	const padLeft = $derived.by(() => {
		if (!yDomain) return 40;
		const [lo, hi] = yDomain;
		const longest = Math.max(...[lo, hi, (lo + hi) / 2].map((v) => formatY(v).length));
		return Math.min(96, Math.max(32, 10 + longest * 6.5));
	});
	const logScale = scaleLog();

	function onBrushEnd(e: BrushEvent) {
		// Read the selection before reset() clears it.
		const active = e.brush.active;
		const [lo, hi] = e.brush.x;
		e.brush.reset();
		if (!active || typeof lo !== 'number' || typeof hi !== 'number') return;
		// Double-click selects the whole visible range: treat that as "zoom out".
		const full = view === null && lo <= xDomain[0] && hi >= xDomain[1];
		const tiny =
			settings.x === 'step' ? hi - lo < 1 : hi - lo <= (xDomain[1] - xDomain[0]) * 1e-6;
		setView(full || tiny ? null : [lo, hi]);
	}

	/** Sets one setting for this chart; the page's value again drops the override. */
	function change<K extends keyof ChartOverride>(key: K, value: ChartOverride[K]) {
		const next: ChartOverride = { ...override, [key]: value };
		if (pageSettings[key] === value) delete next[key];
		onOverride?.(Object.keys(next).length ? next : null);
	}
</script>

<section
	bind:this={card}
	class="group/chart flex min-w-0 flex-col rounded-xl border bg-card p-3 pb-1 text-card-foreground shadow-xs"
	onpointerenter={() => (hovered = true)}
	onpointerleave={() => (hovered = false)}
	aria-label={`${metric} chart`}
>
	<header class="flex min-h-7 items-center gap-2">
		<h3
			class="flex min-w-0 items-center gap-1 truncate text-sm font-medium"
			title={[metric, meta.description].filter(Boolean).join(' — ')}
		>
			{#if computed}<span
					class="shrink-0 font-serif text-xs text-primary italic"
					title="Computed metric (Metrics tab)">ƒ</span
				>{/if}
			{#if href}
				<!-- eslint-disable svelte/no-navigation-without-resolve -- resolved by the caller -->
				<a
					class="truncate hover:underline"
					{href}
					title="Settings of this metric"
					onclick={rememberWayBack}>{meta.label ?? title}</a
				>
				<!-- eslint-enable svelte/no-navigation-without-resolve -->
			{:else}
				<span class="truncate">{meta.label ?? title}</span>
			{/if}
		</h3>
		{#if fromKey}
			<span class="shrink-0 truncate text-xs text-muted-foreground" title="Read from">
				← {fromKey}
			</span>
		{/if}
		{#if drawn && !drawn.complete}
			<span class="size-1.5 animate-pulse rounded-full bg-muted-foreground" title="Loading"
			></span>
		{/if}
		<span class="ml-auto font-mono text-xs text-muted-foreground tabular-nums">
			{#if latest}{formatMetric(metric, latest.y)}{/if}
		</span>
		{#if view}
			<Button
				variant="ghost"
				size="icon-xs"
				onclick={() => setView(null)}
				title="Reset zoom"
				aria-label="Reset zoom"
			>
				<RotateCcw />
			</Button>
		{/if}
		{#if onOverride}
			<DropdownMenu.Root>
				<DropdownMenu.Trigger
					class={[
						'relative rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:opacity-100 data-[state=open]:opacity-100',
						!overridden && 'opacity-0 group-hover/chart:opacity-100'
					]}
					aria-label={`${title} chart settings`}
					title={overridden
						? 'Chart settings (changed for this chart)'
						: 'Chart settings'}
				>
					<SlidersHorizontal class="size-3.5" />
					{#if overridden}
						<span class="absolute top-0.5 right-0.5 size-1.5 rounded-full bg-primary"
						></span>
					{/if}
				</DropdownMenu.Trigger>
				<DropdownMenu.Content align="end" class="w-56">
					<DropdownMenu.Label>This chart</DropdownMenu.Label>
					<DropdownMenu.Sub>
						<DropdownMenu.SubTrigger>
							X axis <span class="ml-auto truncate pl-2 text-muted-foreground"
								>{xLabel(settings.x)}</span
							>
						</DropdownMenu.SubTrigger>
						<DropdownMenu.SubContent class="max-h-72 overflow-y-auto">
							<DropdownMenu.RadioGroup
								value={settings.x}
								onValueChange={(x) => change('x', x as XAxis)}
							>
								{#each xOptions as x (x)}
									<DropdownMenu.RadioItem value={x}
										>{xLabel(x)}</DropdownMenu.RadioItem
									>
								{/each}
							</DropdownMenu.RadioGroup>
						</DropdownMenu.SubContent>
					</DropdownMenu.Sub>
					<DropdownMenu.Sub>
						<DropdownMenu.SubTrigger>
							Smoothing <span
								class="ml-auto pl-2 font-mono text-xs text-muted-foreground"
								>{settings.smoothing.toFixed(2)}</span
							>
						</DropdownMenu.SubTrigger>
						<DropdownMenu.SubContent>
							<DropdownMenu.RadioGroup
								value={String(settings.smoothing)}
								onValueChange={(v) => change('smoothing', Number(v))}
							>
								{#each SMOOTHING_PRESETS as w (w)}
									<DropdownMenu.RadioItem value={String(w)}>
										{w === 0 ? 'Off' : w.toFixed(2)}
									</DropdownMenu.RadioItem>
								{/each}
							</DropdownMenu.RadioGroup>
						</DropdownMenu.SubContent>
					</DropdownMenu.Sub>
					<DropdownMenu.CheckboxItem
						checked={settings.logY}
						closeOnSelect={false}
						onCheckedChange={(on) => change('logY', on)}
					>
						Log y
					</DropdownMenu.CheckboxItem>
					<DropdownMenu.CheckboxItem
						checked={settings.band}
						closeOnSelect={false}
						onCheckedChange={(on) => change('band', on)}
					>
						Range band
					</DropdownMenu.CheckboxItem>
					<DropdownMenu.Separator />
					<DropdownMenu.Item disabled={!overridden} onSelect={() => onOverride?.(null)}>
						Use the page's settings
					</DropdownMenu.Item>
					{#if onRename && !computed}
						<DropdownMenu.Separator />
						<DropdownMenu.Item onSelect={() => onRename(metric)}>
							Rename in this run…
						</DropdownMenu.Item>
					{/if}
				</DropdownMenu.Content>
			</DropdownMenu.Root>
		{/if}
	</header>

	<div
		class="h-48 w-full"
		bind:clientWidth={width}
		ondblclick={() => setView(null)}
		role="presentation"
	>
		{#if drawn}
			<Chart
				data={merged}
				x="x"
				y="y"
				{xDomain}
				{yDomain}
				yScale={settings.logY ? logScale : undefined}
				yNice
				clip
				padding={{ top: 6, right: 6, bottom: 20, left: padLeft }}
				axis
				grid={{ x: false, y: true, stroke: grid, yTicks }}
				highlight={{ points: false, lines: true }}
				tooltipContext={{ mode: 'bisect-x' }}
				brush={{ axis: 'x', onBrushEnd }}
				props={{
					xAxis: { format: formatX, ticks: xTicks, tickLabelProps: { fill: label } },
					yAxis: {
						format: formatY,
						ticks: yTicks,
						tickLabelProps: { fill: label }
					}
				}}
			>
				{#snippet belowMarks()}
					{#if settings.band}
						{#each lines as line (line.series.id)}
							<Area
								data={line.rows}
								x="x"
								y0="min"
								y1="max"
								defined={definedBand}
								fill={colors[line.series.id]}
								opacity={line.series.runs.length > 1
									? 0.2
									: lines.length > 1
										? 0.1
										: 0.16}
								line={false}
							/>
						{/each}
					{/if}
					{#if settings.smoothing > 0}
						{#each lines as line (line.series.id)}
							<Spline
								data={line.rows}
								x="x"
								y="raw"
								defined={definedRaw}
								stroke={colors[line.series.id]}
								strokeWidth={1}
								opacity={0.25}
								fill="none"
							/>
						{/each}
					{/if}
				{/snippet}
				{#snippet marks()}
					{#if settings.best && meta.direction}
						{#each lines as line (line.series.id)}
							<Spline
								data={line.rows}
								x="x"
								y="best"
								defined={(d: Row) => d.best !== null}
								stroke={colors[line.series.id]}
								strokeWidth={1.25}
								class="[stroke-dasharray:4_3]"
								opacity={0.8}
								fill="none"
							/>
						{/each}
					{/if}
					{#each lines as line (line.series.id)}
						<Spline
							data={line.rows}
							x="x"
							y="y"
							defined={definedY}
							stroke={colors[line.series.id]}
							strokeWidth={highlight === line.series.id
								? 2.5
								: line.series.runs.length > 1
									? 2
									: 1.5}
							opacity={highlight && highlight !== line.series.id ? 0.15 : 1}
							fill="none"
						/>
						<Points
							data={isolatedPoints(line.rows, definedY)}
							x="x"
							y="y"
							r={3}
							fill={colors[line.series.id]}
							opacity={highlight && highlight !== line.series.id ? 0.15 : 1}
						/>
					{/each}
				{/snippet}
				{#snippet tooltip()}
					{#if hovered}
						<Tooltip.Root
							variant="none"
							class="max-w-80 rounded-lg border bg-popover px-2.5 py-1.5 text-xs text-popover-foreground shadow-md"
						>
							{#snippet children({ data })}
								{@const values = valuesAt(data.x)}
								<Tooltip.Header value={xTitle(settings.x, data.x)} />
								<Tooltip.List>
									{#each values.slice(0, MAX_TOOLTIP_ROWS) as v (v.series.id)}
										<Tooltip.Item
											label={series.length > 1 || v.series.runs.length > 1
												? v.series.runs.length > 1
													? `${v.series.name} (mean of ${v.n})`
													: v.series.name
												: title}
											value={formatMetric(metric, v.value)}
											color={v.color}
											classes={{ label: 'truncate' }}
										/>
									{/each}
								</Tooltip.List>
								{#if values.length > MAX_TOOLTIP_ROWS}
									<p class="mt-1 text-muted-foreground">
										+{values.length - MAX_TOOLTIP_ROWS} more
									</p>
								{/if}
							{/snippet}
						</Tooltip.Root>
					{/if}
				{/snippet}
			</Chart>
		{/if}
	</div>
</section>
