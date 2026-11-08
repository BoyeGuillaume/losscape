<script lang="ts">
	import { scaleLog, scaleTime } from 'd3-scale';
	import { Tooltip } from 'layerchart';
	import { Chart, Points, Spline } from 'layerchart/canvas';
	import { mode } from 'mode-watcher';
	import { page } from '$app/state';

	import { logTicks } from '#lib/charts/settings.js';
	import { formatMetric, metricAxis } from '#lib/metric-units.js';
	import { currentRegistry } from '#lib/metrics/store.svelte.js';
	import { nav } from '#lib/nav.svelte.js';
	import type { Dot } from '#lib/runs/board.js';

	const MARKER = 4;
	const CATEGORY_LABEL = 14;
	const MINUTE_MS = 60_000;
	const DAY_MS = 86_400_000;
	const X_MARGIN = 0.03;
	const Y_MARGIN = 0.08;

	let {
		metric,
		href,
		dots,
		colors,
		line = [],
		paths,
		x,
		xName,
		baseline = null,
		highlight = null,
		onHover,
		onOpen,
		note,
		emptyText = 'No run has both of these yet.'
	}: {
		metric: string;
		/** Its page. */
		href: string;
		dots: Dot[];
		/** Colors of the runs that have one (charted below); other dots are neutral. */
		colors: ReadonlyMap<string, string>;
		/** The best value so far. */
		line?: { x: number; y: number }[];
		/** Trajectories: `dots` are then all their points, for hovering, and aren't drawn. */
		paths?: { id: string; points: Dot[] }[];
		/** What x is: when the run started, a number, or the index of a category. */
		x:
			| { kind: 'time' }
			| { kind: 'number'; log: boolean }
			| { kind: 'category'; names: string[] }
			| { kind: 'metric'; key: string; log?: boolean };
		xName: string;
		baseline?: number | null;
		highlight?: string | null;
		onHover?: (run: string | null) => void;
		onOpen?: (run: string) => void;
		/** Beside the title. */
		note?: string;
		emptyText?: string;
	} = $props();

	const meta = $derived(currentRegistry().describe(metric));

	const isLog = $derived(x.kind !== 'time' && x.kind !== 'category' && !!x.log);
	const xs = $derived(dots.map((d) => d.x));
	const xDomain = $derived.by((): [number, number] => {
		if (x.kind === 'category') return [-0.5, x.names.length - 0.5];
		if (!xs.length) return [0, 1];
		const [lo, hi] = [Math.min(...xs), Math.max(...xs)];
		if (isLog) {
			const margin = (hi / lo) ** X_MARGIN;
			return lo === hi ? [lo / 2, hi * 2] : [lo / margin, hi * margin];
		}
		if (lo === hi)
			return x.kind === 'time' ? [lo - MINUTE_MS, hi + MINUTE_MS] : [lo - 1, hi + 1];
		// So that dots at the ends stay clear of the axis labels.
		const margin = (hi - lo) * X_MARGIN;
		return [lo - margin, hi + margin];
	});
	// Fitted to the values, not to the metric's range: results are often close together
	// (86-90% of a 0-100% range), and telling them apart is what these charts are for.
	const yDomain = $derived.by((): [number, number] => {
		const ys = dots.map((d) => d.y);
		if (baseline !== null) ys.push(baseline);
		if (!ys.length) return [0, 1];
		const [lo, hi] = [Math.min(...ys), Math.max(...ys)];
		// A single value: around it, at a scale that suits it.
		if (lo === hi)
			return lo === 0 ? [-1, 1] : [lo - Math.abs(lo) * 0.1, hi + Math.abs(hi) * 0.1];
		const margin = (hi - lo) * Y_MARGIN;
		return [lo - margin, hi + margin];
	});
	const xScale = $derived(x.kind === 'time' ? scaleTime() : isLog ? scaleLog() : undefined);

	const formatTime = $derived.by(() => {
		const span = xDomain[1] - xDomain[0];
		const f = new Intl.DateTimeFormat(
			undefined,
			span < 10 * MINUTE_MS
				? { hour: '2-digit', minute: '2-digit', second: '2-digit', hourCycle: 'h23' }
				: span < 2 * DAY_MS
					? { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }
					: { month: 'short', day: 'numeric' }
		);
		return (v: number | Date) => f.format(v);
	});
	const trim = (name: string) =>
		name.length > CATEGORY_LABEL ? `${name.slice(0, CATEGORY_LABEL - 1)}…` : name;
	const formatX = $derived.by(() => {
		if (x.kind === 'time') return formatTime;
		if (x.kind === 'category') {
			const { names } = x;
			return (v: number) => trim(names[Math.round(v)] ?? '');
		}
		if (x.kind === 'metric') return metricAxis(x.key);
		return (v: number) => String(Number(Number(v).toPrecision(3)));
	});
	/** A dot's x for reading. */
	const readX = (d: Dot) => {
		if (x.kind === 'time') return new Date(d.x).toLocaleString();
		if (x.kind === 'metric') return formatMetric(x.key, d.x);
		return x.kind === 'category' ? x.names[Math.round(d.x)] : String(d.x);
	};
	const xTicks = $derived.by(() => {
		if (x.kind === 'category') return x.names.map((_, i) => i);
		if (isLog) return logTicks(xDomain[0], xDomain[1]);
		return x.kind === 'time' ? 4 : 5;
	});

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
	const grid = $derived(resolveColor('var(--chart-grid)'));
	const label = $derived(resolveColor('var(--muted-foreground)'));
	const ink = $derived(resolveColor('var(--foreground)'));
	const surface = $derived(resolveColor('var(--card)'));

	// Neutral dots first, colored ones on top; hollow while running.
	const neutral = $derived(dots.filter((d) => !colors.has(d.id)));
	const colored = $derived(dots.filter((d) => colors.has(d.id)));
	const picked = $derived(highlight && !paths ? dots.find((d) => d.id === highlight) : undefined);
	const referenceLine = $derived(
		baseline === null
			? null
			: [
					{ x: xDomain[0], y: baseline },
					{ x: xDomain[1], y: baseline }
				]
	);

	let hovered = $state(false);
	let overDot = $state(false);

	function rememberWayBack() {
		nav.back = { href: page.url.pathname + page.url.search, label: 'Overview' };
	}
</script>

<section
	class="flex min-w-0 flex-col rounded-xl border bg-card p-3 pb-1 text-card-foreground shadow-xs"
	aria-label={`${meta.label || metric} by ${xName}`}
	style:cursor={overDot ? 'pointer' : undefined}
	onpointerenter={() => (hovered = true)}
	onpointerleave={() => (hovered = false)}
>
	<header class="flex min-h-7 items-baseline gap-2">
		<h3 class="truncate font-mono text-sm font-medium" title={meta.description ?? metric}>
			<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- resolved by the page -->
			<a class="hover:underline" {href} onclick={rememberWayBack}>{meta.label || metric}</a>
		</h3>
		{#if note}<span class="ml-auto shrink-0 text-xs text-muted-foreground">{note}</span>{/if}
	</header>
	<div class="h-52 w-full">
		{#if dots.length}
			<Chart
				data={dots}
				x="x"
				y="y"
				{xScale}
				{xDomain}
				{yDomain}
				yNice
				clip
				padding={{ top: 8, right: 16, bottom: 20, left: 52 }}
				axis
				grid={{ x: false, y: true, stroke: grid }}
				tooltipContext={{
					mode: 'quadtree',
					radius: 24,
					// Hover finds any mark's points: not a run's? Not a place, so lines are left out.
					x: (d: Partial<Dot>) => (d.id ? d.x : NaN),
					y: (d: Partial<Dot>) => (d.id ? d.y : NaN),
					onclick: (_: MouseEvent, { data }: { data?: Dot }) =>
						data?.id && onOpen?.(data.id)
				}}
				props={{
					xAxis: { format: formatX, ticks: xTicks, tickLabelProps: { fill: label } },
					yAxis: { format: metricAxis(metric), ticks: 4, tickLabelProps: { fill: label } }
				}}
			>
				{#snippet marks()}
					{#if referenceLine}
						<Spline
							data={referenceLine}
							x="x"
							y="y"
							stroke={ink}
							strokeWidth={1}
							opacity={0.55}
							class="[stroke-dasharray:4_3]"
							fill="none"
						/>
					{/if}
					{#if line.length}
						<Spline data={line} x="x" y="y" stroke={ink} strokeWidth={2} fill="none" />
					{/if}
					{#if paths}
						{#each paths as path (path.id)}
							{@const color = resolveColor(
								colors.get(path.id) ?? 'var(--foreground)'
							)}
							{@const faded = highlight && highlight !== path.id}
							<Spline
								data={path.points}
								x="x"
								y="y"
								stroke={color}
								strokeWidth={highlight === path.id ? 2.5 : 1.5}
								opacity={faded ? 0.15 : 1}
								fill="none"
							/>
							<!-- Where it starts (hollow) and where it has got to (filled). -->
							<Points
								data={[path.points[0]]}
								x="x"
								y="y"
								r={MARKER - 1}
								fill={surface}
								stroke={color}
								strokeWidth={1.5}
								opacity={faded ? 0.15 : 1}
							/>
							<Points
								data={[path.points[path.points.length - 1]]}
								x="x"
								y="y"
								r={MARKER}
								fill={color}
								stroke={surface}
								strokeWidth={1}
								opacity={faded ? 0.15 : 1}
							/>
						{/each}
					{:else}
						{#each [false, true] as running (running)}
							{@const rows = neutral.filter((d) => d.running === running)}
							{#if rows.length}
								<Points
									data={rows}
									x="x"
									y="y"
									r={MARKER - 0.5}
									fill={running ? surface : label}
									stroke={label}
									strokeWidth={running ? 1.5 : 0}
									opacity={running ? 1 : 0.55}
								/>
							{/if}
						{/each}
						{#each colored as d (d.id)}
							{@const color = resolveColor(colors.get(d.id)!)}
							<Points
								data={[d]}
								x="x"
								y="y"
								r={MARKER}
								fill={d.running ? surface : color}
								stroke={d.running ? color : surface}
								strokeWidth={d.running ? 2 : 1}
							/>
						{/each}
					{/if}
					{#if picked}
						<Points
							data={[picked]}
							x="x"
							y="y"
							r={MARKER + 2.5}
							fill={colors.has(picked.id)
								? resolveColor(colors.get(picked.id)!)
								: label}
							stroke={ink}
							strokeWidth={2}
						/>
					{/if}
				{/snippet}
				{#snippet tooltip()}
					{#if hovered}
						<Tooltip.Root
							variant="none"
							class="max-w-80 rounded-lg border bg-popover px-2.5 py-1.5 text-xs text-popover-foreground shadow-md"
						>
							{#snippet children({ data })}
								<!-- Hovering a dot picks out the run everywhere on the page. -->
								<div
									{@attach () => {
										const id = (data as Dot).id;
										overDot = true;
										onHover?.(id);
										return () => {
											overDot = false;
											onHover?.(null);
										};
									}}
								>
									<Tooltip.Header value={(data as Dot).name} />
									<Tooltip.List>
										<Tooltip.Item
											label={meta.label || metric}
											value={formatMetric(metric, (data as Dot).y)}
										/>
										<Tooltip.Item label={xName} value={readX(data as Dot)} />
										{#if (data as Dot).step !== undefined}
											<Tooltip.Item
												label="Step"
												value={Math.round(
													(data as Dot).step!
												).toLocaleString('en')}
											/>
										{/if}
										{#if (data as Dot).running}
											<Tooltip.Item label="State" value="Running" />
										{/if}
									</Tooltip.List>
								</div>
							{/snippet}
						</Tooltip.Root>
					{/if}
				{/snippet}
			</Chart>
		{:else}
			<p class="pt-16 text-center text-sm text-muted-foreground">{emptyText}</p>
		{/if}
	</div>
</section>
